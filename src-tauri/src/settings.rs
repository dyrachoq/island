use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Запуск вместе с Windows
    pub autostart: bool,
    /// Прятать остров, когда открыта игра или видео на весь экран
    pub hide_in_fullscreen: bool,
    /// true — раскрывать при наведении, false — по клику
    pub expand_on_hover: bool,
    /// Отступ от верхнего края экрана, в логических пикселях
    pub top_offset: u32,
    /// Показывать анимацию при подключении зарядки
    pub charging_activity: bool,
    /// Звук по окончании таймера
    pub timer_sound: bool,
    /// Вести историю буфера обмена
    pub clipboard_history: bool,
    /// Коротко показывать «Скопировано» на острове
    pub clipboard_flash: bool,
    /// Показывать на острове изменение громкости и яркости
    pub osd: bool,
    /// Колесо мыши над свёрнутым островом меняет громкость
    pub scroll_volume: bool,
    /// Прятать системный индикатор громкости Windows (неофициальный способ)
    pub hide_system_osd: bool,
    /// Значки файлов на полке: "system" — как в Проводнике, "simple" — цветные плашки
    pub file_icons: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            autostart: false,
            hide_in_fullscreen: true,
            expand_on_hover: true,
            top_offset: 8,
            charging_activity: true,
            timer_sound: true,
            clipboard_history: true,
            clipboard_flash: true,
            osd: true,
            scroll_volume: true,
            hide_system_osd: true,
            file_icons: "system".into(),
        }
    }
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("settings.json"))
}

pub fn load(app: &AppHandle) -> Settings {
    path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let p = path(app).ok_or("не найдена папка настроек")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(p, text).map_err(|e| e.to_string())
}
