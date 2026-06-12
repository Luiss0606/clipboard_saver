use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use tray_icon::menu::accelerator::{Accelerator, Code, Modifiers};
use tray_icon::menu::{
    CheckMenuItem, Icon, IconMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem,
};

use crate::history::{History, MAX_ITEMS};
use crate::item::ItemKind;

pub const ID_CLEAR: &str = "clear";
pub const ID_QUIT: &str = "quit";
pub const ID_AUTOSTART: &str = "autostart";
pub const ID_UPDATE: &str = "update";

const PREVIEW_CHARS: usize = 48;
const ITEM_ID_PREFIX: &str = "item:";

// Apple system palette (dark-mode friendly) for the type badges.
const BLUE_URL: [u8; 3] = [10, 132, 255]; // systemBlue
const GRAY_TEXT: [u8; 3] = [142, 142, 147]; // systemGray
const PURPLE_IMAGE: [u8; 3] = [191, 90, 242]; // systemPurple

/// Pre-scaled RGBA thumbnails for image items, keyed by item id.
/// Cached so menu rebuilds don't re-decode PNGs from disk.
pub type Thumbs = HashMap<u64, (u32, u32, Vec<u8>)>;

/// Builds the full tray menu from the current history. The menu is rebuilt
/// from scratch on every change — at 40 items that is cheap and avoids
/// tracking per-item menu state.
///
/// `version` is the installed release tag (or "dev"); `pending_update` is
/// the tag of a downloaded release waiting to be installed.
pub fn build(
    history: &History,
    thumbs: &Thumbs,
    autostart_enabled: bool,
    version: &str,
    pending_update: Option<&str>,
) -> Menu {
    let menu = Menu::new();
    let count = history.items().count();

    if history.is_empty() {
        let _ = menu.append(&MenuItem::with_id("empty", "Historial vacío", false, None));
        let _ = menu.append(&MenuItem::with_id(
            "empty-hint",
            "Copia texto o imágenes y aparecerán aquí",
            false,
            None,
        ));
    } else {
        let _ = menu.append(&MenuItem::with_id(
            "header",
            format!("Historial — {count} de {MAX_ITEMS}"),
            false,
            None,
        ));
        let _ = menu.append(&PredefinedMenuItem::separator());
    }

    let now = unix_now();
    for (index, item) in history.items().enumerate() {
        let id = MenuId(format!("{ITEM_ID_PREFIX}{}", item.id));
        let accel = digit_accelerator(index);
        let ago = time_ago(item.copied_at, now);
        match &item.kind {
            ItemKind::Text(text) => {
                let label = format!("{}  ·  {ago}", preview(text));
                let icon = if is_url(text) {
                    rounded_badge(BLUE_URL, &GLYPH_GLOBE)
                } else {
                    rounded_badge(GRAY_TEXT, &GLYPH_TEXT)
                };
                let _ = menu.append(&IconMenuItem::with_id(id, label, true, icon, accel));
            }
            ItemKind::Image { width, height, .. } => {
                let icon = thumbs
                    .get(&item.id)
                    .and_then(|(w, h, rgba)| Icon::from_rgba(rgba.clone(), *w, *h).ok())
                    .or_else(|| rounded_badge(PURPLE_IMAGE, &GLYPH_IMAGE));
                let label = format!("Imagen {width}×{height}  ·  {ago}");
                let _ = menu.append(&IconMenuItem::with_id(id, label, true, icon, accel));
            }
        }
    }

    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id(
        "version",
        format!("Clipboard Saver {version}"),
        false,
        None,
    ));
    if let Some(tag) = pending_update {
        let _ = menu.append(&MenuItem::with_id(
            ID_UPDATE,
            format!("⬇ Actualizar a {tag} y reiniciar"),
            true,
            None,
        ));
    }
    let _ = menu.append(&CheckMenuItem::with_id(
        ID_AUTOSTART,
        "Iniciar con el sistema",
        true,
        autostart_enabled,
        None,
    ));
    let _ = menu.append(&MenuItem::with_id(
        ID_CLEAR,
        "Limpiar historial",
        count > 0,
        Some(Accelerator::new(
            Some(Modifiers::SUPER.union(Modifiers::SHIFT)),
            Code::Backspace,
        )),
    ));
    let _ = menu.append(&MenuItem::with_id(
        ID_QUIT,
        "Salir",
        true,
        Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyQ)),
    ));

    menu
}

pub fn parse_item_id(menu_id: &MenuId) -> Option<u64> {
    menu_id.0.strip_prefix(ITEM_ID_PREFIX)?.parse().ok()
}

/// ⌘1…⌘9 for the nine most recent items — visible as key hints in the menu.
fn digit_accelerator(index: usize) -> Option<Accelerator> {
    const DIGITS: [Code; 9] = [
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ];
    DIGITS
        .get(index)
        .map(|code| Accelerator::new(Some(Modifiers::SUPER), *code))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn time_ago(copied_at: u64, now: u64) -> String {
    let secs = now.saturating_sub(copied_at);
    match secs {
        0..=59 => "ahora".into(),
        60..=3_599 => format!("{} min", secs / 60),
        3_600..=86_399 => format!("{} h", secs / 3_600),
        _ => format!("{} d", secs / 86_400),
    }
}

fn is_url(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("www."))
        && !t.contains(char::is_whitespace)
}

/// One-line label-safe preview: newlines collapsed, `&` escaped (muda treats
/// it as a mnemonic marker), truncated at a char boundary with an ellipsis.
fn preview(text: &str) -> String {
    let one_line: String = text
        .trim()
        .chars()
        .map(|c| if c == '\n' || c == '\r' { '⏎' } else { c })
        .collect();
    let mut out = String::new();
    let mut truncated = false;
    for (taken, c) in one_line.chars().enumerate() {
        if taken >= PREVIEW_CHARS {
            truncated = true;
            break;
        }
        if c == '&' {
            out.push_str("&&");
        } else {
            out.push(c);
        }
    }
    if truncated {
        out.push('…');
    }
    out
}

// 12×12 white glyphs drawn over the colored badge ('#' = white pixel).
const GLYPH_TEXT: [&str; 12] = [
    "............",
    ".##########.",
    ".##########.",
    ".....##.....",
    ".....##.....",
    ".....##.....",
    ".....##.....",
    ".....##.....",
    ".....##.....",
    ".....##.....",
    "............",
    "............",
];

const GLYPH_GLOBE: [&str; 12] = [
    "............",
    "....####....",
    "..##.##.##..",
    ".#...##...#.",
    ".#...##...#.",
    ".##########.",
    ".##########.",
    ".#...##...#.",
    "..##.##.##..",
    "....####....",
    "............",
    "............",
];

const GLYPH_IMAGE: [&str; 12] = [
    "............",
    "............",
    ".##########.",
    ".#........#.",
    ".#..##....#.",
    ".#........#.",
    ".#....#...#.",
    ".#...###..#.",
    ".#..#####.#.",
    ".##########.",
    "............",
    "............",
];

/// Renders a 36×36 rounded-square badge (18pt at 2× retina in the menu)
/// in `rgb` with a white glyph on top.
fn rounded_badge(rgb: [u8; 3], glyph: &[&str; 12]) -> Option<Icon> {
    const SIZE: i32 = 36;
    const RADIUS: i32 = 9;
    const SCALE: i32 = 2;
    const OFFSET: i32 = (SIZE - 12 * SCALE) / 2;

    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut px = if rounded_rect_contains(x, y, SIZE, RADIUS) {
                [rgb[0], rgb[1], rgb[2], 255]
            } else {
                [0, 0, 0, 0]
            };
            let gx = (x - OFFSET) / SCALE;
            let gy = (y - OFFSET) / SCALE;
            if x >= OFFSET
                && y >= OFFSET
                && (0..12).contains(&gx)
                && (0..12).contains(&gy)
                && glyph[gy as usize].as_bytes()[gx as usize] == b'#'
            {
                px = [255, 255, 255, 255];
            }
            rgba.extend_from_slice(&px);
        }
    }
    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32).ok()
}

fn rounded_rect_contains(x: i32, y: i32, size: i32, radius: i32) -> bool {
    let dx = if x < radius {
        radius - x
    } else if x > size - 1 - radius {
        x - (size - 1 - radius)
    } else {
        0
    };
    let dy = if y < radius {
        radius - y
    } else if y > size - 1 - radius {
        y - (size - 1 - radius)
    } else {
        0
    };
    dx * dx + dy * dy <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_collapses_newlines_and_truncates() {
        assert_eq!(preview("hola\nmundo"), "hola⏎mundo");
        let long = "x".repeat(100);
        let p = preview(&long);
        assert_eq!(p.chars().count(), PREVIEW_CHARS + 1);
        assert!(p.ends_with('…'));
    }

    #[test]
    fn preview_escapes_mnemonic_ampersand() {
        assert_eq!(preview("a & b"), "a && b");
    }

    #[test]
    fn parse_item_id_roundtrip() {
        let id = MenuId(format!("{ITEM_ID_PREFIX}17"));
        assert_eq!(parse_item_id(&id), Some(17));
        assert_eq!(parse_item_id(&MenuId("quit".into())), None);
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
        assert_eq!(time_ago(1000, 1000 + 5 * 60), "5 min");
        assert_eq!(time_ago(1000, 1000 + 3 * 3600), "3 h");
        assert_eq!(time_ago(1000, 1000 + 2 * 86_400), "2 d");
        // Clock skew must not panic.
        assert_eq!(time_ago(2000, 1000), "ahora");
    }

    #[test]
    fn digit_accelerators_only_for_first_nine() {
        assert!(digit_accelerator(0).is_some());
        assert!(digit_accelerator(8).is_some());
        assert!(digit_accelerator(9).is_none());
    }

    #[test]
    fn glyphs_are_twelve_by_twelve() {
        for glyph in [&GLYPH_TEXT, &GLYPH_GLOBE, &GLYPH_IMAGE] {
            assert_eq!(glyph.len(), 12);
            for row in glyph.iter() {
                assert_eq!(row.len(), 12, "row {row:?}");
            }
        }
    }

    #[test]
    fn badge_corners_are_transparent_and_center_filled() {
        // Pure-pixel check without macOS APIs: recompute the mask.
        assert!(!rounded_rect_contains(0, 0, 36, 9));
        assert!(!rounded_rect_contains(35, 35, 36, 9));
        assert!(rounded_rect_contains(18, 18, 36, 9));
        assert!(rounded_rect_contains(0, 18, 36, 9));
    }
}
