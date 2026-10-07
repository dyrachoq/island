//! Музыка через Windows Media Controls (SMTC) — то же, что показывает
//! стандартный медиа-виджет Windows. Работает со Spotify, Яндекс Музыкой,
//! YouTube в браузере, AIMP и любыми плеерами, которые сообщают о себе системе.

use serde::Serialize;

#[derive(Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub active: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Красивое имя приложения: Spotify, Chrome…
    pub app: String,
    /// Системный id источника — по нему отправляем команды нужному плееру
    pub source: String,
    pub playing: bool,
    pub can_prev: bool,
    pub can_next: bool,
    pub can_play_pause: bool,
    pub position_ms: i64,
    pub duration_ms: i64,
    /// Когда плеер последний раз сообщил позицию (unix ms), 0 если неизвестно
    pub updated_at: i64,
}

impl MediaInfo {
    /// Ключ трека — меняется, когда сменилась песня
    pub fn track_key(&self) -> String {
        format!("{}|{}|{}|{}", self.source, self.title, self.artist, self.album)
    }
}

/// Делает из id вроде "Spotify.exe" или "...!Microsoft.ZuneMusic" читаемое имя.
fn pretty_app(id: &str) -> String {
    let base = id.rsplit('!').next().unwrap_or(id);
    let base = base.trim_end_matches(".exe").trim_end_matches(".EXE");
    let lower = base.to_lowercase();
    let known = [
        ("spotify", "Spotify"),
        ("msedge", "Edge"),
        ("chrome", "Chrome"),
        ("firefox", "Firefox"),
        ("yandex", "Яндекс"),
        ("opera", "Opera"),
        ("brave", "Brave"),
        ("aimp", "AIMP"),
        ("zunemusic", "Медиаплеер"),
        ("vlc", "VLC"),
        ("telegram", "Telegram"),
    ];
    for (needle, name) in known {
        if lower.contains(needle) {
            return name.to_string();
        }
    }
    let mut c = base.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(windows)]
mod imp {
    use super::{pretty_app, MediaInfo};
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use windows::core::Interface;
    use windows::Foundation::IAsyncOperation;
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };
    use windows::Storage::Streams::{DataReader, IRandomAccessStream};

    /// Разница между эпохой Windows (1601) и Unix (1970) в 100-нс тиках
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000;

    pub struct Media {
        manager: Manager,
    }

    impl Media {
        pub fn new() -> Option<Self> {
            unsafe {
                use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
                let _ = RoInitialize(RO_INIT_MULTITHREADED);
            }
            let manager = Manager::RequestAsync().ok()?.get().ok()?;
            Some(Self { manager })
        }

        fn is_playing(s: &Session) -> bool {
            s.GetPlaybackInfo()
                .and_then(|i| i.PlaybackStatus())
                .map(|st| st == Status::Playing)
                .unwrap_or(false)
        }

        fn source_of(s: &Session) -> String {
            s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default()
        }

        /// Какой плеер показывать: сначала тот, что уже показан и играет,
        /// потом любой играющий, потом показанный на паузе, потом «текущий».
        fn pick(&self, prefer: &str) -> Option<Session> {
            let mut sessions = Vec::new();
            if let Ok(list) = self.manager.GetSessions() {
                for i in 0..list.Size().unwrap_or(0) {
                    if let Ok(s) = list.GetAt(i) {
                        sessions.push(s);
                    }
                }
            }
            let preferred = sessions.iter().find(|s| !prefer.is_empty() && Self::source_of(s) == prefer);
            if let Some(s) = preferred.filter(|s| Self::is_playing(s)) {
                return Some(s.clone());
            }
            if let Some(s) = sessions.iter().find(|s| Self::is_playing(s)) {
                return Some(s.clone());
            }
            if let Some(s) = preferred {
                return Some(s.clone());
            }
            self.manager.GetCurrentSession().ok()
        }

        pub fn snapshot(&self, prefer: &str) -> MediaInfo {
            let Some(s) = self.pick(prefer) else { return MediaInfo::default() };
            let mut m = MediaInfo { active: true, ..Default::default() };

            m.source = Self::source_of(&s);
            m.app = pretty_app(&m.source);

            if let Some(p) = s.TryGetMediaPropertiesAsync().ok().and_then(|op| op.get().ok()) {
                m.title = p.Title().map(|h| h.to_string()).unwrap_or_default();
                m.artist = p.Artist().map(|h| h.to_string()).unwrap_or_default();
                m.album = p.AlbumTitle().map(|h| h.to_string()).unwrap_or_default();
            }

            if let Ok(info) = s.GetPlaybackInfo() {
                m.playing = info.PlaybackStatus().map(|st| st == Status::Playing).unwrap_or(false);
                if let Ok(c) = info.Controls() {
                    m.can_prev = c.IsPreviousEnabled().unwrap_or(false);
                    m.can_next = c.IsNextEnabled().unwrap_or(false);
                    m.can_play_pause = c.IsPlayPauseToggleEnabled().unwrap_or(false)
                        || c.IsPlayEnabled().unwrap_or(false)
                        || c.IsPauseEnabled().unwrap_or(false);
                }
            }

            if let Ok(t) = s.GetTimelineProperties() {
                let start = t.StartTime().map(|x| x.Duration).unwrap_or(0);
                let end = t.EndTime().map(|x| x.Duration).unwrap_or(0);
                let pos = t.Position().map(|x| x.Duration).unwrap_or(0);
                m.duration_ms = ((end - start) / 10_000).max(0);
                m.position_ms = ((pos - start) / 10_000).clamp(0, m.duration_ms.max(0));
                let updated = t.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0);
                m.updated_at = if updated > EPOCH_DIFF { (updated - EPOCH_DIFF) / 10_000 } else { 0 };
            }

            if m.title.is_empty() && m.artist.is_empty() {
                m.active = false;
            }
            m
        }

        /// Обложка текущего трека как data:-URL
        pub fn artwork(&self, prefer: &str) -> Option<String> {
            let s = self.pick(prefer)?;
            let props = s.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
            let thumb = props.Thumbnail().ok()?;
            let stream = thumb.OpenReadAsync().ok()?.get().ok()?;
            let mime = stream
                .ContentType()
                .map(|h| h.to_string())
                .ok()
                .filter(|m| m.starts_with("image/"))
                .unwrap_or_else(|| "image/png".into());

            let ras: IRandomAccessStream = stream.cast().ok()?;
            let size = ras.Size().ok()?;
            if size == 0 || size > 8_000_000 {
                return None;
            }
            let input = ras.GetInputStreamAt(0).ok()?;
            let reader = DataReader::CreateDataReader(&input).ok()?;
            let load: IAsyncOperation<u32> = reader.LoadAsync(size as u32).ok()?.cast().ok()?;
            let loaded = load.get().ok()? as usize;
            let mut buf = vec![0u8; loaded];
            reader.ReadBytes(&mut buf).ok()?;
            Some(format!("data:{mime};base64,{}", STANDARD.encode(&buf)))
        }

        pub fn control(&self, prefer: &str, action: &str, position_ms: Option<i64>) {
            let Some(s) = self.pick(prefer) else { return };
            let _ = match action {
                "toggle" => s.TryTogglePlayPauseAsync().and_then(|op| op.get()),
                "next" => s.TrySkipNextAsync().and_then(|op| op.get()),
                "prev" => s.TrySkipPreviousAsync().and_then(|op| op.get()),
                "seek" => {
                    let start = s
                        .GetTimelineProperties()
                        .and_then(|t| t.StartTime())
                        .map(|x| x.Duration)
                        .unwrap_or(0);
                    let ticks = start + position_ms.unwrap_or(0).max(0) * 10_000;
                    s.TryChangePlaybackPositionAsync(ticks).and_then(|op| op.get())
                }
                _ => Ok(false),
            };
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::MediaInfo;
    pub struct Media;
    impl Media {
        pub fn new() -> Option<Self> { None }
        pub fn snapshot(&self, _: &str) -> MediaInfo { MediaInfo::default() }
        pub fn artwork(&self, _: &str) -> Option<String> { None }
        pub fn control(&self, _: &str, _: &str, _: Option<i64>) {}
    }
}

pub use imp::Media;
