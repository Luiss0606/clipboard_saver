use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::item::{ClipboardItem, ItemKind};

pub const MAX_ITEMS: usize = 40;

/// In-memory clipboard history: newest item at the front, capped at
/// [`MAX_ITEMS`]. Re-copied duplicates are promoted to the front instead
/// of being stored twice.
pub struct History {
    items: VecDeque<ClipboardItem>,
    next_id: u64,
}

impl History {
    pub fn from_items(items: Vec<ClipboardItem>) -> Self {
        let next_id = items.iter().map(|i| i.id + 1).max().unwrap_or(1);
        let mut items: VecDeque<_> = items.into();
        items.truncate(MAX_ITEMS);
        Self { items, next_id }
    }

    pub fn items(&self) -> impl Iterator<Item = &ClipboardItem> {
        self.items.iter()
    }

    pub fn to_vec(&self) -> Vec<ClipboardItem> {
        self.items.iter().cloned().collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn get(&self, id: u64) -> Option<&ClipboardItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn find_text(&self, text: &str) -> Option<u64> {
        self.items
            .iter()
            .find(|i| matches!(&i.kind, ItemKind::Text(t) if t == text))
            .map(|i| i.id)
    }

    pub fn find_image(&self, hash: u64) -> Option<u64> {
        self.items
            .iter()
            .find(|i| matches!(&i.kind, ItemKind::Image { hash: h, .. } if *h == hash))
            .map(|i| i.id)
    }

    /// Moves an existing item to the front. Returns false if the id is unknown.
    pub fn promote(&mut self, id: u64) -> bool {
        match self.items.iter().position(|i| i.id == id) {
            Some(0) => true,
            Some(pos) => {
                if let Some(mut item) = self.items.remove(pos) {
                    item.copied_at = now();
                    self.items.push_front(item);
                }
                true
            }
            None => false,
        }
    }

    /// Inserts a new item at the front. Returns its id and any items evicted
    /// by the [`MAX_ITEMS`] cap, so the caller can clean up their image files.
    pub fn insert_front(&mut self, kind: ItemKind) -> (u64, Vec<ClipboardItem>) {
        let id = self.next_id;
        self.next_id += 1;
        self.items.push_front(ClipboardItem {
            id,
            kind,
            copied_at: now(),
        });
        let mut evicted = Vec::new();
        while self.items.len() > MAX_ITEMS {
            if let Some(item) = self.items.pop_back() {
                evicted.push(item);
            }
        }
        (id, evicted)
    }

    /// Removes everything, returning the removed items for cleanup.
    pub fn clear(&mut self) -> Vec<ClipboardItem> {
        self.items.drain(..).collect()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> ItemKind {
        ItemKind::Text(s.to_string())
    }

    #[test]
    fn insert_caps_at_max_items_and_evicts_oldest() {
        let mut history = History::from_items(Vec::new());
        for i in 0..MAX_ITEMS + 3 {
            history.insert_front(text(&format!("copy {i}")));
        }
        assert_eq!(history.len(), MAX_ITEMS);
        // Newest first; the three oldest ("copy 0".."copy 2") are gone.
        assert!(history.find_text("copy 0").is_none());
        assert!(history.find_text("copy 2").is_none());
        assert!(history.find_text("copy 3").is_some());
        let first = history.items().next().unwrap();
        assert_eq!(first.kind, text(&format!("copy {}", MAX_ITEMS + 2)));
    }

    #[test]
    fn insert_reports_evicted_items() {
        let mut history = History::from_items(Vec::new());
        for i in 0..MAX_ITEMS {
            let (_, evicted) = history.insert_front(text(&format!("copy {i}")));
            assert!(evicted.is_empty());
        }
        let (_, evicted) = history.insert_front(text("one more"));
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0].kind, text("copy 0"));
    }

    #[test]
    fn promote_moves_item_to_front() {
        let mut history = History::from_items(Vec::new());
        let (first_id, _) = history.insert_front(text("a"));
        history.insert_front(text("b"));
        history.insert_front(text("c"));

        assert!(history.promote(first_id));
        assert_eq!(history.items().next().unwrap().id, first_id);
        assert_eq!(history.len(), 3);
        assert!(!history.promote(9999));
    }

    #[test]
    fn find_text_and_find_image_match_by_content() {
        let mut history = History::from_items(Vec::new());
        let (text_id, _) = history.insert_front(text("hello"));
        let (img_id, _) = history.insert_front(ItemKind::Image {
            width: 2,
            height: 2,
            png: "x.png".into(),
            hash: 42,
        });

        assert_eq!(history.find_text("hello"), Some(text_id));
        assert_eq!(history.find_text("nope"), None);
        assert_eq!(history.find_image(42), Some(img_id));
        assert_eq!(history.find_image(7), None);
    }

    #[test]
    fn from_items_truncates_and_continues_ids() {
        let items: Vec<ClipboardItem> = (0..MAX_ITEMS as u64 + 5)
            .map(|id| ClipboardItem {
                id,
                kind: text(&format!("copy {id}")),
                copied_at: 0,
            })
            .collect();
        let mut history = History::from_items(items);
        assert_eq!(history.len(), MAX_ITEMS);
        let (new_id, _) = history.insert_front(text("new"));
        assert_eq!(new_id, MAX_ITEMS as u64 + 5);
    }
}
