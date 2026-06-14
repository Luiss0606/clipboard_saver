#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod history;
mod item;
mod panel;
mod storage;
mod updater;
mod watcher;

use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::utils::config::WindowEffectsConfig;
use tauri::utils::{WindowEffect, WindowEffectState};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_global_shortcut::ShortcutState;
use tauri_plugin_positioner::{Position, WindowExt};

use history::History;
use item::{fnv1a, ClipboardItem, ItemKind};
use panel::{ItemDto, StateDto};
use storage::Storage;
use updater::Update;
use watcher::{Captured, PasteItem, Watcher};

const POLL_INTERVAL: Duration = Duration::from_millis(400);
/// Longest side of panel thumbnails, in pixels.
const THUMB_MAX: u32 = 480;
const PANEL_WIDTH: f64 = 480.0;
const PANEL_HEIGHT: f64 = 560.0;
const SHORTCUT: &str = "cmd+shift+v";

enum ClipboardMsg {
    Text(String),
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
    Items(Vec<PasteItem>),
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
    /// Set while a native drag-out session is active, so the focus-loss
    /// handler doesn't hide the panel out from under the drag.
    dragging: Arc<AtomicBool>,
    /// PID of the app that was frontmost just before the panel opened, so a
    /// drag-drop can hand focus back to it (-1 = none).
    prev_app_pid: Arc<AtomicI32>,
}

/// Payload for a drag-out. The native drag session is either inline text
/// OR a set of files (the plugin can't mix the two), so an all-text
/// selection drags as joined text and any image makes it a file drag.
#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum DragPayload {
    Text { text: String },
    Files { paths: Vec<String> },
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
            ItemKind::Text(text) => Some(ClipboardMsg::Text(text.clone())),
            ItemKind::Image { png, .. } => {
                core.storage
                    .load_image(png)
                    .map(|(w, h, rgba)| ClipboardMsg::Image {
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

/// Wipes the whole history. Shared by the panel command and the tray menu.
fn clear_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut core = state.core.lock().unwrap();
    let removed = core.history.clear();
    core.storage.delete_images(&removed);
    core.thumbs.clear();
    core.persist();
    drop(core);
    let _ = app.emit("state-changed", ());
}

/// Flips launch-at-login and keeps the tray menu checkbox in sync. Shared by
/// the panel command and the tray menu.
fn toggle_autostart_inner(app: &AppHandle) {
    let result = if autostart::is_enabled() {
        autostart::disable()
    } else {
        autostart::enable()
    };
    if let Err(e) = result {
        eprintln!("clipboard_saver: cannot toggle autostart: {e}");
    }
    rebuild_tray_menu(app);
    let _ = app.emit("state-changed", ());
}

#[tauri::command]
fn clear_history(app: AppHandle) {
    clear_all(&app);
}

#[tauri::command]
fn toggle_autostart(app: AppHandle) {
    toggle_autostart_inner(&app);
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
fn delete_items(ids: Vec<u64>, app: AppHandle, state: tauri::State<AppState>) {
    let mut core = state.core.lock().unwrap();
    let removed = core.history.remove_many(&ids);
    for old in &removed {
        core.thumbs.remove(&old.id);
    }
    core.storage.delete_images(&removed);
    core.persist();
    drop(core);
    let _ = app.emit("state-changed", ());
}

#[tauri::command]
fn copy_selected(ids: Vec<u64>, state: tauri::State<AppState>) {
    let core = state.core.lock().unwrap();

    if ids.is_empty() {
        return;
    }

    // Single image: send as image to clipboard.
    if ids.len() == 1 {
        if let Some(item) = core.history.get(ids[0]) {
            let msg = match &item.kind {
                ItemKind::Text(text) => Some(ClipboardMsg::Text(text.clone())),
                ItemKind::Image { png, .. } => {
                    core.storage
                        .load_image(png)
                        .map(|(w, h, rgba)| ClipboardMsg::Image {
                            width: w as usize,
                            height: h as usize,
                            rgba,
                        })
                }
            };
            if let Some(msg) = msg {
                let _ = state.clipboard_tx.send(msg);
            }
            return;
        }
    }

    // Multiple items: text entries joined with newlines become one pasteboard
    // item, each image becomes its own item (file URL + PNG data), so target
    // apps can paste several images at once like a Finder multi-file copy.
    let mut parts: Vec<String> = Vec::new();
    let mut images: Vec<PasteItem> = Vec::new();
    for id in &ids {
        if let Some(item) = core.history.get(*id) {
            match &item.kind {
                ItemKind::Text(text) => parts.push(text.clone()),
                ItemKind::Image { png, .. } => {
                    let path = core.storage.image_path(png);
                    match fs::read(&path) {
                        Ok(bytes) => images.push(PasteItem::Image { path, png: bytes }),
                        Err(e) => eprintln!("clipboard_saver: cannot read image {png}: {e}"),
                    }
                }
            }
        }
    }
    let mut items: Vec<PasteItem> = Vec::new();
    if !parts.is_empty() {
        items.push(PasteItem::Text(parts.join("\n")));
    }
    items.extend(images);
    if !items.is_empty() {
        let _ = state.clipboard_tx.send(ClipboardMsg::Items(items));
    }
}

#[tauri::command]
fn drag_payload(ids: Vec<u64>, state: tauri::State<AppState>) -> Option<DragPayload> {
    let core = state.core.lock().unwrap();

    let mut texts: Vec<String> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for id in &ids {
        if let Some(item) = core.history.get(*id) {
            match &item.kind {
                ItemKind::Text(text) => texts.push(text.clone()),
                ItemKind::Image { png, .. } => {
                    paths.push(core.storage.image_path(png).to_string_lossy().into_owned());
                }
            }
        }
    }

    // All text: drag the joined text inline.
    if paths.is_empty() {
        if texts.is_empty() {
            return None;
        }
        return Some(DragPayload::Text {
            text: texts.join("\n"),
        });
    }

    // Mixed: keep the text by writing it to a temp file alongside the images.
    if !texts.is_empty() {
        let tmp = std::env::temp_dir().join("clipboard_saver_drag.txt");
        if fs::write(&tmp, texts.join("\n")).is_ok() {
            paths.push(tmp.to_string_lossy().into_owned());
        }
    }
    Some(DragPayload::Files { paths })
}

#[tauri::command]
fn set_dragging(on: bool, state: tauri::State<AppState>) {
    state.dragging.store(on, Ordering::SeqCst);
}

/// Called when a drag-out ends: clears the drag guard, hands focus back to
/// the app that was frontmost before the panel opened (so the drop target
/// stays active instead of bouncing), then hides the panel.
#[tauri::command]
fn finish_drag(app: AppHandle, window: tauri::WebviewWindow, state: tauri::State<AppState>) {
    state.dragging.store(false, Ordering::SeqCst);
    activate_prev_app(&app);
    let _ = window.hide();
}

/// Records the app frontmost right now (before the panel takes focus) so a
/// later drag-drop can restore it. No-op off the main thread or if our own
/// app is already frontmost. Call from the main thread (tray/shortcut).
fn capture_frontmost_app(app: &AppHandle) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let our_pid = std::process::id() as i32;
    let pid = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|a| a.processIdentifier())
        .unwrap_or(-1);
    if pid >= 0 && pid != our_pid {
        app.state::<AppState>()
            .prev_app_pid
            .store(pid, Ordering::SeqCst);
    }
}

/// Reactivates the app captured by [`capture_frontmost_app`].
fn activate_prev_app(app: &AppHandle) {
    let pid = app.state::<AppState>().prev_app_pid.load(Ordering::SeqCst);
    if pid < 0 {
        return;
    }
    let _ = app.run_on_main_thread(move || {
        if let Some(running) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            let _ = running
                .activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        }
    });
}

#[tauri::command]
fn hide_panel(window: tauri::WebviewWindow) {
    let _ = window.hide();
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Builds the right-click tray menu, reflecting current autostart and update
/// state. Rebuilt via [`rebuild_tray_menu`] whenever that state changes.
fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let autostart_on = autostart::is_enabled();
    let pending = app
        .state::<AppState>()
        .core
        .lock()
        .unwrap()
        .pending_update
        .clone();

    let show = MenuItem::with_id(app, "show", "Mostrar Clipboard Saver", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Iniciar al iniciar sesión",
        true,
        autostart_on,
        None::<&str>,
    )?;
    let clear = MenuItem::with_id(app, "clear", "Limpiar historial", true, None::<&str>)?;
    let update = match &pending {
        Some(u) => MenuItem::with_id(
            app,
            "update",
            format!("Actualizar a {} y reiniciar", u.tag),
            true,
            None::<&str>,
        )?,
        None => MenuItem::with_id(app, "update", "Buscar actualizaciones", false, None::<&str>)?,
    };
    let sep2 = PredefinedMenuItem::separator(app)?;
    let version = MenuItem::with_id(
        app,
        "version",
        format!("Clipboard Saver {}", updater::RELEASE_TAG.unwrap_or("dev")),
        false,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?;

    Menu::with_items(
        app,
        &[
            &show, &sep1, &autostart, &clear, &update, &sep2, &version, &quit,
        ],
    )
}

/// Rebuilds and reinstalls the tray menu so it reflects fresh state.
fn rebuild_tray_menu(app: &AppHandle) {
    if let Ok(menu) = build_tray_menu(app) {
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

/// Handles a tray menu item selection.
fn on_tray_menu(app: &AppHandle, id: &str) {
    match id {
        "show" => toggle_panel(app),
        "autostart" => toggle_autostart_inner(app),
        "clear" => clear_all(app),
        "update" => {
            let pending = app
                .state::<AppState>()
                .core
                .lock()
                .unwrap()
                .pending_update
                .clone();
            if let Some(u) = pending {
                // On success this exits the process and relaunches.
                if let Err(e) = updater::install_and_relaunch(&u) {
                    eprintln!("clipboard_saver: update failed: {e}");
                }
            }
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

fn toggle_panel(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        // Remember the current app before we steal focus, so a drag-drop can
        // hand focus back to it.
        capture_frontmost_app(app);
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
            Ok(ClipboardMsg::Text(text)) => watcher.set_text(&text),
            Ok(ClipboardMsg::Image {
                width,
                height,
                rgba,
            }) => watcher.set_image(width, height, rgba),
            Ok(ClipboardMsg::Items(items)) => watcher.set_items(items),
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
    let dragging = Arc::new(AtomicBool::new(false));
    let prev_app_pid = Arc::new(AtomicI32::new(-1));

    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_drag::init())
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
            dragging: dragging.clone(),
            prev_app_pid,
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            restore_item,
            clear_history,
            delete_items,
            copy_selected,
            drag_payload,
            set_dragging,
            finish_drag,
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
            let drag_flag = dragging.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Focused(false) = event {
                    // Don't hide while a drag-out is in flight, or the native
                    // drag session would be cancelled mid-gesture.
                    if !drag_flag.load(Ordering::SeqCst) {
                        let _ = win.hide();
                    }
                }
            });

            let tray_menu = build_tray_menu(app.handle())?;
            TrayIconBuilder::with_id("main")
                .icon(Image::new_owned(tray_rgba(), 36, 36))
                .icon_as_template(true)
                .tooltip("Clipboard Saver (⌘⇧V)")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| on_tray_menu(app, event.id().0.as_str()))
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
                rebuild_tray_menu(&handle);
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
