use crate::{display_mode::DisplayModeWatch, framework::Framework, Options, MAGIC};
use anyhow::{bail, Context, Result};
use jni::objects::{GlobalRef, JObject};
use ndk_sys::*;
use std::{
    ffi::CString,
    io::Write,
    os::unix::net::UnixStream,
    ptr,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};

struct Encoder {
    codec: *mut AMediaCodec,
    window: *mut ANativeWindow,
    started: bool,
}
impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: these handles are owned and used only by the video thread.
        unsafe {
            if self.started {
                AMediaCodec_stop(self.codec);
            }
            if !self.window.is_null() {
                ANativeWindow_release(self.window);
            }
            if !self.codec.is_null() {
                AMediaCodec_delete(self.codec);
            }
        }
    }
}
struct Format(*mut AMediaFormat);
impl Drop for Format {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                AMediaFormat_delete(self.0);
            }
        }
    }
}
fn check(status: media_status_t, operation: &str) -> Result<()> {
    if status != media_status_t::AMEDIA_OK {
        bail!("{operation} failed: {status:?}");
    }
    Ok(())
}
fn int(format: &Format, key: &str, value: i32) -> Result<()> {
    let key = CString::new(key)?;
    unsafe {
        AMediaFormat_setInt32(format.0, key.as_ptr(), value);
    }
    Ok(())
}
fn mime(codec: &str) -> &'static str {
    match codec {
        "h265" => "video/hevc",
        "av1" => "video/av01",
        "vp8" => "video/x-vnd.on2.vp8",
        "vp9" => "video/x-vnd.on2.vp9",
        _ => "video/avc",
    }
}
struct Display<'a> {
    framework: &'a Framework,
    display: Option<GlobalRef>,
}
impl Display<'_> {
    fn release(&mut self, env: &mut jni::JNIEnv) -> Result<()> {
        if let Some(display) = self.display.as_ref() {
            env.call_method(display.as_obj(), "release", "()V", &[])?;
            self.display = None;
        }
        Ok(())
    }
}
impl Drop for Display<'_> {
    fn drop(&mut self) {
        if let Some(display) = self.display.take() {
            if let Ok(mut env) = self.framework.vm.attach_current_thread() {
                let _ = env.call_method(display.as_obj(), "release", "()V", &[]);
            }
        }
    }
}

pub(crate) fn run(
    framework: Framework,
    options: Options,
    mut stream: UnixStream,
    stop: &AtomicBool,
    geometry: &AtomicU64,
) -> Result<()> {
    let mut env = framework.vm.attach_current_thread()?;
    while !stop.load(Ordering::Acquire) {
        let initial = framework.display_state(&mut env)?;
        let (dw, dh, dpi) = (initial.width, initial.height, initial.dpi);
        let mut mode = DisplayModeWatch::new(initial.mode_id);
        geometry.store((dw as u64) << 32 | dh as u64, Ordering::Release);
        let scale = if options.max_edge == 0 {
            1.
        } else {
            (options.max_edge as f64 / dw.max(dh) as f64).min(1.)
        };
        let w = ((dw as f64 * scale) as i32 & !7).max(8);
        let h = ((dh as f64 * scale) as i32 & !7).max(8);
        if i64::from(w) * i64::from(h) > 16 * 1024 * 1024 {
            bail!("Video resolution exceeds 16 MP limit");
        }
        let selected = framework
            .encoders(&mut env)?
            .into_iter()
            .find(|line| {
                let fields: Vec<_> = line.split(" | ").collect();
                fields.len() >= 2
                    && fields[1] == mime(&options.codec)
                    && (options.encoder.is_empty() || fields[0] == options.encoder)
            })
            .context("No matching hardware video encoder")?;
        let name = selected
            .split(" | ")
            .next()
            .context("Invalid encoder name")?;
        let name = CString::new(name)?;
        let mime = CString::new(mime(&options.codec))?;
        // SAFETY: NDK handles are checked, released by RAII, and confined to this thread.
        let codec = unsafe { AMediaCodec_createCodecByName(name.as_ptr()) };
        if codec.is_null() {
            bail!("Cannot create hardware encoder");
        }
        let mut encoder = Encoder {
            codec,
            window: ptr::null_mut(),
            started: false,
        };
        let format = Format(unsafe { AMediaFormat_new() });
        if format.0.is_null() {
            bail!("Cannot allocate media format");
        }
        unsafe {
            AMediaFormat_setString(format.0, c"mime".as_ptr(), mime.as_ptr());
        }
        for (key, value) in [
            ("width", w),
            ("height", h),
            ("color-format", 0x7f000789),
            ("bitrate", options.bitrate),
            (
                "frame-rate",
                if options.fps == 0 { 120 } else { options.fps },
            ),
            ("i-frame-interval", 1),
            ("max-bframes", 0),
            ("priority", 0),
            ("latency", 0),
        ] {
            int(&format, key, value)?;
        }
        #[cfg(feature = "qa")]
        match std::env::var("POCKETPLAY_QA_ENCODER_HINT").as_deref() {
            Ok("operating-rate") => unsafe {
                AMediaFormat_setFloat(
                    format.0,
                    c"operating-rate".as_ptr(),
                    options.fps.max(120) as f32,
                );
            },
            Ok("vendor-low-latency") => int(&format, "vendor.qti-ext-enc-low-latency.enable", 1)?,
            _ => {}
        }
        // The virtual display already paces capture. A second timestamp-based
        // encoder limiter discarded valid frames near the requested rate on
        // the tested Xiaomi. Keep that earlier behavior only for QA comparisons.
        #[cfg(feature = "qa")]
        if options.fps > 0
            && std::env::var("POCKETPLAY_QA_ENCODER_HINT").as_deref() == Ok("strict-fps-cap")
        {
            unsafe {
                AMediaFormat_setFloat(format.0, c"max-fps-to-encoder".as_ptr(), options.fps as f32);
            }
        }
        unsafe {
            check(
                AMediaCodec_configure(codec, format.0, ptr::null_mut(), ptr::null_mut(), 1),
                "Configure encoder",
            )?;
            check(
                AMediaCodec_createInputSurface(codec, &mut encoder.window),
                "Create hardware input surface",
            )?;
        }
        if encoder.window.is_null() {
            bail!("No encoder input surface");
        }
        let surface = unsafe {
            JObject::from_raw(
                ANativeWindow_toSurface(env.get_native_interface().cast(), encoder.window).cast(),
            )
        };
        if surface.is_null() {
            bail!("Cannot bridge encoder surface");
        }
        let display = framework.mirror(&mut env, &surface, w, h, dpi, options.fps)?;
        let mut display = Display {
            framework: &framework,
            display: Some(display),
        };
        unsafe {
            check(AMediaCodec_start(codec), "Start encoder")?;
        }
        encoder.started = true;
        println!(
            "Encoder: {} | {w}x{h} | NDK surface",
            name.to_string_lossy()
        );
        for value in [MAGIC, dw as u32, dh as u32, w as u32, h as u32] {
            stream.write_all(&value.to_be_bytes())?;
        }
        let mut checked = Instant::now();
        let mut info = AMediaCodecBufferInfo {
            offset: 0,
            size: 0,
            presentationTimeUs: 0,
            flags: 0,
        };
        while !stop.load(Ordering::Acquire) {
            let index = unsafe { AMediaCodec_dequeueOutputBuffer(codec, &mut info, 10_000) };
            if index >= 0 {
                let result = (|| -> Result<()> {
                    if info.size <= 0 {
                        return Ok(());
                    }
                    let mut capacity = 0;
                    let data = unsafe {
                        AMediaCodec_getOutputBuffer(codec, index as usize, &mut capacity)
                    };
                    let offset = usize::try_from(info.offset)?;
                    let length = usize::try_from(info.size)?;
                    if data.is_null()
                        || length > 16 * 1024 * 1024
                        || offset.checked_add(length).is_none_or(|n| n > capacity)
                    {
                        bail!("Invalid encoded buffer");
                    }
                    let mut header = [0u8; 16];
                    header[..4].copy_from_slice(&(length as u32).to_be_bytes());
                    header[4..8].copy_from_slice(&info.flags.to_be_bytes());
                    header[8..].copy_from_slice(&info.presentationTimeUs.to_be_bytes());
                    let payload = unsafe { std::slice::from_raw_parts(data.add(offset), length) };
                    stream.write_all(&header)?;
                    // Encoded bytes go straight from the owned codec output buffer to the socket.
                    stream.write_all(payload)?;
                    Ok(())
                })();
                unsafe {
                    AMediaCodec_releaseOutputBuffer(codec, index as usize, false);
                }
                result?;
            } else if index < -3 {
                bail!("Encoder dequeue failed: {index}");
            }
            #[cfg(feature = "qa")]
            if index == -2 {
                let output = Format(unsafe { AMediaCodec_getOutputFormat(codec) });
                if !output.0.is_null() {
                    let text = unsafe { AMediaFormat_toString(output.0) };
                    if !text.is_null() {
                        println!(
                            "QA encoder output: {}",
                            unsafe { std::ffi::CStr::from_ptr(text) }.to_string_lossy()
                        );
                    }
                }
            }
            if checked.elapsed() >= Duration::from_millis(150) {
                checked = Instant::now();
                let current = framework.display_state(&mut env)?;
                #[cfg(feature = "qa")]
                if checked.elapsed().as_secs_f64() * 1000. > 1. {
                    println!(
                        "QA geometry query: {:.3} ms",
                        checked.elapsed().as_secs_f64() * 1000.
                    );
                }
                if (dw, dh) != (current.width, current.height) {
                    break;
                }
                if mode.changed(current.mode_id, checked) {
                    // Android caches virtual-display pacing at creation. Rebind
                    // after a stable mode change without resetting the codec,
                    // decoder, touch state, or transport connections.
                    display.release(&mut env)?;
                    display.display = Some(framework.mirror(
                        &mut env,
                        &surface,
                        w,
                        h,
                        current.dpi,
                        options.fps,
                    )?);
                    println!(
                        "Capture display refreshed for physical mode {}",
                        current.mode_id
                    );
                }
            }
        }
        drop(display);
        env.delete_local_ref(surface)?;
    }
    Ok(())
}
