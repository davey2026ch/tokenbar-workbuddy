//! 平台相关：前台应用识别、WorkBuddy 是否在运行、单实例限制、多屏定位。
//! Windows 用 Win32 API；macOS 用 lsappinfo / pgrep（不引入额外依赖）。

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    WorkBuddy,
    SelfApp,
    Other,
}

/// 一块显示器的矩形与缩放。
///
/// 坐标是**物理像素**（Windows 上就是虚拟桌面的绝对坐标，副屏可以是负数），
/// `scale` 是这块屏幕的 DPI 缩放（96 DPI = 1.0）。用物理像素做多屏计算，
/// 跨屏缩放不同也不会算错；只有下发给 egui 时才换成逻辑点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    /// Windows 上是 HMONITOR（macOS 暂未实现，恒为 0）
    pub handle: isize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub scale: f32,
}

impl Screen {
    pub fn new(handle: isize, x: f32, y: f32, w: f32, h: f32, scale: f32) -> Self {
        Self {
            handle,
            x,
            y,
            w,
            h,
            scale: if scale > 0.0 { scale } else { 1.0 },
        }
    }

    /// 换成 egui 的逻辑点：点 = 物理 / 当前窗口缩放
    pub fn to_points(self, pp: f32) -> (f32, f32, f32, f32) {
        let pp = if pp > 0.0 { pp } else { 1.0 };
        (self.x / pp, self.y / pp, self.w / pp, self.h / pp)
    }
}

/// 点落在哪块屏幕里；掉在屏幕之间的缝里就给最近的一块
#[cfg_attr(not(windows), allow(dead_code))]
pub fn screen_at(screens: &[Screen], x: f32, y: f32) -> Option<Screen> {
    if screens.is_empty() {
        return None;
    }
    for s in screens {
        if x >= s.x && x < s.x + s.w && y >= s.y && y < s.y + s.h {
            return Some(*s);
        }
    }
    screens.iter().copied().min_by(|a, b| {
        // 到矩形边的距离（在矩形内是 0），比"到中心"更符合直觉
        let d = |s: &Screen| {
            let dx = (s.x - x).max(0.0).max(x - (s.x + s.w));
            let dy = (s.y - y).max(0.0).max(y - (s.y + s.h));
            dx * dx + dy * dy
        };
        d(a).total_cmp(&d(b))
    })
}

/// 在屏幕里摆一张 w×h 的卡片：水平居中、离屏幕底部留 margin（都是物理像素）
pub fn place_in(screen: Screen, w: f32, h: f32, margin: f32) -> (f32, f32) {
    let x = screen.x + ((screen.w - w) * 0.5).round();
    let y = screen.y + (screen.h - h - margin).max(0.0).round();
    (x, y)
}

#[cfg(windows)]
mod imp {
    use super::{screen_at, Focus, Screen};
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, HWND, LPARAM, RECT, TRUE,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO,
        MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::System::Threading::{
        CreateMutexW, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowTextLengthW,
        GetWindowThreadProcessId, IsWindowVisible, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_NOZORDER,
    };

    static MUTEX: AtomicIsize = AtomicIsize::new(0);

    pub fn single_instance() -> bool {
        let name: Vec<u16> = "Global\\Tokenbar_SingleInstance_v1\0"
            .encode_utf16()
            .collect();
        unsafe {
            let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if h.is_null() {
                return true; // 创建失败就不阻止启动
            }
            MUTEX.store(h as isize, Ordering::SeqCst);
            GetLastError() != 183 // ERROR_ALREADY_EXISTS
        }
    }

    fn process_name(pid: u32) -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            if ok == 0 {
                return None;
            }
            let path = OsString::from_wide(&buf[..len as usize]);
            let s = path.to_string_lossy().replace('\\', "/");
            Some(
                s.rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            )
        }
    }

    fn is_workbuddy(name: &str) -> bool {
        name.contains("workbuddy")
    }

    pub fn focus() -> Focus {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return Focus::Other;
            }
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 {
                return Focus::Other;
            }
            if pid == std::process::id() {
                return Focus::SelfApp;
            }
            match process_name(pid) {
                Some(n) if is_workbuddy(&n) => Focus::WorkBuddy,
                _ => Focus::Other,
            }
        }
    }

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let found = &mut *(lparam as *mut bool);
        if IsWindowVisible(hwnd) != 0 {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != 0 {
                if let Some(n) = process_name(pid) {
                    if is_workbuddy(&n) {
                        *found = true;
                        return 0; // 停止枚举
                    }
                }
            }
        }
        TRUE
    }

    pub fn workbuddy_running() -> bool {
        let mut found = false;
        unsafe {
            EnumWindows(Some(enum_cb), &mut found as *mut bool as LPARAM);
        }
        found
    }

    /// Windows 用 Win32 直接查前台窗口，不存在"没授权就查不到"这回事
    pub fn frontmost_available() -> bool {
        true
    }

    // ---------- 多屏 ----------

    /// 枚举时用来挑"WorkBuddy 主窗口"的累加器
    struct Picker {
        best: (HWND, i64),
    }

    unsafe extern "system" fn pick_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let p = &mut *(lparam as *mut Picker);
        if IsWindowVisible(hwnd) == 0 {
            return TRUE;
        }
        // 只认有标题的窗口：WorkBuddy 还开着一堆无标题的辅助窗口
        if GetWindowTextLengthW(hwnd) == 0 {
            return TRUE;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return TRUE;
        }
        let Some(name) = process_name(pid) else {
            return TRUE;
        };
        if !is_workbuddy(&name) {
            return TRUE;
        }
        let mut r = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(hwnd, &mut r) == 0 {
            return TRUE;
        }
        let area = (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
        if area > p.best.1 {
            p.best = (hwnd, area);
        }
        TRUE
    }

    /// WorkBuddy 主窗口的句柄：正在前台就用它，否则挑面积最大的那个可见窗口
    fn workbuddy_window() -> Option<HWND> {
        unsafe {
            let fg = GetForegroundWindow();
            if !fg.is_null() && GetWindowTextLengthW(fg) > 0 {
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(fg, &mut pid);
                let mine = pid != 0
                    && pid != std::process::id()
                    && process_name(pid).map(|n| is_workbuddy(&n)).unwrap_or(false);
                if mine {
                    let mut r = RECT {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    };
                    if GetWindowRect(fg, &mut r) != 0 && r.right > r.left && r.bottom > r.top {
                        return Some(fg);
                    }
                }
            }
            let mut p = Picker { best: (std::ptr::null_mut(), 0) };
            EnumWindows(Some(pick_cb), &mut p as *mut Picker as LPARAM);
            if p.best.1 > 0 {
                Some(p.best.0)
            } else {
                None
            }
        }
    }

    /// WorkBuddy 当前所在的那块屏幕（前台就是它时最准，否则取主窗口）
    pub fn workbuddy_screen() -> Option<Screen> {
        let hwnd = workbuddy_window()?;
        let (x, y, w, h) = window_rect(hwnd as isize)?;
        // 按窗口中心命中屏幕：跨在两块屏中间时也算"主要在的那块"
        let all = monitors();
        screen_at(&all, x + w * 0.5, y + h * 0.5).or_else(|| monitor_info(hwnd))
    }

    /// 某个窗口现在在哪块屏幕上
    pub fn window_screen(hwnd: isize) -> Option<Screen> {
        if hwnd == 0 {
            return None;
        }
        let (x, y, w, h) = window_rect(hwnd)?;
        let all = monitors();
        screen_at(&all, x + w * 0.5, y + h * 0.5).or_else(|| monitor_info(hwnd as HWND))
    }

    unsafe extern "system" fn monitor_cb(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let out = &mut *(lparam as *mut Vec<Screen>);
        if let Some(s) = screen_of(hmon) {
            out.push(s);
        }
        TRUE
    }

    /// 当前所有显示器
    fn monitors() -> Vec<Screen> {
        let mut out: Vec<Screen> = Vec::new();
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(monitor_cb),
                &mut out as *mut Vec<Screen> as LPARAM,
            );
        }
        out
    }

    fn screen_of(mon: HMONITOR) -> Option<Screen> {
        unsafe {
            if mon.is_null() {
                return None;
            }
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                rcMonitor: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                rcWork: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                dwFlags: 0,
            };
            if GetMonitorInfoW(mon, &mut mi) == 0 {
                return None;
            }
            // 显示器自己的 DPI：WorkBuddy 若是不感知 DPI 的程序，GetDpiForWindow 会失真
            let (mut dx, mut dy) = (0u32, 0u32);
            let scale = if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) == 0
                && dx > 0
            {
                dx as f32 / 96.0
            } else {
                1.0
            };
            let r = mi.rcMonitor;
            Some(Screen::new(
                mon as isize,
                r.left as f32,
                r.top as f32,
                (r.right - r.left).max(0) as f32,
                (r.bottom - r.top).max(0) as f32,
                scale,
            ))
        }
    }

    fn monitor_info(hwnd: HWND) -> Option<Screen> {
        unsafe { screen_of(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)) }
    }

    /// 窗口的物理矩形 (x, y, w, h)
    pub fn window_rect(hwnd: isize) -> Option<(f32, f32, f32, f32)> {
        if hwnd == 0 {
            return None;
        }
        unsafe {
            let mut r = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetWindowRect(hwnd as HWND, &mut r) == 0 {
                return None;
            }
            Some((
                r.left as f32,
                r.top as f32,
                (r.right - r.left).max(0) as f32,
                (r.bottom - r.top).max(0) as f32,
            ))
        }
    }

    /// 把窗口挪到物理坐标：不动大小、不动 Z 序、不抢焦点
    pub fn move_window(hwnd: isize, x: f32, y: f32) -> bool {
        if hwnd == 0 {
            return false;
        }
        unsafe {
            SetWindowPos(
                hwnd as HWND,
                std::ptr::null_mut(),
                x.round() as i32,
                y.round() as i32,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            ) != 0
        }
    }

    pub fn hwnd_of(raw: &raw_window_handle::RawWindowHandle) -> Option<isize> {
        match raw {
            raw_window_handle::RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
            _ => None,
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Focus;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{LazyLock, Mutex};

    /// 上一次查到的前台应用名（小写）
    static LAST_NAME: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));
    /// 是否已经有一个查询在跑
    static QUERYING: AtomicBool = AtomicBool::new(false);
    /// 上次查询是否成功（1=成功/还没查过，0=失败，多半是没授权「自动化」）
    static LAST_OK: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);

    pub fn single_instance() -> bool {
        // 用临时目录锁文件做粗粒度单实例；崩溃残留时允许继续启动
        let path = std::env::temp_dir().join("tokenbar.lock");
        if path.exists() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    let alive = Command::new("kill")
                        .args(["-0", &pid.to_string()])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                    if alive {
                        return false;
                    }
                }
            }
        }
        let _ = std::fs::write(&path, std::process::id().to_string());
        true
    }

    fn frontmost() -> Option<String> {
        // 优先用 lsappinfo：它走 LaunchServices，查前台应用不需要任何授权。
        // （osascript 问 System Events 会弹「自动化」授权框，用户不点就查不到）
        if let Some(name) = frontmost_lsappinfo() {
            return Some(name);
        }
        // 退路：老系统上万一没有 lsappinfo
        frontmost_osascript()
    }

    fn frontmost_lsappinfo() -> Option<String> {
        let front = Command::new("lsappinfo").arg("front").output().ok()?;
        if !front.status.success() {
            return None;
        }
        let asn = String::from_utf8_lossy(&front.stdout).trim().to_string();
        if asn.is_empty() {
            return None;
        }
        let out = Command::new("lsappinfo")
            .args(["info", "-only", "name", &asn])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        // 输出形如：`"Tokenbar" ASN:0x0-0x1a2b3c: (in front)`
        let text = String::from_utf8_lossy(&out.stdout);
        let name = text.split('"').nth(1)?.trim().to_ascii_lowercase();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    fn frontmost_osascript() -> Option<String> {
        let out = Command::new("osascript")
            .args([
                "-e",
                "tell application \"System Events\" to get name of first application process whose frontmost is true",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            // 没授权「自动化」时这里会失败：保留上一次的结果，别当成"切到别的应用"
            return None;
        }
        let name = String::from_utf8_lossy(&out.stdout).trim().to_ascii_lowercase();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    pub fn focus() -> Focus {
        // osascript 会弹「自动化」授权框，首次可能要等人点一下。
        // 放到后台线程查，界面线程永远只读缓存，不然会整个卡住。
        if !QUERYING.swap(true, Ordering::SeqCst) {
            std::thread::spawn(|| {
                match frontmost() {
                    Some(name) => {
                        if let Ok(mut slot) = LAST_NAME.lock() {
                            *slot = name;
                        }
                        LAST_OK.store(1, Ordering::Relaxed);
                    }
                    None => LAST_OK.store(0, Ordering::Relaxed),
                }
                QUERYING.store(false, Ordering::SeqCst);
            });
        }
        let name = LAST_NAME.lock().map(|s| s.clone()).unwrap_or_default();
        if name.contains("workbuddy") {
            Focus::WorkBuddy
        } else if name.contains("tokenbar") {
            Focus::SelfApp
        } else {
            Focus::Other
        }
    }

    pub fn workbuddy_running() -> bool {
        Command::new("pgrep")
            .args(["-x", "WorkBuddy"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// 上一次查前台应用是否成功（没授权「自动化」时会失败）
    pub fn frontmost_available() -> bool {
        LAST_OK.load(Ordering::Relaxed) != 0
    }

    // ---------- 多屏（暂未实现：macOS 版仍按系统默认屏幕摆放） ----------
    pub fn workbuddy_screen() -> Option<super::Screen> {
        None
    }
    pub fn window_screen(_hwnd: isize) -> Option<super::Screen> {
        None
    }
    pub fn window_rect(_hwnd: isize) -> Option<(f32, f32, f32, f32)> {
        None
    }
    pub fn move_window(_hwnd: isize, _x: f32, _y: f32) -> bool {
        false
    }
    pub fn hwnd_of(_raw: &raw_window_handle::RawWindowHandle) -> Option<isize> {
        None
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::Focus;
    pub fn single_instance() -> bool {
        true
    }
    pub fn focus() -> Focus {
        Focus::WorkBuddy
    }
    pub fn workbuddy_running() -> bool {
        true
    }
    pub fn frontmost_available() -> bool {
        true
    }
    pub fn workbuddy_screen() -> Option<super::Screen> {
        None
    }
    pub fn window_screen(_hwnd: isize) -> Option<super::Screen> {
        None
    }
    pub fn window_rect(_hwnd: isize) -> Option<(f32, f32, f32, f32)> {
        None
    }
    pub fn move_window(_hwnd: isize, _x: f32, _y: f32) -> bool {
        false
    }
    pub fn hwnd_of(_raw: &raw_window_handle::RawWindowHandle) -> Option<isize> {
        None
    }
}

pub use imp::{
    focus, frontmost_available, hwnd_of, move_window, single_instance, window_rect, window_screen,
    workbuddy_running, workbuddy_screen,
};

/// 去掉 Windows 11 自带的 1px 边框，并让系统圆角与卡片一致。
#[cfg(windows)]
pub fn polish_window(handle: raw_window_handle::RawWindowHandle) {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    if let RawWindowHandle::Win32(w) = handle {
        let hwnd = w.hwnd.get() as *mut core::ffi::c_void;
        unsafe {
            // DWMWA_BORDER_COLOR = 34, DWMWA_COLOR_NONE = 0xFFFFFFFE
            let none: u32 = 0xFFFF_FFFE;
            let _ = DwmSetWindowAttribute(hwnd, 34, &none as *const u32 as *const _, 4);
            // DWMWA_WINDOW_CORNER_PREFERENCE = 33, DWMWCP_ROUND = 2
            let round: u32 = 2;
            let _ = DwmSetWindowAttribute(hwnd, 33, &round as *const u32 as *const _, 4);
        }
    }
}

#[cfg(not(windows))]
pub fn polish_window<T>(_handle: T) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(x: f32, y: f32, w: f32, h: f32) -> Screen {
        Screen::new(0, x, y, w, h, 1.0)
    }

    #[test]
    fn screen_at_picks_the_monitor_under_the_point() {
        // 主屏 1920，副屏挂在右边，另一块挂在左上（负坐标）
        let layout = [
            screen(0.0, 0.0, 1920.0, 1080.0),
            screen(1920.0, 0.0, 2560.0, 1440.0),
            screen(-1600.0, -200.0, 1600.0, 900.0),
        ];
        assert_eq!(screen_at(&layout, 100.0, 100.0), Some(layout[0]));
        assert_eq!(screen_at(&layout, 2000.0, 100.0), Some(layout[1]));
        assert_eq!(screen_at(&layout, 2000.0, 1300.0), Some(layout[1]));
        assert_eq!(screen_at(&layout, -100.0, 100.0), Some(layout[2]));
        // 掉在缝隙里：给最近的那一块
        assert_eq!(screen_at(&layout, 2000.0, -30.0), Some(layout[1]));
        assert_eq!(screen_at(&layout, -100.0, -300.0), Some(layout[2]));
        assert_eq!(screen_at(&[], 0.0, 0.0), None);
    }

    #[test]
    fn place_in_centers_at_bottom_of_that_screen() {
        let right = screen(1920.0, 0.0, 2560.0, 1440.0);
        let (x, y) = place_in(right, 440.0, 76.0, 118.0);
        assert_eq!(x, 1920.0 + (2560.0 - 440.0) / 2.0);
        assert_eq!(y, 1440.0 - 76.0 - 118.0);

        // 副屏在主屏上方（y 为负）也要落在副屏里
        let above = screen(0.0, -900.0, 1600.0, 900.0);
        let (x, y) = place_in(above, 440.0, 76.0, 118.0);
        assert_eq!(x, (1600.0 - 440.0) / 2.0);
        assert_eq!(y, -900.0 + 900.0 - 76.0 - 118.0);
    }

    #[test]
    fn screen_to_points_divides_by_scale() {
        let s = Screen::new(7, 1920.0, 0.0, 2560.0, 1440.0, 2.0);
        assert_eq!(s.to_points(2.0), (960.0, 0.0, 1280.0, 720.0));
        assert_eq!(s.to_points(0.0), (1920.0, 0.0, 2560.0, 1440.0));
    }
}
