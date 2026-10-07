//! Громкость (Core Audio) и яркость встроенного экрана (WMI).

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, serde::Serialize)]
pub struct VolumeState {
    /// 0.0 … 1.0
    pub level: f32,
    pub muted: bool,
}

pub enum VolumeCmd {
    Set(f32),
    ToggleMute,
}

#[cfg(windows)]
mod imp {
    use super::VolumeState;
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

    pub struct Volume {
        endpoint: IAudioEndpointVolume,
    }

    impl Volume {
        /// Подключается к устройству вывода по умолчанию
        pub fn new() -> Option<Self> {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
                let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()?;
                let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
                Some(Self { endpoint })
            }
        }

        pub fn get(&self) -> Option<VolumeState> {
            unsafe {
                Some(VolumeState {
                    level: self.endpoint.GetMasterVolumeLevelScalar().ok()?,
                    muted: self.endpoint.GetMute().ok()?.as_bool(),
                })
            }
        }

        pub fn set(&self, level: f32) {
            unsafe {
                let _ = self.endpoint.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
                if level > 0.0 {
                    let _ = self.endpoint.SetMute(BOOL::from(false), std::ptr::null());
                }
            }
        }

        pub fn toggle_mute(&self) {
            if let Some(s) = self.get() {
                unsafe {
                    let _ = self.endpoint.SetMute(BOOL::from(!s.muted), std::ptr::null());
                }
            }
        }
    }

    #[derive(serde::Deserialize)]
    #[serde(rename = "WmiMonitorBrightness")]
    #[serde(rename_all = "PascalCase")]
    struct WmiMonitorBrightness {
        current_brightness: u8,
    }

    /// Яркость через WMI. Работает для встроенных экранов ноутбуков;
    /// на ПК с внешним монитором вернёт None.
    pub struct Brightness {
        conn: wmi::WMIConnection,
    }

    impl Brightness {
        pub fn new() -> Option<Self> {
            let com = wmi::COMLibrary::new().ok()?;
            let conn = wmi::WMIConnection::with_namespace_path("root\\WMI", com.into()).ok()?;
            let b = Self { conn };
            b.get()?; // проверяем, что яркость вообще доступна
            Some(b)
        }

        pub fn get(&self) -> Option<u8> {
            let rows: Vec<WmiMonitorBrightness> = self.conn.query().ok()?;
            rows.first().map(|r| r.current_brightness)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::VolumeState;
    pub struct Volume;
    impl Volume {
        pub fn new() -> Option<Self> { None }
        pub fn get(&self) -> Option<VolumeState> { None }
        pub fn set(&self, _: f32) {}
        pub fn toggle_mute(&self) {}
    }
    pub struct Brightness;
    impl Brightness {
        pub fn new() -> Option<Self> { None }
        pub fn get(&self) -> Option<u8> { None }
    }
}

pub use imp::{Brightness, Volume};

/// Поток громкости: каждые 100 мс читает уровень и выполняет команды из канала.
/// Раз в 3 секунды переподключается — вдруг сменилось устройство вывода (наушники).
pub fn run_volume(rx: Receiver<VolumeCmd>, mut on_change: impl FnMut(VolumeState)) {
    let mut vol = Volume::new();
    let mut last: Option<VolumeState> = None;
    let mut ticks = 0u32;
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(cmd) => {
                if let Some(v) = &vol {
                    match cmd {
                        VolumeCmd::Set(l) => v.set(l),
                        VolumeCmd::ToggleMute => v.toggle_mute(),
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        ticks += 1;
        if vol.is_none() || ticks % 30 == 0 {
            vol = Volume::new();
        }
        if let Some(state) = vol.as_ref().and_then(|v| v.get()) {
            if last != Some(state) {
                last = Some(state);
                on_change(state);
            }
        }
    }
}

/// Поток яркости: опрос раз в 400 мс. Если яркость недоступна — поток завершается.
pub fn run_brightness(mut on_change: impl FnMut(u8)) {
    let Some(b) = Brightness::new() else { return };
    let mut last = None;
    loop {
        if let Some(v) = b.get() {
            if last != Some(v) {
                last = Some(v);
                on_change(v);
            }
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}
