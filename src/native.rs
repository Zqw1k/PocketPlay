//! Windows input and window integration. Raw Input owns relative mouse movement.
use anyhow::{bail, Result};
use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering},
        mpsc::Sender,
        Arc, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
type Handle = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
#[repr(C)]
struct Message {
    hwnd: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: Point,
    private: u32,
}
#[repr(C)]
struct WindowClass {
    style: u32,
    proc: Option<unsafe extern "system" fn(Handle, u32, usize, isize) -> isize>,
    cls_extra: i32,
    window_extra: i32,
    instance: Handle,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu: *const u16,
    name: *const u16,
}
#[repr(C)]
struct RawDevice {
    page: u16,
    usage: u16,
    flags: u32,
    target: Handle,
}
#[repr(C)]
struct MouseHook {
    pt: Point,
    data: u32,
    flags: u32,
    time: u32,
    extra: usize,
}
#[repr(C)]
struct KeyHook {
    vk: u32,
    scan: u32,
    flags: u32,
    time: u32,
    extra: usize,
}
#[repr(C)]
struct CursorInfo {
    size: u32,
    flags: u32,
    cursor: Handle,
    point: Point,
}
#[link(name = "user32")]
unsafe extern "system" {
    fn GetForegroundWindow() -> Handle;
    fn WindowFromPoint(point: Point) -> Handle;
    fn GetAncestor(hwnd: Handle, flags: u32) -> Handle;
    fn SendMessageTimeoutW(
        hwnd: Handle,
        message: u32,
        wparam: usize,
        lparam: isize,
        flags: u32,
        timeout: u32,
        result: *mut usize,
    ) -> isize;
    fn FindWindowW(class: *const u16, title: *const u16) -> Handle;
    fn GetClientRect(hwnd: Handle, rect: *mut Rect) -> i32;
    fn ClientToScreen(hwnd: Handle, point: *mut Point) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn GetWindowThreadProcessId(hwnd: Handle, pid: *mut u32) -> u32;
    fn AttachThreadInput(a: u32, b: u32, attach: i32) -> i32;
    fn ShowCursor(show: i32) -> i32;
    fn GetCursorInfo(info: *mut CursorInfo) -> i32;
    fn ShowWindow(hwnd: Handle, show: i32) -> i32;
    fn SetForegroundWindow(hwnd: Handle) -> i32;
    fn SetWindowsHookExW(
        id: i32,
        callback: Option<unsafe extern "system" fn(i32, usize, isize) -> isize>,
        module: Handle,
        thread: u32,
    ) -> Handle;
    fn CallNextHookEx(hook: Handle, code: i32, wparam: usize, lparam: isize) -> isize;
    fn UnhookWindowsHookEx(hook: Handle) -> i32;
    fn RegisterClassW(class: *const WindowClass) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Handle,
        menu: Handle,
        instance: Handle,
        param: Handle,
    ) -> Handle;
    fn DefWindowProcW(hwnd: Handle, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn RegisterRawInputDevices(devices: *const RawDevice, count: u32, size: u32) -> i32;
    fn GetRawInputData(
        input: Handle,
        command: u32,
        data: *mut c_void,
        size: *mut u32,
        header_size: u32,
    ) -> u32;
    fn PeekMessageW(msg: *mut Message, hwnd: Handle, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const Message) -> i32;
    fn DispatchMessageW(msg: *const Message) -> isize;
    fn DestroyWindow(hwnd: Handle) -> i32;
    fn UnregisterClassW(class: *const u16, instance: Handle) -> i32;
    fn MsgWaitForMultipleObjectsEx(
        count: u32,
        handles: *const Handle,
        milliseconds: u32,
        wake_mask: u32,
        flags: u32,
    ) -> u32;
    fn VkKeyScanW(character: u16) -> i16;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn GetCurrentThreadId() -> u32;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn MoveFileExW(source: *const u16, dest: *const u16, flags: u32) -> i32;
    fn CreateMutexW(attributes: Handle, owner: i32, name: *const u16) -> Handle;
    fn GetLastError() -> u32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetCurrentThread() -> Handle;
    fn SetThreadPriority(thread: Handle, priority: i32) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn SetProcessInformation(process: Handle, class: i32, info: *const c_void, size: u32) -> i32;
}
#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(ms: u32) -> u32;
    fn timeEndPeriod(ms: u32) -> u32;
}
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub fn replace_file(source: &Path, dest: &Path) -> Result<()> {
    let a = wide(&source.to_string_lossy());
    let b = wide(&dest.to_string_lossy());
    if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 1 | 8) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

/// Uses the Windows system PowerShell and passes paths as data, never script text.
pub fn create_desktop_shortcut(exe: &Path) -> Result<()> {
    let directory = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Application directory is missing"))?;
    let mut buffer = [0u16; 32768];
    // SAFETY: the writable buffer has the advertised capacity; the API writes UTF-16.
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() {
        bail!("Could not locate the Windows system directory.");
    }
    let powershell = PathBuf::from(String::from_utf16_lossy(&buffer[..length]))
        .join("WindowsPowerShell/v1.0/powershell.exe");
    let script = concat!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut(",
        "(Join-Path ([Environment]::GetFolderPath('Desktop')) 'PocketPlay Native.lnk'));",
        "$s.TargetPath=$env:POCKETPLAY_SHORTCUT_EXE;",
        "$s.WorkingDirectory=$env:POCKETPLAY_SHORTCUT_DIR;",
        "$s.IconLocation=$env:POCKETPLAY_SHORTCUT_EXE+',0';$s.Save()"
    );
    let mut command = crate::process::hidden(&powershell);
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("POCKETPLAY_SHORTCUT_EXE", exe)
        .env("POCKETPLAY_SHORTCUT_DIR", directory);
    crate::process::output_timeout(command, Duration::from_secs(10), None)?;
    Ok(())
}
pub struct Instance(Handle);
impl Instance {
    pub fn acquire() -> Option<Self> {
        let name = wide("Local\\PocketPlayNativeDesktop");
        unsafe {
            let h = CreateMutexW(ptr::null_mut(), 0, name.as_ptr());
            if GetLastError() == 183 {
                let title = wide("PocketPlay Native");
                let hwnd = FindWindowW(ptr::null(), title.as_ptr());
                if !hwnd.is_null() {
                    ShowWindow(hwnd, 9);
                    SetForegroundWindow(hwnd);
                }
                CloseHandle(h);
                None
            } else if h.is_null() {
                None
            } else {
                Some(Self(h))
            }
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub fn mirror() -> isize {
    let title = wide("PocketPlay Android");
    unsafe { FindWindowW(ptr::null(), title.as_ptr()) as isize }
}
pub fn session_token() -> Result<String> {
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn SystemFunction036(buffer: *mut u8, length: u32) -> u8;
    }
    let mut bytes = [0u8; 16];
    // SAFETY: the OS writes exactly the supplied stack buffer length.
    if unsafe { SystemFunction036(bytes.as_mut_ptr(), bytes.len() as u32) } == 0 {
        bail!("Windows could not generate a session token");
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn focused(hwnd: isize) -> bool {
    hwnd != 0 && unsafe { GetForegroundWindow() as isize == hwnd }
}
pub fn focus_mirror() {
    let h = mirror() as Handle;
    if !h.is_null() {
        unsafe {
            ShowWindow(h, 9);
            SetForegroundWindow(h);
        }
    }
}
pub fn cursor() -> Point {
    let mut p = Point::default();
    unsafe {
        GetCursorPos(&mut p);
    }
    p
}
pub fn set_cursor(p: Point) {
    unsafe {
        SetCursorPos(p.x, p.y);
    }
}
pub fn client_bounds(hwnd: isize) -> Option<Rect> {
    let mut r = Rect::default();
    let mut p = Point::default();
    unsafe {
        if hwnd == 0
            || GetClientRect(hwnd as Handle, &mut r) == 0
            || ClientToScreen(hwnd as Handle, &mut p) == 0
        {
            return None;
        }
    }
    Some(Rect {
        left: p.x,
        top: p.y,
        right: p.x + r.right,
        bottom: p.y + r.bottom,
    })
}
fn point_in_client(hwnd: isize, point: Point) -> bool {
    client_bounds(hwnd).is_some_and(|r| {
        point.x >= r.left && point.x < r.right && point.y >= r.top && point.y < r.bottom
    })
}
fn client_under_pointer(hwnd: isize, point: Point) -> bool {
    if !point_in_client(hwnd, point) {
        return false;
    }
    // Foreground ownership alone is insufficient: a taskbar/title-bar click
    // arrives before Windows activates its destination. Respect the actual target.
    let target = unsafe { WindowFromPoint(point) };
    !target.is_null() && unsafe { GetAncestor(target, 2) as isize == hwnd }
}
fn client_button_target(hwnd: isize, point: Point) -> bool {
    if !client_under_pointer(hwnd, point) {
        return false;
    }
    let packed = (point.x as u16 as u32) | ((point.y as u16 as u32) << 16);
    let mut hit = 0;
    // Fail open if the target is hung. Query only button presses, never the
    // high-frequency Raw Input path. HTCLIENT excludes resize/title-bar controls.
    unsafe {
        SendMessageTimeoutW(hwnd as Handle, 0x84, 0, packed as isize, 3, 10, &mut hit) != 0
            && hit == 1
    }
}
pub fn touch_point(hwnd: isize, point: Point, size: (u16, u16)) -> Option<[f64; 2]> {
    let r = client_bounds(hwnd)?;
    map_touch_point(r, point, size)
}
pub fn map_touch_point(r: Rect, point: Point, size: (u16, u16)) -> Option<[f64; 2]> {
    let w = (r.right - r.left) as f64;
    let h = (r.bottom - r.top) as f64;
    if w < 2. || h < 2. || size.0 == 0 || size.1 == 0 {
        return None;
    }
    let [left, top, vw, vh] = crate::geometry::aspect_fit((w, h), size)?;
    let x = (point.x as f64 - r.left as f64 - left) / vw;
    let y = (point.y as f64 - r.top as f64 - top) / vh;
    if !(0.0..1.).contains(&x) || !(0.0..1.).contains(&y) {
        None
    } else {
        Some([x, y])
    }
}
pub fn key_vk(name: &str) -> Option<u16> {
    let k = name.trim().to_lowercase();
    if k.len() == 1 {
        let c = k.chars().next()?;
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase() as u16);
        }
        let v = unsafe { VkKeyScanW(c as u16) };
        return (v != -1).then_some((v as u16) & 255);
    }
    if let Some(n) = k.strip_prefix("vk_").and_then(|s| s.parse::<u16>().ok()) {
        return (n > 0 && n <= 255).then_some(canonical(n));
    }
    if let Some(n) = k.strip_prefix('f').and_then(|s| s.parse::<u16>().ok()) {
        return (1..=24).contains(&n).then_some(111 + n);
    }
    Some(match k.as_str() {
        "mouse_left" => 1,
        "mouse_right" => 2,
        "mouse_middle" => 4,
        "backspace" => 8,
        "tab" => 9,
        "enter" => 13,
        "shift" => 16,
        "ctrl" => 17,
        "alt" | "alt_gr" => 18,
        "pause" => 19,
        "caps_lock" => 20,
        "esc" => 27,
        "space" => 32,
        "page_up" => 33,
        "page_down" => 34,
        "end" => 35,
        "home" => 36,
        "left" => 37,
        "up" => 38,
        "right" => 39,
        "down" => 40,
        "print_screen" => 44,
        "insert" => 45,
        "delete" => 46,
        "cmd" => 91,
        "menu" => 93,
        "num_lock" => 144,
        "scroll_lock" => 145,
        "media_volume_mute" => 173,
        "media_volume_down" => 174,
        "media_volume_up" => 175,
        "media_next" => 176,
        "media_previous" => 177,
        "media_play_pause" => 179,
        _ => return None,
    })
}
fn canonical(v: u16) -> u16 {
    match v {
        160 | 161 => 16,
        162 | 163 => 17,
        164 | 165 => 18,
        _ => v,
    }
}
#[derive(Clone, Debug)]
pub enum Event {
    Key { vk: u16, down: bool },
    Button { vk: u16, down: bool, point: Point },
    Move(Point),
    Delta(f64, f64, Instant),
    Release,
    Toggle,
    Wake,
    Shutdown,
}
pub struct Shared {
    pub mirror: AtomicIsize,
    pub enabled: AtomicBool,
    pub capture: AtomicBool,
    pub alive: AtomicBool,
    mouse_buttons: AtomicU8,
    pub mapped: [AtomicBool; 256],
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            mirror: AtomicIsize::new(0),
            enabled: AtomicBool::new(false),
            capture: AtomicBool::new(false),
            alive: AtomicBool::new(true),
            mouse_buttons: AtomicU8::new(0),
            mapped: std::array::from_fn(|_| AtomicBool::new(false)),
        }
    }
}
static HOOK: OnceLock<(Sender<Event>, Arc<Shared>)> = OnceLock::new();
unsafe extern "system" fn keyboard_hook(code: i32, w: usize, l: isize) -> isize {
    if code >= 0 {
        if let Some((tx, s)) = HOOK.get() {
            if focused(s.mirror.load(Ordering::Relaxed)) {
                let k = &*(l as *const KeyHook);
                if k.flags & 0x10 == 0 {
                    let down = w == 0x100 || w == 0x104;
                    let up = w == 0x101 || w == 0x105;
                    if down || up {
                        let vk = canonical(k.vk as u16);
                        let _ = tx.send(Event::Key { vk, down });
                        if vk == 192
                            || (s.enabled.load(Ordering::Relaxed)
                                && s.mapped[vk.min(255) as usize].load(Ordering::Relaxed))
                        {
                            return 1;
                        }
                    }
                }
            }
        }
    }
    CallNextHookEx(ptr::null_mut(), code, w, l)
}
unsafe extern "system" fn mouse_hook(code: i32, w: usize, l: isize) -> isize {
    if code >= 0 {
        if let Some((tx, s)) = HOOK.get() {
            if s.alive.load(Ordering::Relaxed) {
                let hwnd = s.mirror.load(Ordering::Relaxed);
                let active = focused(hwnd);
                let m = &*(l as *const MouseHook);
                if physical_mouse(m) {
                    let button = match w {
                        0x201 => Some((1, true)),
                        0x202 => Some((1, false)),
                        0x204 => Some((2, true)),
                        0x205 => Some((2, false)),
                        0x207 => Some((4, true)),
                        0x208 => Some((4, false)),
                        _ => None,
                    };
                    if let Some((vk, down)) = button {
                        if route_button(
                            s,
                            vk,
                            down,
                            down && active && client_button_target(hwnd, m.pt),
                        ) {
                            let _ = tx.send(Event::Button {
                                vk,
                                down,
                                point: m.pt,
                            });
                            return 1;
                        }
                        if down && active {
                            // Leave Windows clicks intact and stop game input before
                            // focus changes. No cursor warp may interrupt this click.
                            s.enabled.store(false, Ordering::Relaxed);
                            s.capture.store(false, Ordering::Relaxed);
                            let _ = tx.send(Event::Release);
                        }
                    }
                    if w == 0x200 && active {
                        if s.capture.load(Ordering::Relaxed) {
                            // Use the current cursor, not the proposed physical
                            // displacement: fast camera sweeps can span the window.
                            if point_in_client(hwnd, cursor()) {
                                return 1;
                            }
                            s.capture.store(false, Ordering::Relaxed);
                            s.enabled.store(false, Ordering::Relaxed);
                            let _ = tx.send(Event::Release);
                        } else if s.mouse_buttons.load(Ordering::Relaxed) != 0 {
                            let _ = tx.send(Event::Move(m.pt));
                        }
                    }
                }
            }
        }
    }
    CallNextHookEx(ptr::null_mut(), code, w, l)
}
fn physical_mouse(m: &MouseHook) -> bool {
    if m.flags & 1 == 0 {
        return true;
    }
    // The regression executable exercises the installed hook with tagged
    // SendInput events. This exception is not compiled into the app.
    #[cfg(test)]
    if m.extra == native_regression::TEST_MOUSE_TAG {
        return true;
    }
    false
}
fn route_button(s: &Shared, vk: u16, down: bool, client: bool) -> bool {
    let bit = vk as u8; // VK_LBUTTON=1, VK_RBUTTON=2, VK_MBUTTON=4.
    if down {
        if client {
            s.mouse_buttons.fetch_or(bit, Ordering::Relaxed);
        }
        client
    } else {
        // Only intercept UP when PocketPlay intercepted the matching DOWN.
        // A touch drag released outside must end; a Windows click must keep both edges.
        s.mouse_buttons.fetch_and(!bit, Ordering::Relaxed) & bit != 0
    }
}
unsafe extern "system" fn window_proc(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize {
    if msg == 0xff {
        if let Some((tx, s)) = HOOK.get() {
            if s.capture.load(Ordering::Relaxed) && focused(s.mirror.load(Ordering::Relaxed)) {
                let mut size = 0;
                let header = 24;
                GetRawInputData(l as Handle, 0x10000003, ptr::null_mut(), &mut size, header);
                if (48..=4096).contains(&size) {
                    let mut b = [0u8; 4096];
                    let count = GetRawInputData(
                        l as Handle,
                        0x10000003,
                        b.as_mut_ptr() as _,
                        &mut size,
                        header,
                    );
                    if count == size {
                        if let Some((dx, dy)) = raw_delta(&b[..count as usize]) {
                            if dx != 0 || dy != 0 {
                                let _ = tx.send(Event::Delta(dx as f64, dy as f64, Instant::now()));
                            }
                        }
                    }
                }
            }
        }
    }
    DefWindowProcW(hwnd, msg, w, l)
}
pub fn raw_delta(bytes: &[u8]) -> Option<(i32, i32)> {
    if bytes.len() < 48
        || u32::from_ne_bytes(bytes[0..4].try_into().ok()?) != 0
        || u16::from_ne_bytes(bytes[24..26].try_into().ok()?) & 1 != 0
    {
        return None;
    }
    Some((
        i32::from_ne_bytes(bytes[36..40].try_into().ok()?),
        i32::from_ne_bytes(bytes[40..44].try_into().ok()?),
    ))
}
pub struct NativeHooks {
    pub shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
}
impl NativeHooks {
    pub fn start(tx: Sender<Event>, shared: Arc<Shared>) -> Result<Self> {
        if HOOK.set((tx, shared.clone())).is_err() {
            bail!("Input hooks already initialized.");
        }
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let s = shared.clone();
        let thread = thread::spawn(move || unsafe {
            let module = GetModuleHandleW(ptr::null());
            let name = wide("PocketPlayRawInput24");
            let class = WindowClass {
                style: 0,
                proc: Some(window_proc),
                cls_extra: 0,
                window_extra: 0,
                instance: module,
                icon: ptr::null_mut(),
                cursor: ptr::null_mut(),
                background: ptr::null_mut(),
                menu: ptr::null(),
                name: name.as_ptr(),
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                0,
                name.as_ptr(),
                name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                (-3isize) as Handle,
                ptr::null_mut(),
                module,
                ptr::null_mut(),
            );
            let raw = RawDevice {
                page: 1,
                usage: 2,
                flags: 0x100,
                target: hwnd,
            };
            let registered = RegisterRawInputDevices(&raw, 1, size_of::<RawDevice>() as u32) != 0;
            let key = SetWindowsHookExW(13, Some(keyboard_hook), module, 0);
            let mouse = SetWindowsHookExW(14, Some(mouse_hook), module, 0);
            let good = registered && !hwnd.is_null() && !key.is_null() && !mouse.is_null();
            let _ = ready_tx.send(good);
            if good {
                SetThreadPriority(GetCurrentThread(), 1);
                while s.alive.load(Ordering::Relaxed) {
                    let mut msg: Message = zeroed();
                    while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, 1) != 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    MsgWaitForMultipleObjectsEx(0, ptr::null(), 50, 0x04ff, 4);
                }
            }
            if !key.is_null() {
                UnhookWindowsHookEx(key);
            }
            if !mouse.is_null() {
                UnhookWindowsHookEx(mouse);
            }
            if !hwnd.is_null() {
                DestroyWindow(hwnd);
            }
            UnregisterClassW(name.as_ptr(), module);
        });
        if !ready_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or(false)
        {
            shared.alive.store(false, Ordering::Relaxed);
            let _ = thread.join();
            bail!("Windows could not register native input hooks.");
        }
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }
}
impl Drop for NativeHooks {
    fn drop(&mut self) {
        self.shared.capture.store(false, Ordering::Relaxed);
        self.shared.alive.store(false, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
pub struct CursorCapture {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    previous: Point,
    hwnd: isize,
    restore_position: bool,
}
impl CursorCapture {
    pub fn start(hwnd: isize) -> Self {
        let previous = cursor();
        if let Some(r) = client_bounds(hwnd) {
            set_cursor(Point {
                x: (r.left + r.right) / 2,
                y: (r.top + r.bottom) / 2,
            });
        }
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::spawn(move || unsafe {
            // AttachThreadInput requires both threads to have message queues.
            let mut msg: Message = zeroed();
            PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, 0);
            let target = GetWindowThreadProcessId(hwnd as Handle, ptr::null_mut());
            let current = GetCurrentThreadId();
            if AttachThreadInput(current, target, 1) == 0 {
                return;
            }
            let mut calls = 0;
            while !flag.load(Ordering::Relaxed) && focused(hwnd) {
                let mut info = CursorInfo {
                    size: size_of::<CursorInfo>() as u32,
                    flags: 0,
                    cursor: ptr::null_mut(),
                    point: Point::default(),
                };
                if GetCursorInfo(&mut info) != 0 && info.flags & 1 != 0 {
                    ShowCursor(0);
                    calls += 1;
                }
                thread::sleep(Duration::from_millis(35));
            }
            for _ in 0..calls {
                ShowCursor(1);
            }
            AttachThreadInput(current, target, 0);
        });
        Self {
            stop,
            thread: Some(thread),
            previous,
            hwnd,
            restore_position: false,
        }
    }
    pub fn restore_position_on_unlock(&mut self) {
        self.restore_position = true;
    }
}
impl Drop for CursorCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if self.restore_position && focused(self.hwnd) {
            set_cursor(self.previous);
        }
    }
}
pub struct TimerGuard;
impl TimerGuard {
    pub fn new() -> Self {
        unsafe {
            timeBeginPeriod(1);
            SetThreadPriority(GetCurrentThread(), 1);
            #[repr(C)]
            struct Policy {
                version: u32,
                control: u32,
                state: u32,
            }
            let p = Policy {
                version: 1,
                control: 4,
                state: 0,
            };
            SetProcessInformation(
                GetCurrentProcess(),
                4,
                &p as *const _ as _,
                size_of::<Policy>() as u32,
            );
        }
        Self
    }
}
impl Drop for TimerGuard {
    fn drop(&mut self) {
        unsafe {
            timeEndPeriod(1);
        }
    }
}

#[cfg(test)]
#[path = "native_regression.rs"]
mod native_regression;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn touch_area_follows_video_ratio_window_size_and_black_borders() {
        let portrait = Rect {
            left: 741,
            top: 48,
            right: 1162,
            bottom: 984,
        };
        let p = map_touch_point(portrait, Point { x: 841, y: 282 }, (864, 1920)).unwrap();
        assert!((p[0] - 100. / 421.).abs() < 1e-9);
        assert!((p[1] - 2104. / 8420.).abs() < 1e-9);
        assert!(map_touch_point(portrait, Point { x: 1162, y: 984 }, (864, 1920)).is_none());
        // Landscape in a square window: top/bottom bands are not touchable.
        let square = Rect {
            left: 100,
            top: 50,
            right: 1100,
            bottom: 1050,
        };
        assert!(map_touch_point(square, Point { x: 600, y: 60 }, (1920, 864)).is_none());
        let center = map_touch_point(square, Point { x: 600, y: 550 }, (1920, 864)).unwrap();
        assert!(center.iter().all(|v| (v - 0.5).abs() < 1e-9));
        let edge = map_touch_point(square, Point { x: 850, y: 437 }, (1920, 864)).unwrap();
        assert!((edge[0] - 0.75).abs() < 1e-9);
        assert!((edge[1] - 112. / 450.).abs() < 1e-9);
        // Physical client coordinates after DPI scaling, on a negative-origin monitor.
        let scaled = Rect {
            left: -1300,
            top: -20,
            right: 200,
            bottom: 980,
        };
        let p = map_touch_point(scaled, Point { x: -550, y: 480 }, (864, 1920)).unwrap();
        assert!(p.iter().all(|v| (v - 0.5).abs() < 1e-9));
        assert!(map_touch_point(scaled, Point { x: -1299, y: 480 }, (864, 1920)).is_none());
    }
    #[test]
    fn raw_relative_mouse_layout() {
        let mut bytes = [0u8; 48];
        bytes[4..8].copy_from_slice(&48u32.to_ne_bytes());
        bytes[36..40].copy_from_slice(&(-42i32).to_ne_bytes());
        bytes[40..44].copy_from_slice(&13i32.to_ne_bytes());
        assert_eq!(raw_delta(&bytes), Some((-42, 13)));
        bytes[24] = 1;
        assert_eq!(raw_delta(&bytes), None);
        assert_eq!(raw_delta(&bytes[..20]), None);
    }
    #[test]
    fn bindings_and_modifiers() {
        assert_eq!(key_vk("w"), Some(87));
        assert_eq!(key_vk("vk_160"), Some(16));
        assert_eq!(key_vk("mouse_middle"), Some(4));
        assert_eq!(key_vk("f24"), Some(135));
        assert_eq!(key_vk("f25"), None);
        assert_eq!(key_vk("arbitrary"), None);
    }
    #[test]
    fn mouse_edges_keep_windows_clicks_and_outside_releases_separate() {
        let s = Shared::default();
        for vk in [1, 2, 4] {
            assert!(!route_button(&s, vk, true, false));
            // Even when the mirror becomes focused before release, an external
            // button press must retain its UP edge in Windows.
            assert!(!route_button(&s, vk, false, true));
            assert!(route_button(&s, vk, true, true));
            assert!(route_button(&s, vk, false, false));
            assert!(!route_button(&s, vk, false, true));
        }
        assert_eq!(s.mouse_buttons.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn multiple_mouse_buttons_preserve_independent_ownership() {
        let s = Shared::default();
        assert!(route_button(&s, 1, true, true));
        assert!(route_button(&s, 2, true, true));
        assert!(!route_button(&s, 4, true, false));
        assert_eq!(s.mouse_buttons.load(Ordering::Relaxed), 3);
        assert!(route_button(&s, 1, false, false));
        assert_eq!(s.mouse_buttons.load(Ordering::Relaxed), 2);
        assert!(!route_button(&s, 4, false, true));
        assert!(route_button(&s, 2, false, true));
        assert_eq!(s.mouse_buttons.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn production_injected_mouse_events_are_ignored() {
        let m = MouseHook {
            pt: Point::default(),
            data: 0,
            flags: 1,
            time: 0,
            extra: 0,
        };
        assert!(!physical_mouse(&m));
        let physical = MouseHook { flags: 0, ..m };
        assert!(physical_mouse(&physical));
    }
}
