use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const VERSION: &str = "1.0.0";
pub fn id() -> String {
    format!(
        "{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
pub fn data_dir() -> PathBuf {
    std::env::var_os("POCKETPLAY_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
                .join("PocketPlayNative")
        })
}
pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(".pocketplay-{}.tmp", id()));
    let mut created = false;
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        created = true;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
        drop(file);
        // MoveFileEx replaces atomically on Windows, including existing profiles.
        crate::native::replace_file(&tmp, path).context("Could not save app data")
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub connection_mode: String,
    pub device_serial: String,
    pub wireless_address: String,
    pub active_profile: String,
    pub max_size: u32,
    pub bitrate_mbps: f64,
    pub max_fps: u32,
    pub video_codec: String,
    pub video_encoder: String,
    pub decoder: String,
    pub video_buffer_ms: u32,
    pub screen_off: bool,
    pub fullscreen: bool,
    pub vsync: bool,
    pub audio_mode: String,
    pub audio_codec: String,
    pub audio_bitrate_kbps: u32,
    pub audio_buffer_ms: u32,
    pub touch_drag_sensitivity: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "Midnight".into(),
            connection_mode: "USB".into(),
            device_serial: String::new(),
            wireless_address: String::new(),
            active_profile: "default".into(),
            max_size: 1920,
            bitrate_mbps: 8.,
            max_fps: 60,
            video_codec: "h264".into(),
            video_encoder: String::new(),
            decoder: "auto".into(),
            video_buffer_ms: 0,
            screen_off: true,
            fullscreen: false,
            vsync: false,
            audio_mode: "Phone only".into(),
            audio_codec: "raw".into(),
            audio_bitrate_kbps: 128,
            audio_buffer_ms: 0,
            touch_drag_sensitivity: 0.6,
        }
    }
}
fn choice(value: &str, choices: &[&str]) -> Result<()> {
    if !choices.contains(&value) {
        bail!("Invalid setting: {value}");
    }
    Ok(())
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        choice(&self.theme, &["Midnight", "Light", "Ocean"])?;
        choice(&self.connection_mode, &["USB", "Wi-Fi"])?;
        choice(&self.video_codec, &["h264", "h265", "av1", "vp8", "vp9"])?;
        choice(&self.decoder, &["disabled", "auto", "d3d11va"])?;
        choice(
            &self.audio_mode,
            &["Phone only", "Computer only", "Computer + phone"],
        )?;
        choice(&self.audio_codec, &["opus", "aac", "flac", "raw"])?;
        if !safe_id(&self.active_profile) {
            bail!("Invalid active profile identifier.");
        }
        if self.max_size > 8192
            || (self.max_size > 0 && self.max_size < 320)
            || self.max_fps > 240
            || self.video_buffer_ms != 0
            || self.audio_buffer_ms > 200
        {
            bail!("Resolution/FPS/buffer is outside the supported range.");
        }
        if !self.bitrate_mbps.is_finite()
            || !(0.1..=200.).contains(&self.bitrate_mbps)
            || !self.touch_drag_sensitivity.is_finite()
            || !(0.05..=3.).contains(&self.touch_drag_sensitivity)
            || !(16..=1024).contains(&self.audio_bitrate_kbps)
        {
            bail!("Bitrate or sensitivity is outside the supported range.");
        }
        if !self
            .video_encoder
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        {
            bail!("Invalid encoder name.");
        }
        for text in [
            &self.video_encoder,
            &self.device_serial,
            &self.wireless_address,
            &self.active_profile,
        ] {
            if text.len() > 256 {
                bail!("Setting is too long.");
            }
        }
        Ok(())
    }
    pub fn load(dir: &Path) -> Self {
        crate::files::read_bounded(&dir.join("settings.json"), crate::files::MAX_JSON_BYTES)
            .ok()
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .filter(|s| s.validate().is_ok())
            .unwrap_or_default()
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        self.validate()?;
        atomic_json(&dir.join("settings.json"), self)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Directions {
    pub up: String,
    pub down: String,
    pub left: String,
    pub right: String,
}
impl Default for Directions {
    fn default() -> Self {
        Self {
            up: "w".into(),
            down: "s".into(),
            left: "a".into(),
            right: "d".into(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Control {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub point: [f64; 2],
    pub key: String,
    pub keys: Directions,
    pub radius: f64,
    pub sensitivity: f64,
    pub invert_y: bool,
    /// Zero keeps the gesture active for immediate movement after a pause.
    pub idle_release_ms: u32,
    #[serde(skip_serializing_if = "invalid_position")]
    pub end: [f64; 2],
    pub duration_ms: u32,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: String::new(),
            label: "Tap / hold".into(),
            point: [f64::NAN, f64::NAN],
            key: String::new(),
            keys: Directions::default(),
            radius: 0.12,
            sensitivity: 0.4,
            invert_y: false,
            idle_release_ms: 0,
            end: [f64::NAN, f64::NAN],
            duration_ms: 200,
        }
    }
}
impl Control {
    pub fn new(kind: &str, point: [f64; 2]) -> Self {
        Self {
            id: id(),
            kind: kind.into(),
            label: match kind {
                "button" => "Tap / hold",
                "joystick" => "Joystick",
                "camera" => "Mouse look",
                _ => "Swipe",
            }
            .into(),
            point,
            end: [(point[0] + 0.15).min(0.95), point[1]],
            ..Self::default()
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Profile {
    pub version: u32,
    pub name: String,
    pub controls: Vec<Control>,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            version: 1,
            name: "My first profile".into(),
            controls: vec![],
        }
    }
}
fn position(p: [f64; 2]) -> bool {
    p.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))
}
fn invalid_position(p: &[f64; 2]) -> bool {
    !position(*p)
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.name.trim().is_empty()
            || self.name.chars().count() > 80
            || self.controls.len() > 100
        {
            bail!("Invalid profile format, name, or control count.");
        }
        let mut ids = HashSet::new();
        let mut keys = HashSet::new();
        let mut cameras = 0;
        for c in &self.controls {
            if !safe_id(&c.id) || !ids.insert(&c.id) || !position(c.point) {
                bail!("Controls need unique IDs and coordinates between 0 and 1.");
            }
            if c.label.chars().count() > 80 {
                bail!("Control labels must be at most 80 characters.");
            }
            let bindings: &[&str] = match c.kind.as_str() {
                "button" => &[&c.key],
                "swipe" => {
                    if !position(c.end) || !(20..=5000).contains(&c.duration_ms) {
                        bail!("Invalid swipe endpoint or duration.");
                    }
                    &[&c.key]
                }
                "joystick" => {
                    if !c.radius.is_finite() || !(0.01..=0.45).contains(&c.radius) {
                        bail!("Invalid joystick radius.");
                    }
                    &[&c.keys.up, &c.keys.down, &c.keys.left, &c.keys.right]
                }
                "camera" => {
                    cameras += 1;
                    if cameras > 1
                        || !c.sensitivity.is_finite()
                        || !(0.05..=1.5).contains(&c.sensitivity)
                        || (c.idle_release_ms != 0 && !(250..=5000).contains(&c.idle_release_ms))
                    {
                        bail!("Use one mouse-look control with sensitivity 0.05 to 1.5 and idle release disabled or 250 to 5000 ms.");
                    }
                    &[]
                }
                _ => bail!("Unknown control type."),
            };
            for k in bindings {
                if k.is_empty() {
                    continue;
                }
                if k.len() > 32 {
                    bail!("Binding name is too long.");
                }
                let vk =
                    crate::native::key_vk(k).with_context(|| format!("Invalid binding: {k}"))?;
                if vk == 192 {
                    bail!("The ` key is reserved for switching modes.");
                }
                if !keys.insert(vk) {
                    bail!("Binding {k} is assigned more than once.");
                }
            }
        }
        Ok(())
    }
}
pub fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !["CON", "PRN", "AUX", "NUL"]
            .iter()
            .any(|name| id.eq_ignore_ascii_case(name))
        && !(id.len() == 4
            && (id[..3].eq_ignore_ascii_case("COM") || id[..3].eq_ignore_ascii_case("LPT"))
            && matches!(id.as_bytes()[3], b'1'..=b'9'))
}
pub fn profiles(dir: &Path) -> Result<Vec<(String, Profile)>> {
    let dir = dir.join("profiles");
    fs::create_dir_all(&dir)?;
    let mut p: Vec<(String, Profile)> = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|id| safe_id(id))
        else {
            continue;
        };
        if let Ok(bytes) = crate::files::read_bounded(&path, crate::files::MAX_JSON_BYTES) {
            if let Ok(profile) = serde_json::from_slice::<Profile>(&bytes) {
                if profile.validate().is_ok() {
                    p.push((id.to_owned(), profile));
                }
            }
        }
    }
    if p.is_empty() {
        let profile = Profile::default();
        atomic_json(&dir.join("default.json"), &profile)?;
        p.push(("default".into(), profile));
    }
    p.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(p)
}
pub fn save_profile(dir: &Path, id: &str, p: &Profile) -> Result<()> {
    if !safe_id(id) {
        bail!("Invalid profile ID.");
    }
    p.validate()?;
    atomic_json(&dir.join("profiles").join(format!("{id}.json")), p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_settings_default_to_windowed_playback() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.fullscreen);
    }
    #[test]
    fn fullscreen_setting_survives_json_roundtrip() {
        let settings = Settings {
            fullscreen: true,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert!(restored.fullscreen);
    }
    #[test]
    fn legacy_settings_keep_immediate_presentation() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.vsync);
    }
    #[test]
    fn smooth_presentation_setting_survives_json_roundtrip() {
        let settings = Settings {
            vsync: true,
            ..Settings::default()
        };
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(restored.vsync);
    }
    #[test]
    fn file_identifiers_reject_traversal_and_windows_devices() {
        for id in ["../escape", "CON", "nul", "COM1", "lpt9", "", "a/b", "éabc"] {
            assert!(!safe_id(id), "Accepted unsafe ID: {id}");
        }
        assert!(safe_id("my-profile_1"));
        let settings = Settings {
            active_profile: "../escape".into(),
            ..Settings::default()
        };
        assert!(settings.validate().is_err());
    }
    #[test]
    fn atomic_save_does_not_follow_old_predictable_temporary_path() {
        let dir = std::env::temp_dir().join(format!("pocketplay-atomic-{}", id()));
        fs::create_dir_all(&dir).unwrap();
        let old_tmp = dir.join("settings.json.tmp");
        fs::write(&old_tmp, b"keep me").unwrap();
        Settings::default().save(&dir).unwrap();
        assert_eq!(fs::read(&old_tmp).unwrap(), b"keep me");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_file(old_tmp).unwrap();
        fs::remove_file(dir.join("settings.json")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn settings_reject_nonfinite_bitrate() {
        let s = Settings {
            bitrate_mbps: f64::NAN,
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }
    #[test]
    fn legacy_profile_roundtrip() {
        let json = r#"{"version":1,"name":"Legacy","controls":[{"id":"tap","kind":"button","point":[0.8,0.7],"key":"mouse_left"},{"id":"look","kind":"camera","point":[0.6,0.3],"sensitivity":0.4,"invert_y":false}]}"#;
        let p: Profile = serde_json::from_str(json).unwrap();
        p.validate().unwrap();
        let bytes = serde_json::to_vec(&p).unwrap();
        let loaded: Profile = serde_json::from_slice(&bytes).unwrap();
        loaded.validate().unwrap();
        assert_eq!(loaded.controls[0].key, "mouse_left");
        assert_eq!(loaded.controls[1].idle_release_ms, 0);
    }
    #[test]
    fn camera_idle_release_validation_and_roundtrip() {
        let mut p = Profile::default();
        p.controls.push(Control::new("camera", [0.5, 0.5]));
        p.validate().unwrap();
        p.controls[0].idle_release_ms = 500;
        p.validate().unwrap();
        let loaded: Profile = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert_eq!(loaded.controls[0].idle_release_ms, 500);
        for invalid in [1, 120, 249, 5001] {
            p.controls[0].idle_release_ms = invalid;
            assert!(p.validate().is_err());
        }
    }
    #[test]
    fn empty_profile_and_duplicate_keys() {
        let mut p = Profile::default();
        assert!(p.controls.is_empty());
        let mut a = Control::new("button", [0.5, 0.5]);
        a.key = "w".into();
        let mut b = Control::new("button", [0.6, 0.5]);
        b.key = "vk_87".into();
        p.controls = vec![a, b];
        assert!(p.validate().is_err());
        p.controls.pop();
        p.controls[0].key = "vk_192".into();
        assert!(p.validate().is_err());
    }
    #[test]
    fn malformed_controls_rejected() {
        for json in [
            r#"{"version":1,"name":"Bad","controls":[{"kind":"button","point":[0.5,0.5]}]}"#,
            r#"{"version":1,"name":"Bad","controls":[{"id":"x","kind":"swipe","point":[0.5,0.5]}]}"#,
        ] {
            let p: Profile = serde_json::from_str(json).unwrap();
            assert!(p.validate().is_err());
        }
    }
    #[test]
    fn atomic_profile_persistence() {
        let dir = std::env::temp_dir().join(format!("pocketplay-rust-check-{}", id()));
        fs::create_dir_all(&dir).unwrap();
        let p = Profile::default();
        save_profile(&dir, "default", &p).unwrap();
        let mut p = p;
        p.name = "Updated".into();
        save_profile(&dir, "default", &p).unwrap();
        assert_eq!(profiles(&dir).unwrap()[0].1.name, "Updated");
        assert!(save_profile(&dir, "../escape", &p).is_err());
        fs::remove_file(dir.join("profiles/default.json")).unwrap();
        fs::remove_dir(dir.join("profiles")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
