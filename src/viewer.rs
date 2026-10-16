//! Event-driven native viewer with a single latest-decoded-frame slot.
use crate::{
    adb::Tools,
    geometry::Geometry,
    media::{self, Frame},
};
use anyhow::{bail, Context, Result};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const MAGIC: u32 = 0x50504c31;
const MAX_PACKET: usize = 16 * 1024 * 1024;
struct Received {
    frame: Frame,
    geometry: Geometry,
    received: Instant,
    decode_ms: f64,
    #[cfg(feature = "qa")]
    packet_age_ms: Option<f64>,
}
#[derive(Default)]
struct Shared {
    latest: Option<Received>,
    error: Option<String>,
    done: bool,
    decoded: u64,
    superseded: u64,
}

pub fn connect(port: u16, token: &str, channel: u8) -> Result<TcpStream> {
    if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Invalid session token");
    }
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse()?,
        Duration::from_secs(3),
    )?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(8)))?;
    stream.set_write_timeout(Some(Duration::from_millis(250)))?;
    stream.write_all(token.as_bytes())?;
    stream.write_all(&[channel])?;
    Ok(stream)
}
fn u32be(stream: &mut impl Read) -> Result<u32> {
    let mut b = [0; 4];
    stream.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}
fn geometry(stream: &mut impl Read) -> Result<Geometry> {
    let dims = [
        u32be(stream)?,
        u32be(stream)?,
        u32be(stream)?,
        u32be(stream)?,
    ];
    if dims.iter().any(|d| !(1..=8192).contains(d))
        || u64::from(dims[2]) * u64::from(dims[3]) > 16 * 1024 * 1024
    {
        bail!("Invalid video geometry");
    }
    Ok(Geometry {
        display: (dims[0] as u16, dims[1] as u16),
        frame: (dims[2] as u16, dims[3] as u16),
    })
}
fn initial_size(text: &str) -> Result<(u16, u16)> {
    let (w, h) = text
        .split_once('x')
        .context("Invalid initial display size")?;
    let size = (w.parse::<u16>()?, h.parse::<u16>()?);
    if !(1..=8192).contains(&size.0) || !(1..=8192).contains(&size.1) {
        bail!("Invalid initial display dimensions");
    }
    Ok(size)
}
fn startup_flag(value: Option<&str>, name: &str) -> Result<bool> {
    match value {
        None | Some("0") => Ok(false),
        Some("1") => Ok(true),
        _ => bail!("Invalid {name} option"),
    }
}
pub fn run(args: &[String]) -> Result<()> {
    if !(7..=10).contains(&args.len()) {
        bail!("Invalid viewer arguments");
    }
    let fullscreen = startup_flag(args.get(8).map(String::as_str), "fullscreen")?;
    let vsync = startup_flag(args.get(9).map(String::as_str), "VSync")?;
    let port: u16 = args[0].parse()?;
    let token = &args[1];
    #[cfg(feature = "qa")]
    let clock_offset = std::env::var("POCKETPLAY_FRAME_CLOCK_OFFSET_US")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite());
    let codec = &args[2];
    let hardware = &args[3];
    let tools = Tools::find(PathBuf::from(&args[4]))?;
    println!("Loading native media libraries");
    media::initialize(&tools.dir)?;
    // The control handshake already knows the logical display size. Prepare
    // the expensive graphics/codec objects before asking Android to start video,
    // so the phone does not build a compressed backlog during PC initialization.
    let prepared = args
        .get(7)
        .map(|text| -> Result<_> {
            let viewer = media::Viewer::new(initial_size(text)?, fullscreen, vsync)?;
            let decoder = media::Decoder::new(codec, hardware, viewer.device())?;
            Ok((viewer, decoder))
        })
        .transpose()?;
    println!("Connecting native video channel");
    let mut stream = connect(port, token, 2).context("Could not connect the video channel")?;
    if u32be(&mut stream).context("Phone closed or timed out before the video handshake")? != MAGIC
    {
        bail!("Device protocol mismatch");
    }
    let initial = geometry(&mut stream).context("Could not read the phone's video geometry")?;
    stream.set_read_timeout(None)?;
    let (mut viewer, decoder) = match prepared {
        Some(media) => {
            println!("Media initialized before capture");
            media
        }
        None => {
            let viewer = media::Viewer::new(initial.display, fullscreen, vsync)?;
            let decoder = media::Decoder::new(codec, hardware, viewer.device())?;
            (viewer, decoder)
        }
    };
    // The viewer owns the shared D3D11 device until the decode worker is joined.
    // Its immediate context has D3D11 multithread protection enabled in the shim.
    let device = viewer.device();
    println!(
        "Renderer: {} | VSync: {} | aspect: {}x{}",
        viewer.renderer(),
        if viewer.vsync() { "on" } else { "off" },
        initial.display.0,
        initial.display.1
    );
    println!("Geometry: {}", initial.encode());
    println!("Texture: {}x{}", initial.frame.0, initial.frame.1);
    let shared = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
    let captured = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let mut audio_abort = None;
    let audio_worker = if args[5] != "0" {
        let mut audio_stream = connect(port, token, 3)?;
        audio_abort = Some(audio_stream.try_clone()?);
        let flag = stop.clone();
        let buffer_ms = args[6].parse()?;
        Some(thread::spawn(move || {
            let result = (|| -> Result<()> {
                if u32be(&mut audio_stream)? != MAGIC
                    || u32be(&mut audio_stream)? != 48000
                    || u32be(&mut audio_stream)? != 2
                {
                    bail!("Invalid audio handshake");
                }
                audio_stream.set_read_timeout(None)?;
                let mut audio = media::Audio::new(buffer_ms)?;
                let mut pcm = Vec::with_capacity(4096);
                println!("Audio playback: PCM stereo 48000 Hz");
                let mut last_audio_log = Instant::now();
                while !flag.load(Ordering::Acquire) {
                    let length = u32be(&mut audio_stream)? as usize;
                    if length == 0 || length > 65536 || !length.is_multiple_of(4) {
                        bail!("Invalid PCM frame size");
                    }
                    pcm.resize(length, 0);
                    audio_stream.read_exact(&mut pcm)?;
                    audio.push(&pcm)?;
                    if last_audio_log.elapsed() >= Duration::from_secs(1) {
                        let [queued, device] = audio.stats()?;
                        println!("Audio queue: {queued:.1} ms · Device buffer: {device:.1} ms · Added prebuffer: {buffer_ms} ms");
                        last_audio_log = Instant::now();
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                if !flag.load(Ordering::Acquire) {
                    println!("Audio unavailable: {error:#}");
                }
            }
        }))
    } else {
        None
    };
    let wanted = Arc::new(AtomicU64::new(initial.encode()));
    let abort = stream.try_clone()?;
    let target = shared.clone();
    let capture_count = captured.clone();
    let flag = stop.clone();
    let geometry_slot = wanted.clone();
    let codec = codec.to_owned();
    let hardware = hardware.to_owned();
    let worker = thread::spawn(move || {
        let result = (|| -> Result<()> {
            let mut decoder = decoder;
            let mut packet = Vec::with_capacity(256 * 1024);
            let mut configuration = Vec::new();
            let mut configured_packet = Vec::new();
            let mut reported = false;
            while !flag.load(Ordering::Acquire) {
                let length = u32be(&mut stream)?;
                if length == MAGIC {
                    let g = geometry(&mut stream)?;
                    geometry_slot.store(g.encode(), Ordering::Release);
                    decoder = media::Decoder::new(&codec, &hardware, device)?;
                    reported = false;
                    configuration.clear();
                    continue;
                }
                let length = length as usize;
                if length == 0 || length > MAX_PACKET {
                    bail!("Invalid encoded packet length");
                }
                let flags = u32be(&mut stream)?;
                let mut timestamp = [0; 8];
                stream.read_exact(&mut timestamp)?;
                packet.resize(length, 0);
                stream.read_exact(&mut packet)?;
                if flags & 2 != 0 {
                    configuration.clear();
                    configuration.extend_from_slice(&packet);
                    continue;
                }
                capture_count.fetch_add(1, Ordering::Relaxed);
                let input = if flags & 1 != 0 && !configuration.is_empty() {
                    configured_packet.clear();
                    configured_packet.extend_from_slice(&configuration);
                    configured_packet.extend_from_slice(&packet);
                    &configured_packet
                } else {
                    &packet
                };
                let received = Instant::now();
                #[cfg(feature = "qa")]
                let packet_age_ms = clock_offset.map(|offset| {
                    (media::clock_micros() as f64 + offset - i64::from_be_bytes(timestamp) as f64)
                        / 1000.
                });
                if let Some(frame) = decoder.decode(input, i64::from_be_bytes(timestamp))? {
                    let size = frame.size()?;
                    let g = Geometry::decode(geometry_slot.load(Ordering::Acquire))
                        .context("Missing geometry")?;
                    if size != g.frame {
                        bail!("Decoder output does not match capture dimensions");
                    }
                    if !reported {
                        println!(
                            "Decoder: {}",
                            if decoder.hardware() {
                                "D3D11 hardware"
                            } else {
                                "software"
                            }
                        );
                        reported = true;
                    }
                    let mut state = target.0.lock().unwrap_or_else(|p| p.into_inner());
                    state.decoded += 1;
                    if state.latest.is_some() {
                        state.superseded += 1;
                    }
                    state.latest = Some(Received {
                        frame,
                        geometry: g,
                        received,
                        decode_ms: received.elapsed().as_secs_f64() * 1000.,
                        #[cfg(feature = "qa")]
                        packet_age_ms,
                    });
                    target.1.notify_one();
                }
            }
            Ok(())
        })();
        let mut state = target.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(e) = result {
            if !flag.load(Ordering::Acquire) {
                state.error = Some(format!("{e:#}"));
            }
        }
        state.done = true;
        target.1.notify_one();
    });
    let mut last_log = Instant::now();
    let mut displayed = 0;
    let mut ready_reported = false;
    let mut previous_encoded = 0;
    let mut previous_decoded = 0;
    let mut decode_total = 0.;
    let mut ready_total = 0.;
    let mut present_total = 0.;
    let mut sampled = 0;
    let mut presented_geometry = initial;
    #[cfg(feature = "qa")]
    let mut validation_ages = (0.0_f64, 0.0_f64, 0.0_f64, 0u64);
    #[cfg(feature = "qa")]
    let mut validation_display = (0.0_f64, 0.0_f64, 0.0_f64, 0u64, 0u64);
    #[cfg(feature = "qa")]
    let trace_pacing = std::env::var_os("POCKETPLAY_QA_FRAME_PACING").is_some();
    let result = (|| -> Result<()> {
        while viewer.open()? {
            #[cfg(feature = "qa")]
            if let (Some(offset), Some([pts, sync, submitted, refresh_delta, id, refresh])) =
                (clock_offset, viewer.presentation_feedback())
            {
                if trace_pacing {
                    println!("QA feedback: pts={pts} sync={sync} submitted={submitted} refresh_delta={refresh_delta} id={id} refresh={refresh}");
                }
                if refresh_delta == 0 && pts != i64::MIN {
                    let age = (sync as f64 + offset - pts as f64) / 1000.;
                    validation_display.0 += age;
                    validation_display.1 += (sync - submitted) as f64 / 1000.;
                    validation_display.2 = validation_display.2.max(age);
                    validation_display.3 += 1;
                } else {
                    validation_display.4 += 1;
                }
            }
            let mut state = shared.0.lock().unwrap_or_else(|p| p.into_inner());
            if state.latest.is_none() && !state.done {
                state = shared
                    .1
                    .wait_timeout(state, Duration::from_millis(4))
                    .unwrap_or_else(|p| p.into_inner())
                    .0;
            }
            let frame = state.latest.take();
            let done = state.done;
            let error = state.error.take();
            let decoded = state.decoded;
            let encoded = captured.load(Ordering::Relaxed);
            let superseded = state.superseded;
            drop(state);
            if let Some(error) = error {
                bail!("{error}");
            }
            if let Some(frame) = frame {
                sampled += 1;
                ready_total += frame.received.elapsed().as_secs_f64() * 1000.;
                decode_total += frame.decode_ms;
                let started = Instant::now();
                let rendered = viewer.present(&frame.frame, frame.geometry)?;
                if rendered && frame.geometry != presented_geometry {
                    presented_geometry = frame.geometry;
                    println!("Geometry: {}", presented_geometry.encode());
                    println!(
                        "Texture: {}x{}",
                        presented_geometry.frame.0, presented_geometry.frame.1
                    );
                }
                if rendered {
                    if !ready_reported {
                        println!("Video ready: first frame presented");
                        ready_reported = true;
                    }
                    #[cfg(feature = "qa")]
                    if trace_pacing {
                        println!(
                            "QA submitted: pts={} at={}",
                            frame.frame.timestamp(),
                            media::clock_micros()
                        );
                    }
                    #[cfg(feature = "qa")]
                    if let (Some(offset), Some(packet_age)) = (clock_offset, frame.packet_age_ms) {
                        let pts = frame.frame.timestamp();
                        if pts != i64::MIN {
                            let age = (media::clock_micros() as f64 + offset - pts as f64) / 1000.;
                            validation_ages.0 += packet_age;
                            validation_ages.1 += age;
                            validation_ages.2 = validation_ages.2.max(age);
                            validation_ages.3 += 1;
                        }
                    }
                    present_total += started.elapsed().as_secs_f64() * 1000.;
                    displayed += 1;
                }
            } else if done {
                break;
            }
            if last_log.elapsed() >= Duration::from_secs(1) {
                let elapsed = last_log.elapsed().as_secs_f64();
                let count = (displayed as f64).max(1.);
                let samples = (sampled as f64).max(1.);
                println!("{:.1} fps | capture_fps={:.1} decode_fps={:.1} decoded={decoded} superseded={superseded} decode_ms={:.3} ready_ms={:.3} present_ms={:.3}",displayed as f64/elapsed,encoded.saturating_sub(previous_encoded) as f64/elapsed,decoded.saturating_sub(previous_decoded) as f64/elapsed,decode_total/samples,ready_total/samples,present_total/count);
                previous_encoded = encoded;
                #[cfg(feature = "qa")]
                if validation_ages.3 > 0 {
                    println!(
                        "QA video age: packet_ms={:.3} submitted_ms={:.3} submitted_max_ms={:.3}",
                        validation_ages.0 / validation_ages.3 as f64,
                        validation_ages.1 / validation_ages.3 as f64,
                        validation_ages.2
                    );
                    validation_ages = (0., 0., 0., 0);
                }
                #[cfg(feature = "qa")]
                if validation_display.3 > 0 || validation_display.4 > 0 {
                    let count = validation_display.3.max(1) as f64;
                    println!(
                        "QA display feedback: sync_age_ms={:.3} submit_to_sync_ms={:.3} sync_max_ms={:.3} samples={} unmapped={}",
                        validation_display.0/count, validation_display.1/count, validation_display.2,
                        validation_display.3, validation_display.4
                    );
                    validation_display = (0., 0., 0., 0, 0);
                }
                previous_decoded = decoded;
                last_log = Instant::now();
                displayed = 0;
                decode_total = 0.;
                ready_total = 0.;
                present_total = 0.;
                sampled = 0;
            }
        }
        Ok(())
    })();
    stop.store(true, Ordering::Release);
    let _ = abort.shutdown(Shutdown::Both);
    if let Some(audio) = audio_abort {
        let _ = audio.shutdown(Shutdown::Both);
    }
    let _ = worker.join();
    if let Some(audio) = audio_worker {
        let _ = audio.join();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn dimensions(values: [u32; 4]) -> Cursor<Vec<u8>> {
        Cursor::new(values.into_iter().flat_map(u32::to_be_bytes).collect())
    }
    #[test]
    fn metadata_preserves_logical_and_encoded_dimensions() {
        assert_eq!(
            geometry(&mut dimensions([2400, 1080, 1920, 864])).unwrap(),
            Geometry {
                display: (2400, 1080),
                frame: (1920, 864)
            }
        );
    }
    #[test]
    fn metadata_rejects_zero_dimensions() {
        assert!(geometry(&mut dimensions([0, 1080, 1920, 864])).is_err());
    }
    #[test]
    fn metadata_rejects_excessive_pixel_count() {
        assert!(geometry(&mut dimensions([8192, 8192, 8192, 8192])).is_err());
    }
    #[test]
    fn metadata_rejects_truncated_headers() {
        assert!(geometry(&mut Cursor::new([0u8; 12])).is_err());
    }
    #[test]
    fn channel_rejects_invalid_token_before_connecting() {
        assert!(connect(1, "invalid", 1).is_err());
    }
    #[test]
    fn initial_display_size_accepts_both_orientations() {
        assert_eq!(initial_size("1080x2400").unwrap(), (1080, 2400));
        assert_eq!(initial_size("2400x1080").unwrap(), (2400, 1080));
    }
    #[test]
    fn initial_display_size_rejects_invalid_bounds() {
        assert!(initial_size("0x2400").is_err());
        assert!(initial_size("8193x1080").is_err());
    }
    #[test]
    fn initial_display_size_rejects_extra_fields() {
        assert!(initial_size("1080x2400x60").is_err());
    }
    #[test]
    fn fullscreen_defaults_to_windowed_for_older_launchers() {
        assert!(!startup_flag(None, "fullscreen").unwrap());
    }
    #[test]
    fn fullscreen_accepts_explicit_boolean_flags() {
        assert!(startup_flag(Some("1"), "fullscreen").unwrap());
        assert!(!startup_flag(Some("0"), "fullscreen").unwrap());
    }
    #[test]
    fn fullscreen_rejects_malformed_flags() {
        assert!(startup_flag(Some("true"), "fullscreen").is_err());
    }
    #[test]
    fn vsync_defaults_to_immediate_for_older_launchers() {
        assert!(!startup_flag(None, "VSync").unwrap());
    }
    #[test]
    fn vsync_accepts_explicit_smooth_presentation() {
        assert!(startup_flag(Some("1"), "VSync").unwrap());
    }
    #[test]
    fn vsync_rejects_malformed_flags() {
        assert!(startup_flag(Some("-1"), "VSync").is_err());
    }
}
