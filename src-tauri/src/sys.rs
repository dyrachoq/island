//! Системные штуки Windows: курсор, батарея, полноэкранные приложения.

#[derive(Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Battery {
    pub has_battery: bool,
    pub percent: u8,
    pub charging: bool,
}

#[cfg(windows)]
mod imp {
    use super::Battery;
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetCursorPos, GetDesktopWindow, GetForegroundWindow, GetShellWindow,
        GetWindowRect,
    };

    /// Начинает системное перетаскивание файлов из острова — в Проводник,
    /// Telegram, браузер и т.д. Вызывать на главном потоке, пока зажата кнопка мыши.
    /// Блокирует, пока пользователь не отпустит файл.
    pub fn drag_files(paths: &[String]) -> bool {
        use windows::core::HSTRING;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
        use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_LINK};
        use windows::Win32::UI::Shell::Common::ITEMIDLIST;
        use windows::Win32::UI::Shell::{
            BHID_DataObject, ILCreateFromPathW, IShellItemArray, SHCreateShellItemArrayFromIDLists,
            SHDoDragDrop,
        };

        unsafe {
            let mut pidls: Vec<*const ITEMIDLIST> = Vec::new();
            for path in paths {
                let pidl = ILCreateFromPathW(&HSTRING::from(path.as_str()));
                if !pidl.is_null() {
                    pidls.push(pidl as *const ITEMIDLIST);
                }
            }
            if pidls.is_empty() {
                return false;
            }

            let result = (|| -> windows::core::Result<()> {
                let array: IShellItemArray = SHCreateShellItemArrayFromIDLists(&pidls)?;
                let data: IDataObject = array.BindToHandler(None, &BHID_DataObject)?;
                // MOVE не разрешаем — Проводник не утащит оригинал, только копию
                let _ = SHDoDragDrop(HWND::default(), &data, None, DROPEFFECT_COPY | DROPEFFECT_LINK);
                Ok(())
            })();

            for pidl in pidls {
                CoTaskMemFree(Some(pidl as *const core::ffi::c_void));
            }
            result.is_ok()
        }
    }

    // ---------- системный индикатор громкости ----------
    //
    // Неофициальный способ (как в ModernFlyouts / HideVolumeOSD / FluentFlyout):
    // находим окно индикатора Explorer и сворачиваем его — тогда Windows его не показывает.
    //   Windows 10 и 11 до 22H2: NativeHWNDHost ⊃ DirectUIHWND
    //   Windows 11 22H2+:        XamlExplorerHostIslandWindow ⊃ DesktopWindowContentBridge
    //                            и DesktopWindowXamlSource, ненулевой размер, «слой» окна 18

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, IsIconic, ShowWindowAsync, SW_MINIMIZE, SW_RESTORE};

    /// Окна, которые мы свернули (храним как числа — HWND нельзя передавать между потоками)
    static HIDDEN_OSD: std::sync::Mutex<Vec<isize>> = std::sync::Mutex::new(Vec::new());

    fn class_name(h: HWND) -> String {
        let mut buf = [0u16; 128];
        let len = unsafe { GetClassNameW(h, &mut buf) } as usize;
        String::from_utf16_lossy(&buf[..len.min(buf.len())])
    }

    /// Все окна заданного класса (или все дочерние, если class = null).
    /// Курсор hwndChildAfter обязателен, иначе FindWindowEx возвращает одно и то же окно.
    fn find_all(parent: HWND, class: PCWSTR) -> Vec<HWND> {
        let mut out = Vec::new();
        let mut prev = HWND::default();
        while out.len() < 256 {
            match unsafe { FindWindowExW(parent, prev, class, PCWSTR::null()) } {
                Ok(h) if !h.is_invalid() => {
                    out.push(h);
                    prev = h;
                }
                _ => break,
            }
        }
        out
    }

    /// Недокументированная GetWindowBand из user32 — «слой» окна в Z-порядке
    fn window_band(h: HWND) -> Option<u32> {
        use windows::core::s;
        use windows::Win32::Foundation::BOOL;
        use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        type GetWindowBand = unsafe extern "system" fn(HWND, *mut u32) -> BOOL;
        unsafe {
            let user32 = GetModuleHandleW(w!("user32.dll")).ok()?;
            let proc = GetProcAddress(user32, s!("GetWindowBand"))?;
            let f: GetWindowBand = std::mem::transmute(proc);
            let mut band = 0u32;
            f(h, &mut band).as_bool().then_some(band)
        }
    }

    fn has_size(h: HWND) -> bool {
        let mut r = RECT::default();
        unsafe { GetWindowRect(h, &mut r).is_ok() && r.right > r.left && r.bottom > r.top }
    }

    fn find_osd_windows() -> Vec<HWND> {
        let mut found = Vec::new();

        // Windows 10 / ранний Windows 11
        for host in find_all(HWND::default(), w!("NativeHWNDHost")) {
            if !find_all(host, w!("DirectUIHWND")).is_empty() {
                found.push(host);
            }
        }

        // Windows 11 22H2 и новее
        let mut candidates = Vec::new();
        for island in find_all(HWND::default(), w!("XamlExplorerHostIslandWindow")) {
            let kids = find_all(island, PCWSTR::null());
            let mut has_bridge = false;
            let mut has_xaml = false;
            for kid in &kids {
                let c = class_name(*kid);
                has_bridge |= c == "Windows.UI.Composition.DesktopWindowContentBridge";
                has_xaml |= c == "DesktopWindowXamlSource";
                for grand in find_all(*kid, PCWSTR::null()) {
                    has_xaml |= class_name(grand) == "DesktopWindowXamlSource";
                }
            }
            if has_bridge && has_xaml && has_size(island) {
                candidates.push(island);
            }
        }
        // Тот же класс у Alt+Tab и Snap Layouts. Если Windows умеет сообщать «слой»
        // окна и среди кандидатов есть слой 18 (индикаторы) — берём только их.
        let banded: Vec<HWND> = candidates
            .iter()
            .copied()
            .filter(|h| window_band(*h) == Some(18))
            .collect();
        found.extend(if banded.is_empty() { candidates } else { banded });
        found
    }

    /// Прячет системный индикатор громкости/яркости. Возвращает число найденных окон.
    pub fn hide_native_osd() -> usize {
        let windows = find_osd_windows();
        let mut hidden = HIDDEN_OSD.lock().unwrap();
        for h in &windows {
            unsafe {
                if !IsIconic(*h).as_bool() {
                    let _ = ShowWindowAsync(*h, SW_MINIMIZE);
                }
            }
            let raw = h.0 as isize;
            if !hidden.contains(&raw) {
                hidden.push(raw);
            }
        }
        windows.len()
    }

    /// Возвращает системный индикатор как было
    pub fn restore_native_osd() {
        let mut hidden = HIDDEN_OSD.lock().unwrap();
        for raw in hidden.drain(..) {
            let h = HWND(raw as *mut core::ffi::c_void);
            unsafe {
                if IsIconic(h).as_bool() {
                    let _ = ShowWindowAsync(h, SW_RESTORE);
                }
            }
        }
    }

    // ---------- значки файлов как в Проводнике ----------

    /// Значок или миниатюра файла от самой Windows (то же, что в Проводнике), в PNG.
    /// Вызывать в потоке с COM в режиме STA.
    pub fn file_icon_png(path: &str, size: i32) -> Option<Vec<u8>> {
        use core::ffi::c_void;
        use windows::core::HSTRING;
        use windows::Win32::Foundation::SIZE;
        use windows::Win32::Graphics::Gdi::{
            CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, BITMAPINFO, BITMAPINFOHEADER,
            DIB_RGB_COLORS, HDC, HGDIOBJ,
        };
        use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT};

        unsafe {
            let factory: IShellItemImageFactory =
                SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
            let bmp = factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_RESIZETOFIT).ok()?;

            let dc = CreateCompatibleDC(HDC::default());
            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            // первый вызов без буфера — узнаём размеры картинки
            let ok = GetDIBits(dc, bmp, 0, 0, None, &mut info, DIB_RGB_COLORS) != 0;
            let (w, h) = (info.bmiHeader.biWidth, info.bmiHeader.biHeight.abs());
            let mut pixels = Vec::new();
            if ok && w > 0 && h > 0 && w <= 1024 && h <= 1024 {
                info.bmiHeader.biBitCount = 32;
                info.bmiHeader.biCompression = 0; // BI_RGB — без сжатия
                info.bmiHeader.biHeight = -h; // строки сверху вниз
                pixels = vec![0u8; (w * h * 4) as usize];
                let lines = GetDIBits(
                    dc,
                    bmp,
                    0,
                    h as u32,
                    Some(pixels.as_mut_ptr() as *mut c_void),
                    &mut info,
                    DIB_RGB_COLORS,
                );
                if lines == 0 {
                    pixels.clear();
                }
            }
            let _ = DeleteDC(dc);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            if pixels.is_empty() {
                return None;
            }

            // BGRA (с premultiplied alpha) → обычный RGBA
            let opaque = pixels.chunks_exact(4).all(|p| p[3] == 0); // у миниатюр JPEG альфы нет
            for p in pixels.chunks_exact_mut(4) {
                let a = if opaque { 255 } else { p[3] };
                let (b, g, r) = (p[0], p[1], p[2]);
                let un = |c: u8| if a == 0 || a == 255 { c } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
                p[0] = un(r);
                p[1] = un(g);
                p[2] = un(b);
                p[3] = a;
            }

            let img = image::RgbaImage::from_raw(w as u32, h as u32, pixels)?;
            let mut png = Vec::new();
            image::DynamicImage::ImageRgba8(img)
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .ok()?;
            Some(png)
        }
    }

    /// Счётчик изменений буфера обмена — растёт при каждом копировании
    pub fn clipboard_seq() -> u32 {
        unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }
    }

    /// Менеджеры паролей (KeePass, 1Password, Bitwarden…) помечают свои данные
    /// специальными форматами — такое в историю не сохраняем.
    pub fn clipboard_is_private() -> bool {
        use windows::core::w;
        use windows::Win32::System::DataExchange::{IsClipboardFormatAvailable, RegisterClipboardFormatW};
        unsafe {
            for name in [
                w!("ExcludeClipboardContentFromMonitorProcessing"),
                w!("Clipboard Viewer Ignore"),
                w!("CanIncludeInClipboardHistory"),
            ] {
                let format = RegisterClipboardFormatW(name);
                if format != 0 && IsClipboardFormatAvailable(format).is_ok() {
                    return true;
                }
            }
        }
        false
    }

    /// Позиция курсора в физических пикселях экрана.
    pub fn cursor_pos() -> Option<(f64, f64)> {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p).ok()? };
        Some((p.x as f64, p.y as f64))
    }

    pub fn battery() -> Battery {
        let mut s = SYSTEM_POWER_STATUS::default();
        if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
            return Battery { has_battery: false, percent: 0, charging: false };
        }
        // 128 — батареи нет (обычный ПК), 255 — неизвестно
        let has_battery = s.BatteryFlag != 128 && s.BatteryFlag != 255;
        Battery {
            has_battery,
            percent: if s.BatteryLifePercent <= 100 { s.BatteryLifePercent } else { 0 },
            charging: s.ACLineStatus == 1,
        }
    }

    /// Открыта ли сейчас игра / видео / презентация на весь экран.
    pub fn fullscreen_app_active() -> bool {
        unsafe {
            if let Ok(state) = SHQueryUserNotificationState() {
                if state == QUNS_BUSY
                    || state == QUNS_RUNNING_D3D_FULL_SCREEN
                    || state == QUNS_PRESENTATION_MODE
                {
                    return true;
                }
            }

            // Запасной вариант для borderless-игр: окно переднего плана
            // целиком закрывает свой монитор.
            let fg = GetForegroundWindow();
            if fg.is_invalid() || fg == GetDesktopWindow() || fg == GetShellWindow() {
                return false;
            }
            let mut class = [0u16; 64];
            let len = GetClassNameW(fg, &mut class) as usize;
            let class = String::from_utf16_lossy(&class[..len.min(class.len())]);
            if class == "Progman" || class == "WorkerW" {
                return false; // это рабочий стол
            }

            let mut r = RECT::default();
            if GetWindowRect(fg, &mut r).is_err() {
                return false;
            }
            let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(mon, &mut mi).as_bool() {
                return false;
            }
            let m = mi.rcMonitor;
            r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Battery;
    pub fn cursor_pos() -> Option<(f64, f64)> { None }
    pub fn clipboard_seq() -> u32 { 0 }
    pub fn drag_files(_: &[String]) -> bool { false }
    pub fn hide_native_osd() -> usize { 0 }
    pub fn restore_native_osd() {}
    pub fn file_icon_png(_: &str, _: i32) -> Option<Vec<u8>> { None }
    pub fn clipboard_is_private() -> bool { false }
    pub fn battery() -> Battery { Battery { has_battery: false, percent: 0, charging: false } }
    pub fn fullscreen_app_active() -> bool { false }
}

pub use imp::*;

/// Открыть файл программой по умолчанию
pub fn open_path(path: &str) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

/// Показать файл в Проводнике (выделенным)
pub fn reveal_path(path: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer капризен к кавычкам, поэтому собираем аргумент вручную
        let _ = std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path))
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = path;
}
