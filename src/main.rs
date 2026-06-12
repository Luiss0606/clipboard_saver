mod autostart;
mod history;
mod item;
mod menu;
mod storage;
mod updater;
mod watcher;

use std::time::{Duration, Instant};

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent};
use tray_icon::{TrayIcon, TrayIconBuilder};

use history::History;
use item::{fnv1a, ItemKind};
use menu::Thumbs;
use storage::Storage;
use watcher::{Captured, Watcher};

const POLL_INTERVAL: Duration = Duration::from_millis(400);
/// Longest side of menu thumbnails, in pixels.
const THUMB_MAX: u32 = 32;

enum UserEvent {
    Menu(MenuEvent),
    UpdateReady(updater::Update),
}

enum MenuAction {
    None,
    Refresh,
    Quit,
}

struct App {
    history: History,
    storage: Storage,
    watcher: Watcher,
    thumbs: Thumbs,
    pending_update: Option<updater::Update>,
}

impl App {
    fn load() -> Self {
        let storage = Storage::new(Storage::default_dir()).expect("cannot create data directory");
        let history = History::from_items(storage.load());
        let watcher = Watcher::new().expect("cannot access clipboard");

        let mut thumbs = Thumbs::new();
        for item in history.items() {
            if let Some(file) = item.png_file() {
                if let Some((w, h, rgba)) = storage.load_image(file) {
                    thumbs.insert(item.id, make_thumb(w, h, rgba));
                }
            }
        }

        Self {
            history,
            storage,
            watcher,
            thumbs,
            pending_update: None,
        }
    }

    fn persist(&self) {
        if let Err(e) = self.storage.save(&self.history.to_vec()) {
            eprintln!("clipboard_saver: cannot save history: {e}");
        }
    }

    fn build_menu(&self) -> Menu {
        menu::build(
            &self.history,
            &self.thumbs,
            autostart::is_enabled(),
            updater::RELEASE_TAG.unwrap_or("dev"),
            self.pending_update.as_ref().map(|u| u.tag.as_str()),
        )
    }

    /// Polls the clipboard; returns true when the menu needs a rebuild.
    fn poll_clipboard(&mut self) -> bool {
        let Some(captured) = self.watcher.poll() else {
            return false;
        };
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
                            self.thumbs
                                .insert(id, make_thumb(width as u32, height as u32, rgba));
                            self.remove_evicted(&evicted);
                        }
                        Err(e) => {
                            eprintln!("clipboard_saver: cannot save image: {e}");
                            return false;
                        }
                    }
                }
            }
        }
        self.persist();
        true
    }

    fn remove_evicted(&mut self, evicted: &[item::ClipboardItem]) {
        for old in evicted {
            self.thumbs.remove(&old.id);
        }
        self.storage.delete_images(evicted);
    }

    fn handle_menu(&mut self, event: &MenuEvent) -> MenuAction {
        match event.id.0.as_str() {
            menu::ID_QUIT => MenuAction::Quit,
            menu::ID_CLEAR => {
                let removed = self.history.clear();
                self.storage.delete_images(&removed);
                self.thumbs.clear();
                self.persist();
                MenuAction::Refresh
            }
            menu::ID_UPDATE => {
                if let Some(update) = &self.pending_update {
                    // On success this exits the process and relaunches.
                    if let Err(e) = updater::install_and_relaunch(update) {
                        eprintln!("clipboard_saver: update failed: {e}");
                    }
                }
                MenuAction::None
            }
            menu::ID_AUTOSTART => {
                let result = if autostart::is_enabled() {
                    autostart::disable()
                } else {
                    autostart::enable()
                };
                if let Err(e) = result {
                    eprintln!("clipboard_saver: cannot toggle autostart: {e}");
                }
                MenuAction::Refresh
            }
            _ => {
                if let Some(id) = menu::parse_item_id(&event.id) {
                    self.restore(id);
                }
                // The watcher picks up the clipboard change on the next poll
                // and promotes the item, so no immediate rebuild is needed.
                MenuAction::None
            }
        }
    }

    /// Puts a history item back onto the system clipboard.
    fn restore(&mut self, id: u64) {
        let Some(item) = self.history.get(id) else {
            return;
        };
        match &item.kind {
            ItemKind::Text(text) => {
                let text = text.clone();
                self.watcher.set_text(&text);
            }
            ItemKind::Image { png, .. } => {
                let png = png.clone();
                if let Some((w, h, rgba)) = self.storage.load_image(&png) {
                    self.watcher.set_image(w as usize, h as usize, rgba);
                } else {
                    eprintln!("clipboard_saver: stored image {png} is missing");
                }
            }
        }
    }
}

fn main() {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    // Menu-bar-only app: no Dock icon, no app switcher entry.
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    let update_proxy = event_loop.create_proxy();
    updater::spawn(move |update| {
        let _ = update_proxy.send_event(UserEvent::UpdateReady(update));
    });

    let mut app = App::load();
    let mut tray: Option<TrayIcon> = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + POLL_INTERVAL);

        match event {
            // On macOS the tray icon must be created once the event loop runs.
            Event::NewEvents(StartCause::Init) => {
                tray = Some(
                    TrayIconBuilder::new()
                        .with_menu(Box::new(app.build_menu()))
                        .with_icon(tray_glyph())
                        .with_icon_as_template(true)
                        .with_tooltip("Clipboard Saver")
                        .build()
                        .expect("cannot create menu bar icon"),
                );
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if app.poll_clipboard() {
                    if let Some(tray) = &tray {
                        tray.set_menu(Some(Box::new(app.build_menu())));
                    }
                }
            }
            Event::UserEvent(UserEvent::UpdateReady(update)) => {
                app.pending_update = Some(update);
                if let Some(tray) = &tray {
                    tray.set_menu(Some(Box::new(app.build_menu())));
                }
            }
            Event::UserEvent(UserEvent::Menu(menu_event)) => match app.handle_menu(&menu_event) {
                MenuAction::Quit => *control_flow = ControlFlow::Exit,
                MenuAction::Refresh => {
                    if let Some(tray) = &tray {
                        tray.set_menu(Some(Box::new(app.build_menu())));
                    }
                }
                MenuAction::None => {}
            },
            _ => {}
        }
    });
}

/// Downscales RGBA pixels so the longest side is at most [`THUMB_MAX`].
fn make_thumb(width: u32, height: u32, rgba: Vec<u8>) -> (u32, u32, Vec<u8>) {
    let Some(img) = image::RgbaImage::from_raw(width, height, rgba) else {
        return (1, 1, vec![0, 0, 0, 0]);
    };
    let longest = width.max(height).max(1);
    if longest <= THUMB_MAX {
        return (width, height, img.into_raw());
    }
    let scale = THUMB_MAX as f32 / longest as f32;
    let tw = ((width as f32 * scale).round() as u32).max(1);
    let th = ((height as f32 * scale).round() as u32).max(1);
    let thumb = image::imageops::thumbnail(&img, tw, th);
    (tw, th, thumb.into_raw())
}

/// 18×18 clipboard glyph drawn as a template icon (black + alpha), so macOS
/// adapts it to light/dark menu bars. No external assets needed.
fn tray_glyph() -> tray_icon::Icon {
    const SIZE: u32 = 18;
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
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for row in ART {
        debug_assert_eq!(row.len(), SIZE as usize);
        for c in row.chars() {
            let alpha = if c == '#' { 255 } else { 0 };
            rgba.extend_from_slice(&[0, 0, 0, alpha]);
        }
    }
    tray_icon::Icon::from_rgba(rgba, SIZE, SIZE).expect("static glyph is valid")
}
