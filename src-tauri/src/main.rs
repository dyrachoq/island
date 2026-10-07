// В релизе не показываем консольное окно
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod clipboard;
mod media;
mod settings;
mod shelf;
mod sys;

use settings::Settings;
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

const ISLAND: &str = "island";
const SETTINGS: &str = "settings";

/// Прямоугольник острова внутри окна (логические px). Всё, что вне него,
/// пропускает клики насквозь к окнам под островом.
#[derive(Default, Clone, Copy)]
struct HitRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

struct AppState {
    settings: Mutex<Settings>,
    hit: Mutex<HitRect>,
    /// Последнее состояние плеера и обложка (ключ трека, data-URL)
    media: Mutex<media::MediaInfo>,
    art: Mutex<Option<(String, String)>>,
    clip: Mutex<clipboard::History>,
    shelf: Mutex<shelf::Shelf>,
    /// Команды потоку громкости
    volume_tx: Mutex<mpsc::Sender<audio::VolumeCmd>>,
    volume: Mutex<Option<audio::VolumeState>>,
    brightness: Mutex<Option<u8>>,
    /// «Пинок» потоку, который прячет системный индикатор
    osd_ping: Mutex<mpsc::Sender<()>>,
    /// Запросы значков файлов (id, путь)
    icon_tx: Mutex<mpsc::Sender<(u64, String)>>,
}

#[derive(Clone, serde::Serialize)]
struct FileIcon {
    id: u64,
    url: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Artwork {
    key: String,
    url: Option<String>,
}

// ---------- команды для фронтенда ----------

#[tauri::command]
fn get_settings(app: AppHandle, state: tauri::State<AppState>) -> Settings {
    let mut s = state.settings.lock().unwrap().clone();
    // реальное состояние автозапуска берём из системы
    s.autostart = app.autolaunch().is_enabled().unwrap_or(s.autostart);
    s
}

#[tauri::command]
fn save_settings(app: AppHandle, state: tauri::State<AppState>, value: Settings) -> Result<(), String> {
    let launcher = app.autolaunch();
    let enabled = launcher.is_enabled().unwrap_or(false);
    if value.autostart && !enabled {
        launcher.enable().map_err(|e| format!("не удалось включить автозапуск: {e}"))?;
    } else if !value.autostart && enabled {
        launcher.disable().map_err(|e| format!("не удалось выключить автозапуск: {e}"))?;
    }
    if !value.clipboard_history {
        state.clip.lock().unwrap().clear(false);
        emit_clipboard(&app, false);
    }
    settings::save(&app, &value)?;
    *state.settings.lock().unwrap() = value.clone();
    let _ = state.osd_ping.lock().unwrap().send(());
    position_island(&app);
    let _ = app.emit("settings-changed", &value);
    Ok(())
}

#[tauri::command]
fn set_hit_rect(state: tauri::State<AppState>, x: f64, y: f64, w: f64, h: f64) {
    *state.hit.lock().unwrap() = HitRect { x, y, w, h };
}

#[tauri::command]
fn get_battery() -> sys::Battery {
    sys::battery()
}

#[tauri::command]
fn get_media(state: tauri::State<AppState>) -> (media::MediaInfo, Option<Artwork>) {
    let info = state.media.lock().unwrap().clone();
    let art = state
        .art
        .lock()
        .unwrap()
        .clone()
        .map(|(key, url)| Artwork { key, url: Some(url) });
    (info, art)
}

/// action: "toggle" | "next" | "prev" | "seek"
#[tauri::command]
fn media_control(state: tauri::State<AppState>, action: String, position_ms: Option<i64>) {
    let source = state.media.lock().unwrap().source.clone();
    // WinRT-вызовы блокирующие — уводим с главного потока
    std::thread::spawn(move || {
        if let Some(m) = media::Media::new() {
            m.control(&source, &action, position_ms);
        }
    });
}

// ---------- буфер обмена ----------

fn emit_clipboard(app: &AppHandle, added: bool) {
    let items = app.state::<AppState>().clip.lock().unwrap().views();
    let _ = app.emit_to(ISLAND, "clipboard", clipboard::Update { items, added });
}

#[tauri::command]
fn get_clipboard(state: tauri::State<AppState>) -> Vec<clipboard::ItemView> {
    state.clip.lock().unwrap().views()
}

#[tauri::command]
fn clipboard_copy(state: tauri::State<AppState>, id: u64) -> Result<(), String> {
    state.clip.lock().unwrap().copy_to_system(id)
}

#[tauri::command]
fn clipboard_pin(app: AppHandle, state: tauri::State<AppState>, id: u64) {
    state.clip.lock().unwrap().toggle_pin(id);
    emit_clipboard(&app, false);
}

#[tauri::command]
fn clipboard_delete(app: AppHandle, state: tauri::State<AppState>, id: u64) {
    state.clip.lock().unwrap().remove(id);
    emit_clipboard(&app, false);
}

#[tauri::command]
fn clipboard_clear(app: AppHandle, state: tauri::State<AppState>) {
    state.clip.lock().unwrap().clear(true);
    emit_clipboard(&app, false);
}

// ---------- полка файлов ----------

fn emit_shelf(app: &AppHandle) {
    let items = app.state::<AppState>().shelf.lock().unwrap().items();
    let _ = app.emit_to(ISLAND, "shelf", items);
}

fn save_shelf(app: &AppHandle) {
    app.state::<AppState>().shelf.lock().unwrap().save(app);
}

#[tauri::command]
fn shelf_get(state: tauri::State<AppState>) -> Vec<shelf::ShelfItem> {
    state.shelf.lock().unwrap().items()
}

#[tauri::command]
fn shelf_add(app: AppHandle, state: tauri::State<AppState>, paths: Vec<String>) {
    state.shelf.lock().unwrap().add(&paths);
    save_shelf(&app);
    emit_shelf(&app);
}

#[tauri::command]
fn shelf_remove(app: AppHandle, state: tauri::State<AppState>, id: u64) {
    state.shelf.lock().unwrap().remove(id);
    save_shelf(&app);
    emit_shelf(&app);
}

#[tauri::command]
fn shelf_clear(app: AppHandle, state: tauri::State<AppState>) {
    state.shelf.lock().unwrap().clear();
    save_shelf(&app);
    emit_shelf(&app);
}

#[tauri::command]
fn shelf_open(state: tauri::State<AppState>, id: u64) {
    if let Some(p) = state.shelf.lock().unwrap().path(id) {
        sys::open_path(&p);
    }
}

#[tauri::command]
fn shelf_reveal(state: tauri::State<AppState>, id: u64) {
    if let Some(p) = state.shelf.lock().unwrap().path(id) {
        sys::reveal_path(&p);
    }
}

/// Вытаскивание файлов с полки. Системный drag-and-drop обязан идти
/// на главном (UI) потоке, поэтому переносим вызов туда.
#[tauri::command]
fn shelf_drag(app: AppHandle, state: tauri::State<AppState>, ids: Vec<u64>) {
    let paths = state.shelf.lock().unwrap().paths(&ids);
    if paths.is_empty() {
        return;
    }
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        sys::drag_files(&paths);
        // drag_files блокирует до конца перетаскивания — сообщаем фронту, что всё
        let _ = a.emit_to(ISLAND, "shelf-drag-end", ());
    });
}

/// Значок файла как в Проводнике — считается в фоне и приходит событием "shelf-icon"
#[tauri::command]
fn shelf_icon(state: tauri::State<AppState>, id: u64) {
    let path = state.shelf.lock().unwrap().path(id);
    if let Some(path) = path {
        let _ = state.icon_tx.lock().unwrap().send((id, path));
    }
}

/// Отдельный поток для значков: Shell API хочет COM в режиме STA
fn spawn_icon_worker(app: AppHandle, rx: mpsc::Receiver<(u64, String)>) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let mut cache: std::collections::HashMap<String, String> = Default::default();
        while let Ok((id, path)) = rx.recv() {
            let url = match cache.get(&path) {
                Some(u) => Some(u.clone()),
                None => sys::file_icon_png(&path, 96).map(|png| {
                    use base64::Engine;
                    let u = format!(
                        "data:image/png;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(png)
                    );
                    cache.insert(path.clone(), u.clone());
                    u
                }),
            };
            if let Some(url) = url {
                let _ = app.emit_to(ISLAND, "shelf-icon", FileIcon { id, url });
            }
        }
    });
}

/// Прячет системный индикатор громкости. Просыпается при каждом изменении
/// громкости/яркости (окно индикатора Windows создаёт только при первом показе)
/// и раз в 3 секунды — на случай перезапуска Проводника.
fn spawn_native_osd_keeper(app: AppHandle, rx: mpsc::Receiver<()>) {
    std::thread::spawn(move || {
        let mut active = false;
        loop {
            match rx.recv_timeout(Duration::from_secs(3)) {
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                _ => {}
            }
            let want = {
                let s = app.state::<AppState>().settings.lock().unwrap().clone();
                s.osd && s.hide_system_osd
            };
            if want {
                sys::hide_native_osd();
                active = true;
            } else if active {
                sys::restore_native_osd();
                active = false;
            }
        }
    });
}

// ---------- громкость и яркость ----------

#[tauri::command]
fn get_levels(state: tauri::State<AppState>) -> (Option<audio::VolumeState>, Option<u8>) {
    (*state.volume.lock().unwrap(), *state.brightness.lock().unwrap())
}

#[tauri::command]
fn set_volume(state: tauri::State<AppState>, level: f32) {
    let _ = state.volume_tx.lock().unwrap().send(audio::VolumeCmd::Set(level));
}

#[tauri::command]
fn toggle_mute(state: tauri::State<AppState>) {
    let _ = state.volume_tx.lock().unwrap().send(audio::VolumeCmd::ToggleMute);
}

fn spawn_levels_watch(app: AppHandle, rx: mpsc::Receiver<audio::VolumeCmd>) {
    let a = app.clone();
    std::thread::spawn(move || {
        audio::run_volume(rx, |v| {
            let state = a.state::<AppState>();
            *state.volume.lock().unwrap() = Some(v);
            let _ = state.osd_ping.lock().unwrap().send(());
            let _ = a.emit_to(ISLAND, "volume", v);
        });
    });
    std::thread::spawn(move || {
        audio::run_brightness(|b| {
            let state = app.state::<AppState>();
            *state.brightness.lock().unwrap() = Some(b);
            let _ = state.osd_ping.lock().unwrap().send(());
            let _ = app.emit_to(ISLAND, "brightness", b);
        });
    });
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    show_settings(&app);
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

// ---------- окна ----------

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App("settings.html".into()))
        .title("Island — настройки")
        .inner_size(460.0, 680.0)
        .resizable(false)
        .maximizable(false)
        .center()
        .build();
}

/// Ставит остров по центру сверху на основном мониторе.
fn position_island(app: &AppHandle) {
    let Some(win) = app.get_webview_window(ISLAND) else { return };
    let monitor = win
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| win.current_monitor().ok().flatten());
    let (Some(mon), Ok(size)) = (monitor, win.outer_size()) else { return };

    let offset = app.state::<AppState>().settings.lock().unwrap().top_offset as f64;
    let x = mon.position().x + (mon.size().width as i32 - size.width as i32) / 2;
    let y = mon.position().y + (offset * mon.scale_factor()).round() as i32;
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let settings_item = MenuItem::with_id(app, "settings", "Настройки", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Выйти", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&settings_item, &separator, &quit_item])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Island")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_settings(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

// ---------- фоновые потоки ----------

/// Каждые 25 мс проверяем, находится ли курсор над островом.
/// Над островом — окно ловит мышь, вне — клики проходят насквозь.
fn spawn_hit_test(app: AppHandle) {
    std::thread::spawn(move || {
        let mut ignoring: Option<bool> = None;
        loop {
            std::thread::sleep(Duration::from_millis(25));
            let Some(win) = app.get_webview_window(ISLAND) else { continue };
            let Some((cx, cy)) = sys::cursor_pos() else { continue };
            let (Ok(pos), Ok(scale)) = (win.outer_position(), win.scale_factor()) else { continue };

            let r = *app.state::<AppState>().hit.lock().unwrap();
            let lx = (cx - pos.x as f64) / scale;
            let ly = (cy - pos.y as f64) / scale;
            let inside = r.w > 0.0 && lx >= r.x && lx <= r.x + r.w && ly >= r.y && ly <= r.y + r.h;

            if ignoring != Some(!inside) && win.set_ignore_cursor_events(!inside).is_ok() {
                ignoring = Some(!inside);
                let _ = app.emit_to(ISLAND, "hover", inside);
            }
        }
    });
}

/// Раз в секунду: батарея и полноэкранные приложения.
fn spawn_system_watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_battery: Option<sys::Battery> = None;
        let mut last_fullscreen: Option<bool> = None;
        let mut tick = 0u64;
        loop {
            let battery = sys::battery();
            if last_battery != Some(battery) {
                last_battery = Some(battery);
                let _ = app.emit_to(ISLAND, "battery", battery);
            }

            let hide = app.state::<AppState>().settings.lock().unwrap().hide_in_fullscreen;
            let fullscreen = hide && sys::fullscreen_app_active();
            if last_fullscreen != Some(fullscreen) {
                last_fullscreen = Some(fullscreen);
                let _ = app.emit_to(ISLAND, "fullscreen", fullscreen);
            }

            tick += 1;
            if tick % 3 == 0 {
                // замок отпускаем до emit_shelf — он берёт его снова
                let changed = app.state::<AppState>().shelf.lock().unwrap().refresh();
                if changed {
                    emit_shelf(&app);
                }
            }

            std::thread::sleep(Duration::from_millis(1000));
        }
    });
}

/// Дважды в секунду опрашиваем плеер. Обложку грузим только при смене трека
/// (и ещё раз через пару секунд — Spotify и браузеры отдают её с задержкой).
fn spawn_media_watch(app: AppHandle) {
    std::thread::spawn(move || {
        let media = loop {
            if let Some(m) = media::Media::new() {
                break m;
            }
            std::thread::sleep(Duration::from_secs(5));
        };
        let mut last_key = String::new();
        let mut art_retries = 0u8;
        let mut tick = 0u32;
        loop {
            let state = app.state::<AppState>();
            let prefer = state.media.lock().unwrap().source.clone();
            let info = media.snapshot(&prefer);

            let changed = *state.media.lock().unwrap() != info;
            if changed {
                *state.media.lock().unwrap() = info.clone();
                let _ = app.emit_to(ISLAND, "media", &info);
            }

            let key = info.track_key();
            if key != last_key {
                last_key = key.clone();
                art_retries = 4;
                tick = 0;
            }
            // попытки на 0, 1.5, 3 и 6 секунде после смены трека
            if art_retries > 0 && [0, 3, 6, 12].contains(&tick) {
                art_retries -= 1;
                let url = if info.active { media.artwork(&info.source) } else { None };
                let prev = state.art.lock().unwrap().clone();
                let new_track = prev.as_ref().map(|(k, _)| k != &key).unwrap_or(true);
                let new_image = url.is_some() && prev.as_ref().map(|(_, u)| u) != url.as_ref();
                if new_track || new_image {
                    *state.art.lock().unwrap() = url.clone().map(|u| (key.clone(), u));
                    let _ = app.emit_to(ISLAND, "media-art", Artwork { key: key.clone(), url });
                }
            }
            tick = tick.saturating_add(1);
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// Следим за счётчиком изменений буфера — это дёшево, сам буфер читаем
/// только когда что-то скопировали.
fn spawn_clipboard_watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_seq = sys::clipboard_seq();
        loop {
            std::thread::sleep(Duration::from_millis(350));
            let seq = sys::clipboard_seq();
            if seq == last_seq {
                continue;
            }
            last_seq = seq;

            let state = app.state::<AppState>();
            if !state.settings.lock().unwrap().clipboard_history || sys::clipboard_is_private() {
                continue;
            }
            // приложение могло ещё не дописать все форматы — даём ему мгновение
            std::thread::sleep(Duration::from_millis(80));
            let added = clipboard::capture(&state.clip);
            emit_clipboard(&app, added);
        }
    });
}

fn main() {
    tauri::Builder::default()
        // повторный запуск .exe просто открывает настройки
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_settings(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            set_hit_rect,
            get_battery,
            get_media,
            media_control,
            get_clipboard,
            clipboard_copy,
            clipboard_pin,
            clipboard_delete,
            clipboard_clear,
            shelf_get,
            shelf_add,
            shelf_remove,
            shelf_clear,
            shelf_open,
            shelf_reveal,
            shelf_drag,
            shelf_icon,
            get_levels,
            set_volume,
            toggle_mute,
            open_settings,
            quit
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let (volume_tx, volume_rx) = mpsc::channel();
            let (osd_tx, osd_rx) = mpsc::channel();
            let (icon_tx, icon_rx) = mpsc::channel();
            app.manage(AppState {
                settings: Mutex::new(settings::load(&handle)),
                hit: Mutex::new(HitRect::default()),
                media: Mutex::new(media::MediaInfo::default()),
                art: Mutex::new(None),
                clip: Mutex::new(clipboard::History::default()),
                shelf: Mutex::new(shelf::Shelf::load(&handle)),
                volume_tx: Mutex::new(volume_tx),
                volume: Mutex::new(None),
                brightness: Mutex::new(None),
                osd_ping: Mutex::new(osd_tx),
                icon_tx: Mutex::new(icon_tx),
            });

            build_tray(&handle)?;
            position_island(&handle);
            if let Some(win) = app.get_webview_window(ISLAND) {
                // пока фронт не сообщил размеры острова — всё прозрачно для мыши
                let _ = win.set_ignore_cursor_events(true);
                win.show()?;
            }

            spawn_hit_test(handle.clone());
            spawn_media_watch(handle.clone());
            spawn_clipboard_watch(handle.clone());
            spawn_levels_watch(handle.clone(), volume_rx);
            spawn_native_osd_keeper(handle.clone(), osd_rx);
            spawn_icon_worker(handle.clone(), icon_rx);
            spawn_system_watch(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            // остров нельзя закрыть Alt+F4 — только через трей
            if window.label() == ISLAND {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("не удалось запустить Island")
        .run(|_app, event| {
            // при выходе возвращаем системный индикатор громкости
            if let tauri::RunEvent::Exit = event {
                sys::restore_native_osd();
            }
        });
}
