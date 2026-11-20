//! The audited Windows calls (the rimelight-space rule: unsafe code lives in one module). Everything here is a thin,
//! checked wrapper; callers get plain Rust values. Other platforms get empty answers.

#![allow(unsafe_code)]

/// A rectangle in physical screen pixels (virtual-desktop coordinates, can be negative).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A top-level window that can be snipped or recorded.
#[derive(Debug, Clone)]
pub struct TopWindow {
    pub handle: isize,
    pub title: String,
    pub rect: ScreenRect,
}

#[cfg(windows)]
mod imp {
    use super::{ScreenRect, TopWindow};
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
    };
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, GetSystemMetrics, GetWindowLongW, GetWindowTextLengthW,
        GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SM_CXVIRTUALSCREEN,
        SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WS_EX_TOOLWINDOW,
    };

    pub fn virtual_screen() -> ScreenRect {
        // SAFETY: GetSystemMetrics has no preconditions.
        unsafe {
            ScreenRect {
                x: GetSystemMetrics(SM_XVIRTUALSCREEN),
                y: GetSystemMetrics(SM_YVIRTUALSCREEN),
                w: GetSystemMetrics(SM_CXVIRTUALSCREEN),
                h: GetSystemMetrics(SM_CYVIRTUALSCREEN),
            }
        }
    }

    fn title_of(hwnd: HWND) -> String {
        // SAFETY: hwnd comes from EnumWindows; the buffer is sized from GetWindowTextLengthW plus the terminator.
        unsafe {
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                return String::new();
            }
            let mut buf = vec![0u16; len as usize + 1];
            let got = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            String::from_utf16_lossy(&buf[..got.max(0) as usize])
        }
    }

    fn bounds_of(hwnd: HWND) -> Option<ScreenRect> {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: r is a valid RECT and the size matches DWMWA_EXTENDED_FRAME_BOUNDS's output.
        let hr = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                (&mut r as *mut RECT).cast::<c_void>(),
                std::mem::size_of::<RECT>() as u32,
            )
        };
        (hr == 0 && r.right > r.left && r.bottom > r.top).then_some(ScreenRect {
            x: r.left,
            y: r.top,
            w: r.right - r.left,
            h: r.bottom - r.top,
        })
    }

    fn is_cloaked(hwnd: HWND) -> bool {
        let mut cloaked: u32 = 0;
        // SAFETY: cloaked is a valid u32, which is DWMWA_CLOAKED's output size.
        let hr = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED as u32,
                (&mut cloaked as *mut u32).cast::<c_void>(),
                4,
            )
        };
        hr == 0 && cloaked != 0
    }

    struct Collect {
        own_pid: u32,
        out: Vec<TopWindow>,
    }

    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam is the &mut Collect passed to EnumWindows below, alive for the whole enumeration.
        let collect = unsafe { &mut *(lparam as *mut Collect) };
        // SAFETY: plain queries on a window handle EnumWindows gave us.
        let (visible, minimised, ex_style, pid) = unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            (
                IsWindowVisible(hwnd) != 0,
                IsIconic(hwnd) != 0,
                GetWindowLongW(hwnd, GWL_EXSTYLE) as u32,
                pid,
            )
        };
        if !visible || minimised || ex_style & WS_EX_TOOLWINDOW != 0 || pid == collect.own_pid {
            return 1;
        }
        let title = title_of(hwnd);
        if title.is_empty() || is_cloaked(hwnd) {
            return 1;
        }
        if let Some(rect) = bounds_of(hwnd) {
            collect.out.push(TopWindow { handle: hwnd as isize, title, rect });
        }
        1
    }

    pub fn top_windows() -> Vec<TopWindow> {
        let mut collect = Collect { own_pid: std::process::id(), out: Vec::new() };
        // SAFETY: the callback only uses lparam as &mut Collect, which outlives this call.
        unsafe {
            EnumWindows(Some(each), (&mut collect as *mut Collect) as LPARAM);
        }
        collect.out
    }

    pub fn file_types_changed() {
        // SAFETY: SHCNE_ASSOCCHANGED takes no items.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
    }

    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GdiFlush, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, ICONINFO,
    };

    /// Copies screen pixels with GDI into one reusable bitmap.
    pub struct ScreenGrabber {
        rect: ScreenRect,
        screen_dc: HDC,
        mem_dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
        bits: *mut u8,
    }

    impl ScreenGrabber {
        pub fn new(rect: ScreenRect) -> Option<ScreenGrabber> {
            if rect.w <= 0 || rect.h <= 0 {
                return None;
            }
            // SAFETY: standard GDI set-up; every handle is checked and released in Drop (or below on failure).
            unsafe {
                let screen_dc = GetDC(std::ptr::null_mut());
                if screen_dc.is_null() {
                    return None;
                }
                let mem_dc = CreateCompatibleDC(screen_dc);
                let mut info: BITMAPINFO = std::mem::zeroed();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: rect.w,
                    biHeight: -rect.h, // top-down rows
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                };
                let mut bits: *mut c_void = std::ptr::null_mut();
                let bitmap = CreateDIBSection(mem_dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
                if mem_dc.is_null() || bitmap.is_null() || bits.is_null() {
                    if !bitmap.is_null() {
                        DeleteObject(bitmap);
                    }
                    if !mem_dc.is_null() {
                        DeleteDC(mem_dc);
                    }
                    ReleaseDC(std::ptr::null_mut(), screen_dc);
                    return None;
                }
                let old = SelectObject(mem_dc, bitmap);
                Some(ScreenGrabber { rect, screen_dc, mem_dc, bitmap, old, bits: bits.cast() })
            }
        }

        pub fn rect(&self) -> ScreenRect {
            self.rect
        }

        /// The screen area as BGRA rows (top-down, width * 4 bytes each; alpha undefined). `cursor` draws the pointer.
        pub fn grab(&mut self, cursor: bool) -> Option<&[u8]> {
            let r = self.rect;
            // SAFETY: the DCs and bitmap live as long as self; `bits` points to the DIB section of r.w * r.h * 4 bytes,
            // read only after GdiFlush finished drawing into it.
            unsafe {
                if BitBlt(self.mem_dc, 0, 0, r.w, r.h, self.screen_dc, r.x, r.y, SRCCOPY) == 0 {
                    return None;
                }
                if cursor {
                    let mut ci: CURSORINFO = std::mem::zeroed();
                    ci.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
                    if GetCursorInfo(&mut ci) != 0 && ci.flags & CURSOR_SHOWING != 0 {
                        let mut ii: ICONINFO = std::mem::zeroed();
                        let (hx, hy) = if GetIconInfo(ci.hCursor, &mut ii) != 0 {
                            if !ii.hbmMask.is_null() {
                                DeleteObject(ii.hbmMask);
                            }
                            if !ii.hbmColor.is_null() {
                                DeleteObject(ii.hbmColor);
                            }
                            (ii.xHotspot as i32, ii.yHotspot as i32)
                        } else {
                            (0, 0)
                        };
                        DrawIconEx(
                            self.mem_dc,
                            ci.ptScreenPos.x - r.x - hx,
                            ci.ptScreenPos.y - r.y - hy,
                            ci.hCursor,
                            0,
                            0,
                            0,
                            std::ptr::null_mut(),
                            DI_NORMAL,
                        );
                    }
                }
                GdiFlush();
                Some(std::slice::from_raw_parts(self.bits, (r.w * r.h * 4) as usize))
            }
        }
    }

    impl Drop for ScreenGrabber {
        fn drop(&mut self) {
            // SAFETY: releases exactly what new() created, once.
            unsafe {
                SelectObject(self.mem_dc, self.old);
                DeleteObject(self.bitmap);
                DeleteDC(self.mem_dc);
                ReleaseDC(std::ptr::null_mut(), self.screen_dc);
            }
        }
    }

    pub fn exclude_from_capture(hwnd: isize) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};
        // SAFETY: hwnd is one of our own live windows (from winit); a stale handle only makes the call fail.
        unsafe {
            SetWindowDisplayAffinity(hwnd as HWND, WDA_EXCLUDEFROMCAPTURE);
        }
    }

    pub fn local_time() -> [u16; 6] {
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        // SAFETY: GetLocalTime fills the SYSTEMTIME it is given.
        let t = unsafe {
            let mut t = std::mem::zeroed();
            GetLocalTime(&mut t);
            t
        };
        [t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond]
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{ScreenRect, TopWindow};
    pub fn virtual_screen() -> ScreenRect {
        ScreenRect::default()
    }
    pub fn top_windows() -> Vec<TopWindow> {
        Vec::new()
    }
    pub fn file_types_changed() {}
    pub fn exclude_from_capture(_: isize) {}
    pub struct ScreenGrabber;
    impl ScreenGrabber {
        pub fn new(_: ScreenRect) -> Option<ScreenGrabber> {
            None
        }
        pub fn rect(&self) -> ScreenRect {
            ScreenRect::default()
        }
        pub fn grab(&mut self, _: bool) -> Option<&[u8]> {
            None
        }
    }
    pub fn local_time() -> [u16; 6] {
        [2000, 1, 1, 0, 0, 0]
    }
}

/// Keeps one of our windows (the recording bar) out of screenshots and recordings.
pub fn exclude_from_capture(hwnd: isize) {
    imp::exclude_from_capture(hwnd)
}

/// Year, month, day, hour, minute, second on the local clock.
pub fn local_time() -> [u16; 6] {
    imp::local_time()
}

/// Grabs screen pixels (one area, reusable for every frame of a recording).
pub use imp::ScreenGrabber;

/// The bounding box of every monitor, in physical pixels.
pub fn virtual_screen() -> ScreenRect {
    imp::virtual_screen()
}

/// Visible top-level windows of other apps, front-most first.
pub fn top_windows() -> Vec<TopWindow> {
    imp::top_windows()
}

/// Tells Explorer that file associations changed, so icons and "Open with" refresh.
pub fn file_types_changed() {
    imp::file_types_changed()
}
