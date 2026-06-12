#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod history;
mod item;
mod panel;
mod storage;
mod updater;
mod watcher;

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::utils::config::WindowEffectsConfig;
use tauri::utils::{WindowEffect, WindowEffectState};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_positioner::{Position, WindowExt};

use history::History;
use item::{fnv1a, ClipboardItem, ItemKind};
use panel::{ItemDto, StateDto};
use storage::Storage;
use updater::Update;
use watcher::{Captured, Watcher};

const POLL_INTERVAL: Duration = Duration::from_millis(400);
/// Longest side of panel thumbnails, in pixels.
const THUMB_MAX: u32 = 320;
const PANEL_WIDTH: f64 = 380.0;
const PANEL_HEIGHT: f64 = 540.0;
const SHORTCUT: &str = "cmd+shift+v";

enum ClipboardMsg {
    SetText(String),
    SetImage {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
}

/// Shared app data. The clipboard itself is NOT here: NSPasteboard handles
/// are not Send, so a dedicated thread owns the [`Watcher`] and everything
/// talks to it through `clipboard_tx`.
struct Core {
    history: History,
    storage: Storage,
    /// id → PNG data URL, pre-encoded so panel refreshes are cheap.
    thumbs: HashMap<u64, String>,
    pending_update: Option<Update>,
}

struct AppState {
    core: Mutex<Core>,
    clipboard_tx: Sender<ClipboardMsg>,
}

impl Core {
    fn load() -> Self {
        let storage = Storage::new(Storage::default_dir()).expect("cannot create data directory");
        let history = History::from_items(storage.load());
        let mut thumbs = HashMap::new();
        for item in history.items() {
            if let Some(file) = item.png_file() {
                if let Some((w, h, rgba)) = storage.load_image(file) {
                    if let Some(url) = thumb_data_url(w, h, rgba) {
                        thumbs.insert(item.id, url);
                    }
                }
            }
        }
        Self {
            history,
            storage,
            thumbs,
            pending_update: None,
        }
    }

    fn persist(&self) {
        if let Err(e) = self.storage.save(&self.history.to_vec()) {
            eprintln!("clipboard_saver: cannot save history: {e}");
        }
    }

    fn snapshot(&self) -> StateDto {
        let now = panel::unix_now();
        let items: Vec<ItemDto> = self
            .history
            .items()
            .map(|item| panel::item_dto(item, self.thumbs.get(&item.id).cloned(), now))
            .collect();
        StateDto {
            items,
            autostart: autostart::is_enabled(),
            version: updater::RELEASE_TAG.unwrap_or("dev").to_string(),
            pending_update: self.pending_update.as_ref().map(|u| u.tag.clone()),
            max_items: history::MAX_ITEMS,
        }
    }

    fn ingest(&mut self, captured: Captured) {
        match captured {
            Captured::Text(text) => {
                if let Some(id) = self.history.find_text(&text) {
                    self.history.promote(id);
                } else {
                    let (_, evicted) = self.history.insert_front(ItemKind::Text(text));
                    self.remove_evicted(&evicted);
                }
            }
            Captured::Image {
                width,
                height,
                rgba,
            } => {
                let hash = fnv1a(&rgba);
                if let Some(id) = self.history.find_image(hash) {
                    self.history.promote(id);
                } else {
                    match self.storage.save_image(hash, width, height, &rgba) {
                        Ok(png) => {
                            let (id, evicted) = self.history.insert_front(ItemKind::Image {
                                width,
                                height,
                                png,
                                hash,
                            });
                            if let Some(url) = thumb_data_url(width as u32, height as u32, rgba) {
                                self.thumbs.insert(id, url);
                            }
                            self.remove_evicted(&evicted);
                        }
                        Err(e) => {
                            eprintln!("clipboard_saver: cannot save image: {e}");
                            return;
                        }
                    }
                }
            }
        }
        self.persist();
    }

    fn remove_evicted(&mut self, evicted: &[ClipboardItem]) {
        for old in evicted {
            self.thumbs.remove(&old.id);
        }
        self.storage.delete_images(evicted);
    }
}

#[tauri::command]
fn get_state(state: tauri::State<AppState>) -> StateDto {
    state.core.lock().unwrap().snapshot()
}

#[tauri::command]
fn restore_item(id: u64, state: tauri::State<AppState>, window: tauri::WebviewWindow) {
    let core = state.core.lock().unwrap();
    if let Some(item) = core.history.get(id) {
        let msg = match &item.kind {
            ItemKind::Text(text) => Some(ClipboardMsg::SetText(text.clone())),
            ItemKind::Image { png, .. } => {
                core.storage
                    .load_image(png)
                    .map(|(w, h, rgba)| ClipboardMsg::SetImage {
                        width: w as usize,
                        height: h as usize,
                        rgba,
                    })
            }
        };
        if let Some(msg) = msg {
            let _ = state.clipboard_tx.send(msg);
        }
    }
    drop(core);
    let _ = window.hide();
}

#[tauri::command]
fn clear_history(app: AppHandle, state: tauri::State<AppState>) {
    let mut core = state.core.lock().unwrap();
    let removed = core.history.clear();
    core.storage.delete_images(&removed);
    core.thumbs.clear();
    core.persist();
    drop(core);
    let _ = app.emit("state-changed", ());
}

#[tauri::command]
fn toggle_autostart(app: AppHandle) {
    let result = if autostart::is_enabled() {
        autostart::disable()
    } else {
        autostart::enable()
    };
    if let Err(e) = result {
        eprintln!("clipboard_saver: cannot toggle autostart: {e}");
    }
    let _ = app.emit("state-changed", ());
}

#[tauri::command]
fn install_update(state: tauri::State<AppState>) -> Result<(), String> {
    let update = state
        .core
        .lock()
        .unwrap()
        .pending_update
        .clone()
        .ok_or("no pending update")?;
    // On success this exits the process and relaunches.
    updater::install_and_relaunch(&update)
}

#[tauri::command]
fn hide_panel(window: tauri::WebviewWindow) {
    let _ = window.hide();
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

fn toggle_panel(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        // Tray-relative when the tray position is known; fallback otherwise
        // (e.g. the panel was summoned by hotkey before any tray event).
        if win.move_window(Position::TrayBottomCenter).is_err() {
            let _ = win.move_window(Position::TopRight);
        }
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Owns the Watcher (not Send): polls for changes and serves write requests.
fn clipboard_thread(app: AppHandle, rx: Receiver<ClipboardMsg>) {
    let mut watcher = match Watcher::new() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("clipboard_saver: cannot access clipboard: {e}");
            return;
        }
    };
    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(ClipboardMsg::SetText(text)) => watcher.set_text(&text),
            Ok(ClipboardMsg::SetImage {
                width,
                height,
                rgba,
            }) => watcher.set_image(width, height, rgba),
            Err(RecvTimeoutError::Timeout) => {
                if let Some(captured) = watcher.poll() {
                    let state = app.state::<AppState>();
                    state.core.lock().unwrap().ingest(captured);
                    let _ = app.emit("state-changed", ());
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn main() {
    let (clipboard_tx, clipboard_rx) = mpsc::channel::<ClipboardMsg>();
    let clipboard_rx = Mutex::new(Some(clipboard_rx));

    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts([SHORTCUT])
                .expect("valid shortcut")
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_panel(app);
                    }
                })
                .build(),
        )
        .manage(AppState {
            core: Mutex::new(Core::load()),
            clipboard_tx,
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            restore_item,
            clear_history,
            toggle_autostart,
            install_update,
            hide_panel,
            quit_app
        ])
        .setup(move |app| {
            // Menu-bar-only app: no Dock icon, no app switcher entry.
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("Clipboard Saver")
                .inner_size(PANEL_WIDTH, PANEL_HEIGHT)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .visible(false)
                .effects(WindowEffectsConfig {
                    effects: vec![WindowEffect::Popover],
                    state: Some(WindowEffectState::Active),
                    radius: Some(13.0),
                    color: None,
                })
                .build()?;

            let win = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Focused(false) = event {
                    let _ = win.hide();
                }
            });

            TrayIconBuilder::with_id("main")
                .icon(Image::new_owned(tray_rgba(), 36, 36))
                .icon_as_template(true)
                .tooltip("Clipboard Saver (⌘⇧V)")
                .on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_panel(tray.app_handle());
                    }
                })
                .build(app)?;

            let rx = clipboard_rx
                .lock()
                .unwrap()
                .take()
                .expect("clipboard receiver taken once");
            let handle = app.handle().clone();
            thread::spawn(move || clipboard_thread(handle, rx));

            let handle = app.handle().clone();
            updater::spawn(move |update| {
                handle
                    .state::<AppState>()
                    .core
                    .lock()
                    .unwrap()
                    .pending_update = Some(update);
                let _ = handle.emit("state-changed", ());
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Downscales and encodes RGBA pixels as a PNG data URL for the panel.
fn thumb_data_url(width: u32, height: u32, rgba: Vec<u8>) -> Option<String> {
    let img = image::RgbaImage::from_raw(width, height, rgba)?;
    let longest = width.max(height).max(1);
    let thumb = if longest <= THUMB_MAX {
        img
    } else {
        let scale = THUMB_MAX as f32 / longest as f32;
        let tw = ((width as f32 * scale).round() as u32).max(1);
        let th = ((height as f32 * scale).round() as u32).max(1);
        image::imageops::thumbnail(&img, tw, th)
    };
    let mut buf = Vec::new();
    thumb
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .ok()?;
    Some(format!("data:image/png;base64,{}", BASE64.encode(buf)))
}

/// Clipboard glyph for the menu bar, rendered at 2× (36px) for retina.
/// Template icon (black + alpha): macOS recolors it for light/dark bars.
fn tray_rgba() -> Vec<u8> {
    const SIZE: usize = 18;
    const SCALE: usize = 2;
    const ART: [&str; 18] = [
        "......######......",
        "......#....#......",
        "..#####....#####..",
        "..#....####....#..",
        "..#............#..",
        "..#..########..#..",
        "..#............#..",
        "..#..########..#..",
        "..#............#..",
        "..#..########..#..",
        "..#............#..",
        "..#..######....#..",
        "..#............#..",
        "..#............#..",
        "..#............#..",
        "..##############..",
        "..................",
        "..................",
    ];
    let px = SIZE * SCALE;
    let mut rgba = Vec::with_capacity(px * px * 4);
    for row in ART {
        debug_assert_eq!(row.len(), SIZE);
        for _ in 0..SCALE {
            for c in row.chars() {
                let alpha = if c == '#' { 255 } else { 0 };
                for _ in 0..SCALE {
                    rgba.extend_from_slice(&[0, 0, 0, alpha]);
                }
            }
        }
    }
    rgba
}
