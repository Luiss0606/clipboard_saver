use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

/// Release tag baked in by CI through the `APP_RELEASE_TAG` env var.
/// `None` on local/dev builds, which disables the updater entirely — a dev
/// build must never replace itself with a release.
pub const RELEASE_TAG: Option<&str> = option_env!("APP_RELEASE_TAG");

const RELEASES_URL: &str =
    "https://api.github.com/repos/Luiss0606/clipboard_saver/releases/latest";
const USER_AGENT: &str = "clipboard-saver-updater";
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Cap for the .app.zip download (ureq defaults to 10MB otherwise).
const MAX_DOWNLOAD_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// A downloaded release, ready to install.
pub struct Update {
    pub tag: String,
    pub zip_path: PathBuf,
}

/// Spawns the background update loop: checks GitHub Releases shortly after
/// startup and then every 6 hours. When a new release has been downloaded,
/// `notify` is invoked (from the worker thread) and the loop stops — the
/// pending update is applied when the user clicks the menu item.
pub fn spawn<F>(notify: F)
where
    F: Fn(Update) + Send + 'static,
{
    let Some(current_tag) = RELEASE_TAG else {
        return; // dev build
    };
    thread::spawn(move || {
        thread::sleep(FIRST_CHECK_DELAY);
        loop {
            match check_and_download(current_tag) {
                Ok(Some(update)) => {
                    notify(update);
                    return;
                }
                Ok(None) => {}
                Err(e) => eprintln!("clipboard_saver: update check failed: {e}"),
            }
            thread::sleep(CHECK_INTERVAL);
        }
    });
}

fn check_and_download(current_tag: &str) -> Result<Option<Update>, String> {
    let mut response = ureq::get(RELEASES_URL)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let release: Release = response
        .body_mut()
        .read_json()
        .map_err(|e| e.to_string())?;

    if release.tag_name == current_tag {
        return Ok(None);
    }
    let Some(asset) = pick_asset(&release.assets) else {
        return Err(format!(
            "release {} has no .app.zip asset",
            release.tag_name
        ));
    };

    let dir = std::env::temp_dir().join(format!("clipboard_saver_update_{}", release.tag_name));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip_path = dir.join(&asset.name);

    let mut response = ureq::get(&asset.browser_download_url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| e.to_string())?;
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD_BYTES)
        .reader();
    let mut file = fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;

    Ok(Some(Update {
        tag: release.tag_name,
        zip_path,
    }))
}

fn pick_asset(assets: &[Asset]) -> Option<&Asset> {
    assets.iter().find(|a| a.name.ends_with(".app.zip"))
}

/// Replaces the installed .app with the downloaded one and relaunches.
/// On success this never returns (the process exits).
pub fn install_and_relaunch(update: &Update) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // …/Clipboard Saver.app/Contents/MacOS/clipboard_saver → up 3 levels.
    let app_path = exe
        .ancestors()
        .nth(3)
        .ok_or("unexpected bundle layout")?
        .to_path_buf();
    if app_path.extension() != Some(OsStr::new("app")) {
        return Err("not running from an .app bundle; cannot self-update".into());
    }

    let staging = update
        .zip_path
        .parent()
        .ok_or("zip has no parent directory")?
        .join("extracted");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    run("ditto", &[
        OsStr::new("-x"),
        OsStr::new("-k"),
        update.zip_path.as_os_str(),
        staging.as_os_str(),
    ])?;

    let new_app = fs::read_dir(&staging)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .find(|p| p.extension() == Some(OsStr::new("app")))
        .ok_or("downloaded zip does not contain an .app")?;

    // Swap with rollback: old app aside, new app in, then drop the old one.
    let file_name = app_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("invalid app path")?;
    let backup = app_path.with_file_name(format!("{file_name}.old"));
    let _ = fs::remove_dir_all(&backup);
    run("mv", &[app_path.as_os_str(), backup.as_os_str()])?;
    if let Err(e) = run("mv", &[new_app.as_os_str(), app_path.as_os_str()]) {
        let _ = run("mv", &[backup.as_os_str(), app_path.as_os_str()]);
        return Err(e);
    }
    let _ = fs::remove_dir_all(&backup);

    Command::new("open")
        .arg("-n")
        .arg(&app_path)
        .spawn()
        .map_err(|e| e.to_string())?;
    std::process::exit(0);
}

/// Runs a command, mapping a non-zero exit into an error with stderr.
fn run(cmd: &str, args: &[&OsStr]) -> Result<(), String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{cmd} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_asset_finds_app_zip() {
        let assets = vec![
            Asset {
                name: "ClipboardSaver.dmg".into(),
                browser_download_url: "u1".into(),
            },
            Asset {
                name: "ClipboardSaver.app.zip".into(),
                browser_download_url: "u2".into(),
            },
        ];
        assert_eq!(pick_asset(&assets).unwrap().name, "ClipboardSaver.app.zip");
        assert!(pick_asset(&assets[..1]).is_none());
    }

    #[test]
    fn release_json_deserializes() {
        let json = r#"{
            "tag_name": "v0.1.7",
            "name": "Clipboard Saver v0.1.7",
            "assets": [
                {"name": "ClipboardSaver.app.zip",
                 "browser_download_url": "https://example.com/a.zip",
                 "size": 123}
            ]
        }"#;
        let release: Release = serde_json::from_str(json).unwrap();
        assert_eq!(release.tag_name, "v0.1.7");
        assert_eq!(release.assets.len(), 1);
    }
}
