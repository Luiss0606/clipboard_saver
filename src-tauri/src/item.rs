use serde::{Deserialize, Serialize};

/// A single entry captured from the clipboard.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub id: u64,
    pub kind: ItemKind,
    /// Unix timestamp (seconds) of when the item was captured.
    pub copied_at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ItemKind {
    Text(String),
    Image {
        width: usize,
        height: usize,
        /// PNG file name inside the storage `images/` directory.
        png: String,
        /// FNV-1a hash of the raw RGBA bytes, used for deduplication.
        hash: u64,
    },
}

impl ClipboardItem {
    pub fn png_file(&self) -> Option<&str> {
        match &self.kind {
            ItemKind::Image { png, .. } => Some(png),
            ItemKind::Text(_) => None,
        }
    }
}

/// FNV-1a 64-bit hash. Enough to deduplicate identical image copies
/// without pulling in a hashing dependency.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
