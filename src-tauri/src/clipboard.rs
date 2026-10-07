//! История буфера обмена: текст и картинки.
//! Хранится только в памяти — после перезапуска не сохраняется.

use serde::Serialize;
use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ITEMS: usize = 40;
const MAX_IMAGES: usize = 8;
const MAX_TEXT: usize = 200_000;
const PREVIEW_CHARS: usize = 300;
const THUMB_SIZE: u32 = 112;

enum Data {
    Text(String),
    Image { width: usize, height: usize, rgba: Vec<u8> },
}

struct Item {
    id: u64,
    hash: u64,
    data: Data,
    thumb: Option<String>,
    pinned: bool,
    at: i64,
}

/// То, что уходит во фронтенд (без полных данных)
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    id: u64,
    kind: &'static str,
    text: String,
    thumb: Option<String>,
    width: usize,
    height: usize,
    chars: usize,
    pinned: bool,
    at: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub items: Vec<ItemView>,
    /// true — появилась новая запись (для анимации «Скопировано»)
    pub added: bool,
}

#[derive(Default)]
pub struct History {
    items: Vec<Item>,
    next_id: u64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn hash_of(kind: u8, bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    kind.hash(&mut h);
    bytes.hash(&mut h);
    h.finish()
}

fn make_thumb(width: usize, height: usize, rgba: &[u8]) -> Option<String> {
    use base64::Engine;
    let img = image::RgbaImage::from_raw(width as u32, height as u32, rgba.to_vec())?;
    let scale = (THUMB_SIZE as f32 / width.max(height) as f32).min(1.0);
    let tw = ((width as f32 * scale).round() as u32).max(1);
    let th = ((height as f32 * scale).round() as u32).max(1);
    let small = image::imageops::thumbnail(&img, tw, th);
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(small)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
}

impl History {
    fn insert(&mut self, hash: u64, make: impl FnOnce() -> (Data, Option<String>)) -> bool {
        // уже есть такая запись — просто поднимаем наверх
        if let Some(pos) = self.items.iter().position(|i| i.hash == hash) {
            let mut item = self.items.remove(pos);
            item.at = now_ms();
            self.items.insert(0, item);
            return false;
        }
        let (data, thumb) = make();
        self.next_id += 1;
        self.items.insert(
            0,
            Item { id: self.next_id, hash, data, thumb, pinned: false, at: now_ms() },
        );
        self.trim();
        true
    }

    pub fn add_text(&mut self, text: String) -> bool {
        if text.trim().is_empty() || text.len() > MAX_TEXT {
            return false;
        }
        let hash = hash_of(0, text.as_bytes());
        self.insert(hash, || (Data::Text(text), None))
    }

    pub fn add_image(&mut self, width: usize, height: usize, rgba: Vec<u8>) -> bool {
        if width == 0 || height == 0 || rgba.len() != width * height * 4 {
            return false;
        }
        let hash = hash_of(1, &rgba);
        self.insert(hash, || {
            let thumb = make_thumb(width, height, &rgba);
            (Data::Image { width, height, rgba }, thumb)
        })
    }

    /// Удаляем самые старые незакреплённые записи сверх лимитов
    fn trim(&mut self) {
        let mut images = 0;
        let mut total = 0;
        self.items.retain(|i| {
            if i.pinned {
                return true;
            }
            total += 1;
            if let Data::Image { .. } = i.data {
                images += 1;
                if images > MAX_IMAGES {
                    return false;
                }
            }
            total <= MAX_ITEMS
        });
    }

    pub fn toggle_pin(&mut self, id: u64) {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.pinned = !i.pinned;
        }
        self.trim();
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|i| i.id != id);
    }

    pub fn clear(&mut self, keep_pinned: bool) {
        self.items.retain(|i| keep_pinned && i.pinned);
    }

    pub fn views(&self) -> Vec<ItemView> {
        let mut out: Vec<ItemView> = self
            .items
            .iter()
            .map(|i| match &i.data {
                Data::Text(t) => ItemView {
                    id: i.id,
                    kind: "text",
                    text: t.chars().take(PREVIEW_CHARS).collect(),
                    thumb: None,
                    width: 0,
                    height: 0,
                    chars: t.chars().count(),
                    pinned: i.pinned,
                    at: i.at,
                },
                Data::Image { width, height, .. } => ItemView {
                    id: i.id,
                    kind: "image",
                    text: String::new(),
                    thumb: i.thumb.clone(),
                    width: *width,
                    height: *height,
                    chars: 0,
                    pinned: i.pinned,
                    at: i.at,
                },
            })
            .collect();
        // закреплённые сверху, порядок внутри групп сохраняется
        out.sort_by_key(|v| !v.pinned);
        out
    }

    /// Кладёт запись обратно в системный буфер обмена
    pub fn copy_to_system(&self, id: u64) -> Result<(), String> {
        let item = self.items.iter().find(|i| i.id == id).ok_or("запись не найдена")?;
        let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        match &item.data {
            Data::Text(t) => cb.set_text(t.clone()).map_err(|e| e.to_string()),
            Data::Image { width, height, rgba } => cb
                .set_image(arboard::ImageData {
                    width: *width,
                    height: *height,
                    bytes: Cow::Borrowed(rgba),
                })
                .map_err(|e| e.to_string()),
        }
    }
}

/// Читает текущее содержимое буфера и добавляет в историю.
/// Возвращает true, если появилась новая запись.
pub fn capture(history: &std::sync::Mutex<History>) -> bool {
    let Ok(mut cb) = arboard::Clipboard::new() else { return false };
    if let Ok(text) = cb.get_text() {
        return history.lock().unwrap().add_text(text);
    }
    if let Ok(img) = cb.get_image() {
        let (w, h) = (img.width, img.height);
        return history.lock().unwrap().add_image(w, h, img.bytes.into_owned());
    }
    false
}
