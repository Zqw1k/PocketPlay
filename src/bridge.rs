//! PocketPlay protocol and owned native session resources.
use crate::{
    adb::{terminate, Tools},
    config::Settings,
    geometry::{Geometry, GeometryWatcher},
};
use anyhow::{bail, Context, Result};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    process::Child,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
pub const DOWN: u8 = 0;
pub const UP: u8 = 1;
pub const MOVE: u8 = 2;
pub struct Touch {
    pub stream: TcpStream,
    pub size: (u16, u16),
    pub frame_size: (u16, u16),
    pub geometry: Arc<AtomicU64>,
    last: HashMap<u64, (i32, i32)>,
}
impl Touch {
    pub fn new(stream: TcpStream, size: (u16, u16)) -> Self {
        Self {
            stream,
            size,
            frame_size: size,
            geometry: Arc::new(AtomicU64::new(
                Geometry {
                    display: size,
                    frame: size,
                }
                .encode(),
            )),
            last: HashMap::new(),
        }
    }
    pub fn apply_geometry(&mut self, geometry: Geometry) {
        self.size = geometry.display;
        self.frame_size = geometry.frame;
        self.last.clear();
    }
    pub fn send(&mut self, action: u8, pointer: u64, point: [f64; 2]) -> Result<bool> {
        let (w, h) = self.size;
        let px = (point[0] * w as f64).round().clamp(0., (w - 1) as f64) as i32;
        let py = (point[1] * h as f64).round().clamp(0., (h - 1) as f64) as i32;
        if action == MOVE && self.last.get(&pointer) == Some(&(px, py)) {
            return Ok(false);
        }
        self.stream
            .write_all(&packet(action, pointer, px, py, w, h))?;
        if action == UP {
            self.last.remove(&pointer);
        } else {
            self.last.insert(pointer, (px, py));
        }
        Ok(true)
    }
    pub fn reanchor(&mut self, pointer: u64, end: [f64; 2], anchor: [f64; 2]) -> Result<u64> {
        let (w, h) = self.size;
        let pixel = |p: [f64; 2]| {
            (
                (p[0] * w as f64).round().clamp(0., (w - 1) as f64) as i32,
                (p[1] * h as f64).round().clamp(0., (h - 1) as f64) as i32,
            )
        };
        let end = pixel(end);
        let anchor = pixel(anchor);
        let mut bytes = [0u8; 96];
        let mut len = 0;
        // Keep the final MOVE and matching UP/DOWN in one write_all call.
        // TCP remains a byte stream; Android still receives ordered messages.
        if self.last.get(&pointer) != Some(&end) {
            bytes[..32].copy_from_slice(&packet(MOVE, pointer, end.0, end.1, w, h));
            len += 32;
        }
        for (action, p) in [(UP, end), (DOWN, anchor)] {
            bytes[len..len + 32].copy_from_slice(&packet(action, pointer, p.0, p.1, w, h));
            len += 32;
        }
        self.stream.write_all(&bytes[..len])?;
        self.last.insert(pointer, anchor);
        Ok((len / 32) as u64)
    }
}
pub fn packet(action: u8, pointer: u64, x: i32, y: i32, w: u16, h: u16) -> [u8; 32] {
    let mut p = [0u8; 32];
    p[0] = 0x10;
    p[1] = action;
    p[2..10].copy_from_slice(&pointer.to_be_bytes());
    p[10..14].copy_from_slice(&x.to_be_bytes());
    p[14..18].copy_from_slice(&y.to_be_bytes());
    p[18..20].copy_from_slice(&w.to_be_bytes());
    p[20..22].copy_from_slice(&h.to_be_bytes());
    p[22..24].copy_from_slice(&(if action == UP { 0u16 } else { u16::MAX }).to_be_bytes());
    p
}
pub struct Session {
    pub tools: Tools,
    pub serial: String,
    pub touch: Option<Arc<Mutex<Touch>>>,
    pub mirror: Option<Child>,
    pub warning: Option<String>,
    server: Option<Child>,
    port: Option<u16>,
    remote: String,
    geometry_watcher: Option<GeometryWatcher>,
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        bail!("Connection cancelled.");
    }
    Ok(())
}
impl Session {
    pub fn start(tools: Tools, settings: &Settings, cancel: Arc<AtomicBool>) -> Result<Self> {
        settings.validate()?;
        let serial = tools.resolve(&settings.device_serial, &settings.connection_mode)?;
        cancelled(&cancel)?;
        let mut session = Self {
            tools,
            serial,
            touch: None,
            mirror: None,
            warning: None,
            server: None,
            port: None,
            remote: String::new(),
            geometry_watcher: None,
        };
        session.connect(settings, &cancel)?;
        Ok(session)
    }
    fn connect(&mut self, settings: &Settings, cancel: &AtomicBool) -> Result<()> {
        let token = crate::native::session_token()?;
        self.remote = format!("/data/local/tmp/pocketplay-{token}");
        for (file, suffix) in [
            ("pocketplay-launcher.jar", ".jar"),
            ("libpocketplay_device.so", ".so"),
        ] {
            self.tools.run(
                &self.serial,
                &[
                    "push",
                    &self.tools.dir.join(file).to_string_lossy(),
                    &format!("{}{suffix}", self.remote),
                ],
                30,
            )?;
            cancelled(cancel)?;
        }
        let port = self
            .tools
            .run(
                &self.serial,
                &[
                    "forward",
                    "tcp:0",
                    &format!("localabstract:pocketplay_{token}"),
                ],
                10,
            )?
            .trim()
            .parse()?;
        self.port = Some(port);
        let args = vec![
            "shell".into(),
            format!(
                "CLASSPATH={}.jar:/system/framework/services.jar",
                self.remote
            ),
            "app_process".into(),
            "/".into(),
            "com.pocketplay.server.Main".into(),
            format!("{}.so", self.remote),
            token.clone(),
            settings.max_size.to_string(),
            format!("{:.0}", settings.bitrate_mbps * 1_000_000.),
            settings.max_fps.to_string(),
            settings.video_codec.clone(),
            if settings.video_encoder.is_empty() {
                "auto".into()
            } else {
                settings.video_encoder.clone()
            },
            match settings.audio_mode.as_str() {
                "Computer only" => "1",
                "Computer + phone" => "2",
                _ => "0",
            }
            .into(),
        ];
        let refs: Vec<_> = args.iter().map(String::as_str).collect();
        self.server = Some(self.tools.log_spawn(
            self.tools.adb_command(&self.serial, &refs),
            "native-device.log",
        )?);
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut connected = None;
        while Instant::now() < deadline {
            cancelled(cancel)?;
            if self
                .server
                .as_mut()
                .context("Missing native service")?
                .try_wait()?
                .is_some()
            {
                bail!(
                    "Native device service exited: {}",
                    self.log_tail("native-device.log")
                );
            }
            if let Ok(mut stream) = crate::viewer::connect(port, &token, 1) {
                stream.set_read_timeout(Some(Duration::from_millis(250)))?;
                let mut header = [0u8; 12];
                if stream.read_exact(&mut header).is_ok()
                    && u32::from_be_bytes(header[..4].try_into()?) == crate::viewer::MAGIC
                {
                    let w = u32::from_be_bytes(header[4..8].try_into()?);
                    let h = u32::from_be_bytes(header[8..12].try_into()?);
                    if !(1..=8192).contains(&w) || !(1..=8192).contains(&h) {
                        bail!("Invalid touch geometry");
                    }
                    connected = Some((stream, (w as u16, h as u16)));
                    break;
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        let (stream, size) =
            connected.context("Timed out connecting to the native phone service")?;
        stream.set_read_timeout(None)?;
        self.touch = Some(Arc::new(Mutex::new(Touch::new(stream, size))));
        cancelled(cancel)?;
        let mut viewer = crate::adb::hidden(&std::env::current_exe()?);
        let audio = if settings.audio_mode == "Phone only" {
            "0"
        } else {
            "1"
        };
        viewer.args([
            "--native-viewer",
            &port.to_string(),
            &token,
            &settings.video_codec,
            &settings.decoder,
            &self.tools.data.to_string_lossy(),
            audio,
            &settings.audio_buffer_ms.to_string(),
            &format!("{}x{}", size.0, size.1),
            if settings.fullscreen { "1" } else { "0" },
            if settings.vsync { "1" } else { "0" },
        ]);
        self.mirror = Some(self.tools.log_spawn(viewer, "native-viewer.log")?);
        let geometry = self
            .touch
            .as_ref()
            .context("No input channel")?
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .geometry
            .clone();
        self.geometry_watcher = Some(GeometryWatcher::start(
            self.tools.data.join("native-viewer.log"),
            self.tools.clone(),
            self.serial.clone(),
            geometry,
        ));
        // A live process is not proof that capture has initialized. In
        // particular, do not power the panel off while Android is still
        // creating its mirror or the PC is preparing its graphics device.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            cancelled(cancel)?;
            if let Some(error) = self.failure() {
                bail!("{error}");
            }
            if crate::files::read_log_tail(&self.tools.data.join("native-viewer.log")).is_ok_and(
                |log| {
                    log.lines()
                        .any(|line| line == "Video ready: first frame presented")
                },
            ) {
                break;
            }
            if Instant::now() >= deadline {
                bail!(
                    "Timed out waiting for the first video frame. Keep the phone unlocked and its display on while connecting.\nPhone: {}\nPC: {}",
                    self.log_tail("native-device.log"),
                    self.log_tail("native-viewer.log")
                );
            }
            thread::sleep(Duration::from_millis(50));
        }
        cancelled(cancel)?;
        if settings.screen_off {
            let mut touch = self
                .touch
                .as_ref()
                .context("No input channel")?
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            touch
                .stream
                .set_read_timeout(Some(Duration::from_secs(3)))?;
            touch.stream.write_all(&[0x20, 1])?;
            let mut reply = [0u8; 2];
            if touch.stream.read_exact(&mut reply).is_err() || reply != [0x20, 1] {
                self.warning =
                    Some("Display-off was not confirmed; the phone panel remains on.".into());
            }
            touch.stream.set_read_timeout(None)?;
        }
        Ok(())
    }
    pub fn log_tail(&self, name: &str) -> String {
        let text = crate::files::read_log_tail(&self.tools.data.join(name)).unwrap_or_default();
        let mut lines: Vec<_> = text.lines().rev().take(8).collect();
        lines.reverse();
        lines.join(" | ")
    }
    pub fn failure(&mut self) -> Option<String> {
        let cause = [
            ("Native phone service", &mut self.server),
            ("Native viewer", &mut self.mirror),
        ]
        .into_iter()
        .find_map(|(name, child)| match child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(None) => None,
                Ok(Some(status)) => Some(format!("{name} exited ({status})")),
                Err(error) => Some(format!("Could not check {name}: {error}")),
            },
            None => Some(format!("{name} is not running")),
        });
        cause.map(|cause| {
            format!(
                "{cause}\nPhone: {}\nPC: {}",
                self.log_tail("native-device.log"),
                self.log_tail("native-viewer.log")
            )
        })
    }
    #[cfg(feature = "qa")]
    pub fn alive(&mut self) -> bool {
        [&mut self.mirror, &mut self.server]
            .into_iter()
            .all(|c| c.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None))))
    }
    pub fn stop(&mut self) {
        self.geometry_watcher.take();
        if let Some(touch) = self.touch.take() {
            let mut touch = touch.lock().unwrap_or_else(|p| p.into_inner());
            let _ = touch.stream.write_all(&[0x7f]);
            let _ = touch.stream.shutdown(Shutdown::Both);
        }
        if let Some(mut c) = self.mirror.take() {
            terminate(&mut c);
        }
        if let Some(mut c) = self.server.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while matches!(c.try_wait(), Ok(None)) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            terminate(&mut c);
        }
        if let Some(port) = self.port.take() {
            let _ = self.tools.run(
                &self.serial,
                &["forward", "--remove", &format!("tcp:{port}")],
                5,
            );
        }
        if !self.remote.is_empty() {
            let _ = self.tools.run(
                &self.serial,
                &[
                    "shell",
                    "rm",
                    "-f",
                    &format!("{}.jar", self.remote),
                    &format!("{}.so", self.remote),
                ],
                5,
            );
            self.remote.clear();
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn touch_packet_matches_pocketplay_protocol() {
        let p = packet(DOWN, 1000, 1200, 540, 2400, 1080);
        assert_eq!(p.len(), 32);
        assert_eq!(&p[..2], &[0x10, 0]);
        assert_eq!(u64::from_be_bytes(p[2..10].try_into().unwrap()), 1000);
        assert_eq!(i32::from_be_bytes(p[10..14].try_into().unwrap()), 1200);
        assert_eq!(u16::from_be_bytes(p[22..24].try_into().unwrap()), 65535);
        assert_eq!(&p[24..], &[0; 8]);
        let up = packet(UP, 1000, 1200, 540, 2400, 1080);
        assert_eq!(&up[22..24], &[0, 0]);
    }
}
