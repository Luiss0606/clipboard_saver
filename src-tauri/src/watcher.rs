use std::borrow::Cow;
use std::path::PathBuf;

use arboard::{Clipboard, ImageData};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypePNG,
    NSPasteboardTypeString, NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSString, NSURL};

/// One element of a multi-item clipboard write. Images carry both the
/// stored PNG file path and its bytes so target apps can paste either
/// files (Finder-style) or image data.
pub enum PasteItem {
    Text(String),
    Image { path: PathBuf, png: Vec<u8> },
}

/// Content read from the clipboard after a change was detected.
pub enum Captured {
    Text(String),
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
}

/// Detects clipboard changes by polling `NSPasteboard.changeCount` (macOS has
/// no change notification API) and reads the new content through arboard.
/// Must live on the main thread.
pub struct Watcher {
    pasteboard: Retained<NSPasteboard>,
    clipboard: Clipboard,
    last_count: isize,
}

impl Watcher {
    pub fn new() -> Result<Self, arboard::Error> {
        let pasteboard = NSPasteboard::generalPasteboard();
        let last_count = pasteboard.changeCount();
        Ok(Self {
            pasteboard,
            clipboard: Clipboard::new()?,
            last_count,
        })
    }

    /// Returns the new clipboard content if it changed since the last poll.
    /// Comparing `changeCount` first keeps the idle path cheap — no content
    /// is read unless something was actually copied.
    pub fn poll(&mut self) -> Option<Captured> {
        let count = self.pasteboard.changeCount();
        if count == self.last_count {
            return None;
        }
        self.last_count = count;

        if let Ok(text) = self.clipboard.get_text() {
            if !text.trim().is_empty() {
                return Some(Captured::Text(text));
            }
        }
        if let Ok(image) = self.clipboard.get_image() {
            return Some(Captured::Image {
                width: image.width,
                height: image.height,
                rgba: image.bytes.into_owned(),
            });
        }
        None
    }

    pub fn set_text(&mut self, text: &str) {
        if let Err(e) = self.clipboard.set_text(text.to_string()) {
            eprintln!("clipboard_saver: cannot write text to clipboard: {e}");
        }
    }

    pub fn set_image(&mut self, width: usize, height: usize, rgba: Vec<u8>) {
        let image = ImageData {
            width,
            height,
            bytes: Cow::Owned(rgba),
        };
        if let Err(e) = self.clipboard.set_image(image) {
            eprintln!("clipboard_saver: cannot write image to clipboard: {e}");
        }
    }

    /// Writes several entries as one pasteboard item each, like copying
    /// multiple files in Finder. arboard cannot do this (single item only),
    /// so this goes through NSPasteboard directly.
    pub fn set_items(&mut self, items: Vec<PasteItem>) {
        let objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = items
            .into_iter()
            .map(|item| {
                let pb_item = NSPasteboardItem::new();
                match item {
                    PasteItem::Text(text) => {
                        let ty = unsafe { NSPasteboardTypeString };
                        pb_item.setString_forType(&NSString::from_str(&text), ty);
                    }
                    PasteItem::Image { path, png } => {
                        let path = NSString::from_str(&path.to_string_lossy());
                        if let Some(url) = NSURL::fileURLWithPath(&path).absoluteString() {
                            let ty = unsafe { NSPasteboardTypeFileURL };
                            pb_item.setString_forType(&url, ty);
                        }
                        let ty = unsafe { NSPasteboardTypePNG };
                        pb_item.setData_forType(&NSData::with_bytes(&png), ty);
                    }
                }
                ProtocolObject::from_retained(pb_item)
            })
            .collect();

        self.pasteboard.clearContents();
        let array = NSArray::from_retained_slice(&objects);
        if !self.pasteboard.writeObjects(&array) {
            eprintln!("clipboard_saver: cannot write items to clipboard");
        }
        // Skip self-capture: poll() re-decoding our PNG could yield different
        // RGBA bytes (hence a different hash) and duplicate history entries.
        self.last_count = self.pasteboard.changeCount();
    }
}
