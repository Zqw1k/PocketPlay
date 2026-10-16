//! Native input dispatch runs independently of egui and video rendering.
use crate::{
    bridge::{Touch, DOWN, MOVE, UP},
    config::Profile,
    geometry::Geometry,
    native::{self, CursorCapture, Event, NativeHooks, Point, Shared, TimerGuard},
};
use anyhow::Result;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const MOUSE_SEND_INTERVAL: Duration = Duration::from_millis(1);
const EVENT_BATCH_LIMIT: usize = 64;
const EVENT_BATCH_BUDGET: Duration = Duration::from_micros(500);

#[derive(Default, Clone)]
pub struct Metrics {
    pub mode: bool,
    pub packets: u64,
    pub mouse_samples: u64,
    pub send_ms: f64,
    pub queue_ms: f64,
    pub max_queue_ms: f64,
    pub max_send_ms: f64,
    pub camera_reanchors: u64,
    pub error: Option<String>,
    pub display_size: (u16, u16),
}
enum Command {
    Attach(Arc<Mutex<Touch>>, Profile, f64),
    Profile(Profile, f64),
    Detach(Sender<()>),
}
pub struct Engine {
    pub metrics: Arc<Mutex<Metrics>>,
    tx: Sender<Event>,
    commands: Sender<Command>,
    worker: Option<thread::JoinHandle<()>>,
    hooks: Option<NativeHooks>,
}
impl Engine {
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let (commands, control_rx) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let metrics = Arc::new(Mutex::new(Metrics::default()));
        let hooks = NativeHooks::start(tx.clone(), shared.clone())?;
        let s = shared.clone();
        let m = metrics.clone();
        let worker = thread::spawn(move || Worker::new(s, m).run(rx, control_rx));
        Ok(Self {
            metrics,
            tx,
            commands,
            worker: Some(worker),
            hooks: Some(hooks),
        })
    }
    pub fn attach(&self, touch: Arc<Mutex<Touch>>, profile: Profile, sensitivity: f64) {
        let _ = self
            .commands
            .send(Command::Attach(touch, profile, sensitivity));
        let _ = self.tx.send(Event::Wake);
    }
    pub fn profile(&self, profile: Profile, sensitivity: f64) {
        let _ = self.commands.send(Command::Profile(profile, sensitivity));
        let _ = self.tx.send(Event::Wake);
    }
    pub fn detach(&self) {
        let (tx, rx) = mpsc::channel();
        let _ = self.commands.send(Command::Detach(tx));
        let _ = self.tx.send(Event::Wake);
        let _ = rx.recv_timeout(Duration::from_secs(2));
    }
    pub fn toggle(&self) {
        native::focus_mirror();
        let _ = self.tx.send(Event::Toggle);
    }
    pub fn snapshot(&self) -> Metrics {
        self.metrics
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.detach();
        self.hooks.take();
        let _ = self.tx.send(Event::Shutdown);
        if let Some(t) = self.worker.take() {
            let _ = t.join();
        }
    }
}
struct Swipe {
    start: Instant,
    position: [f64; 2],
}
struct Worker {
    shared: Arc<Shared>,
    metrics: Arc<Mutex<Metrics>>,
    touch: Option<Arc<Mutex<Touch>>>,
    geometry_source: Option<Arc<AtomicU64>>,
    applied_geometry: u64,
    profile: Profile,
    right_sensitivity: f64,
    keys: HashSet<u16>,
    pointers: HashMap<u64, [f64; 2]>,
    swipes: HashMap<usize, Swipe>,
    swipe_down: HashSet<usize>,
    mode: bool,
    focused: bool,
    capture: Option<CursorCapture>,
    timer: Option<TimerGuard>,
    camera: [f64; 2],
    delta: [f64; 2],
    last_pointer: Point,
    delta_since: Option<Instant>,
    camera_motion_at: Option<Instant>,
    queue_ms: f64,
    max_queue_ms: f64,
    last_send: Instant,
}
impl Worker {
    fn new(shared: Arc<Shared>, metrics: Arc<Mutex<Metrics>>) -> Self {
        Self {
            shared,
            metrics,
            touch: None,
            geometry_source: None,
            applied_geometry: 0,
            profile: Profile::default(),
            right_sensitivity: 0.6,
            keys: HashSet::new(),
            pointers: HashMap::new(),
            swipes: HashMap::new(),
            swipe_down: HashSet::new(),
            mode: false,
            focused: false,
            capture: None,
            timer: None,
            camera: [0.5, 0.5],
            delta: [0., 0.],
            last_pointer: Point::default(),
            delta_since: None,
            camera_motion_at: None,
            queue_ms: 0.,
            max_queue_ms: 0.,
            last_send: Instant::now(),
        }
    }
    fn run(&mut self, rx: Receiver<Event>, commands: Receiver<Command>) {
        'worker: loop {
            for cmd in commands.try_iter() {
                match cmd {
                    Command::Attach(t, p, s) => {
                        self.release();
                        self.geometry_source =
                            Some(t.lock().unwrap_or_else(|e| e.into_inner()).geometry.clone());
                        self.applied_geometry = 0;
                        self.touch = Some(t);
                        self.queue_ms = 0.;
                        self.max_queue_ms = 0.;
                        self.profile = p;
                        self.right_sensitivity = s;
                        self.map_keys();
                        *self
                            .metrics
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Metrics::default();
                    }
                    Command::Profile(p, s) => {
                        self.release();
                        self.profile = p;
                        self.right_sensitivity = s;
                        self.map_keys();
                    }
                    Command::Detach(done) => {
                        self.release();
                        self.touch = None;
                        self.geometry_source = None;
                        self.applied_geometry = 0;
                        self.shared.mirror.store(0, Ordering::Relaxed);
                        let _ = done.send(());
                    }
                }
            }
            let hwnd = if self.touch.is_some() {
                native::mirror()
            } else {
                0
            };
            self.shared.mirror.store(hwnd, Ordering::Relaxed);
            let focused = native::focused(hwnd) && self.touch.is_some();
            if self.focused && !focused {
                self.release();
            }
            self.focused = focused;
            if focused && self.timer.is_none() {
                self.timer = Some(TimerGuard::new());
            } else if !focused {
                self.timer.take();
            }
            if let Err(error) = self.sync_geometry() {
                self.failure(error.to_string());
            }
            let timeout = if focused {
                if self.mode && self.delta != [0., 0.] {
                    MOUSE_SEND_INTERVAL.saturating_sub(self.last_send.elapsed())
                } else {
                    Duration::from_millis(2)
                }
            } else {
                Duration::from_millis(60)
            };
            match rx.recv_timeout(timeout) {
                Ok(Event::Shutdown) => break,
                Ok(event) => {
                    if let Err(e) = self.event(event) {
                        self.failure(e.to_string());
                    }
                    // Coalesce an existing backlog without repeating Windows
                    // window/focus discovery for every raw mouse sample. Keep
                    // button edges in order and bound the batch so a busy mouse
                    // cannot postpone dispatch or command/focus processing.
                    match self.drain_events(&rx) {
                        Ok(false) => break 'worker,
                        Ok(true) => {}
                        Err(e) => self.failure(e.to_string()),
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(_) => {}
            }
            if self.focused && self.touch.is_some() {
                if let Err(e) = self.tick() {
                    self.failure(e.to_string());
                }
            }
        }
        self.release();
        self.timer.take();
    }
    fn drain_events(&mut self, rx: &Receiver<Event>) -> Result<bool> {
        let batch_started = Instant::now();
        for _ in 1..EVENT_BATCH_LIMIT {
            if batch_started.elapsed() >= EVENT_BATCH_BUDGET {
                break;
            }
            let Ok(event) = rx.try_recv() else { break };
            if matches!(event, Event::Shutdown) {
                return Ok(false);
            }
            self.event(event)?;
        }
        Ok(true)
    }
    fn map_keys(&self) {
        for k in &self.shared.mapped {
            k.store(false, Ordering::Relaxed);
        }
        for c in &self.profile.controls {
            let names: Vec<&str> = match c.kind.as_str() {
                "button" | "swipe" => vec![&c.key],
                "joystick" => vec![&c.keys.up, &c.keys.down, &c.keys.left, &c.keys.right],
                _ => vec![],
            };
            for name in names {
                if let Some(v) = native::key_vk(name) {
                    self.shared.mapped[v as usize].store(true, Ordering::Relaxed);
                }
            }
        }
    }
    fn sync_geometry(&mut self) -> Result<()> {
        let Some(source) = &self.geometry_source else {
            return Ok(());
        };
        let encoded = source.load(Ordering::Acquire);
        if encoded == self.applied_geometry {
            return Ok(());
        }
        let Some(geometry) = Geometry::decode(encoded) else {
            return Ok(());
        };
        let resume = self.mode && self.focused;
        let held_toggle = self.keys.contains(&192);
        // End old-coordinate gestures before swapping both spaces atomically.
        self.release();
        if let Some(touch) = &self.touch {
            touch
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .apply_geometry(geometry);
        }
        self.applied_geometry = encoded;
        self.metrics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .display_size = geometry.display;
        if resume {
            self.set_mode(true)?;
        }
        if held_toggle {
            self.keys.insert(192);
        }
        Ok(())
    }
    fn send(&mut self, action: u8, id: u64, point: [f64; 2]) -> Result<()> {
        if let Some(t) = &self.touch {
            if t.lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(action, id, point)?
            {
                self.metrics
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .packets += 1;
            }
        }
        if action == UP {
            self.pointers.remove(&id);
        } else {
            self.pointers.insert(id, point);
        }
        Ok(())
    }
    fn release(&mut self) {
        // Stop consuming Windows input before any potentially slow socket writes.
        self.shared.enabled.store(false, Ordering::Relaxed);
        self.shared.capture.store(false, Ordering::Relaxed);
        let p: Vec<_> = self.pointers.iter().map(|(id, p)| (*id, *p)).collect();
        for (id, point) in p {
            if self.send(UP, id, point).is_err() {
                break;
            }
        }
        self.pointers.clear();
        self.keys.clear();
        self.swipes.clear();
        self.swipe_down.clear();
        self.delta = [0., 0.];
        self.delta_since = None;
        self.queue_ms = 0.;
        self.camera_motion_at = None;
        self.mode = false;
        self.capture.take();
        self.metrics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .mode = false;
    }
    fn failure(&mut self, error: String) {
        self.release();
        self.touch = None;
        self.geometry_source = None;
        self.metrics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .error = Some(error);
    }
    fn set_mode(&mut self, enabled: bool) -> Result<()> {
        if let Some(capture) = &mut self.capture {
            if native::focused(self.shared.mirror.load(Ordering::Relaxed)) {
                capture.restore_position_on_unlock();
            }
        }
        self.release();
        if enabled && self.focused && self.touch.is_some() {
            self.mode = true;
            self.shared.enabled.store(true, Ordering::Relaxed);
            self.metrics
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .mode = true;
            if let Some(c) = self
                .profile
                .controls
                .iter()
                .find(|c| c.kind == "camera")
                .cloned()
            {
                self.capture = Some(CursorCapture::start(
                    self.shared.mirror.load(Ordering::Relaxed),
                ));
                self.shared.capture.store(true, Ordering::Relaxed);
                self.arm_camera(&c)?;
            }
        }
        Ok(())
    }
    fn arm_camera(&mut self, c: &crate::config::Control) -> Result<()> {
        self.camera = c.point;
        if c.idle_release_ms == 0 {
            // Retain the pre-24.0.4 gesture lifetime by default. This avoids
            // restarting Android's drag recognition on pauses.
            self.send(DOWN, 900, self.camera)?;
        }
        Ok(())
    }
    fn size(&self) -> (u16, u16) {
        self.touch
            .as_ref()
            .map(|t| t.lock().unwrap_or_else(|e| e.into_inner()).size)
            .unwrap_or((2400, 1080))
    }
    fn event(&mut self, e: Event) -> Result<()> {
        if matches!(e, Event::Release) {
            self.release();
            return Ok(());
        }
        if !self.focused {
            return Ok(());
        }
        match e {
            Event::Toggle => self.set_mode(!self.mode)?,
            Event::Key { vk, down } => {
                if vk == 192 {
                    let first = down && !self.keys.contains(&192);
                    if down {
                        self.keys.insert(192);
                    } else {
                        self.keys.remove(&192);
                    }
                    if first {
                        self.set_mode(!self.mode)?;
                        self.keys.insert(192);
                    }
                } else {
                    if down {
                        self.keys.insert(vk);
                    } else {
                        self.keys.remove(&vk);
                    }
                    self.buttons()?;
                }
            }
            Event::Button { vk, down, point } => {
                self.last_pointer = point;
                if down {
                    self.keys.insert(vk);
                } else {
                    self.keys.remove(&vk);
                }
                if self.mode {
                    self.buttons()?;
                } else if let Some(p) = native::touch_point(
                    self.shared.mirror.load(Ordering::Relaxed),
                    point,
                    self.size(),
                ) {
                    if vk == 1 {
                        if down {
                            self.send(DOWN, 950, p)?;
                        } else if self.pointers.contains_key(&950) {
                            self.send(UP, 950, p)?;
                        }
                    }
                    if vk == 2 {
                        if down {
                            self.camera = p;
                            self.send(DOWN, 901, p)?;
                        } else if let Some(p) = self.pointers.get(&901).copied() {
                            self.send(UP, 901, p)?;
                        }
                    }
                } else if !down {
                    let id = if vk == 1 { 950 } else { 901 };
                    if let Some(p) = self.pointers.get(&id).copied() {
                        self.send(UP, id, p)?;
                    }
                }
            }
            Event::Move(point) => {
                if !self.mode {
                    if self.keys.contains(&1) {
                        if let Some(p) = native::touch_point(
                            self.shared.mirror.load(Ordering::Relaxed),
                            point,
                            self.size(),
                        ) {
                            if self.pointers.contains_key(&950) {
                                self.send(MOVE, 950, p)?;
                            }
                        }
                    }
                    if self.keys.contains(&2) && self.pointers.contains_key(&901) {
                        let dx = (point.x - self.last_pointer.x) as f64;
                        let dy = (point.y - self.last_pointer.y) as f64;
                        self.drag(dx, dy, 901, self.right_sensitivity, false, false)?;
                    }
                    self.last_pointer = point;
                }
            }
            Event::Delta(dx, dy, captured_at) if self.mode && self.capture.is_some() => {
                self.captured_delta(dx, dy, captured_at, Instant::now())?;
            }
            _ => {}
        }
        Ok(())
    }
    fn held(&self, name: &str) -> bool {
        native::key_vk(name).is_some_and(|k| self.keys.contains(&k))
    }
    fn buttons(&mut self) -> Result<()> {
        if !self.mode {
            return Ok(());
        }
        let controls = self.profile.controls.clone();
        for (i, c) in controls.iter().enumerate() {
            let id = 1000 + i as u64;
            match c.kind.as_str() {
                "button" => {
                    let held = self.held(&c.key);
                    let active = self.pointers.contains_key(&id);
                    if held && !active {
                        self.send(DOWN, id, c.point)?;
                    } else if !held && active {
                        self.send(UP, id, c.point)?;
                    }
                }
                "joystick" => {
                    let dx =
                        (self.held(&c.keys.right) as i32 - self.held(&c.keys.left) as i32) as f64;
                    let dy = (self.held(&c.keys.down) as i32 - self.held(&c.keys.up) as i32) as f64;
                    if dx == 0. && dy == 0. {
                        if let Some(p) = self.pointers.get(&id).copied() {
                            self.send(UP, id, p)?;
                        }
                    } else {
                        let length = (dx * dx + dy * dy).sqrt().max(1.);
                        let size = self.size();
                        let target = [
                            (c.point[0] + dx / length * c.radius * size.1 as f64 / size.0 as f64)
                                .clamp(0., 1.),
                            (c.point[1] + dy / length * c.radius).clamp(0., 1.),
                        ];
                        if !self.pointers.contains_key(&id) {
                            self.send(DOWN, id, c.point)?;
                        }
                        self.send(MOVE, id, target)?;
                    }
                }
                "swipe" => {
                    let held = self.held(&c.key);
                    if held && !self.swipe_down.contains(&i) && !self.swipes.contains_key(&i) {
                        self.send(DOWN, id, c.point)?;
                        self.swipes.insert(
                            i,
                            Swipe {
                                start: Instant::now(),
                                position: c.point,
                            },
                        );
                    }
                    if held {
                        self.swipe_down.insert(i);
                    } else {
                        self.swipe_down.remove(&i);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn mouse_delta(&mut self, dx: f64, dy: f64, now: Instant) -> Result<()> {
        if dx == 0. && dy == 0. {
            return Ok(());
        }
        // Check the gap before refreshing the timestamp, including when the
        // worker wakes on an event before its idle timeout tick has run.
        self.end_idle_camera(now)?;
        self.camera_motion_at = Some(now);
        self.delta[0] += dx;
        self.delta[1] += dy;
        self.delta_since.get_or_insert(now);
        Ok(())
    }
    fn captured_delta(
        &mut self,
        dx: f64,
        dy: f64,
        captured_at: Instant,
        received_at: Instant,
    ) -> Result<()> {
        self.mouse_delta(dx, dy, received_at)?;
        if dx != 0. || dy != 0. {
            self.queue_ms = received_at
                .saturating_duration_since(captured_at)
                .as_secs_f64()
                * 1000.;
            self.max_queue_ms = self.max_queue_ms.max(self.queue_ms);
            self.delta_since = Some(self.delta_since.unwrap_or(captured_at).min(captured_at));
        }
        Ok(())
    }
    fn end_idle_camera(&mut self, now: Instant) -> Result<()> {
        let Some(idle_ms) = self
            .profile
            .controls
            .iter()
            .find(|c| c.kind == "camera")
            .map(|c| c.idle_release_ms)
            .filter(|delay| *delay != 0)
        else {
            return Ok(());
        };
        if self.camera_motion_at.is_some_and(|last| {
            now.saturating_duration_since(last) >= Duration::from_millis(idle_ms as u64)
        }) {
            if let Some(point) = self.pointers.get(&900).copied() {
                self.send(UP, 900, point)?;
            }
            self.camera_motion_at = None;
            self.delta = [0., 0.];
            self.delta_since = None;
        }
        Ok(())
    }
    fn tick(&mut self) -> Result<()> {
        self.tick_at(Instant::now())
    }
    fn tick_at(&mut self, now: Instant) -> Result<()> {
        self.end_idle_camera(now)?;
        let swipes: Vec<_> = self.swipes.iter().map(|(i, s)| (*i, s.start)).collect();
        for (i, start) in swipes {
            let c = &self.profile.controls[i];
            let f = (start.elapsed().as_secs_f64() * 1000. / c.duration_ms as f64).min(1.);
            let p = [
                c.point[0] + (c.end[0] - c.point[0]) * f,
                c.point[1] + (c.end[1] - c.point[1]) * f,
            ];
            self.send(MOVE, 1000 + i as u64, p)?;
            if f >= 1. {
                self.send(UP, 1000 + i as u64, p)?;
                self.swipes.remove(&i);
            } else if let Some(s) = self.swipes.get_mut(&i) {
                s.position = p;
            }
        }
        if self.mode
            && (self.delta[0] != 0. || self.delta[1] != 0.)
            && now.saturating_duration_since(self.last_send) >= MOUSE_SEND_INTERVAL
        {
            if let Some((sensitivity, invert_y)) = self
                .profile
                .controls
                .iter()
                .find(|c| c.kind == "camera")
                .map(|c| (c.sensitivity, c.invert_y))
            {
                let d = self.delta;
                self.delta = [0., 0.];
                self.drag(d[0], d[1], 900, sensitivity, invert_y, true)?;
                // Base the next interval on completion, not the timestamp
                // taken before a potentially slow write or boundary sequence.
                self.last_send = now.max(Instant::now());
                if let Some(start) = self.delta_since.take() {
                    let mut m = self
                        .metrics
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    m.mouse_samples += 1;
                    m.send_ms = start.elapsed().as_secs_f64() * 1000.;
                    m.max_send_ms = m.max_send_ms.max(m.send_ms);
                    m.queue_ms = self.queue_ms;
                    m.max_queue_ms = self.max_queue_ms;
                }
            }
        }
        Ok(())
    }
    fn drag(
        &mut self,
        dx: f64,
        dy: f64,
        id: u64,
        sens: f64,
        invert: bool,
        wrap: bool,
    ) -> Result<()> {
        let size = self.size();
        let mut remain = [
            dx * sens / size.0 as f64,
            dy * sens / size.1 as f64 * if invert { -1. } else { 1. },
        ];
        if !wrap {
            self.camera = [
                (self.camera[0] + remain[0]).clamp(0.01, 0.99),
                (self.camera[1] + remain[1]).clamp(0.01, 0.99),
            ];
            return self.send(MOVE, id, self.camera);
        }
        let anchor = self
            .profile
            .controls
            .iter()
            .find(|c| c.kind == "camera")
            .map(|c| c.point)
            .unwrap_or([0.5, 0.5]);
        if !self.pointers.contains_key(&id) {
            self.camera = [anchor[0].clamp(0.045, 0.955), anchor[1].clamp(0.045, 0.955)];
            self.send(DOWN, id, self.camera)?;
        }
        for _ in 0..64 {
            let mut fraction: f64 = 1.;
            for (pos, d) in self.camera.into_iter().zip(remain) {
                if d > 0. {
                    fraction = fraction.min(((0.965 - pos) / d).max(0.));
                } else if d < 0. {
                    fraction = fraction.min(((0.035 - pos) / d).max(0.));
                }
            }
            self.camera = [
                self.camera[0] + remain[0] * fraction,
                self.camera[1] + remain[1] * fraction,
            ];
            if fraction >= 1. {
                self.send(MOVE, id, self.camera)?;
                break;
            }
            remain = [remain[0] * (1. - fraction), remain[1] * (1. - fraction)];
            let new_anchor = [anchor[0].clamp(0.045, 0.955), anchor[1].clamp(0.045, 0.955)];
            if let Some(touch) = &self.touch {
                let packets = touch.lock().unwrap_or_else(|e| e.into_inner()).reanchor(
                    id,
                    self.camera,
                    new_anchor,
                )?;
                let mut metrics = self
                    .metrics
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                metrics.packets += packets;
                metrics.camera_reanchors += 1;
            }
            self.camera = new_anchor;
            self.pointers.insert(id, new_anchor);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Control;
    use std::{
        io::Read,
        net::{TcpListener, TcpStream},
    };
    fn worker() -> (Worker, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut w = Worker::new(
            Arc::new(Shared::default()),
            Arc::new(Mutex::new(Metrics::default())),
        );
        w.touch = Some(Arc::new(Mutex::new(Touch::new(stream, (2400, 1080)))));
        w.focused = true;
        w.mode = true;
        (w, peer)
    }
    const CAMERA_IDLE: Duration = Duration::from_millis(500);
    fn idle_camera() -> Control {
        let mut camera = Control::new("camera", [0.5, 0.5]);
        camera.idle_release_ms = CAMERA_IDLE.as_millis() as u32;
        camera
    }
    fn packet(peer: &mut TcpStream) -> [u8; 32] {
        let mut p = [0u8; 32];
        peer.read_exact(&mut p).unwrap();
        p
    }
    #[test]
    fn charged_attack_and_quick_release() {
        let (mut w, mut peer) = worker();
        let mut c = Control::new("button", [0.82, 0.77]);
        c.key = "mouse_left".into();
        w.profile.controls.push(c);
        w.keys.insert(1);
        w.buttons().unwrap();
        assert_eq!(packet(&mut peer)[1], DOWN);
        thread::sleep(Duration::from_millis(120));
        w.buttons().unwrap();
        let mut p = [0; 32];
        assert!(peer.read(&mut p).is_err());
        w.keys.remove(&1);
        w.buttons().unwrap();
        assert_eq!(packet(&mut peer)[1], UP);
        w.keys.insert(1);
        w.buttons().unwrap();
        w.keys.remove(&1);
        w.buttons().unwrap();
        assert_eq!(packet(&mut peer)[1], DOWN);
        assert_eq!(packet(&mut peer)[1], UP);
    }
    #[test]
    fn diagonal_joystick_and_focus_release() {
        let (mut w, mut peer) = worker();
        w.profile
            .controls
            .push(Control::new("joystick", [0.2, 0.7]));
        w.keys.insert(87);
        w.keys.insert(68);
        w.buttons().unwrap();
        assert_eq!(packet(&mut peer)[1], DOWN);
        assert_eq!(packet(&mut peer)[1], MOVE);
        let p = w.pointers[&1000];
        let dx = (p[0] - 0.2) * 2400. / 1080.;
        let dy = p[1] - 0.7;
        assert!((dx.hypot(dy) - 0.12).abs() < 1e-8);
        w.release();
        assert_eq!(packet(&mut peer)[1], UP);
        assert!(w.pointers.is_empty() && !w.mode);
    }
    #[test]
    fn swipe_finishes_after_key_release() {
        let (mut w, mut peer) = worker();
        let mut c = Control::new("swipe", [0.3, 0.3]);
        c.key = "e".into();
        c.duration_ms = 20;
        w.profile.controls.push(c);
        w.keys.insert(69);
        w.buttons().unwrap();
        assert_eq!(packet(&mut peer)[1], DOWN);
        w.keys.clear();
        w.buttons().unwrap();
        thread::sleep(Duration::from_millis(25));
        w.tick().unwrap();
        assert_eq!(packet(&mut peer)[1], MOVE);
        assert_eq!(packet(&mut peer)[1], UP);
        assert!(w.swipes.is_empty());
    }
    #[test]
    fn camera_wraps_both_axes_without_cursor_resets() {
        let (mut w, mut peer) = worker();
        w.profile.controls.push(Control::new("camera", [0.5, 0.5]));
        w.camera = [0.5, 0.5];
        w.send(DOWN, 900, w.camera).unwrap();
        packet(&mut peer);
        w.drag(9000., -9000., 900, 0.4, false, true).unwrap();
        assert!(w.camera.iter().all(|v| (0.035..=0.965).contains(v)));
        let mut ups = 0;
        let mut moves = 0;
        let count = w.metrics.lock().unwrap().packets - 1;
        for _ in 0..count {
            match packet(&mut peer)[1] {
                UP => ups += 1,
                MOVE => moves += 1,
                _ => {}
            }
        }
        assert!(ups >= 2 && moves >= 3);
    }
    #[test]
    fn boundary_reanchors_preserve_distance_and_direction_in_both_axes() {
        for (dx, dy) in [
            (9000., 7000.),
            (-9000., -7000.),
            (9000., -7000.),
            (-9000., 7000.),
        ] {
            let (mut w, mut peer) = worker();
            let camera = Control::new("camera", [0.5, 0.5]);
            w.profile.controls.push(camera.clone());
            w.arm_camera(&camera).unwrap();
            packet(&mut peer);
            w.drag(dx, dy, 900, 0.4, false, true).unwrap();
            let count = w.metrics.lock().unwrap().packets - 1;
            let mut last = [1200, 540];
            let mut distance = [0, 0];
            for _ in 0..count {
                let p = packet(&mut peer);
                let point = [
                    i32::from_be_bytes(p[10..14].try_into().unwrap()),
                    i32::from_be_bytes(p[14..18].try_into().unwrap()),
                ];
                if p[1] == MOVE {
                    let step = [point[0] - last[0], point[1] - last[1]];
                    assert!(
                        step[0] as f64 * dx >= 0. && step[1] as f64 * dy >= 0.,
                        "Reversed MOVE: {step:?}"
                    );
                    distance[0] += step[0];
                    distance[1] += step[1];
                }
                last = point;
            }
            assert!((distance[0] as f64 - dx * 0.4).abs() <= 4.);
            assert!((distance[1] as f64 - dy * 0.4).abs() <= 4.);
            assert!(w.metrics.lock().unwrap().camera_reanchors >= 2);
        }
    }
    #[test]
    fn batched_reanchor_keeps_matching_up_down_and_duplicate_suppression() {
        let (mut w, mut peer) = worker();
        let touch = w.touch.as_ref().unwrap().clone();
        w.send(DOWN, 900, [0.5, 0.5]).unwrap();
        packet(&mut peer);
        assert_eq!(
            touch
                .lock()
                .unwrap()
                .reanchor(900, [0.965, 0.6], [0.5, 0.5])
                .unwrap(),
            3
        );
        let end = packet(&mut peer);
        let up = packet(&mut peer);
        let down = packet(&mut peer);
        assert_eq!([end[1], up[1], down[1]], [MOVE, UP, DOWN]);
        assert_eq!(&end[10..22], &up[10..22]);
        assert_eq!(&up[22..24], &[0, 0]);
        assert_eq!(&down[22..24], &[255, 255]);
        assert!(!touch.lock().unwrap().send(MOVE, 900, [0.5, 0.5]).unwrap());
    }
    #[test]
    fn reanchor_at_the_already_sent_endpoint_omits_duplicate_move() {
        let (mut w, mut peer) = worker();
        let touch = w.touch.as_ref().unwrap().clone();
        w.send(DOWN, 900, [0.965, 0.5]).unwrap();
        packet(&mut peer);
        assert_eq!(
            touch
                .lock()
                .unwrap()
                .reanchor(900, [0.965, 0.5], [0.5, 0.5])
                .unwrap(),
            2
        );
        assert_eq!(packet(&mut peer)[1], UP);
        assert_eq!(packet(&mut peer)[1], DOWN);
    }
    #[test]
    fn captured_motion_reports_queue_wait_and_preserves_oldest_timestamp() {
        let (mut w, _) = worker();
        let received = Instant::now();
        let captured = received - Duration::from_millis(30);
        w.captured_delta(10., 0., captured, received).unwrap();
        w.captured_delta(20., 0., received - Duration::from_millis(2), received)
            .unwrap();
        assert_eq!(w.delta_since, Some(captured));
        assert_eq!(w.max_queue_ms, 30.);
        assert_eq!(w.queue_ms, 2.);
        assert_eq!(w.delta, [30., 0.]);
    }
    #[test]
    fn dispatch_deadline_starts_after_transport_completion() {
        let (mut w, mut peer) = worker();
        let camera = Control::new("camera", [0.5, 0.5]);
        w.profile.controls.push(camera.clone());
        w.arm_camera(&camera).unwrap();
        packet(&mut peer);
        let old_tick = Instant::now() - Duration::from_millis(20);
        w.last_send = old_tick - Duration::from_millis(2);
        w.mouse_delta(40., 0., old_tick).unwrap();
        let before = Instant::now();
        w.tick_at(old_tick).unwrap();
        assert_eq!(packet(&mut peer)[1], MOVE);
        assert!(w.last_send >= before);
    }
    fn pointer_id(p: [u8; 32]) -> u64 {
        u64::from_be_bytes(p[2..10].try_into().unwrap())
    }
    #[test]
    fn default_camera_resumes_pauses_without_new_down_or_anchor_reset() {
        let (mut w, mut peer) = worker();
        let camera = Control::new("camera", [0.5, 0.5]);
        assert_eq!(camera.idle_release_ms, 0);
        w.profile.controls.push(camera.clone());
        w.arm_camera(&camera).unwrap();
        assert_eq!(packet(&mut peer)[1], DOWN);
        let start = w.last_send + Duration::from_millis(3);
        w.mouse_delta(40., 20., start).unwrap();
        w.tick_at(start).unwrap();
        assert_eq!(packet(&mut peer)[1], MOVE);
        let first = w.camera;
        let pause = start + Duration::from_secs(3);
        w.tick_at(pause).unwrap();
        assert!(w.pointers.contains_key(&900));
        w.mouse_delta(20., 10., pause).unwrap();
        // No extra dispatch delay on the first event after a pause.
        w.tick_at(pause).unwrap();
        assert_eq!(packet(&mut peer)[1], MOVE);
        assert!((w.camera[0] - first[0] - 20. * 0.4 / 2400.).abs() < 1e-9);
        assert_eq!(w.metrics.lock().unwrap().packets, 3);
    }
    #[test]
    fn camera_dispatch_is_due_at_one_ms_not_two() {
        let (mut w, mut peer) = worker();
        let camera = Control::new("camera", [0.5, 0.5]);
        w.profile.controls.push(camera.clone());
        w.arm_camera(&camera).unwrap();
        packet(&mut peer);
        let last = w.last_send;
        w.mouse_delta(40., 0., last).unwrap();
        w.tick_at(last + Duration::from_micros(999)).unwrap();
        assert_eq!(w.metrics.lock().unwrap().packets, 1);
        w.tick_at(last + MOUSE_SEND_INTERVAL).unwrap();
        assert_eq!(packet(&mut peer)[1], MOVE);
        assert_eq!(w.metrics.lock().unwrap().packets, 2);
    }
    #[test]
    fn opting_into_idle_release_does_not_press_camera_until_movement() {
        let (mut w, mut peer) = worker();
        let camera = idle_camera();
        w.profile.controls.push(camera.clone());
        w.arm_camera(&camera).unwrap();
        assert!(w.pointers.is_empty());
        assert_eq!(w.metrics.lock().unwrap().packets, 0);
        let start = w.last_send + Duration::from_millis(3);
        w.mouse_delta(40., 0., start).unwrap();
        w.tick_at(start).unwrap();
        assert_eq!(packet(&mut peer)[1], DOWN);
        assert_eq!(packet(&mut peer)[1], MOVE);
    }
    #[test]
    fn queued_button_edges_remain_ordered_and_shutdown_stops_the_batch() {
        let (mut w, mut peer) = worker();
        let mut button = Control::new("button", [0.8, 0.7]);
        button.key = "q".into();
        w.profile.controls.push(button);
        let (tx, rx) = mpsc::channel();
        tx.send(Event::Key { vk: 81, down: true }).unwrap();
        tx.send(Event::Key {
            vk: 81,
            down: false,
        })
        .unwrap();
        tx.send(Event::Shutdown).unwrap();
        tx.send(Event::Key { vk: 81, down: true }).unwrap();
        // A slow CI machine can exhaust the time budget before shutdown.
        while w.drain_events(&rx).unwrap() {}
        assert_eq!(packet(&mut peer)[1], DOWN);
        assert_eq!(packet(&mut peer)[1], UP);
        assert!(!w.keys.contains(&81) && !w.pointers.contains_key(&1000));
        assert!(matches!(
            rx.try_recv().unwrap(),
            Event::Key { vk: 81, down: true }
        ));
    }
    #[test]
    fn queued_events_are_bounded_even_when_the_sender_remains_busy() {
        let (mut w, _) = worker();
        let (tx, rx) = mpsc::channel();
        for _ in 0..(EVENT_BATCH_LIMIT * 2) {
            tx.send(Event::Wake).unwrap();
        }
        assert!(w.drain_events(&rx).unwrap());
        let remaining = rx.try_iter().count();
        assert!(remaining > EVENT_BATCH_LIMIT);
        assert!(remaining < EVENT_BATCH_LIMIT * 2);
    }
    #[test]
    #[ignore = "Local transport timing probe; reports host timing, not phone latency"]
    fn resumed_camera_host_dispatch_probe() {
        let (mut w, mut peer) = worker();
        w.touch
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .stream
            .set_nodelay(true)
            .unwrap();
        let camera = Control::new("camera", [0.5, 0.5]);
        w.profile.controls.push(camera.clone());
        w.arm_camera(&camera).unwrap();
        packet(&mut peer);
        let mut samples = Vec::new();
        for i in 0..2000 {
            let now = Instant::now();
            w.last_send = now - Duration::from_secs(1);
            w.camera_motion_at = Some(now - Duration::from_secs(1));
            let dx = if i % 2 == 0 { 40. } else { -40. };
            let start = Instant::now();
            w.mouse_delta(dx, 0., now).unwrap();
            w.tick_at(now).unwrap();
            assert_eq!(packet(&mut peer)[1], MOVE);
            samples.push(start.elapsed().as_secs_f64() * 1_000_000.);
        }
        samples.sort_by(f64::total_cmp);
        println!("Resumed MOVE to local TCP peer (2000 samples): median {:.1} us, p95 {:.1} us, p99 {:.1} us, max {:.1} us", samples[1000], samples[1900], samples[1980], samples[1999]);
        assert_eq!(w.metrics.lock().unwrap().packets, 2001);
    }
    #[test]
    fn idle_camera_restarts_with_down_before_move() {
        let (mut w, mut peer) = worker();
        w.profile.controls.push(idle_camera());
        let start = w.last_send + Duration::from_millis(3);
        w.mouse_delta(40., 20., start).unwrap();
        w.tick_at(start).unwrap();
        assert_eq!(packet(&mut peer)[1], DOWN);
        assert_eq!(packet(&mut peer)[1], MOVE);
        let idle = start + CAMERA_IDLE;
        w.tick_at(idle).unwrap();
        assert_eq!(packet(&mut peer)[1], UP);
        assert!(w.mode && !w.pointers.contains_key(&900));
        let count = w.metrics.lock().unwrap().packets;
        w.tick_at(idle + CAMERA_IDLE).unwrap();
        assert_eq!(w.metrics.lock().unwrap().packets, count);
        w.mouse_delta(-30., 10., idle + CAMERA_IDLE).unwrap();
        w.tick_at(idle + CAMERA_IDLE).unwrap();
        let down = packet(&mut peer);
        assert_eq!(down[1], DOWN);
        assert_eq!(i32::from_be_bytes(down[10..14].try_into().unwrap()), 1200);
        assert_eq!(packet(&mut peer)[1], MOVE);
    }
    #[test]
    fn idle_camera_preserves_charged_attack_joystick_and_capture_flags() {
        let (mut w, mut peer) = worker();
        w.shared.enabled.store(true, Ordering::Relaxed);
        w.shared.capture.store(true, Ordering::Relaxed);
        let mut attack = Control::new("button", [0.8, 0.7]);
        attack.key = "mouse_left".into();
        w.profile.controls = vec![attack, Control::new("joystick", [0.2, 0.7]), idle_camera()];
        w.keys.extend([1, 87]);
        w.buttons().unwrap();
        assert_eq!(pointer_id(packet(&mut peer)), 1000);
        assert_eq!(pointer_id(packet(&mut peer)), 1001);
        assert_eq!(pointer_id(packet(&mut peer)), 1001);
        let start = w.last_send + Duration::from_millis(3);
        w.mouse_delta(40., 0., start).unwrap();
        w.tick_at(start).unwrap();
        packet(&mut peer);
        packet(&mut peer);
        w.tick_at(start + CAMERA_IDLE).unwrap();
        let up = packet(&mut peer);
        assert_eq!((up[1], pointer_id(up)), (UP, 900));
        assert!(w.mode && w.keys.contains(&1) && w.keys.contains(&87));
        assert!(w.pointers.contains_key(&1000) && w.pointers.contains_key(&1001));
        assert!(w.shared.enabled.load(Ordering::Relaxed));
        assert!(w.shared.capture.load(Ordering::Relaxed));
        w.keys.remove(&1);
        w.buttons().unwrap();
        let up = packet(&mut peer);
        assert_eq!((up[1], pointer_id(up)), (UP, 1000));
    }
    #[test]
    fn continuous_camera_motion_does_not_periodically_restart() {
        let (mut w, mut peer) = worker();
        w.profile.controls.push(idle_camera());
        let start = w.last_send + Duration::from_millis(3);
        for i in 0..200 {
            let now = start + Duration::from_millis(i * 10);
            w.mouse_delta(4., 0., now).unwrap();
            w.tick_at(now).unwrap();
            if i == 0 {
                assert_eq!(packet(&mut peer)[1], DOWN);
            }
            assert_eq!(packet(&mut peer)[1], MOVE);
        }
        assert_eq!(w.metrics.lock().unwrap().packets, 201);
        assert!(w.pointers.contains_key(&900));
    }
    #[test]
    fn new_event_after_idle_gap_releases_stale_touch_before_tick() {
        let (mut w, mut peer) = worker();
        w.profile.controls.push(idle_camera());
        let start = w.last_send + Duration::from_millis(3);
        w.mouse_delta(40., 0., start).unwrap();
        w.tick_at(start).unwrap();
        packet(&mut peer);
        packet(&mut peer);
        // There was no timeout tick during this simulated cinematic pause.
        let next = start + Duration::from_secs(3);
        w.mouse_delta(50., 0., next).unwrap();
        assert_eq!(packet(&mut peer)[1], UP);
        w.tick_at(next).unwrap();
        assert_eq!(packet(&mut peer)[1], DOWN);
        assert_eq!(packet(&mut peer)[1], MOVE);
    }
    #[test]
    fn pixel_duplicate_moves_are_suppressed() {
        let (mut w, mut peer) = worker();
        w.send(DOWN, 10, [0.5, 0.5]).unwrap();
        packet(&mut peer);
        w.send(MOVE, 10, [0.5, 0.5]).unwrap();
        let mut p = [0; 32];
        assert!(peer.read(&mut p).is_err());
        w.send(MOVE, 10, [0.51, 0.5]).unwrap();
        assert_eq!(packet(&mut peer)[1], MOVE);
    }
    #[test]
    fn windows_click_release_stops_game_input_even_after_focus_changes() {
        let (mut w, mut peer) = worker();
        w.shared.enabled.store(true, Ordering::Relaxed);
        w.shared.capture.store(true, Ordering::Relaxed);
        w.keys.insert(1);
        w.send(DOWN, 1000, [0.8, 0.7]).unwrap();
        assert_eq!(packet(&mut peer)[1], DOWN);
        w.focused = false;
        w.event(Event::Release).unwrap();
        assert_eq!(packet(&mut peer)[1], UP);
        assert!(!w.shared.enabled.load(Ordering::Relaxed));
        assert!(!w.shared.capture.load(Ordering::Relaxed));
        assert!(!w.mode && w.keys.is_empty() && w.pointers.is_empty());
    }
    #[test]
    fn rotation_releases_old_touches_and_sends_new_logical_coordinates() {
        let (mut w, mut peer) = worker();
        let source = w.touch.as_ref().unwrap().lock().unwrap().geometry.clone();
        let portrait = Geometry {
            display: (1080, 2400),
            frame: (864, 1920),
        };
        w.touch
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .apply_geometry(portrait);
        source.store(portrait.encode(), Ordering::Release);
        w.geometry_source = Some(source.clone());
        w.applied_geometry = portrait.encode();
        w.send(DOWN, 950, [0.75, 0.25]).unwrap();
        let old = packet(&mut peer);
        assert_eq!(i32::from_be_bytes(old[10..14].try_into().unwrap()), 810);
        assert_eq!(i32::from_be_bytes(old[14..18].try_into().unwrap()), 600);
        let landscape = Geometry {
            display: (2400, 1080),
            frame: (1920, 864),
        };
        source.store(landscape.encode(), Ordering::Release);
        w.sync_geometry().unwrap();
        assert_eq!(packet(&mut peer)[1], UP);
        assert!(w.pointers.is_empty());
        assert_eq!(w.size(), (2400, 1080));
        assert_eq!(
            w.touch.as_ref().unwrap().lock().unwrap().frame_size,
            (1920, 864)
        );
        w.send(DOWN, 950, [0.75, 0.25]).unwrap();
        let new = packet(&mut peer);
        assert_eq!(i32::from_be_bytes(new[10..14].try_into().unwrap()), 1800);
        assert_eq!(i32::from_be_bytes(new[14..18].try_into().unwrap()), 270);
        assert_eq!(u16::from_be_bytes(new[18..20].try_into().unwrap()), 2400);
        assert_eq!(u16::from_be_bytes(new[20..22].try_into().unwrap()), 1080);
    }
}
