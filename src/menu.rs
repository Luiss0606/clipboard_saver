use std::collections::HashMap;

use tray_icon::menu::{
    CheckMenuItem, Icon, IconMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem,
};

use crate::history::History;
use crate::item::ItemKind;

pub const ID_CLEAR: &str = "clear";
pub const ID_QUIT: &str = "quit";
pub const ID_AUTOSTART: &str = "autostart";
pub const ID_UPDATE: &str = "update";

const PREVIEW_CHARS: usize = 60;
const ITEM_ID_PREFIX: &str = "item:";

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

    if history.is_empty() {
        let empty = MenuItem::with_id("empty", "Historial vacío", false, None);
        let _ = menu.append(&empty);
    }

    for (index, item) in history.items().enumerate() {
        let id = MenuId(format!("{ITEM_ID_PREFIX}{}", item.id));
        match &item.kind {
            ItemKind::Text(text) => {
                let label = format!("{}. {}", index + 1, preview(text));
                let _ = menu.append(&MenuItem::with_id(id, label, true, None));
            }
            ItemKind::Image { width, height, .. } => {
                let icon = thumbs
                    .get(&item.id)
                    .and_then(|(w, h, rgba)| Icon::from_rgba(rgba.clone(), *w, *h).ok());
                let label = format!("{}. Imagen ({width}×{height})", index + 1);
                let _ = menu.append(&IconMenuItem::with_id(id, label, true, icon, None));
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
        !history.is_empty(),
        None,
    ));
    let _ = menu.append(&MenuItem::with_id(ID_QUIT, "Salir", true, None));

    menu
}

pub fn parse_item_id(menu_id: &MenuId) -> Option<u64> {
    menu_id.0.strip_prefix(ITEM_ID_PREFIX)?.parse().ok()
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
}
