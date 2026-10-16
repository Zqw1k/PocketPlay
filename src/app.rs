use crate::{
    adb::{Device, Tools},
    bridge::Session,
    config::{self, Control, Profile, Settings, VERSION},
    input::Engine,
    native,
};
use anyhow::Result;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Connect,
    Keymap,
    Stream,
    Appearance,
    Help,
}
enum Reply {
    Devices(Vec<Device>),
    Session(Box<Session>),
    Stopped(bool),
    Killed,
    Text(String),
    Connected(String),
    Discovered(Vec<String>),
    Image(Vec<u8>),
    Encoders(Vec<String>),
}
type JobResult = std::result::Result<Reply, String>;
enum Pending {
    Load(String, Profile),
    New,
    Delete,
    Import(PathBuf),
    Close,
}
struct Recording {
    index: usize,
    field: String,
    started: Instant,
}
pub struct PocketPlay {
    data: PathBuf,
    tools: Option<Tools>,
    settings: Settings,
    profile_id: String,
    profile: Profile,
    profiles: Vec<(String, Profile)>,
    dirty: bool,
    page: Page,
    status: String,
    error: Option<String>,
    engine: Engine,
    session: Option<Session>,
    busy: bool,
    connecting: bool,
    cancel: Arc<AtomicBool>,
    tx: Sender<JobResult>,
    rx: Receiver<JobResult>,
    pair_address: String,
    pair_code: String,
    discovered: Vec<String>,
    devices: Vec<Device>,
    encoders: Vec<String>,
    texture: Option<egui::TextureHandle>,
    selected: Option<usize>,
    add_kind: Option<String>,
    dragging: Option<(usize, bool)>,
    record: Option<Recording>,
    pending: Option<Pending>,
    confirm_delete: bool,
    allow_close: bool,
    exit_requested: bool,
    restart_after_stop: bool,
    last_log_read: Instant,
    fps: Option<f64>,
    video_details: String,
    audio_details: String,
    image_size: Vec2,
    #[cfg(feature = "qa")]
    qa_capture: Option<QaCapture>,
}
impl PocketPlay {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self> {
        let data = config::data_dir();
        fs::create_dir_all(&data)?;
        let settings = Settings::load(&data);
        let profiles = config::profiles(&data)?;
        let (profile_id, profile) = profiles
            .iter()
            .find(|(id, _)| id == &settings.active_profile)
            .unwrap_or(&profiles[0])
            .clone();
        let (tx, rx) = mpsc::channel();
        let tools = Tools::find(data.clone());
        let error = tools.as_ref().err().map(ToString::to_string);
        let mut app = Self {
            data,
            tools: tools.ok(),
            settings,
            profile_id,
            profile,
            profiles,
            dirty: false,
            page: Page::Connect,
            status: "Ready. Connect an Android device to begin.".into(),
            error,
            engine: Engine::new()?,
            session: None,
            busy: false,
            connecting: false,
            cancel: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            pair_address: String::new(),
            pair_code: String::new(),
            discovered: vec![],
            devices: vec![],
            encoders: vec![],
            texture: None,
            selected: None,
            add_kind: None,
            dragging: None,
            record: None,
            pending: None,
            confirm_delete: false,
            allow_close: false,
            exit_requested: false,
            restart_after_stop: false,
            last_log_read: Instant::now(),
            fps: None,
            video_details: String::new(),
            audio_details: String::new(),
            image_size: Vec2::new(2400., 1080.),
            #[cfg(feature = "qa")]
            qa_capture: QaCapture::new(),
        };
        app.theme(&cc.egui_ctx);
        app.refresh();
        Ok(app)
    }
    fn accent(&self) -> Color32 {
        match self.settings.theme.as_str() {
            "Light" => Color32::from_rgb(8, 126, 119),
            "Ocean" => Color32::from_rgb(90, 200, 250),
            _ => Color32::from_rgb(57, 216, 195),
        }
    }
    fn theme(&self, ctx: &egui::Context) {
        let mut v = if self.settings.theme == "Light" {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        let light = self.settings.theme == "Light";
        v.override_text_color = Some(if light {
            Color32::from_rgb(23, 38, 61)
        } else {
            Color32::from_rgb(233, 240, 255)
        });
        v.panel_fill = if light {
            Color32::from_rgb(238, 243, 250)
        } else if self.settings.theme == "Ocean" {
            Color32::from_rgb(7, 28, 42)
        } else {
            Color32::from_rgb(16, 21, 34)
        };
        v.window_fill = if light {
            Color32::WHITE
        } else {
            Color32::from_rgb(25, 34, 52)
        };
        v.extreme_bg_color = if light {
            Color32::from_rgb(244, 247, 252)
        } else {
            Color32::from_rgb(12, 18, 30)
        };
        v.selection.bg_fill = self.accent().gamma_multiply(0.4);
        v.selection.stroke = Stroke::new(1.0_f32, self.accent());
        v.widgets.active.bg_fill = self.accent().gamma_multiply(0.35);
        v.widgets.hovered.bg_fill = self.accent().gamma_multiply(0.18);
        v.widgets.inactive.bg_fill = if light {
            Color32::from_rgb(225, 233, 244)
        } else {
            Color32::from_rgb(36, 49, 70)
        };
        v.widgets.inactive.weak_bg_fill = v.widgets.inactive.bg_fill;
        v.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
        ctx.set_visuals(v);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = Vec2::new(10., 10.);
            s.spacing.button_padding = Vec2::new(13., 9.);
            s.spacing.interact_size.y = 32.;
            s.text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(15.));
            s.text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.));
            s.text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(28.));
        });
    }
    fn fail(&mut self, e: impl ToString) {
        let text = e.to_string();
        self.status = text.clone();
        self.error = Some(text.clone());
        let _ = fs::write(self.data.join("pocketplay-last-error.log"), text);
    }
    fn job(&mut self, title: &str, f: impl FnOnce() -> Result<Reply> + Send + 'static) {
        if self.busy {
            self.status = "Another device request is in progress. Please wait.".into();
            return;
        }
        self.busy = true;
        self.status = title.into();
        self.error = None;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(f().map_err(|e| format!("{e:#}")));
        });
    }
    fn refresh(&mut self) {
        if let Some(t) = self.tools.clone() {
            let mode = self.settings.connection_mode.clone();
            self.job("Looking for devices...", move || {
                Ok(Reply::Devices(t.devices(&mode)?))
            });
        }
    }
    fn save_profile(&mut self) -> bool {
        match config::save_profile(&self.data, &self.profile_id, &self.profile) {
            Ok(()) => {
                self.dirty = false;
                self.settings.active_profile = self.profile_id.clone();
                self.profiles =
                    config::profiles(&self.data).unwrap_or_else(|_| self.profiles.clone());
                self.engine
                    .profile(self.profile.clone(), self.settings.touch_drag_sensitivity);
                self.status = "Profile saved.".into();
                true
            }
            Err(e) => {
                self.fail(e);
                false
            }
        }
    }
    fn save_settings(&mut self) -> bool {
        match self.settings.save(&self.data) {
            Ok(()) => {
                self.status = "Settings saved. Stream changes apply on the next session.".into();
                true
            }
            Err(e) => {
                self.fail(e);
                false
            }
        }
    }
    fn start(&mut self) {
        if self.busy || self.session.is_some() {
            return;
        }
        if !self.save_profile() || !self.save_settings() {
            return;
        }
        if let Some(t) = self.tools.clone() {
            self.cancel = Arc::new(AtomicBool::new(false));
            let cancel = self.cancel.clone();
            let s = self.settings.clone();
            self.connecting = true;
            self.job("Connecting to your phone...", move || {
                Ok(Reply::Session(Box::new(Session::start(t, &s, cancel)?)))
            });
        }
    }
    fn stop(&mut self, kill: bool) {
        if self.connecting {
            self.cancel.store(true, Ordering::Relaxed);
            self.status = "Cancelling connection...".into();
            return;
        }
        if self.busy {
            return;
        }
        self.engine.detach();
        self.fps = None;
        self.video_details.clear();
        self.audio_details.clear();
        if let Some(mut s) = self.session.take() {
            let tools = s.tools.clone();
            self.job(
                if kill {
                    "Stopping session before killing ADB..."
                } else {
                    "Stopping session and restoring the phone display..."
                },
                move || {
                    s.stop();
                    if kill {
                        tools.kill_server()?;
                        Ok(Reply::Killed)
                    } else {
                        Ok(Reply::Stopped(false))
                    }
                },
            );
        } else if kill {
            if let Some(t) = self.tools.clone() {
                self.job("Stopping ADB server...", move || {
                    t.kill_server()?;
                    Ok(Reply::Killed)
                });
            }
        }
    }
    fn apply_reply(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.rx.try_recv() {
            self.busy = false;
            self.connecting = false;
            match result {
                Err(e) => {
                    self.restart_after_stop = false;
                    self.fail(e);
                }
                Ok(Reply::Devices(devices)) => {
                    if !devices
                        .iter()
                        .any(|d| d.serial == self.settings.device_serial)
                    {
                        self.settings.device_serial = if devices.len() == 1 {
                            devices[0].serial.clone()
                        } else {
                            String::new()
                        };
                    }
                    self.status = if devices.is_empty() {
                        format!("No {} ADB devices found. Unlock your phone, enable debugging, reconnect and Refresh.",self.settings.connection_mode)
                    } else {
                        format!(
                            "{} {} device(s) found.",
                            devices.len(),
                            self.settings.connection_mode
                        )
                    };
                    self.devices = devices;
                }
                Ok(Reply::Session(session)) => {
                    if self.cancel.load(Ordering::Relaxed) {
                        let mut s = *session;
                        self.job("Cleaning up cancelled connection...", move || {
                            s.stop();
                            Ok(Reply::Stopped(false))
                        });
                    } else {
                        let s = *session;
                        self.settings.device_serial = s.serial.clone();
                        let Some(touch) = s.touch.as_ref() else {
                            self.fail("The session has no input control channel.");
                            continue;
                        };
                        let size = touch
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .size;
                        self.engine.attach(
                            Arc::clone(touch),
                            self.profile.clone(),
                            self.settings.touch_drag_sensitivity,
                        );
                        self.status = format!(
                            "Connected | {} x {} | Click the mirror; press ` to toggle keymap.",
                            size.0, size.1
                        );
                        if let Some(w) = &s.warning {
                            self.status.push_str(&format!(" {w}"));
                        }
                        self.session = Some(s);
                    }
                }
                Ok(Reply::Killed) => {
                    self.devices.clear();
                    self.settings.device_serial.clear();
                    self.status =
                        "ADB server stopped. Refresh devices or start a session to restart it."
                            .into();
                }
                Ok(Reply::Stopped(refresh)) => {
                    self.status = "Session stopped. Phone display restored.".into();
                    if self.restart_after_stop {
                        self.restart_after_stop = false;
                        self.start();
                    } else if refresh {
                        self.refresh();
                    }
                }
                Ok(Reply::Connected(address)) => {
                    self.settings.device_serial = address;
                    self.refresh();
                }
                Ok(Reply::Discovered(a)) => {
                    if a.len() == 1 {
                        self.settings.wireless_address = a[0].clone();
                    }
                    self.status = format!("{} wireless service(s) discovered.", a.len());
                    self.discovered = a;
                }
                Ok(Reply::Image(b)) => {
                    if let Err(e) = self.load_image(ctx, &b) {
                        self.fail(e);
                    } else {
                        self.status =
                            "Screenshot loaded. Place your controls and save your profile.".into();
                    }
                }
                Ok(Reply::Encoders(e)) => {
                    self.encoders = e;
                    self.status =
                        "Encoder list loaded. Choose a matching encoder or leave it automatic."
                            .into();
                }
                Ok(Reply::Text(t)) => self.status = t,
            }
        }
    }
    fn load_image(&mut self, ctx: &egui::Context, bytes: &[u8]) -> Result<()> {
        let image = crate::files::decode_screenshot(bytes)?;
        let size = [image.width() as usize, image.height() as usize];
        self.image_size = Vec2::new(size[0] as f32, size[1] as f32);
        let image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        self.texture =
            Some(ctx.load_texture("phone-snapshot", image, egui::TextureOptions::LINEAR));
        Ok(())
    }
    fn request(&mut self, pending: Pending) {
        if self.dirty {
            self.pending = Some(pending);
        } else {
            self.perform(pending);
        }
    }
    fn perform(&mut self, pending: Pending) {
        match pending {
            Pending::Load(id, p) => {
                self.profile_id = id;
                self.profile = p;
                self.dirty = false;
                self.selected = None;
                self.settings.active_profile = self.profile_id.clone();
                self.engine
                    .profile(self.profile.clone(), self.settings.touch_drag_sensitivity);
            }
            Pending::New => {
                self.profile_id = config::id();
                self.profile = Profile {
                    name: "Untitled profile".into(),
                    ..Profile::default()
                };
                self.dirty = true;
                self.selected = None;
                self.engine
                    .profile(self.profile.clone(), self.settings.touch_drag_sensitivity);
            }
            Pending::Delete => self.confirm_delete = true,
            Pending::Import(path) => {
                let result = (|| {
                    let p: Profile = serde_json::from_slice(&crate::files::read_bounded(
                        &path,
                        crate::files::MAX_JSON_BYTES,
                    )?)?;
                    p.validate()?;
                    Ok::<_, anyhow::Error>(p)
                })();
                match result {
                    Ok(p) => {
                        self.profile_id = config::id();
                        self.profile = p;
                        self.dirty = true;
                        self.selected = None;
                        self.save_profile();
                    }
                    Err(e) => self.fail(e),
                }
            }
            Pending::Close => {
                self.exit_requested = true;
                self.restart_after_stop = false;
                self.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
    fn header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.heading(title);
        ui.add_space(2.);
        ui.label(RichText::new(subtitle).weak());
        ui.add_space(18.);
    }
    fn card(ui: &mut egui::Ui, title: &str, f: impl FnOnce(&mut egui::Ui)) {
        egui::Frame::new()
            .fill(ui.visuals().window_fill)
            .corner_radius(12)
            .inner_margin(20)
            .show(ui, |ui| {
                ui.set_min_width((ui.available_width() - 2.).max(20.));
                ui.label(RichText::new(title).size(18.).strong());
                ui.add_space(10.);
                f(ui);
            });
        ui.add_space(12.);
    }
    fn connect_page(&mut self, ui: &mut egui::Ui) {
        Self::header(
            ui,
            "Your phone. Your desktop.",
            "USB or Wi-Fi. Native rendering. Your own controls.",
        );
        Self::card(ui, "Connection", |ui| {
            ui.label(RichText::new("Unlock your phone and allow debugging when prompted.").weak());
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let usb_changed = ui.selectable_value(
                        &mut self.settings.connection_mode, "USB".into(), "USB"
                    ).changed();
                    let wifi_changed = ui.selectable_value(
                        &mut self.settings.connection_mode, "Wi-Fi".into(), "Wi-Fi"
                    ).changed();
                    if usb_changed || wifi_changed {
                        self.settings.device_serial.clear();
                        self.devices.clear();
                        self.refresh();
                    }
                    let label = self.devices.iter()
                        .find(|device| device.serial == self.settings.device_serial)
                        .map(|device| format!("{} | {}", device.model, device.state))
                        .unwrap_or_else(|| "Select a device".into());
                    egui::ComboBox::from_id_salt("device")
                        .selected_text(label).width(270.).show_ui(ui, |ui| {
                            for device in &self.devices {
                                ui.selectable_value(&mut self.settings.device_serial,
                                    device.serial.clone(),
                                    format!("{} | {} | {}", device.model, device.serial, device.state));
                            }
                        });
                    if ui.button("Refresh").clicked() { self.refresh(); }
                    if ui.button("Kill ADB server").on_hover_text(
                        "Stops the shared PC ADB server after ending this app's session. Refresh starts it again."
                    ).clicked() { self.stop(true); }
                });
            });
            ui.add_space(6.);
            if self.settings.connection_mode == "USB" {
                ui.label("Enable Developer options > USB debugging and use a data cable.");
                ui.label(RichText::new("Xiaomi touch injection may also require USB debugging (Security settings).").weak());
            } else {
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.columns(2, |columns| {
                        let ui = &mut columns[0];
                        ui.strong("1. Pair once");
                        ui.label("Phone > Wireless debugging > Pair using pairing code");
                        ui.label("Pairing IP:port");
                        ui.text_edit_singleline(&mut self.pair_address);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.pair_code)
                                    .password(true)
                                    .hint_text("6-digit code")
                                    .desired_width(120.),
                            );
                            if ui.button("Pair device").clicked() {
                                if let Some(t) = self.tools.clone() {
                                    let a = self.pair_address.trim().to_string();
                                    let code = std::mem::take(&mut self.pair_code);
                                    self.job("Pairing with your phone...", move || {
                                        Ok(Reply::Text(t.pair(&a, &code)?))
                                    });
                                }
                            }
                        });
                        let ui = &mut columns[1];
                        ui.strong("2. Connect");
                        ui.label("Use the main Wireless debugging screen's IP:port.");
                        ui.label("Connection IP:port");
                        ui.text_edit_singleline(&mut self.settings.wireless_address);
                        if !self.discovered.is_empty() {
                            egui::ComboBox::from_id_salt("discovered")
                                .selected_text("Discovered services")
                                .show_ui(ui, |ui| {
                                    for address in &self.discovered {
                                        if ui.selectable_label(false, address).clicked() {
                                            self.settings.wireless_address = address.clone();
                                        }
                                    }
                                });
                        }
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Discover").clicked() {
                                if let Some(t) = self.tools.clone() {
                                    self.job("Discovering wireless services...", move || {
                                        Ok(Reply::Discovered(t.discover()?))
                                    });
                                }
                            }
                            if ui.button("Connect Wi-Fi").clicked() {
                                if let Some(t) = self.tools.clone() {
                                    let a = self.settings.wireless_address.trim().to_string();
                                    self.job("Connecting over Wi-Fi...", move || {
                                        Ok(Reply::Connected(t.connect(&a)?))
                                    });
                                }
                            }
                            if ui.button("Disconnect").clicked() {
                                self.disconnect_wifi();
                            }
                        });
                    });
                });
                ui.label(RichText::new("Both devices must use the same network. USB usually provides more consistent latency.").weak());
            }
        });
        Self::card(ui, "Start your session", |ui| {
            ui.label("Stream and audio changes apply on your next session.");
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.busy && self.session.is_none(),
                        egui::Button::new(RichText::new("Start session").strong())
                            .fill(self.accent().gamma_multiply(0.35)),
                    )
                    .clicked()
                {
                    self.start();
                }
                if ui
                    .add_enabled(
                        self.session.is_some() || self.connecting,
                        egui::Button::new(if self.connecting { "Cancel" } else { "Stop" }),
                    )
                    .clicked()
                {
                    self.stop(false);
                }
                let mode = self.engine.snapshot().mode;
                if ui
                    .add_enabled(
                        self.session.is_some() && !self.busy,
                        egui::Button::new(if mode {
                            "Disable keymap"
                        } else {
                            "Enable keymap"
                        }),
                    )
                    .clicked()
                {
                    self.engine.toggle();
                }
                if ui.button("Show mirror").clicked() {
                    native::focus_mirror();
                }
            });
            ui.label(
                RichText::new(
                    "Press ` beside 1 in the mirror to switch between keymap and touch mode.",
                )
                .weak(),
            );
        });
        Self::card(ui, "Build your own layout", |ui| {
            ui.label("Profiles start empty. Add controls wherever you want in Keymap Studio.");
            if ui.button("Open Keymap Studio").clicked() {
                self.page = Page::Keymap;
            }
        });
    }
    fn disconnect_wifi(&mut self) {
        if self.busy {
            return;
        }
        let serial = self.settings.device_serial.clone();
        if serial.is_empty() {
            self.status = "Select a wireless device first.".into();
            return;
        }
        if let Some(t) = self.tools.clone() {
            self.engine.detach();
            let session = self.session.take();
            self.job("Disconnecting Wi-Fi...", move || {
                if let Some(mut s) = session {
                    s.stop();
                }
                t.run("", &["disconnect", &serial], 15)?;
                Ok(Reply::Stopped(true))
            });
        }
    }
    fn screenshot(&mut self) {
        let serial = self
            .session
            .as_ref()
            .map(|s| s.serial.clone())
            .unwrap_or_else(|| self.settings.device_serial.clone());
        if serial.is_empty() {
            self.status = "Select a phone in Connect first.".into();
            return;
        }
        if let Some(t) = self.tools.clone() {
            self.engine
                .profile(self.profile.clone(), self.settings.touch_drag_sensitivity);
            self.job("Capturing phone screenshot...", move || {
                Ok(Reply::Image(t.snapshot(&serial)?))
            });
        }
    }
    fn keymap_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Self::header(
            ui,
            "Keymap Studio",
            "Create, place and bind your controls. Profiles scale with the phone's display.",
        );
        ui.horizontal_wrapped(|ui| {
            let mut chosen = None;
            egui::ComboBox::from_id_salt("profile")
                .selected_text(&self.profile.name)
                .width(200.)
                .show_ui(ui, |ui| {
                    for (id, p) in &self.profiles {
                        if ui
                            .selectable_label(id == &self.profile_id, &p.name)
                            .clicked()
                        {
                            chosen = Some((id.clone(), p.clone()));
                        }
                    }
                });
            if let Some((id, p)) = chosen {
                self.request(Pending::Load(id, p));
            }
            if ui.button("New").clicked() {
                self.request(Pending::New);
            }
            if ui.button("Import").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("PocketPlay profile", &["json"])
                    .pick_file()
                {
                    self.request(Pending::Import(path));
                }
            }
            if ui.button("Export").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("PocketPlay profile", &["json"])
                    .set_file_name("PocketPlay-profile.json")
                    .save_file()
                {
                    if let Err(e) = self
                        .profile
                        .validate()
                        .and_then(|_| config::atomic_json(&path, &self.profile))
                    {
                        self.fail(e);
                    } else {
                        self.status = "Profile exported.".into();
                    }
                }
            }
            if ui.button("Save profile").clicked() {
                self.save_profile();
            }
            ui.label(
                RichText::new(if self.dirty {
                    "Unsaved changes"
                } else {
                    "Saved"
                })
                .color(if self.dirty {
                    Color32::from_rgb(255, 188, 83)
                } else {
                    self.accent()
                }),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Profile name");
            if ui.text_edit_singleline(&mut self.profile.name).changed() {
                self.dirty = true;
            }
            if ui.button("Delete profile").clicked() {
                self.request(Pending::Delete);
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!self.busy, egui::Button::new("Refresh screenshot"))
                .clicked()
            {
                self.screenshot();
            }
            if ui.button("Load image").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Screenshot", &["png", "jpg", "jpeg"])
                    .pick_file()
                {
                    match crate::files::read_bounded(&path, crate::files::MAX_IMAGE_BYTES)
                        .and_then(|b| self.load_image(ctx, &b))
                    {
                        Ok(()) => {}
                        Err(e) => self.fail(e),
                    }
                }
            }
            for (kind, label) in [
                ("button", "+ Tap / hold"),
                ("joystick", "+ Joystick"),
                ("camera", "+ Mouse look"),
                ("swipe", "+ Swipe"),
            ] {
                if ui
                    .selectable_label(self.add_kind.as_deref() == Some(kind), label)
                    .clicked()
                {
                    self.add_kind = Some(kind.into());
                }
            }
        });
        if let Some(kind) = &self.add_kind {
            ui.label(
                RichText::new(format!(
                    "Click the screenshot to place {kind}. Press Esc to cancel."
                ))
                .color(self.accent()),
            );
        } else {
            ui.label(
                RichText::new("Drag controls to reposition. Select one to edit its properties.")
                    .weak(),
            );
        }
        ui.add_space(5.);
        let available = ui.available_size();
        let props_width = 260.;
        ui.horizontal_top(|ui| {
            ui.allocate_ui(
                Vec2::new(
                    (available.x - props_width - 18.).max(260.),
                    available.y.max(280.),
                ),
                |ui| {
                    self.editor_canvas(ui);
                },
            );
            egui::ScrollArea::vertical()
                .id_salt("properties")
                .max_width(props_width)
                .show(ui, |ui| {
                    ui.set_width(props_width);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(props_width);
                        self.properties(ui);
                    });
                });
        });
    }
    fn editor_canvas(&mut self, ui: &mut egui::Ui) {
        let size = ui.available_size();
        let scale = (size.x / self.image_size.x).min(size.y.max(220.) / self.image_size.y);
        let display = self.image_size * scale;
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(size.x, display.y.max(220.)),
            Sense::click_and_drag(),
        );
        let image_rect = Rect::from_center_size(rect.center(), display);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 8, ui.visuals().extreme_bg_color);
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                image_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                Color32::WHITE,
            );
        } else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Refresh a phone screenshot or load an image\nto place your controls.",
                egui::FontId::proportional(16.),
                ui.visuals().weak_text_color(),
            );
        }
        let screen = |p: [f64; 2]| {
            Pos2::new(
                image_rect.left() + p[0] as f32 * image_rect.width(),
                image_rect.top() + p[1] as f32 * image_rect.height(),
            )
        };
        for (i, c) in self.profile.controls.iter().enumerate() {
            let p = screen(c.point);
            let accent = if self.selected == Some(i) {
                Color32::from_rgb(255, 205, 88)
            } else {
                self.accent()
            };
            if c.kind == "joystick" {
                painter.circle_stroke(
                    p,
                    c.radius as f32 * image_rect.height(),
                    Stroke::new(2.0_f32, accent),
                );
            }
            if c.kind == "swipe" {
                let end = screen(c.end);
                painter.line_segment([p, end], Stroke::new(2.0_f32, accent));
                painter.circle_filled(end, 7., accent);
            }
            painter.circle_filled(p, 18., Color32::from_black_alpha(180));
            painter.circle_stroke(p, 18., Stroke::new(2.0_f32, accent));
            let label = if c.kind == "camera" {
                "LOOK".into()
            } else if c.kind == "joystick" {
                "MOVE".into()
            } else if c.key.is_empty() {
                "?".into()
            } else {
                c.key.to_uppercase().replace("MOUSE_", "M")
            };
            painter.text(
                p,
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(12.),
                Color32::WHITE,
            );
        }
        if let Some(pos) = response.interact_pointer_pos() {
            let point = [
                ((pos.x - image_rect.left()) / image_rect.width()).clamp(0., 1.) as f64,
                ((pos.y - image_rect.top()) / image_rect.height()).clamp(0., 1.) as f64,
            ];
            let hit = self
                .profile
                .controls
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    if c.kind == "swipe" && screen(c.end).distance(pos) < 14. {
                        Some((i, true, screen(c.end).distance(pos)))
                    } else {
                        let distance = screen(c.point).distance(pos);
                        (distance < 24.).then_some((i, false, distance))
                    }
                })
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .map(|(i, end, _)| (i, end));
            if response.drag_started() {
                self.dragging = hit;
                self.selected = hit.map(|(i, _)| i);
            }
            if response.dragged() {
                if let Some((i, end)) = self.dragging {
                    if let Some(c) = self.profile.controls.get_mut(i) {
                        if end {
                            c.end = point;
                        } else {
                            c.point = point;
                        }
                        self.dirty = true;
                    }
                }
            }
            if response.clicked() && image_rect.contains(pos) {
                if let Some(kind) = self.add_kind.take() {
                    if self.profile.controls.len() >= 100 {
                        self.fail("A profile supports up to 100 controls.");
                    } else if kind == "camera"
                        && self.profile.controls.iter().any(|c| c.kind == "camera")
                    {
                        self.fail("A profile supports one mouse-look control.");
                    } else {
                        self.profile.controls.push(Control::new(&kind, point));
                        self.selected = Some(self.profile.controls.len() - 1);
                        self.dirty = true;
                    }
                } else {
                    self.selected = hit.map(|(i, _)| i);
                }
            }
        }
        if response.drag_stopped() {
            self.dragging = None;
        }
    }
    fn properties(&mut self, ui: &mut egui::Ui) {
        ui.strong("CONTROL PROPERTIES");
        if let Some(i) = self.selected.filter(|i| *i < self.profile.controls.len()) {
            let mut c = self.profile.controls[i].clone();
            ui.label(RichText::new(&c.kind).color(self.accent()));
            ui.label("Label");
            ui.text_edit_singleline(&mut c.label);
            ui.horizontal(|ui| {
                ui.label("X");
                ui.add(
                    egui::DragValue::new(&mut c.point[0])
                        .speed(0.001)
                        .range(0.0..=1.0),
                );
                ui.label("Y");
                ui.add(
                    egui::DragValue::new(&mut c.point[1])
                        .speed(0.001)
                        .range(0.0..=1.0),
                );
            });
            if c.kind == "button" || c.kind == "swipe" {
                self.binding(ui, i, "key", "Binding", &mut c.key);
            }
            if c.kind == "joystick" {
                self.binding(ui, i, "up", "Up", &mut c.keys.up);
                self.binding(ui, i, "down", "Down", &mut c.keys.down);
                self.binding(ui, i, "left", "Left", &mut c.keys.left);
                self.binding(ui, i, "right", "Right", &mut c.keys.right);
                ui.label("Radius / screen height");
                ui.add(
                    egui::DragValue::new(&mut c.radius)
                        .speed(0.005)
                        .range(0.01..=0.45),
                );
            }
            if c.kind == "camera" {
                ui.label("Mouse-look sensitivity");
                ui.add(
                    egui::DragValue::new(&mut c.sensitivity)
                        .speed(0.01)
                        .range(0.05..=1.5),
                );
                ui.checkbox(&mut c.invert_y, "Invert vertical movement");
                let mut idle_release = c.idle_release_ms != 0;
                if ui
                    .checkbox(&mut idle_release, "Release mouse look when idle")
                    .changed()
                {
                    c.idle_release_ms = if idle_release { 500 } else { 0 };
                }
                if idle_release {
                    ui.label("Idle release delay (ms)");
                    ui.add(egui::DragValue::new(&mut c.idle_release_ms).range(250..=5000));
                }
                ui.label(
                    RichText::new(
                        "Raw relative movement. Idle release is off by default for immediate movement after pauses. Enable it to renew gestures after transitions.",
                    )
                    .weak(),
                );
            }
            if c.kind == "swipe" {
                ui.label("Endpoint");
                ui.horizontal(|ui| {
                    ui.label("X");
                    ui.add(
                        egui::DragValue::new(&mut c.end[0])
                            .speed(0.001)
                            .range(0.0..=1.0),
                    );
                    ui.label("Y");
                    ui.add(
                        egui::DragValue::new(&mut c.end[1])
                            .speed(0.001)
                            .range(0.0..=1.0),
                    );
                });
                ui.label("Duration (ms)");
                ui.add(egui::DragValue::new(&mut c.duration_ms).range(20..=5000));
            }
            if serde_json::to_value(&c).ok() != serde_json::to_value(&self.profile.controls[i]).ok()
            {
                self.profile.controls[i] = c;
                self.dirty = true;
            }
            ui.add_space(12.);
            if ui.button("Delete control").clicked() {
                self.profile.controls.remove(i);
                self.selected = None;
                self.record = None;
                self.dirty = true;
            }
        } else {
            ui.label(
                RichText::new("Select a control on the screenshot or in the list below.").weak(),
            );
        }
        ui.separator();
        ui.strong("CONTROLS");
        let mut selected = self.selected;
        for (i, c) in self.profile.controls.iter().enumerate() {
            if ui
                .selectable_label(selected == Some(i), format!("{}  ·  {}", c.label, c.kind))
                .clicked()
            {
                selected = Some(i);
            }
        }
        self.selected = selected;
    }
    fn binding(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        field: &str,
        label: &str,
        key: &mut String,
    ) {
        ui.label(label);
        ui.horizontal(|ui| {
            if ui
                .add(egui::TextEdit::singleline(key).desired_width(160.))
                .changed()
            {
                *key = key.trim().to_lowercase();
            }
            if ui.small_button("Record").clicked() {
                self.record = Some(Recording {
                    index,
                    field: field.into(),
                    started: Instant::now(),
                });
            }
        });
    }
    fn stream_page(&mut self, ui: &mut egui::Ui) {
        Self::header(
            ui,
            "Stream & Audio",
            "Choose stream quality freely. Game graphics and game FPS stay under your control.",
        );
        ui.horizontal_wrapped(|ui| {
            for (name, size, rate) in [
                ("Low latency", 1920, 8.),
                ("Smooth", 1280, 5.),
                ("Quality", 0, 16.),
            ] {
                if ui.button(name).clicked() {
                    self.settings.max_size = size;
                    self.settings.bitrate_mbps = rate;
                    self.settings.max_fps = 60;
                    self.settings.video_codec = "h264".into();
                    self.settings.video_encoder.clear();
                    self.settings.decoder = "auto".into();
                    self.settings.video_buffer_ms = 0;
                    self.settings.audio_buffer_ms = 0;
                    if name == "Low latency" {
                        self.settings.screen_off = false;
                        self.settings.fullscreen = true;
                        self.settings.vsync = false;
                    }
                }
            }
        });
        ui.add_space(8.);
        ui.columns(2, |columns| {
            self.video_stream_card(&mut columns[0]);
            self.phone_display_card(&mut columns[1]);
        });
        self.audio_card(ui);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Save settings").clicked() {
                self.save_settings();
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new("Apply & restart session"))
                .clicked()
                && self.save_settings()
            {
                if self.session.is_some() {
                    self.restart_after_stop = true;
                    self.stop(false);
                } else {
                    self.start();
                }
            }
        });
    }
    fn video_stream_card(&mut self, ui: &mut egui::Ui) {
        Self::card(ui, "Video stream", |ui| {
            egui::Grid::new("video-fields")
                .num_columns(2)
                .spacing([24., 12.])
                .show(ui, |ui| {
                    ui.label("Max edge (px)");
                    ui.add(
                        egui::DragValue::new(&mut self.settings.max_size)
                            .range(0..=8192)
                            .speed(16.),
                    );
                    ui.end_row();
                    ui.label("Video bitrate (Mbps)");
                    ui.add(
                        egui::DragValue::new(&mut self.settings.bitrate_mbps)
                            .range(0.1..=200.)
                            .speed(0.1),
                    );
                    ui.end_row();
                    ui.label("Capture FPS target");
                    ui.add(egui::DragValue::new(&mut self.settings.max_fps).range(0..=240));
                    ui.end_row();
                });
            ui.label(RichText::new("0 px = native size · 0 FPS = no cap").weak());
            ui.add_space(10.);
            ui.checkbox(&mut self.settings.fullscreen, "Fullscreen playback (F11)");
            ui.checkbox(
                &mut self.settings.vsync,
                "Synchronize to PC display (VSync)",
            )
            .on_hover_text("Can smooth presentation and add video latency.");
            ui.add_space(8.);
            ui.collapsing("Advanced video settings", |ui| {
                egui::Grid::new("advanced-video-fields")
                    .num_columns(2)
                    .spacing([12., 12.])
                    .show(ui, |ui| {
                        ui.label("Video codec");
                        combo(
                            ui,
                            "codec",
                            &mut self.settings.video_codec,
                            &["h264", "h265", "av1", "vp8", "vp9"],
                        );
                        ui.end_row();
                        ui.label("Decoder");
                        combo(
                            ui,
                            "decoder",
                            &mut self.settings.decoder,
                            &["disabled", "auto", "d3d11va"],
                        );
                        ui.end_row();
                        ui.label("Device encoder");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings.video_encoder)
                                .hint_text("Automatic")
                                .desired_width(180.),
                        );
                        ui.end_row();
                    });
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Find available encoders"))
                    .clicked()
                {
                    let serial = self
                        .session
                        .as_ref()
                        .map(|s| s.serial.clone())
                        .unwrap_or_else(|| self.settings.device_serial.clone());
                    if serial.is_empty() {
                        self.status = "Select a phone in Connect first.".into();
                    } else if let Some(t) = self.tools.clone() {
                        self.job("Reading device encoders...", move || {
                            Ok(Reply::Encoders(t.encoders(&serial)?))
                        });
                    }
                }
                for line in self.encoders.clone() {
                    if ui.selectable_label(false, &line).clicked() {
                        if let Some(name) = line.split(" | ").next() {
                            self.settings.video_encoder = name
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .trim_matches(['\'', '"'])
                                .into();
                        }
                    }
                }
            });
        });
    }
    fn phone_display_card(&mut self, ui: &mut egui::Ui) {
        Self::card(ui, "Phone display", |ui| {
            ui.checkbox(
                &mut self.settings.screen_off,
                "Turn off phone display while streaming",
            );
            ui.label(
                RichText::new("Android stays awake during streaming. Stop restores the display.")
                    .weak(),
            );
            ui.add_space(8.);
            ui.label(RichText::new(
                "Xiaomi: the screen goes black while refresh timing stays active for high FPS. This uses more power than fully powering the panel off."
            ).weak());
            ui.separator();
            ui.label("Right-drag sensitivity");
            ui.add(
                egui::DragValue::new(&mut self.settings.touch_drag_sensitivity)
                    .range(0.05..=3.)
                    .speed(0.01),
            );
        });
    }
    fn audio_card(&mut self, ui: &mut egui::Ui) {
        Self::card(ui, "Audio", |ui| {
            egui::Grid::new("audio-fields")
                .spacing([24., 12.])
                .show(ui, |ui| {
                    ui.label("Playback");
                    combo(
                        ui,
                        "audio-mode",
                        &mut self.settings.audio_mode,
                        &["Phone only", "Computer only", "Computer + phone"],
                    );
                    ui.end_row();
                    ui.label("Audio format");
                    ui.label("PCM · 48 kHz · stereo · 16-bit");
                    ui.end_row();
                    ui.label("Audio buffer (ms)");
                    ui.add(egui::DragValue::new(&mut self.settings.audio_buffer_ms).range(0..=200));
                    ui.end_row();
                });
            ui.label(
                RichText::new(
                    "0 ms = no added prebuffer. Device and transport latency still apply.",
                )
                .weak(),
            );
            ui.label(RichText::new("If audio crackles over Wi-Fi, try a 10–20 ms buffer.").weak());
            if self.session.is_some() && !self.audio_details.is_empty() {
                ui.add_space(6.);
                ui.label(&self.audio_details);
            }
            ui.label(
                RichText::new(
                    "Computer + phone requires Android 13+; some apps block audio capture.",
                )
                .weak(),
            );
        });
    }
    fn appearance_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Self::header(ui,"Make it yours","Themes apply immediately. Profiles and settings remain in your existing app data folder.");
        Self::card(ui, "Theme", |ui| {
            ui.horizontal(|ui| {
                let before = self.settings.theme.clone();
                for theme in ["Midnight", "Light", "Ocean"] {
                    ui.selectable_value(&mut self.settings.theme, theme.into(), theme);
                }
                if before != self.settings.theme {
                    self.theme(ctx);
                    self.save_settings();
                }
            });
        });
        Self::card(ui, "App & saved data", |ui| {
            ui.label(format!("PocketPlay {VERSION} · Native Rust"));
            ui.label(self.data.display().to_string());
            ui.horizontal_wrapped(|ui| {
                if ui.button("Open app data folder").clicked() {
                    let _ = std::process::Command::new("explorer.exe")
                        .arg(&self.data)
                        .spawn();
                }
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Create desktop shortcut"))
                    .clicked()
                {
                    self.shortcut();
                }
            });
            ui.label(
                RichText::new(
                    "Native profiles and settings are stored in a separate PocketPlayNative folder.",
                )
                .weak(),
            );
        });
    }
    fn shortcut(&mut self) {
        self.job("Creating desktop shortcut...", move || {
            native::create_desktop_shortcut(&std::env::current_exe()?)?;
            Ok(Reply::Text("Desktop shortcut created.".into()))
        });
    }
    fn help_page(&mut self, ui: &mut egui::Ui) {
        Self::header(
            ui,
            "A quick guide",
            "From connection to your own control layout.",
        );
        ui.label("PocketPlay Native uses FFmpeg libraries under the LGPL. Corresponding source and license notices are included with the release.");
        let sections = [
            (
                "01 · Connect",
                "USB: enable USB debugging, unlock the phone and allow this computer. Wi-Fi: pair with the six-digit code, then connect using the main screen IP:port.",
            ),
            (
                "02 · Create your profile",
                "Refresh the screenshot in Keymap Studio. Add Tap / hold, Joystick, Mouse look or Swipe. Click to place, drag to move, and edit keys in the properties panel.",
            ),
            (
                "03 · Save and play",
                "Save your profile. Start a session and click the mirror. Press ` beside 1 to enable keyboard/mouse mapping; press again for normal mouse touch mode.",
            ),
            (
                "04 · Touch mode",
                "Left click touches the pointer position and supports holding/dragging. Right drag starts at the pointer position and has its own sensitivity.",
            ),
            (
                "05 · Tune the stream",
                "The FPS limit is a ceiling, not a guarantee. Game output, phone encoding, USB/Wi-Fi and PC decoding affect visible FPS. Rust input timing excludes Android processing and returned video delay.",
            ),
            (
                "06 · ADB troubleshooting",
                "Kill ADB server stops the shared host server. Refresh starts it again. A phone visible only as MTP needs debugging enabled or a suitable ADB driver before this app can connect.",
            ),
        ];
        for (title, text) in sections {
            Self::card(ui, title, |ui| {
                ui.label(text);
            });
        }
        ui.hyperlink_to("Android NDK", "https://developer.android.com/ndk");
        ui.hyperlink_to(
            "Android USB / Wi-Fi debugging",
            "https://developer.android.com/tools/adb",
        );
    }
    fn record_input(&mut self, ctx: &egui::Context) {
        let Some(r) = self.record.as_ref() else {
            return;
        };
        if r.started.elapsed() < Duration::from_millis(200) {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        let modifiers = ctx.input(|i| i.modifiers);
        let mut name = if modifiers.shift {
            Some("shift".into())
        } else if modifiers.ctrl {
            Some("ctrl".into())
        } else if modifiers.alt {
            Some("alt".into())
        } else {
            None
        };
        let mut cancel = false;
        for e in events {
            match e {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    ..
                } => {
                    let key = physical_key.unwrap_or(key);
                    if key == egui::Key::Escape {
                        cancel = true;
                        break;
                    }
                    let raw = format!("{key:?}").to_lowercase();
                    let key = match raw.as_str() {
                        "arrowup" => "up",
                        "arrowdown" => "down",
                        "arrowleft" => "left",
                        "arrowright" => "right",
                        _ => raw.strip_prefix("num").unwrap_or(&raw),
                    };
                    name = Some(key.to_string());
                    break;
                }
                egui::Event::PointerButton {
                    button,
                    pressed: true,
                    ..
                } => {
                    name = Some(
                        match button {
                            egui::PointerButton::Primary => "mouse_left",
                            egui::PointerButton::Secondary => "mouse_right",
                            egui::PointerButton::Middle => "mouse_middle",
                            _ => continue,
                        }
                        .into(),
                    );
                    break;
                }
                _ => {}
            }
        }
        if cancel {
            self.record = None;
            return;
        }
        if let Some(name) = name {
            let Some(r) = self.record.take() else {
                return;
            };
            if name == "`" || name == "backtick" || native::key_vk(&name) == Some(192) {
                self.fail("The ` key is reserved for switching modes.");
                return;
            }
            if native::key_vk(&name).is_none() {
                self.fail(format!(
                    "Unsupported recorded key: {name}. Enter a vk_N binding if needed."
                ));
                return;
            }
            if let Some(c) = self.profile.controls.get_mut(r.index) {
                match r.field.as_str() {
                    "up" => c.keys.up = name,
                    "down" => c.keys.down = name,
                    "left" => c.keys.left = name,
                    "right" => c.keys.right = name,
                    _ => c.key = name,
                }
                self.dirty = true;
            }
        }
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.record.is_some() {
            egui::Window::new("Record binding")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
                .show(ctx, |ui| {
                    ui.label("Press a key or mouse button. Esc cancels.");
                });
        }
        if self.pending.is_some() {
            egui::Window::new("Unsaved profile")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
                .show(ctx, |ui| {
                    ui.label("Save changes to this profile before continuing?");
                    ui.horizontal(|ui| {
                        if ui.button("Save & continue").clicked() && self.save_profile() {
                            if let Some(p) = self.pending.take() {
                                self.perform(p);
                            }
                        }
                        if ui.button("Discard").clicked() {
                            self.dirty = false;
                            if let Some(p) = self.pending.take() {
                                self.perform(p);
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending = None;
                        }
                    });
                });
        }
        if self.confirm_delete {
            egui::Window::new("Delete profile")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
                .show(ctx, |ui| {
                    ui.label(format!("Delete saved profile \"{}\"?", self.profile.name));
                    ui.horizontal(|ui| {
                        if ui.button("Delete").clicked() {
                            if config::safe_id(&self.profile_id) {
                                match fs::remove_file(
                                    self.data
                                        .join("profiles")
                                        .join(format!("{}.json", self.profile_id)),
                                ) {
                                    Ok(()) => match config::profiles(&self.data) {
                                        Ok(p) => {
                                            self.profiles = p;
                                            let (id, p) = self.profiles[0].clone();
                                            self.perform(Pending::Load(id, p));
                                        }
                                        Err(e) => self.fail(e),
                                    },
                                    Err(e) => self.fail(e),
                                }
                            }
                            self.confirm_delete = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_delete = false;
                        }
                    });
                });
        }
    }
    fn status_panel(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(ctx.style().visuals.window_fill).inner_margin(14))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if self.busy { ui.spinner(); }
                    ui.label(&self.status);
                });
                let metrics = self.engine.snapshot();
                let connected = self.session.is_some();
                let fps = self.fps.map(|fps| format!("{fps:0.0} FPS"))
                    .unwrap_or_else(|| if connected { "Waiting for frames".into() } else { "Disconnected".into() });
                let area = if connected && metrics.display_size.0 > 0 {
                    format!("{}×{}", metrics.display_size.0, metrics.display_size.1)
                } else { "—".into() };
                let mode = if metrics.mode { "Keymap" } else { "Touch" };
                ui.label(RichText::new(format!(
                    "Submitted: {fps} · Input: {mode} · Touch area: {area} · Packets: {} · Mouse dispatch: {:.2} ms (PC only)",
                    metrics.packets, metrics.send_ms
                )).size(12.).weak()).on_hover_text("Counts frames submitted by the viewer. Multiple submissions can share one display refresh when VSync is off; this is not game FPS or a physical displayed-frame measurement.");
                if connected {
                    if !self.video_details.is_empty() {ui.label(RichText::new(&self.video_details).size(12.).weak());}
                    ui.label(RichText::new(format!(
                        "PC input queue: {:.2} ms · Queue peak: {:.2} ms · Dispatch peak: {:.2} ms · Camera reanchors: {}",
                        metrics.queue_ms, metrics.max_queue_ms, metrics.max_send_ms, metrics.camera_reanchors
                    )).size(12.).weak()).on_hover_text(
                        "Peaks and reanchor count reset on a new session. These values exclude Android and returned video latency."
                    );
                }
            });
    }
    fn metrics(&mut self) {
        if self.last_log_read.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_log_read = Instant::now();
        if self.session.is_none() {
            return;
        }
        if let Ok(text) = crate::files::read_log_tail(&self.data.join("native-viewer.log")) {
            self.audio_details = text
                .lines()
                .rev()
                .find(|line| line.starts_with("Audio queue:"))
                .unwrap_or_default()
                .to_owned();
            self.fps = text.lines().rev().find_map(|l| {
                l.find(" fps")
                    .and_then(|i| l[..i].split_whitespace().last()?.parse().ok())
            });
            let decoder = text
                .lines()
                .rev()
                .find(|l| l.starts_with("Decoder:"))
                .unwrap_or("Decoder: starting");
            if let Some(line) = text.lines().rev().find(|l| l.contains("decode_ms=")) {
                self.video_details = format!(
                    "{decoder} · {} · {} (PC video stages)",
                    line.split_once("capture_fps=")
                        .and_then(|(_, v)| v.split_whitespace().next())
                        .map(|v| format!("Capture: {v} FPS"))
                        .unwrap_or_default(),
                    line.split_once("decode_ms=")
                        .map(|(_, metrics)| format!("decode_ms={metrics}"))
                        .unwrap_or_default()
                );
            }
            if let Some(warning) = text
                .lines()
                .rev()
                .find(|l| l.starts_with("Audio unavailable:"))
            {
                self.status = format!("Connected · {warning}");
            }
        }
        if let Some(s) = &mut self.session {
            if let Some(detail) = s.failure() {
                self.stop(false);
                self.fail(detail);
            }
        }
        if let Some(error) = self.engine.snapshot().error {
            self.stop(false);
            self.fail(format!("Input transport lost: {error}"));
        }
    }
}
fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, choices: &[&str]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(value.as_str())
        .width(240.)
        .show_ui(ui, |ui| {
            for choice in choices {
                ui.selectable_value(value, (*choice).into(), *choice);
            }
        });
}
impl eframe::App for PocketPlay {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_reply(ctx);
        self.record_input(ctx);
        self.metrics();
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.dirty {
                self.pending = Some(Pending::Close);
            } else {
                self.perform(Pending::Close);
            }
        }
        if self.exit_requested {
            if !self.busy && self.session.is_some() {
                self.stop(false);
            }
            if !self.busy && self.session.is_none() {
                self.allow_close = true;
            } else {
                self.status =
                    "Closing: finishing device cleanup and restoring the phone display...".into();
            }
        }
        if self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S)) {
            self.save_profile();
        }
        if self.record.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.add_kind = None;
        }
        let accent = self.accent();
        egui::SidePanel::left("sidebar")
            .exact_width(205.)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.extreme_bg_color)
                    .inner_margin(20),
            )
            .show(ctx, |ui| {
                ui.add_space(15.);
                ui.label(RichText::new("PocketPlay").size(27.).strong().color(accent));
                ui.label(RichText::new("PLAY YOUR WAY").size(11.).weak());
                ui.add_space(40.);
                for (page, label) in [
                    (Page::Connect, "01   Connect"),
                    (Page::Keymap, "02   Keymap Studio"),
                    (Page::Stream, "03   Stream & Audio"),
                    (Page::Appearance, "04   Appearance"),
                    (Page::Help, "05   Guide & About"),
                ] {
                    if ui
                        .add_sized(
                            [165., 42.],
                            egui::Button::new(label).selected(self.page == page),
                        )
                        .clicked()
                    {
                        self.page = page;
                        self.record = None;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(12.);
                    ui.label(
                        RichText::new(format!(
                            "PocketPlay {VERSION}\nNative Rust · PocketPlay engine"
                        ))
                        .size(12.)
                        .weak(),
                    );
                    ui.add_space(12.);
                    ui.label(
                        RichText::new(if self.connecting {
                            "CONNECTING"
                        } else if self.session.is_some() {
                            "CONNECTED"
                        } else if self.busy {
                            "WORKING"
                        } else {
                            "DISCONNECTED"
                        })
                        .strong()
                        .color(accent),
                    );
                    ui.label(RichText::new("DEVICE STATUS").size(11.).weak());
                });
            });
        self.status_panel(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(28),
            )
            .show(ctx, |ui| {
                if let Some(error) = self.error.clone() {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(100, 35, 42))
                        .corner_radius(8)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(error).color(Color32::WHITE));
                                if ui.small_button("Dismiss").clicked() {
                                    self.error = None;
                                }
                            });
                        });
                    ui.add_space(12.);
                }
                ui.add_enabled_ui(!self.exit_requested, |ui| {
                    if self.page == Page::Keymap {
                        self.keymap_page(ui, ctx);
                    } else {
                        egui::ScrollArea::vertical()
                            .id_salt("main-page")
                            .show(ui, |ui| match self.page {
                                Page::Connect => self.connect_page(ui),
                                Page::Stream => self.stream_page(ui),
                                Page::Appearance => self.appearance_page(ui, ctx),
                                Page::Help => self.help_page(ui),
                                _ => {}
                            });
                    }
                });
            });
        self.dialogs(ctx);
        #[cfg(feature = "qa")]
        self.capture_qa(ctx);
        ctx.request_repaint_after(Duration::from_millis(if self.busy {
            50
        } else if self.session.is_some() {
            200
        } else {
            500
        }));
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.engine.detach();
        if let Some(mut s) = self.session.take() {
            s.stop();
        }
        let _ = self.settings.save(&self.data);
    }
}

#[cfg(feature = "qa")]
struct QaCapture {
    dir: PathBuf,
    step: usize,
    phase: u8,
    due: Instant,
}
#[cfg(feature = "qa")]
impl QaCapture {
    fn new() -> Option<Self> {
        std::env::var_os("POCKETPLAY_DATA_DIR")?;
        Some(Self {
            dir: PathBuf::from(std::env::var_os("POCKETPLAY_CAPTURE_DIR")?),
            step: 0,
            phase: 0,
            due: Instant::now() + Duration::from_secs(1),
        })
    }
}
#[cfg(feature = "qa")]
impl PocketPlay {
    fn capture_qa(&mut self, ctx: &egui::Context) {
        let Some(mut qa) = self.qa_capture.take() else {
            return;
        };
        let pages = [
            (Page::Connect, "Midnight", "connect"),
            (Page::Keymap, "Midnight", "keymap"),
            (Page::Stream, "Midnight", "stream"),
            (Page::Appearance, "Light", "appearance"),
            (Page::Help, "Light", "guide"),
            (Page::Connect, "Ocean", "ocean"),
        ];
        if qa.step >= pages.len() {
            self.perform(Pending::Close);
            return;
        }
        if qa.phase == 2 {
            let events = ctx.input(|i| i.events.clone());
            for e in events {
                if let egui::Event::Screenshot { image, .. } = e {
                    let _ = fs::create_dir_all(&qa.dir);
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    if let Err(e) = image::save_buffer(
                        qa.dir.join(format!("{}.png", pages[qa.step].2)),
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    ) {
                        self.fail(e);
                    }
                    qa.step += 1;
                    qa.phase = 0;
                    qa.due = Instant::now();
                    break;
                }
            }
        }
        if qa.step < pages.len() && qa.due <= Instant::now() {
            match qa.phase {
                0 => {
                    self.page = pages[qa.step].0;
                    if self.page == Page::Keymap && !self.profile.controls.is_empty() {
                        self.selected = Some(0);
                    }
                    self.settings.theme = pages[qa.step].1.into();
                    self.theme(ctx);
                    qa.phase = 1;
                    qa.due = Instant::now() + Duration::from_millis(300);
                }
                1 => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                        egui::UserData::default(),
                    ));
                    qa.phase = 2;
                }
                _ => {}
            }
        }
        self.qa_capture = Some(qa);
        ctx.request_repaint_after(Duration::from_millis(30));
    }
}
