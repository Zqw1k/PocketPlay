//! Interactive regression coverage using only two disposable, owned Win32 windows.
//! Run this test on an unlocked desktop, serially, with PocketPlay itself stopped.
//! Tagged SendInput events exercise the real low-level hook. The tag exception
//! exists only in the test executable; production still ignores injected input.
use super::*;
use std::sync::{atomic::AtomicUsize, mpsc::Receiver};
use std::time::Instant;

// Keep this within 32 bits: desktop injection/forwarding may truncate extra-info.
pub(super) const TEST_MOUSE_TAG: usize = 0x5050_2425;
const WM_STOP: u32 = 0x8000 + 37;

#[repr(C)]
#[derive(Clone, Copy)]
struct MouseInput {
    dx: i32,
    dy: i32,
    data: u32,
    flags: u32,
    time: u32,
    extra: usize,
}
#[repr(C)]
union InputData {
    mouse: MouseInput,
}
#[repr(C)]
struct Input {
    kind: u32,
    data: InputData,
}
#[repr(C)]
struct CreateStruct {
    param: Handle,
    instance: Handle,
    menu: Handle,
    parent: Handle,
    height: i32,
    width: i32,
    y: i32,
    x: i32,
    style: i32,
    name: *const u16,
    class: *const u16,
    ex_style: u32,
}
#[link(name = "user32")]
unsafe extern "system" {
    fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
    fn GetWindowRect(hwnd: Handle, rect: *mut Rect) -> i32;
    fn GetWindowLongPtrW(hwnd: Handle, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: Handle, index: i32, value: isize) -> isize;
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetSystemMetrics(index: i32) -> i32;
    fn SystemParametersInfoW(action: u32, param: u32, value: Handle, flags: u32) -> i32;
    fn IsWindow(hwnd: Handle) -> i32;
    fn IsIconic(hwnd: Handle) -> i32;
    fn IsZoomed(hwnd: Handle) -> i32;
    fn PostMessageW(hwnd: Handle, message: u32, wparam: usize, lparam: isize) -> i32;
    fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
    fn BringWindowToTop(hwnd: Handle) -> i32;
    fn SetWindowPos(
        hwnd: Handle,
        after: Handle,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    fn SetProcessDpiAwarenessContext(context: Handle) -> i32;
    fn SetThreadDpiAwarenessContext(context: Handle) -> Handle;
}
#[link(name = "dwmapi", kind = "raw-dylib")]
unsafe extern "system" {
    fn DwmGetWindowAttribute(hwnd: Handle, attribute: u32, value: *mut c_void, size: u32) -> i32;
}

#[derive(Default)]
struct WindowState {
    client_down: AtomicUsize,
    client_up: AtomicUsize,
    nc_down: AtomicUsize,
    nc_hit: AtomicUsize,
    close: AtomicUsize,
    sys_command: AtomicUsize,
}
unsafe extern "system" fn fixture_proc(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize {
    if msg == 0x81 {
        let create = &*(l as *const CreateStruct);
        SetWindowLongPtrW(hwnd, -21, create.param as isize);
    }
    let state = GetWindowLongPtrW(hwnd, -21) as *const WindowState;
    if !state.is_null() {
        let state = &*state;
        match msg {
            0x201 => {
                state.client_down.fetch_add(1, Ordering::SeqCst);
            }
            0x202 => {
                state.client_up.fetch_add(1, Ordering::SeqCst);
            }
            0xa1 => {
                state.nc_down.fetch_add(1, Ordering::SeqCst);
                state.nc_hit.store(w, Ordering::SeqCst);
            }
            0x112 => {
                state.sys_command.store(w, Ordering::SeqCst);
            }
            0x10 => {
                state.close.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, msg, w, l)
}

struct DesktopRestore {
    foreground: isize,
    point: Point,
}
impl DesktopRestore {
    fn new() -> Self {
        Self {
            foreground: unsafe { GetForegroundWindow() as isize },
            point: cursor(),
        }
    }
}
impl Drop for DesktopRestore {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.foreground as Handle) != 0 {
                foreground(self.foreground);
            }
        }
        set_cursor(self.point);
    }
}

struct TestWindows {
    hwnd: [isize; 2],
    state: [Arc<WindowState>; 2],
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl TestWindows {
    fn create() -> Self {
        let state = [
            Arc::new(WindowState::default()),
            Arc::new(WindowState::default()),
        ];
        let thread_state = state.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Result<[isize; 2], String>>();
        let handle = thread::spawn(move || unsafe {
            SetThreadDpiAwarenessContext(-4isize as Handle);
            let module = GetModuleHandleW(ptr::null());
            let class_name = wide(&format!("PocketPlayRegression{}", GetCurrentThreadId()));
            let class = WindowClass {
                style: 0,
                proc: Some(fixture_proc),
                cls_extra: 0,
                window_extra: 0,
                instance: module,
                icon: ptr::null_mut(),
                cursor: LoadCursorW(ptr::null_mut(), 32512usize as *const u16),
                background: 6usize as Handle,
                menu: ptr::null(),
                name: class_name.as_ptr(),
            };
            if RegisterClassW(&class) == 0 {
                let _ = tx.send(Err(format!("RegisterClassW: {}", GetLastError())));
                return;
            }
            let work = work_area();
            let width = ((work.right - work.left - 60) / 2).clamp(320, 600);
            let height = (work.bottom - work.top - 100).clamp(250, 420);
            let mut hwnd = [0isize; 2];
            for i in 0..2 {
                let title = wide(if i == 0 {
                    "PocketPlay regression mirror"
                } else {
                    "PocketPlay regression other app"
                });
                hwnd[i] = CreateWindowExW(
                    0,
                    class_name.as_ptr(),
                    title.as_ptr(),
                    0x00cf0000,
                    work.left + 20 + i as i32 * (width + 20),
                    work.top + 30,
                    width,
                    height,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    module,
                    Arc::as_ptr(&thread_state[i]) as Handle,
                ) as isize;
                if hwnd[i] == 0 {
                    let _ = tx.send(Err(format!("CreateWindowExW: {}", GetLastError())));
                    for h in hwnd {
                        if h != 0 {
                            DestroyWindow(h as Handle);
                        }
                    }
                    UnregisterClassW(class_name.as_ptr(), module);
                    return;
                }
                ShowWindow(hwnd[i] as Handle, 1);
                // Keep both owned targets visible above the user's existing
                // maximized windows without activating either target here.
                SetWindowPos(hwnd[i] as Handle, -1isize as Handle, 0, 0, 0, 0, 0x13);
            }
            let _ = tx.send(Ok(hwnd));
            while !thread_stop.load(Ordering::SeqCst) {
                let mut message: Message = zeroed();
                while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, 1) != 0 {
                    if message.message == WM_STOP {
                        thread_stop.store(true, Ordering::SeqCst);
                    }
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                MsgWaitForMultipleObjectsEx(0, ptr::null(), 15, 0x04ff, 4);
            }
            for h in hwnd {
                if IsWindow(h as Handle) != 0 {
                    DestroyWindow(h as Handle);
                }
            }
            UnregisterClassW(class_name.as_ptr(), module);
        });
        let ready = rx.recv_timeout(Duration::from_secs(5));
        match ready {
            Ok(Ok(hwnd)) => Self {
                hwnd,
                state,
                stop,
                thread: Some(handle),
            },
            error => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                panic!("Could not create regression windows: {error:?}");
            }
        }
    }
}
impl Drop for TestWindows {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        unsafe {
            PostMessageW(self.hwnd[1] as Handle, WM_STOP, 0, 0);
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn work_area() -> Rect {
    let mut rect = Rect::default();
    assert_ne!(
        unsafe { SystemParametersInfoW(0x30, 0, &mut rect as *mut _ as Handle, 0) },
        0
    );
    rect
}
fn frame(hwnd: isize) -> Rect {
    let mut rect = Rect::default();
    assert_ne!(unsafe { GetWindowRect(hwnd as Handle, &mut rect) }, 0);
    rect
}
fn center(hwnd: isize) -> Point {
    let r = client_bounds(hwnd).expect("Fixture must have a client area");
    Point {
        x: (r.left + r.right) / 2,
        y: (r.top + r.bottom) / 2,
    }
}
fn wait_for(label: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(predicate(), "Timed out: {label}");
}
fn focus(hwnd: isize) {
    unsafe {
        ShowWindow(hwnd as Handle, 9);
    }
    foreground(hwnd);
    wait_for("owned fixture foreground", || focused(hwnd));
}
fn foreground(hwnd: isize) {
    unsafe {
        SetForegroundWindow(hwnd as Handle);
        if focused(hwnd) {
            return;
        }
        // PeekMessage creates this caller's queue before input attachment.
        let mut message: Message = zeroed();
        PeekMessageW(&mut message, ptr::null_mut(), 0, 0, 0);
        let current = GetCurrentThreadId();
        let active = GetWindowThreadProcessId(GetForegroundWindow(), ptr::null_mut());
        let attached =
            active != 0 && active != current && AttachThreadInput(current, active, 1) != 0;
        SetForegroundWindow(hwnd as Handle);
        BringWindowToTop(hwnd as Handle);
        if attached {
            AttachThreadInput(current, active, 0);
        }
    }
}
fn own_pointer(hwnd: isize, point: Point) {
    set_cursor(point);
    wait_for("pointer target belongs to fixture", || unsafe {
        GetAncestor(WindowFromPoint(point), 2) as isize == hwnd
    });
}
fn inject(flags: u32, dx: i32, dy: i32) {
    let input = Input {
        kind: 0,
        data: InputData {
            mouse: MouseInput {
                dx,
                dy,
                data: 0,
                flags,
                time: 0,
                extra: TEST_MOUSE_TAG,
            },
        },
    };
    assert_eq!(
        unsafe { SendInput(1, &input, size_of::<Input>() as i32) },
        1,
        "Tagged SendInput failed: {}",
        unsafe { GetLastError() }
    );
}
struct MousePress(bool);
impl MousePress {
    fn down() -> Self {
        inject(2, 0, 0);
        Self(true)
    }
    fn release(mut self) {
        inject(4, 0, 0);
        self.0 = false;
    }
}
impl Drop for MousePress {
    fn drop(&mut self) {
        if self.0 {
            let input = Input {
                kind: 0,
                data: InputData {
                    mouse: MouseInput {
                        dx: 0,
                        dy: 0,
                        data: 0,
                        flags: 4,
                        time: 0,
                        extra: TEST_MOUSE_TAG,
                    },
                },
            };
            unsafe {
                SendInput(1, &input, size_of::<Input>() as i32);
            }
        }
    }
}
fn click(hwnd: isize, point: Point) {
    own_pointer(hwnd, point);
    inject(1, 0, 0);
    thread::sleep(Duration::from_millis(100));
    let press = MousePress::down();
    thread::sleep(Duration::from_millis(80));
    press.release();
    thread::sleep(Duration::from_millis(80));
}
fn move_input(point: Point) {
    let work = work_area();
    assert!(
        point.x >= work.left
            && point.x < work.right
            && point.y >= work.top
            && point.y < work.bottom
    );
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(76),
            GetSystemMetrics(77),
            GetSystemMetrics(78),
            GetSystemMetrics(79),
        )
    };
    let x = ((i64::from(point.x - left) * 65535) / i64::from(width - 1)) as i32;
    let y = ((i64::from(point.y - top) * 65535) / i64::from(height - 1)) as i32;
    inject(0x8000 | 0x4000 | 1, x, y);
}
fn hit(hwnd: isize, point: Point) -> usize {
    let packed = (point.x as u16 as u32) | ((point.y as u16 as u32) << 16);
    let mut result = 0;
    assert_ne!(
        unsafe {
            SendMessageTimeoutW(hwnd as Handle, 0x84, 0, packed as isize, 3, 50, &mut result)
        },
        0
    );
    result
}
fn hit_point(hwnd: isize, wanted: usize) -> Point {
    let r = frame(hwnd);
    if matches!(wanted, 8 | 9 | 20) {
        let mut buttons = Rect::default();
        // Native DefWindowProc's manually queried hit regions can disagree with
        // the actual DWM caption buttons. Use their rendered bounds if present.
        let status = unsafe {
            DwmGetWindowAttribute(
                hwnd as Handle,
                5,
                &mut buttons as *mut _ as *mut c_void,
                size_of::<Rect>() as u32,
            )
        };
        if status >= 0 && buttons.right > buttons.left && buttons.bottom > buttons.top {
            let index = match wanted {
                8 => 0,
                9 => 1,
                _ => 2,
            };
            let width = buttons.right - buttons.left;
            let point = Point {
                x: r.left + buttons.left + width * (2 * index + 1) / 6,
                y: r.top + (buttons.top + buttons.bottom) / 2,
            };
            return point;
        }
    }
    let scan = if wanted == 17 {
        Rect {
            left: r.right - 20,
            top: r.bottom - 20,
            right: r.right,
            bottom: r.bottom,
        }
    } else if wanted == 2 {
        Rect {
            left: r.left + 50,
            top: r.top,
            right: r.right - 180,
            bottom: r.top + 65,
        }
    } else {
        Rect {
            left: r.right - 200,
            top: r.top,
            right: r.right,
            bottom: r.top + 65,
        }
    };
    let mut points = Vec::new();
    for y in (scan.top..scan.bottom).step_by(3) {
        for x in (scan.left..scan.right).step_by(3) {
            let p = Point { x, y };
            if hit(hwnd, p) == wanted {
                points.push(p);
            }
        }
    }
    assert!(!points.is_empty(), "Fixture has no hit-test area {wanted}");
    let point = Point {
        x: points.iter().map(|p| p.x).sum::<i32>() / points.len() as i32,
        y: points.iter().map(|p| p.y).sum::<i32>() / points.len() as i32,
    };
    assert_eq!(hit(hwnd, point), wanted);
    point
}
fn drain(rx: &Receiver<Event>) -> Vec<Event> {
    rx.try_iter().collect()
}
fn button(rx: &Receiver<Event>, down: bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = rx
            .recv_timeout(remaining)
            .expect("Expected intercepted client button edge");
        if let Event::Button {
            vk: 1, down: edge, ..
        } = event
        {
            assert_eq!(edge, down, "Client mouse edges must retain their order");
            return;
        }
    }
}
fn arm(shared: &Shared, rx: &Receiver<Event>) {
    shared.enabled.store(true, Ordering::SeqCst);
    shared.capture.store(false, Ordering::SeqCst);
    drain(rx);
}
fn no_buttons(rx: &Receiver<Event>) {
    assert!(
        !drain(rx).iter().any(|e| matches!(e, Event::Button { .. })),
        "A Windows non-client or other-window click reached phone input"
    );
}

#[test]
fn windows_client_non_client_and_focus_loss_regression() {
    unsafe {
        SetProcessDpiAwarenessContext(-4isize as Handle);
        SetThreadDpiAwarenessContext(-4isize as Handle);
    }
    assert_eq!(
        size_of::<Input>(),
        if size_of::<usize>() == 8 { 40 } else { 28 }
    );
    assert_eq!(
        unsafe { GetAsyncKeyState(1) } & i16::MIN,
        0,
        "Run with no mouse button held"
    );
    let _desktop = DesktopRestore::new();
    let windows = TestWindows::create();
    let mirror = windows.hwnd[0];
    let other = windows.hwnd[1];
    let (tx, rx) = std::sync::mpsc::channel();
    let shared = Arc::new(Shared::default());
    shared.mirror.store(mirror, Ordering::SeqCst);
    let _hooks = NativeHooks::start(tx, shared.clone()).expect("Real native hooks must register");

    // Actual client input is intercepted by the installed hook, on both edges.
    focus(mirror);
    arm(&shared, &rx);
    click(mirror, center(mirror));
    button(&rx, true);
    button(&rx, false);
    assert_eq!(windows.state[0].client_down.load(Ordering::SeqCst), 0);
    assert_eq!(windows.state[0].client_up.load(Ordering::SeqCst), 0);
    eprintln!("Native regression: client DOWN/UP intercepted");

    // Foreground is still the mirror at DOWN time; the destination determines
    // ownership. This models selecting another app, without touching Explorer.
    arm(&shared, &rx);
    click(other, center(other));
    wait_for("other-window click activation", || focused(other));
    assert_eq!(windows.state[1].client_down.load(Ordering::SeqCst), 1);
    assert_eq!(windows.state[1].client_up.load(Ordering::SeqCst), 1);
    no_buttons(&rx);
    assert!(!shared.enabled.load(Ordering::SeqCst));
    eprintln!("Native regression: other owned window receives complete click");

    // A captured touch ends even outside the mirror, without leaking a lone UP.
    focus(mirror);
    arm(&shared, &rx);
    own_pointer(mirror, center(mirror));
    let held = MousePress::down();
    button(&rx, true);
    own_pointer(other, center(other));
    held.release();
    button(&rx, false);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(windows.state[1].client_up.load(Ordering::SeqCst), 1);
    assert_eq!(windows.state[0].client_up.load(Ordering::SeqCst), 0);
    eprintln!("Native regression: captured outside release does not leak desktop UP");

    // Use the rendered DWM caption bounds, with WM_NCHITTEST as a fallback.
    // Native DefWindowProc receives actual clicks with the hooks running.
    arm(&shared, &rx);
    click(mirror, hit_point(mirror, 8));
    wait_for("title-bar minimize", || unsafe {
        IsIconic(mirror as Handle) != 0
    });
    assert_eq!(windows.state[0].nc_hit.load(Ordering::SeqCst), 8);
    assert_eq!(
        windows.state[0].sys_command.load(Ordering::SeqCst) & 0xfff0,
        0xf020
    );
    no_buttons(&rx);
    focus(mirror);
    arm(&shared, &rx);
    click(mirror, hit_point(mirror, 9));
    wait_for("title-bar maximize", || unsafe {
        IsZoomed(mirror as Handle) != 0
    });
    no_buttons(&rx);
    arm(&shared, &rx);
    click(mirror, hit_point(mirror, 9));
    wait_for("title-bar restore", || unsafe {
        IsZoomed(mirror as Handle) == 0
    });
    no_buttons(&rx);
    eprintln!("Native regression: minimize/maximize/restore buttons work");

    focus(mirror);
    arm(&shared, &rx);
    let before = frame(mirror);
    let from = hit_point(mirror, 2);
    own_pointer(mirror, from);
    let held = MousePress::down();
    thread::sleep(Duration::from_millis(80));
    move_input(Point {
        x: from.x + 48,
        y: from.y + 32,
    });
    thread::sleep(Duration::from_millis(100));
    held.release();
    wait_for("title-bar drag", || {
        let r = frame(mirror);
        r.left > before.left + 8 && r.top > before.top + 8
    });
    no_buttons(&rx);

    arm(&shared, &rx);
    let before = frame(mirror);
    let from = hit_point(mirror, 17);
    own_pointer(mirror, from);
    let held = MousePress::down();
    thread::sleep(Duration::from_millis(80));
    move_input(Point {
        x: from.x + 60,
        y: from.y + 40,
    });
    thread::sleep(Duration::from_millis(100));
    held.release();
    wait_for("native resize", || {
        let r = frame(mirror);
        r.right - r.left > before.right - before.left + 20
            && r.bottom - r.top > before.bottom - before.top + 20
    });
    no_buttons(&rx);
    eprintln!("Native regression: title-bar drag and border resize work");

    // Losing focus must not warp the pointer back into the phone window.
    focus(mirror);
    shared.capture.store(false, Ordering::SeqCst);
    let old = center(mirror);
    set_cursor(Point {
        x: old.x - 30,
        y: old.y - 30,
    });
    let capture = CursorCapture::start(mirror);
    thread::sleep(Duration::from_millis(80));
    focus(other);
    let external = center(other);
    set_cursor(external);
    drop(capture);
    let actual = cursor();
    assert_eq!(
        (actual.x, actual.y),
        (external.x, external.y),
        "Focus-loss cleanup warped the external pointer"
    );
    eprintln!("Native regression: focus-loss CursorCapture cleanup preserves pointer");

    focus(mirror);
    arm(&shared, &rx);
    click(mirror, hit_point(mirror, 20));
    wait_for("title-bar close", || unsafe {
        IsWindow(mirror as Handle) == 0
    });
    assert_eq!(windows.state[0].close.load(Ordering::SeqCst), 1);
    no_buttons(&rx);
    assert!(windows.state[0].nc_down.load(Ordering::SeqCst) >= 5);
    eprintln!("Native regression: close button works; desktop state will be restored");
}
