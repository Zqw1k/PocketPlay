#![windows_subsystem = "windows"]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
mod adb;
mod app;
mod bridge;
mod config;
mod files;
mod geometry;
mod input;
mod media;
mod native;
mod process;
mod viewer;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--native-viewer") {
        if let Err(e) = viewer::run(&args[1..]) {
            eprintln!("Viewer failed: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(feature = "qa")]
    if args.first().is_some_and(|a| a == "--native-session-probe") {
        let result = (|| -> anyhow::Result<()> {
            let data = config::data_dir();
            std::fs::create_dir_all(&data)?;
            let settings = config::Settings::load(&data);
            let tools = adb::Tools::find(data.clone())?;
            let mut session = bridge::Session::start(
                tools,
                &settings,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )?;
            let engine = input::Engine::new()?;
            if let Some(touch) = &session.touch {
                engine.attach(
                    touch.clone(),
                    config::profiles(&data)?
                        .into_iter()
                        .next()
                        .map(|(_, profile)| profile)
                        .unwrap_or_default(),
                    settings.touch_drag_sensitivity,
                );
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(18);
            while session.alive() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            engine.detach();
            session.stop();
            println!("Native session probe completed");
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Session probe failed: {error:#}");
            std::process::exit(1);
        }
        return;
    }
    let Some(_instance) = native::Instance::acquire() else {
        return;
    };
    let icon = image::load_from_memory(include_bytes!("../assets/pocketplay.png"))
        .map(|i| {
            let i = i.into_rgba8();
            eframe::egui::IconData {
                width: i.width(),
                height: i.height(),
                rgba: i.into_raw(),
            }
        })
        .ok();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1180., 800.])
        .with_min_inner_size([1000., 720.]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };
    let result = eframe::run_native(
        "PocketPlay Native",
        options,
        Box::new(|cc| Ok(Box::new(app::PocketPlay::new(cc)?))),
    );
    if let Err(e) = result {
        let data = config::data_dir();
        let _ = std::fs::create_dir_all(&data);
        let _ = std::fs::write(data.join("pocketplay-startup-error.log"), format!("{e:#}"));
    }
}
