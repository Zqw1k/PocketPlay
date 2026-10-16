//! Display coordinates and video coordinates are separate on a control-only server.
use crate::adb::Tools;
use anyhow::{Context, Result};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub display: (u16, u16),
    pub frame: (u16, u16),
}
impl Geometry {
    pub fn encode(self) -> u64 {
        u64::from(self.display.0) << 48
            | u64::from(self.display.1) << 32
            | u64::from(self.frame.0) << 16
            | u64::from(self.frame.1)
    }
    pub fn decode(value: u64) -> Option<Self> {
        let g = Self {
            display: ((value >> 48) as u16, (value >> 32) as u16),
            frame: ((value >> 16) as u16, value as u16),
        };
        (g.display.0 != 0 && g.display.1 != 0 && g.frame.0 != 0 && g.frame.1 != 0).then_some(g)
    }
    pub fn with_frame(mut self, frame: (u16, u16)) -> Self {
        if (self.display.0 > self.display.1) != (frame.0 > frame.1) {
            self.display = (self.display.1, self.display.0);
        }
        self.frame = frame;
        self
    }
}
/// Center the phone's content uniformly inside a viewport, without stretching.
pub fn aspect_fit(bounds: (f64, f64), content: (u16, u16)) -> Option<[f64; 4]> {
    let (width, height) = bounds;
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.
        || height <= 0.
        || content.0 == 0
        || content.1 == 0
    {
        return None;
    }
    let scale = (width / f64::from(content.0)).min(height / f64::from(content.1));
    let w = f64::from(content.0) * scale;
    let h = f64::from(content.1) * scale;
    Some([(width - w) / 2., (height - h) / 2., w, h])
}
fn dimensions(text: &str) -> Option<(u16, u16)> {
    let (w, h) = text.trim().split_once('x')?;
    let size = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (size.0 != 0 && size.1 != 0).then_some(size)
}
pub fn texture_size(line: &str) -> Option<(u16, u16)> {
    let suffix = line.split_once("Texture")?.1;
    if !suffix.starts_with(':') && !suffix.starts_with(" (") {
        return None;
    }
    dimensions(suffix.split_once(':')?.1.split_whitespace().next()?)
}
pub fn viewport_size(input: &str) -> Option<(u16, u16)> {
    input
        .lines()
        .filter(|line| {
            line.contains("Viewport")
                && line.contains("INTERNAL")
                && line.contains("displayId=0,")
                && !line.contains("isActive=[0]")
                && !line.contains("isActive=false")
        })
        .find_map(|line| {
            let value = line.split_once("logicalFrame=[")?.1.split_once(']')?.0;
            let values: Vec<i32> = value
                .split(',')
                .map(|v| v.trim().parse())
                .collect::<std::result::Result<_, _>>()
                .ok()?;
            if values.len() != 4 {
                return None;
            }
            let w = u16::try_from(values[2] - values[0]).ok()?;
            let h = u16::try_from(values[3] - values[1]).ok()?;
            (w != 0 && h != 0).then_some((w, h))
        })
}
pub fn display_size(tools: &Tools, serial: &str) -> Result<(u16, u16)> {
    let input = tools.run(serial, &["shell", "dumpsys", "input"], 3)?;
    if let Some(size) = viewport_size(&input) {
        return Ok(size);
    }
    let wm = tools.run(serial, &["shell", "wm", "size"], 3)?;
    let mut size = wm
        .lines()
        .rev()
        .find_map(|line| dimensions(line.split_once(':')?.1))
        .context("Could not read Android display dimensions")?;
    let rotation = input
        .lines()
        .find(|line| line.contains("Viewport") && line.contains("INTERNAL"))
        .and_then(|line| line.split_once("orientation=").map(|(_, v)| v))
        .and_then(|v| v.chars().next())
        .and_then(|v| v.to_digit(10));
    if rotation.is_some_and(|v| v % 2 == 1) {
        size = (size.1, size.0);
    }
    Ok(size)
}

pub struct GeometryWatcher {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl GeometryWatcher {
    pub fn start(path: PathBuf, tools: Tools, serial: String, wanted: Arc<AtomicU64>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::spawn(move || {
            let Ok(file) = File::open(path) else {
                return;
            };
            let mut reader = BufReader::new(file);
            let mut partial = String::new();
            let mut last_frame = None;
            while !flag.load(Ordering::Acquire) {
                let mut latest = None;
                loop {
                    match reader.read_line(&mut partial) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            if !partial.ends_with('\n') {
                                break;
                            }
                            if let Some(encoded) = partial
                                .trim()
                                .strip_prefix("Geometry: ")
                                .and_then(|v| v.parse::<u64>().ok())
                            {
                                if let Some(geometry) = Geometry::decode(encoded) {
                                    wanted.store(geometry.encode(), Ordering::Release);
                                    last_frame = Some(geometry.frame);
                                }
                            }
                            if let Some(frame) = texture_size(&partial) {
                                latest = Some(frame);
                            }
                            partial.clear();
                        }
                    }
                }
                if let Some(frame) = latest.filter(|frame| Some(*frame) != last_frame) {
                    last_frame = Some(frame);
                    if let Some(old) = Geometry::decode(wanted.load(Ordering::Acquire)) {
                        // Publish orientation immediately using the known logical size.
                        // Reconcile override changes once per texture change, not per frame.
                        let mut geometry = old.with_frame(frame);
                        wanted.store(geometry.encode(), Ordering::Release);
                        if !flag.load(Ordering::Acquire) {
                            if let Ok(display) = display_size(&tools, &serial) {
                                let relative = (f64::from(display.0) / f64::from(display.1))
                                    / (f64::from(frame.0) / f64::from(frame.1));
                                // Ignore an in-flight viewport for a different rotation.
                                if (relative - 1.).abs() < 0.02 {
                                    geometry.display = display;
                                    wanted.store(geometry.encode(), Ordering::Release);
                                }
                            }
                        }
                    }
                }
                thread::sleep(Duration::from_millis(40));
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for GeometryWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portrait_content_fits_without_stretching() {
        assert_eq!(
            aspect_fit((1000., 1000.), (1080, 2400)),
            Some([275., 0., 450., 1000.])
        );
    }
    #[test]
    fn landscape_content_fits_without_stretching() {
        assert_eq!(
            aspect_fit((1000., 1000.), (2400, 1080)),
            Some([0., 275., 1000., 450.])
        );
    }
    #[test]
    fn fitting_rejects_invalid_dimensions() {
        assert!(aspect_fit((f64::NAN, 500.), (1920, 1080)).is_none());
        assert!(aspect_fit((1000., 0.), (1920, 1080)).is_none());
    }
    #[test]
    fn logical_viewport_uses_override_and_current_rotation() {
        let portrait = "Viewport INTERNAL: displayId=0, orientation=0, logicalFrame=[0, 0, 1080, 2400], physicalFrame=[0, 0, 1440, 3200]";
        let landscape = "Viewport INTERNAL: displayId=0, orientation=1, logicalFrame=[0, 0, 2400, 1080], physicalFrame=[0, 0, 3200, 1440]";
        assert_eq!(viewport_size(portrait), Some((1080, 2400)));
        assert_eq!(viewport_size(landscape), Some((2400, 1080)));
        assert_eq!(
            viewport_size("Viewport EXTERNAL: displayId=1, logicalFrame=[0,0,640,480]"),
            None
        );
    }
    #[test]
    fn texture_rotation_keeps_video_and_display_sizes_separate() {
        let old = Geometry {
            display: (1080, 2400),
            frame: (864, 1920),
        };
        let frame = texture_size("INFO: Texture: 1920x864\n").unwrap();
        let new = old.with_frame(frame);
        assert_eq!(new.display, (2400, 1080));
        assert_eq!(new.frame, (1920, 864));
        assert_eq!(Geometry::decode(new.encode()), Some(new));
        assert_eq!(new.with_frame((864, 1920)), old);
        assert_eq!(texture_size("INFO: 60 fps"), None);
        assert_eq!(
            texture_size("INFO: Texture (D3D11VA): 1920x864"),
            Some((1920, 864))
        );
        assert_eq!(Geometry::decode(0), None);
    }
    #[test]
    #[ignore = "Requires an authorized phone and active native mirror"]
    fn live_geometry_probe() {
        let log = PathBuf::from(
            std::env::var_os("POCKETPLAY_GEOMETRY_PROBE_LOG").expect("Specify mirror log"),
        );
        let serial =
            std::env::var("POCKETPLAY_GEOMETRY_PROBE_SERIAL").expect("Specify authorized serial");
        let tools = Tools::find(log.parent().unwrap().to_path_buf()).unwrap();
        let display = display_size(&tools, &serial).unwrap();
        let frame = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .filter_map(texture_size)
            .next_back()
            .unwrap();
        let stale = (display.1, display.0);
        let desired = Arc::new(AtomicU64::new(
            Geometry {
                display: stale,
                frame: stale,
            }
            .encode(),
        ));
        let _watcher = GeometryWatcher::start(log, tools, serial, desired.clone());
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        while std::time::Instant::now() < deadline {
            let geometry = Geometry::decode(desired.load(Ordering::Acquire)).unwrap();
            if geometry.display == display && geometry.frame == frame {
                eprintln!(
                    "Live geometry synchronized: Android={}x{}, video={}x{}",
                    display.0, display.1, frame.0, frame.1
                );
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "Live geometry did not reconcile cached orientation: {:?}",
            Geometry::decode(desired.load(Ordering::Acquire))
        );
    }
}
