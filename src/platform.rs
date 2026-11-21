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
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
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
        /// Our own window that may be snipped or recorded (the main window); other windows of ours are skipped.
        keep: isize,
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
        if !visible || minimised || ex_style & WS_EX_TOOLWINDOW != 0 || (pid == collect.own_pid && hwnd as isize != collect.keep) {
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

    pub fn top_windows(keep: isize) -> Vec<TopWindow> {
        let mut collect = Collect { own_pid: std::process::id(), keep, out: Vec::new() };
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

    pub fn cursor_monitor() -> Option<((i32, i32), ScreenRect)> {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        // SAFETY: plain queries filling structs we own; cbSize is set as GetMonitorInfoW requires.
        unsafe {
            let mut p = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut p) == 0 {
                return None;
            }
            let mon = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(mon, &mut info) == 0 {
                return None;
            }
            let r = info.rcMonitor;
            Some(((p.x, p.y), ScreenRect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top }))
        }
    }

    /// Takes the "starting or running" lock for this user. True for the one process that holds it; Windows frees it when
    /// that process ends. The handle is kept for the life of the process.
    pub fn take_start_lock() -> bool {
        use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let name: Vec<u16> = "Local\\RimelightMedia.Instance".encode_utf16().chain([0]).collect();
        // SAFETY: a NUL-terminated name; the handle is deliberately never closed.
        unsafe {
            let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            !h.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
        }
    }

    pub fn recycle(path: &std::path::Path) -> bool {
        use std::os::windows::ffi::OsStrExt as _;
        use windows_sys::Win32::UI::Shell::{
            FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW,
        };
        // Double-NUL-terminated list of one path.
        let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
        from.extend_from_slice(&[0, 0]);
        // SAFETY: `from` outlives the call and is double-NUL-terminated as SHFileOperationW requires.
        unsafe {
            let mut op: SHFILEOPSTRUCTW = std::mem::zeroed();
            op.wFunc = FO_DELETE;
            op.pFrom = from.as_ptr();
            op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16;
            SHFileOperationW(&mut op) == 0 && op.fAnyOperationsAborted == 0
        }
    }

    /// A user folder (Pictures, Videos, Music), wherever Windows keeps it (OneDrive can move them).
    pub fn known_folder(which: super::UserFolder) -> Option<std::path::PathBuf> {
        use std::os::windows::ffi::OsStringExt as _;
        use windows_sys::Win32::System::Com::CoTaskMemFree;
        use windows_sys::Win32::UI::Shell::{FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos, SHGetKnownFolderPath};
        let id = match which {
            super::UserFolder::Pictures => &FOLDERID_Pictures,
            super::UserFolder::Videos => &FOLDERID_Videos,
            super::UserFolder::Music => &FOLDERID_Music,
        };
        // SAFETY: SHGetKnownFolderPath allocates the string; it is read up to its NUL and freed with CoTaskMemFree.
        unsafe {
            let mut p: *mut u16 = std::ptr::null_mut();
            let hr = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut p);
            if hr != 0 || p.is_null() {
                if !p.is_null() {
                    CoTaskMemFree(p.cast());
                }
                return None;
            }
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let path = std::ffi::OsString::from_wide(std::slice::from_raw_parts(p, len));
            CoTaskMemFree(p.cast());
            Some(path.into())
        }
    }

    /// Light or dark pop-up menus (the tray menu) for this process: 1 follow Windows, 2 dark, 3 light. Uses uxtheme's
    /// SetPreferredAppMode/FlushMenuThemes (by ordinal: not in the public headers, but how Windows' own apps and
    /// Chromium do it). Does nothing when they are missing.
    pub fn set_menu_theme(mode: i32) {
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
        let name = wide("uxtheme.dll");
        // SAFETY: uxtheme stays loaded; the ordinals 135 (SetPreferredAppMode(int) -> int) and 136 (FlushMenuThemes())
        // have had these signatures since Windows 10 1903, and a missing one is skipped.
        unsafe {
            let lib = LoadLibraryW(name.as_ptr());
            if lib.is_null() {
                return;
            }
            if let Some(f) = GetProcAddress(lib, 135usize as *const u8) {
                let set: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
                set(mode);
            }
            if let Some(f) = GetProcAddress(lib, 136usize as *const u8) {
                let flush: unsafe extern "system" fn() = std::mem::transmute(f);
                flush();
            }
        }
    }

    /// Opens Windows' Cast panel (Win+K): show this screen on a TV or wireless display.
    pub fn open_cast_panel() {
        // The Settings page that finds wireless displays. It runs in its own process, so nothing is sent into our
        // window (sending Win+K to our own window is what made the XAML cast picker overrun the stack).
        if open_uri("ms-settings-connectabledevices:devicediscovery") {
            return;
        }
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_LWIN,
        };
        let key = |vk: u16, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
            },
        };
        let inputs = [key(VK_LWIN, false), key(b'K' as u16, false), key(b'K' as u16, true), key(VK_LWIN, true)];
        // SAFETY: four well-formed keyboard inputs, sized as SendInput requires.
        unsafe {
            SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        }
    }

    /// The text in a picture, line by line, with Windows' built-in text recognition (the languages of the user's
    /// profile).
    pub fn recognise_text(rgba: &[u8], w: u32, h: u32) -> Result<String, String> {
        use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
        use windows::Media::Ocr::OcrEngine;
        use windows::Storage::Streams::DataWriter;
        let e = |e: windows::core::Error| e.message().to_string();
        let max = OcrEngine::MaxImageDimension().map_err(e)?;
        if w > max || h > max {
            return Err(format!("the area is too large to read (at most {max} pixels a side)"));
        }
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()
            .map_err(|_| "Windows has no text recognition for your languages (Settings > Time & language > Language: add the OCR feature)".to_string())?;
        let mut bgra = Vec::with_capacity(rgba.len());
        for p in rgba.chunks_exact(4) {
            bgra.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
        let writer = DataWriter::new().map_err(e)?;
        writer.WriteBytes(&bgra).map_err(e)?;
        let buffer = writer.DetachBuffer().map_err(e)?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w as i32, h as i32).map_err(e)?;
        let result = engine.RecognizeAsync(&bitmap).map_err(e)?.join().map_err(e)?;
        let lines = result.Lines().map_err(e)?;
        let mut out = Vec::new();
        for i in 0..lines.Size().map_err(e)? {
            out.push(lines.GetAt(i).map_err(e)?.Text().map_err(e)?.to_string());
        }
        Ok(out.join("\n"))
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn open_uri(uri: &str) -> bool {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let (verb, target) = (wide("open"), wide(uri));
        // SAFETY: both strings are NUL-terminated and outlive the call; a result above 32 means success.
        let r = unsafe {
            ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
        };
        r as isize > 32
    }

    /// Prints one picture, fitted to the page with a small margin, after Windows' print dialog. Ok(false) when the
    /// person cancels.
    pub fn print_image(owner: isize, rgba: &[u8], w: u32, h: u32, title: &str) -> Result<bool, String> {
        use windows_sys::Win32::Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps, HALFTONE, HORZRES, SRCCOPY,
            SetBrushOrgEx, SetStretchBltMode, StretchDIBits, VERTRES,
        };
        use windows_sys::Win32::Storage::Xps::{DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
        use windows_sys::Win32::UI::Controls::Dialogs::{PD_NOPAGENUMS, PD_NOSELECTION, PD_RETURNDC, PRINTDLGW, PrintDlgW};
        // BGRA rows for GDI.
        let mut bgra = Vec::with_capacity(rgba.len());
        for p in rgba.chunks_exact(4) {
            bgra.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
        let name = wide(title);
        // SAFETY: PRINTDLGW is zeroed and sized as PrintDlgW requires; the device context it returns is used for one
        // document and deleted; `bgra` and `name` outlive every call that reads them.
        unsafe {
            let mut pd: PRINTDLGW = std::mem::zeroed();
            pd.lStructSize = std::mem::size_of::<PRINTDLGW>() as u32;
            pd.hwndOwner = owner as HWND;
            pd.Flags = PD_RETURNDC | PD_NOPAGENUMS | PD_NOSELECTION;
            pd.nCopies = 1;
            if PrintDlgW(&mut pd) == 0 || pd.hDC.is_null() {
                return Ok(false);
            }
            let dc = pd.hDC;
            let (pw, ph) = (GetDeviceCaps(dc, HORZRES as i32), GetDeviceCaps(dc, VERTRES as i32));
            let margin = (pw.min(ph) as f64 * 0.04) as i32;
            let (aw, ah) = (f64::from(pw - 2 * margin), f64::from(ph - 2 * margin));
            let scale = (aw / f64::from(w)).min(ah / f64::from(h));
            let (dw, dh) = ((f64::from(w) * scale) as i32, (f64::from(h) * scale) as i32);
            let (dx, dy) = ((pw - dw) / 2, (ph - dh) / 2);
            let doc = DOCINFOW {
                cbSize: std::mem::size_of::<DOCINFOW>() as i32,
                lpszDocName: name.as_ptr(),
                lpszOutput: std::ptr::null(),
                lpszDatatype: std::ptr::null(),
                fwType: 0,
            };
            let mut ok = StartDocW(dc, &doc) > 0 && StartPage(dc) > 0;
            if ok {
                let mut info: BITMAPINFO = std::mem::zeroed();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w as i32,
                    biHeight: -(h as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                };
                SetStretchBltMode(dc, HALFTONE);
                SetBrushOrgEx(dc, 0, 0, std::ptr::null_mut());
                ok = StretchDIBits(dc, dx, dy, dw, dh, 0, 0, w as i32, h as i32, bgra.as_ptr().cast(), &info, DIB_RGB_COLORS, SRCCOPY) != 0;
                ok &= EndPage(dc) > 0;
                ok &= EndDoc(dc) > 0;
            }
            DeleteDC(dc);
            if ok { Ok(true) } else { Err("the printer refused the page".into()) }
        }
    }

    thread_local! {
        static SHARE_HOOK: std::cell::Cell<Option<(isize, i64)>> = const { std::cell::Cell::new(None) };
    }

    /// Opens Windows' Share panel for a picture file.
    pub fn share_file(owner: isize, path: &std::path::Path, title: &str) -> Result<(), String> {
        use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
        use windows::Foundation::TypedEventHandler;
        use windows::Storage::StorageFile;
        use windows::Storage::Streams::RandomAccessStreamReference;
        use windows::Win32::UI::Shell::IDataTransferManagerInterop;
        use windows::core::{HSTRING, Ref, factory};
        let e = |e: windows::core::Error| e.message().to_string();
        // Checks the file now (a clear error), then again inside the handler (Windows' file object stays on its thread).
        let path = HSTRING::from(path);
        StorageFile::GetFileFromPathAsync(&path).map_err(e)?.join().map_err(e)?;
        let interop = factory::<DataTransferManager, IDataTransferManagerInterop>().map_err(e)?;
        let hwnd = windows::Win32::Foundation::HWND(owner as *mut _);
        // SAFETY: hwnd is our own top-level window.
        let manager: DataTransferManager = unsafe { interop.GetForWindow(hwnd) }.map_err(e)?;
        // One handler per window: drop the one an earlier share added.
        if let Some((w, token)) = SHARE_HOOK.with(|h| h.take())
            && w == owner
        {
            let _ = manager.RemoveDataRequested(token);
        }
        let title = HSTRING::from(title);
        let handler = TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(
            move |_, args: Ref<DataRequestedEventArgs>| {
                let args = args.ok()?;
                let data = args.Request()?.Data()?;
                data.Properties()?.SetTitle(&title)?;
                let file = StorageFile::GetFileFromPathAsync(&path)?.join()?;
                data.SetBitmap(&RandomAccessStreamReference::CreateFromFile(&file)?)?;
                Ok(())
            },
        );
        let token = manager.DataRequested(&handler).map_err(e)?;
        SHARE_HOOK.with(|h| h.set(Some((owner, token))));
        // SAFETY: as above.
        unsafe { interop.ShowShareUIForWindow(hwnd) }.map_err(e)
    }

    pub fn exclude_from_capture(hwnd: isize) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};
        // SAFETY: hwnd is one of our own live windows (from winit); a stale handle only makes the call fail.
        unsafe {
            SetWindowDisplayAffinity(hwnd as HWND, WDA_EXCLUDEFROMCAPTURE);
        }
    }

    /// Turns off Windows' show/hide and move animations for one of our windows (the snip overlay appears at once).
    pub fn no_transitions(hwnd: isize) {
        use windows_sys::Win32::Graphics::Dwm::DWMWA_TRANSITIONS_FORCEDISABLED;
        let on: i32 = 1;
        // SAFETY: `on` is a BOOL-sized value that lives for the call, as DWMWA_TRANSITIONS_FORCEDISABLED expects; a
        // stale handle only makes the call fail.
        unsafe {
            windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
                hwnd as HWND,
                DWMWA_TRANSITIONS_FORCEDISABLED as u32,
                (&on as *const i32).cast::<c_void>(),
                4,
            );
        }
    }

    /// Starts the Win+Shift+S hook (a low-level keyboard hook on its own thread, with a message loop).
    pub fn start_win_shift_s_hook() {
        use std::sync::atomic::Ordering;
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, MSG, SetTimer, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
            WH_KEYBOARD_LL, WM_TIMER,
        };
        if HOOK_STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| {
            // SAFETY: the hook procedure is a static fn; the module handle is the process's own; the message loop only
            // dispatches messages to the hook.
            unsafe {
                let module = GetModuleHandleW(std::ptr::null());
                let mut hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(win_shift_s_proc), module as _, 0);
                hook_log(format!("hook installed: {}", if hook.is_null() { "FAILED" } else { "ok" }));
                if hook.is_null() {
                    HOOK_STARTED.store(false, Ordering::SeqCst);
                    return;
                }
                // Low-level hooks run newest first, so a hook installed later by another process (or a hook Windows
                // dropped as too slow) can let Win+Shift+S reach Windows first. Re-installing puts this one first again.
                // Not proven to be the cause of the occasional misses: hook-log.txt records what the hook sees.
                SetTimer(std::ptr::null_mut(), 0, 30_000, None);
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    if msg.message == WM_TIMER {
                        let fresh = SetWindowsHookExW(WH_KEYBOARD_LL, Some(win_shift_s_proc), module as _, 0);
                        if fresh.is_null() {
                            hook_log("hook refresh FAILED".into());
                        } else {
                            UnhookWindowsHookEx(hook);
                            hook = fresh;
                        }
                        continue;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
    }

    /// Appends a line to %APPDATA%\Rimelight Media\hook-log.txt (from a background thread; the hook never waits).
    fn hook_log(line: String) {
        use std::sync::{Mutex, OnceLock, mpsc};
        static TX: OnceLock<Mutex<mpsc::Sender<String>>> = OnceLock::new();
        let tx = TX.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<String>();
            std::thread::spawn(move || {
                use std::io::Write as _;
                let path = crate::prefs::data_dir().join("hook-log.txt");
                for line in rx {
                    let t = local_time();
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                        let _ = writeln!(f, "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {line}", t[0], t[1], t[2], t[3], t[4], t[5]);
                    }
                }
            });
            Mutex::new(tx)
        });
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(line);
        }
    }

    static HOOK_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static WIN_S_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static SWALLOW_S_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static WIN_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static SHIFT_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// Swallows Win+Shift+S and flags it for the app to snip. The Win and Shift states are tracked here from the hook's
    /// own events (the low-level hook sees VK_LSHIFT/VK_RSHIFT, and the async key state lags behind it). The Win key-up
    /// is passed on: Shift was pressed in between, so the Start menu stays shut.
    unsafe extern "system" fn win_shift_s_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        use std::sync::atomic::Ordering;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_LSHIFT, VK_LWIN, VK_RSHIFT, VK_RWIN, VK_SHIFT};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CallNextHookEx, HC_ACTION, KBDLLHOOKSTRUCT, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        };
        if code == HC_ACTION as i32 {
            // SAFETY: for HC_ACTION, lparam points to a KBDLLHOOKSTRUCT that Windows keeps alive for the call.
            let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let msg = wparam as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            let vk = kb.vkCode;
            if vk == VK_LWIN as u32 || vk == VK_RWIN as u32 {
                WIN_HELD.store(down, Ordering::SeqCst);
            } else if vk == VK_LSHIFT as u32 || vk == VK_RSHIFT as u32 || vk == VK_SHIFT as u32 {
                SHIFT_HELD.store(down, Ordering::SeqCst);
            } else if vk == b'S' as u32 {
                let (win, shift) = (WIN_HELD.load(Ordering::SeqCst), SHIFT_HELD.load(Ordering::SeqCst));
                if down && (win || shift) {
                    // Diagnostics for the Win+Shift+S cases that still reach Windows' own snipping: what this hook saw
                    // against the system's key state. Sent to a logger thread so the hook itself stays fast.
                    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
                    // SAFETY: plain key-state queries.
                    let async_win = unsafe { GetAsyncKeyState(VK_LWIN as i32) < 0 || GetAsyncKeyState(VK_RWIN as i32) < 0 };
                    // SAFETY: as above.
                    let async_shift = unsafe { GetAsyncKeyState(VK_SHIFT as i32) < 0 };
                    hook_log(format!(
                        "S down: tracked win={win} shift={shift}, system win={async_win} shift={async_shift}, flags={:#x}, swallowed={}",
                        kb.flags,
                        win && shift
                    ));
                }
                if down && win && shift {
                    SWALLOW_S_UP.store(true, Ordering::SeqCst);
                    WIN_S_PENDING.store(true, Ordering::SeqCst);
                    return 1;
                }
                if up && SWALLOW_S_UP.swap(false, Ordering::SeqCst) {
                    return 1;
                }
            }
        }
        // SAFETY: passes the event on to the next hook, as the hook contract requires.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    /// True once after each Win+Shift+S.
    pub fn take_win_shift_s() -> bool {
        WIN_S_PENDING.swap(false, std::sync::atomic::Ordering::SeqCst)
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
    pub fn top_windows(_: isize) -> Vec<TopWindow> {
        Vec::new()
    }
    pub fn file_types_changed() {}
    pub fn exclude_from_capture(_: isize) {}
    pub fn no_transitions(_: isize) {}
    pub fn start_win_shift_s_hook() {}
    pub fn take_win_shift_s() -> bool {
        false
    }
    pub fn cursor_monitor() -> Option<((i32, i32), ScreenRect)> {
        None
    }
    pub fn take_start_lock() -> bool {
        true
    }
    pub fn recycle(path: &std::path::Path) -> bool {
        std::fs::remove_file(path).is_ok()
    }
    pub fn open_uri(_: &str) -> bool {
        false
    }
    pub fn known_folder(_: super::UserFolder) -> Option<std::path::PathBuf> {
        None
    }
    pub fn set_menu_theme(_: i32) {}
    pub fn open_cast_panel() {}
    pub fn recognise_text(_: &[u8], _: u32, _: u32) -> Result<String, String> {
        Err("text recognition is Windows-only for now".into())
    }
    pub fn print_image(_: isize, _: &[u8], _: u32, _: u32, _: &str) -> Result<bool, String> {
        Err("printing is Windows-only for now".into())
    }
    pub fn share_file(_: isize, _: &std::path::Path, _: &str) -> Result<(), String> {
        Err("sharing is Windows-only for now".into())
    }
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

/// The pointer position and the rectangle of the monitor it is on (physical pixels).
pub fn cursor_monitor() -> Option<((i32, i32), ScreenRect)> {
    imp::cursor_monitor()
}

#[derive(Debug, Clone, Copy)]
pub enum UserFolder {
    Pictures,
    Videos,
    Music,
}

/// A user folder where Windows keeps it (OneDrive may move it); None when Windows doesn't say.
pub fn known_folder(which: UserFolder) -> Option<std::path::PathBuf> {
    imp::known_folder(which)
}

/// Pop-up menus (the tray's) in 1 Windows' theme, 2 dark or 3 light.
pub fn set_menu_theme(mode: i32) {
    imp::set_menu_theme(mode)
}

/// Windows' Cast panel (Win+K).
pub fn open_cast_panel() {
    imp::open_cast_panel()
}

/// Text in a picture (RGBA), with Windows' built-in text recognition.
pub fn recognise_text(rgba: &[u8], w: u32, h: u32) -> Result<String, String> {
    imp::recognise_text(rgba, w, h)
}

/// Opens a web address or a Windows settings link (ms-settings:...) with the system.
pub fn open_uri(uri: &str) -> bool {
    imp::open_uri(uri)
}

/// Prints a picture (RGBA) after Windows' print dialog; Ok(false) when cancelled.
pub fn print_image(owner: isize, rgba: &[u8], w: u32, h: u32, title: &str) -> Result<bool, String> {
    imp::print_image(owner, rgba, w, h, title)
}

/// Opens Windows' Share panel for a picture file.
pub fn share_file(owner: isize, path: &std::path::Path, title: &str) -> Result<(), String> {
    imp::share_file(owner, path, title)
}

/// Moves a file to the Recycle Bin (undoable from there). True when it went.
pub fn recycle(path: &std::path::Path) -> bool {
    imp::recycle(path)
}

/// True for the first Rimelight Media process of this user (the one that becomes the running copy).
pub fn take_start_lock() -> bool {
    imp::take_start_lock()
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

/// Visible top-level windows front-most first: other apps' windows, and our own `keep` window (the main window).
pub fn top_windows(keep: isize) -> Vec<TopWindow> {
    imp::top_windows(keep)
}

/// Turns off Windows' animations for one of our windows, so it appears at once.
pub fn no_transitions(hwnd: isize) {
    imp::no_transitions(hwnd)
}

/// Starts the Win+Shift+S hook (once). Win+Shift+S is then swallowed and reported by `take_win_shift_s`.
pub fn start_win_shift_s_hook() {
    imp::start_win_shift_s_hook()
}

/// True once after each Win+Shift+S that the hook swallowed.
pub fn take_win_shift_s() -> bool {
    imp::take_win_shift_s()
}

/// Tells Explorer that file associations changed, so icons and "Open with" refresh.
pub fn file_types_changed() {
    imp::file_types_changed()
}
