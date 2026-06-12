use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::item::{ClipboardItem, ItemKind};

const PREVIEW_CHARS: usize = 120;

/// One history entry as the panel UI consumes it.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ItemDto {
    pub id: u64,
    pub kind: &'static str, // "text" | "image"
    pub preview: String,
    pub is_url: bool,
    pub ago: String,
    /// PNG data URL for image items.
    pub thumb: Option<String>,
}

/// Full panel state pushed to the UI.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StateDto {
    pub items: Vec<ItemDto>,
    pub autostart: bool,
    pub version: String,
    pub pending_update: Option<String>,
    pub max_items: usize,
}

pub fn item_dto(item: &ClipboardItem, thumb: Option<String>, now: u64) -> ItemDto {
    match &item.kind {
        ItemKind::Text(text) => ItemDto {
            id: item.id,
            kind: "text",
            preview: preview(text),
            is_url: is_url(text),
            ago: time_ago(item.copied_at, now),
            thumb: None,
        },
        ItemKind::Image { width, height, .. } => ItemDto {
            id: item.id,
            kind: "image",
            preview: format!("Imagen {width}×{height}"),
            is_url: false,
            ago: time_ago(item.copied_at, now),
            thumb,
        },
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn time_ago(copied_at: u64, now: u64) -> String {
    let secs = now.saturating_sub(copied_at);
    match secs {
        0..=59 => "ahora".into(),
        60..=3_599 => format!("hace {} min", secs / 60),
        3_600..=86_399 => format!("hace {} h", secs / 3_600),
        _ => format!("hace {} d", secs / 86_400),
    }
}

pub fn is_url(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("www."))
        && !t.contains(char::is_whitespace)
}

/// One-line-ish preview: trimmed, newlines collapsed, char-boundary truncated.
pub fn preview(text: &str) -> String {
    let one_line: String = text
        .trim()
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let mut out: String = one_line.chars().take(PREVIEW_CHARS).collect();
    if one_line.chars().count() > PREVIEW_CHARS {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_collapses_newlines_and_truncates() {
        assert_eq!(preview("hola\nmundo"), "hola mundo");
        let long = "x".repeat(300);
        let p = preview(&long);
        assert_eq!(p.chars().count(), PREVIEW_CHARS + 1);
        assert!(p.ends_with('…'));
    }

    #[test]
    fn url_detection() {
        assert!(is_url("https://github.com/Luiss0606"));
        assert!(is_url("  http://example.com"));
        assert!(is_url("www.apple.com"));
        assert!(!is_url("hola https://example.com"));
        assert!(!is_url("texto normal"));
    }

    #[test]
    fn time_ago_buckets() {
        assert_eq!(time_ago(1000, 1030), "ahora");
        assert_eq!(time_ago(1000, 1000 + 5 * 60), "hace 5 min");
        assert_eq!(time_ago(1000, 1000 + 3 * 3600), "hace 3 h");
        assert_eq!(time_ago(1000, 1000 + 2 * 86_400), "hace 2 d");
        assert_eq!(time_ago(2000, 1000), "ahora");
    }

    #[test]
    fn item_dto_maps_kinds() {
        let text_item = ClipboardItem {
            id: 1,
            kind: ItemKind::Text("https://tauri.app".into()),
            copied_at: 0,
        };
        let dto = item_dto(&text_item, None, 30);
        assert_eq!(dto.kind, "text");
        assert!(dto.is_url);
        assert!(dto.thumb.is_none());

        let img_item = ClipboardItem {
            id: 2,
            kind: ItemKind::Image {
                width: 800,
                height: 600,
                png: "x.png".into(),
                hash: 1,
            },
            copied_at: 0,
        };
        let dto = item_dto(&img_item, Some("data:image/png;base64,AA==".into()), 30);
        assert_eq!(dto.kind, "image");
        assert_eq!(dto.preview, "Imagen 800×600");
        assert!(dto.thumb.is_some());
    }
}
