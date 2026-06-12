use std::fs;
use std::io;
use std::path::PathBuf;

use crate::item::ClipboardItem;

/// Disk persistence: a JSON index (`history.json`) plus one PNG per image
/// item under `images/`, all inside the app's data directory.
pub struct Storage {
    base: PathBuf,
}

impl Storage {
    pub fn new(base: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(base.join("images"))?;
        Ok(Self { base })
    }

    /// `~/Library/Application Support/clipboard_saver` on macOS.
    pub fn default_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("clipboard_saver")
    }

    fn index_path(&self) -> PathBuf {
        self.base.join("history.json")
    }

    pub fn image_path(&self, file: &str) -> PathBuf {
        self.base.join("images").join(file)
    }

    pub fn load(&self) -> Vec<ClipboardItem> {
        fs::read(self.index_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, items: &[ClipboardItem]) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(items).map_err(io::Error::other)?;
        // Write-then-rename so a crash mid-write never corrupts the index.
        let tmp = self.base.join("history.json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(tmp, self.index_path())
    }

    /// Encodes RGBA pixels as PNG named after the content hash.
    /// Returns the file name to store in the item.
    pub fn save_image(
        &self,
        hash: u64,
        width: usize,
        height: usize,
        rgba: &[u8],
    ) -> Result<String, String> {
        let file = format!("{hash:016x}.png");
        let img = image::RgbaImage::from_raw(width as u32, height as u32, rgba.to_vec())
            .ok_or("RGBA buffer does not match dimensions")?;
        img.save(self.image_path(&file))
            .map_err(|e| e.to_string())?;
        Ok(file)
    }

    /// Decodes a stored PNG back to (width, height, RGBA bytes).
    pub fn load_image(&self, file: &str) -> Option<(u32, u32, Vec<u8>)> {
        let img = image::open(self.image_path(file)).ok()?.into_rgba8();
        Some((img.width(), img.height(), img.into_raw()))
    }

    pub fn delete_images(&self, items: &[ClipboardItem]) {
        for item in items {
            if let Some(file) = item.png_file() {
                let _ = fs::remove_file(self.image_path(file));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::ItemKind;

    fn temp_storage(tag: &str) -> Storage {
        let dir =
            std::env::temp_dir().join(format!("clipboard_saver_test_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Storage::new(dir).unwrap()
    }

    #[test]
    fn index_roundtrip() {
        let storage = temp_storage("index");
        let items = vec![
            ClipboardItem {
                id: 1,
                kind: ItemKind::Text("hola".into()),
                copied_at: 10,
            },
            ClipboardItem {
                id: 2,
                kind: ItemKind::Image {
                    width: 4,
                    height: 4,
                    png: "abc.png".into(),
                    hash: 99,
                },
                copied_at: 20,
            },
        ];
        storage.save(&items).unwrap();
        let loaded = storage.load();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, 1);
        assert_eq!(loaded[1].kind, items[1].kind);
    }

    #[test]
    fn load_returns_empty_when_index_missing() {
        let storage = temp_storage("missing");
        assert!(storage.load().is_empty());
    }

    #[test]
    fn image_roundtrip_and_delete() {
        let storage = temp_storage("image");
        let rgba: Vec<u8> = vec![
            255, 0, 0, 255, /**/ 0, 255, 0, 255, //
            0, 0, 255, 255, /**/ 255, 255, 255, 255,
        ];
        let file = storage.save_image(7, 2, 2, &rgba).unwrap();
        let (w, h, loaded) = storage.load_image(&file).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(loaded, rgba);

        let item = ClipboardItem {
            id: 1,
            kind: ItemKind::Image {
                width: 2,
                height: 2,
                png: file.clone(),
                hash: 7,
            },
            copied_at: 0,
        };
        storage.delete_images(&[item]);
        assert!(storage.load_image(&file).is_none());
    }
}
