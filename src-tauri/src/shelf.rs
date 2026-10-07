//! Полка файлов: перетаскиваешь файлы на остров — они лежат тут,
//! пока не понадобятся. Хранятся только ссылки (пути), сами файлы не копируются.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const MAX_ITEMS: usize = 60;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfItem {
    pub id: u64,
    pub path: String,
    pub name: String,
    pub ext: String,
    pub is_dir: bool,
    pub size: u64,
    #[serde(default)]
    pub exists: bool,
    pub added: i64,
}

#[derive(Default)]
pub struct Shelf {
    items: Vec<ShelfItem>,
    next_id: u64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn same_path(a: &str, b: &str) -> bool {
    // в Windows пути нечувствительны к регистру
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

fn describe(id: u64, path: &str, added: i64) -> ShelfItem {
    let p = Path::new(path);
    let meta = std::fs::metadata(p).ok();
    let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
    ShelfItem {
        id,
        path: path.to_string(),
        name: p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        ext: if is_dir {
            String::new()
        } else {
            p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
        },
        is_dir,
        size: meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()).unwrap_or(0),
        exists: meta.is_some(),
        added,
    }
}

fn store_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("shelf.json"))
}

impl Shelf {
    pub fn load(app: &AppHandle) -> Self {
        let saved: Vec<ShelfItem> = store_path(app)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let mut shelf = Shelf::default();
        for item in saved {
            shelf.next_id += 1;
            shelf.items.push(describe(shelf.next_id, &item.path, item.added));
        }
        shelf
    }

    pub fn save(&self, app: &AppHandle) {
        let Some(p) = store_path(app) else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.items) {
            let _ = std::fs::write(p, text);
        }
    }

    /// Добавляет файлы; уже лежащие на полке просто поднимаются наверх
    pub fn add(&mut self, paths: &[String]) {
        for path in paths.iter().rev() {
            if path.trim().is_empty() {
                continue;
            }
            if let Some(pos) = self.items.iter().position(|i| same_path(&i.path, path)) {
                let item = self.items.remove(pos);
                self.items.insert(0, item);
                continue;
            }
            self.next_id += 1;
            self.items.insert(0, describe(self.next_id, path, now_ms()));
        }
        self.items.truncate(MAX_ITEMS);
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|i| i.id != id);
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Перепроверяет, на месте ли файлы
    pub fn refresh(&mut self) -> bool {
        let mut changed = false;
        for item in &mut self.items {
            let exists = Path::new(&item.path).exists();
            if exists != item.exists {
                item.exists = exists;
                changed = true;
            }
        }
        changed
    }

    pub fn items(&self) -> Vec<ShelfItem> {
        self.items.clone()
    }

    pub fn paths(&self, ids: &[u64]) -> Vec<String> {
        self.items
            .iter()
            .filter(|i| i.exists && ids.contains(&i.id))
            .map(|i| i.path.clone())
            .collect()
    }

    pub fn path(&self, id: u64) -> Option<String> {
        self.items.iter().find(|i| i.id == id).map(|i| i.path.clone())
    }
}
