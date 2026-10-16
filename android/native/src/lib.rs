//! Native device transport, hardware encoding and multitouch injection.
#![deny(clippy::unwrap_used, clippy::expect_used)]
mod audio;
mod display_mode;
mod framework;
mod video;

use anyhow::{bail, Result};
use jni::{
    objects::{JClass, JObjectArray, JString},
    JNIEnv,
};
use std::{
    io::{Read, Write},
    mem::{size_of, zeroed},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::net::{UnixListener, UnixStream},
    },
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const MAGIC: u32 = 0x50504c31;

#[derive(Clone)]
struct Options {
    token: String,
    max_edge: i32,
    bitrate: i32,
    fps: i32,
    codec: String,
    encoder: String,
    audio: u8,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        if args.len() != 7 {
            bail!("Expected seven native service arguments");
        }
        let o = Self {
            token: args[0].clone(),
            max_edge: args[1].parse()?,
            bitrate: args[2].parse()?,
            fps: args[3].parse()?,
            codec: args[4].clone(),
            encoder: if args[5] == "auto" {
                String::new()
            } else {
                args[5].clone()
            },
            audio: args[6].parse()?,
        };
        if o.token.len() != 32
            || !o.token.bytes().all(|b| b.is_ascii_hexdigit())
            || !(0..=8192).contains(&o.max_edge)
            || !(100_000..=200_000_000).contains(&o.bitrate)
            || !(0..=240).contains(&o.fps)
            || !["h264", "h265", "av1", "vp8", "vp9"].contains(&o.codec.as_str())
            || o.audio > 2
            || o.encoder.len() > 256
            || !o
                .encoder
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            bail!("Invalid native service configuration");
        }
        Ok(o)
    }
}

/// Entry point invoked once by the small app_process launcher.
#[no_mangle]
pub extern "system" fn Java_com_pocketplay_server_Main_run(
    mut env: JNIEnv,
    _class: JClass,
    args: JObjectArray,
) {
    let result = (|| -> Result<()> {
        let n = env.get_array_length(&args)?;
        if n > 8 {
            bail!("Too many service arguments");
        }
        let mut strings: Vec<String> = Vec::with_capacity(n as usize);
        for i in 0..n {
            let s = JString::from(env.get_object_array_element(&args, i)?);
            strings.push(env.get_string(&s)?.into());
            env.delete_local_ref(s)?;
        }
        let framework = framework::Framework::new(&mut env)?;
        #[cfg(feature = "qa")]
        if strings == ["--validation-tone"] {
            return audio::validation_tone(&mut env);
        }
        #[cfg(feature = "qa")]
        if strings.len() == 2 && strings[0] == "--encoder-parameters" {
            return framework.encoder_parameters(&mut env, &strings[1]);
        }
        if strings == ["--encoders"] {
            for line in framework.encoders(&mut env)? {
                println!("{line}");
            }
            return Ok(());
        }
        serve(&mut env, framework, Options::parse(&strings)?)
    })();
    if let Err(error) = result {
        eprintln!("PocketPlay native service failed: {error:#}");
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        // Owned resources have unwound. Framework Binder threads still exist;
        // avoid racing system-library static destructors during process exit.
        unsafe {
            libc::_exit(1);
        }
    }
    unsafe {
        libc::_exit(0);
    }
}

fn listener(name: &str) -> Result<UnixListener> {
    if name.len() >= 107 {
        bail!("Socket name too long");
    }
    // SAFETY: socket returns an owned descriptor; OwnedFd closes all failure paths.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut address: libc::sockaddr_un = unsafe { zeroed() };
    address.sun_family = libc::AF_UNIX as _;
    for (slot, byte) in address.sun_path[1..].iter_mut().zip(name.bytes()) {
        *slot = byte as _;
    }
    let len = size_of::<libc::sa_family_t>() + 1 + name.len();
    // SAFETY: address is initialized and the abstract name length fits sockaddr_un.
    if unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            len as _,
        )
    } != 0
        || unsafe { libc::listen(fd.as_raw_fd(), 4) } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let socket = UnixListener::from(fd);
    socket.set_nonblocking(true)?;
    Ok(socket)
}

fn accept(
    listener: &UnixListener,
    token: &str,
    channel: u8,
    stop: &AtomicBool,
) -> Result<UnixStream> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        match listener.accept() {
            Ok((mut socket, _)) => {
                let mut peer: libc::ucred = unsafe { zeroed() };
                let mut length = size_of::<libc::ucred>() as libc::socklen_t;
                // A forwarded connection comes from adbd (shell or root), never
                // an ordinary phone app. The token also identifies this session.
                if unsafe {
                    libc::getsockopt(
                        socket.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_PEERCRED,
                        (&mut peer as *mut libc::ucred).cast(),
                        &mut length,
                    )
                } != 0
                    || (peer.uid != unsafe { libc::getuid() } && peer.uid != 0)
                {
                    continue;
                }
                socket.set_read_timeout(Some(Duration::from_secs(2)))?;
                let mut auth = [0u8; 33];
                if socket.read_exact(&mut auth).is_err() {
                    continue;
                }
                let difference = auth[..32]
                    .iter()
                    .zip(token.bytes())
                    .fold(0u8, |a, (b, c)| a | (b ^ c));
                if difference != 0 || auth[32] != channel {
                    continue;
                }
                socket.set_read_timeout(None)?;
                socket.set_write_timeout(Some(Duration::from_millis(500)))?;
                return Ok(socket);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(e) => return Err(e.into()),
        }
    }
    bail!("Timed out waiting for channel {channel}")
}

fn serve(env: &mut JNIEnv, framework: framework::Framework, options: Options) -> Result<()> {
    let _session_power = framework.keep_awake(env)?;
    let listener = listener(&format!("pocketplay_{}", options.token))?;
    let stop = Arc::new(AtomicBool::new(false));
    let (w, h, _) = framework.geometry(env)?;
    let geometry = Arc::new(AtomicU64::new((w as u64) << 32 | h as u64));
    println!("PocketPlay native Rust protocol 1 ready");
    let mut control = accept(&listener, &options.token, 1, &stop)?;
    for value in [MAGIC, w as u32, h as u32] {
        control.write_all(&value.to_be_bytes())?;
    }
    let control_shutdown = control.try_clone()?;
    let stop_video = stop.clone();
    let geometry_video = geometry.clone();
    let video_framework = framework.clone();
    let worker = thread::spawn(move || {
        let result = (|| -> Result<()> {
            let video = accept(&listener, &options.token, 2, &stop_video)?;
            let audio = if options.audio > 0 {
                let audio_framework = video_framework.clone();
                let token = options.token.clone();
                let flag = stop_video.clone();
                let mode = options.audio;
                Some(thread::spawn(move || {
                    let result = (|| -> Result<()> {
                        let socket = accept(&listener, &token, 3, &flag)?;
                        audio::run(audio_framework, socket, &flag, mode)
                    })();
                    if let Err(error) = &result {
                        eprintln!("Audio unavailable: {error:#}");
                    }
                    result
                }))
            } else {
                None
            };
            let result = video::run(
                video_framework,
                options,
                video,
                &stop_video,
                &geometry_video,
            );
            stop_video.store(true, Ordering::Release);
            if let Some(audio) = audio {
                let _ = audio.join();
            }
            result
        })();
        if let Err(e) = &result {
            eprintln!("Video failed: {e:#}");
        }
        stop_video.store(true, Ordering::Release);
        let _ = control_shutdown.shutdown(std::net::Shutdown::Both);
        result
    });
    let mut input = framework::Input::new(env)?;
    let result = control_loop(env, &framework, &mut input, &mut control, &stop, &geometry);
    stop.store(true, Ordering::Release);
    let _ = input.cancel(env);
    let _ = framework.panel_power(env, false);
    let video_result = worker
        .join()
        .map_err(|_| anyhow::anyhow!("Video worker failed"))?;
    result.and(video_result)
}

fn control_loop(
    env: &mut JNIEnv,
    framework: &framework::Framework,
    input: &mut framework::Input,
    stream: &mut UnixStream,
    stop: &AtomicBool,
    geometry: &AtomicU64,
) -> Result<()> {
    let mut packet = [0u8; 31];
    while !stop.load(Ordering::Acquire) {
        let mut kind = [0u8];
        if let Err(e) = stream.read_exact(&mut kind) {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                return Ok(());
            }
            return Err(e.into());
        }
        match kind[0] {
            0x7f => return Ok(()),
            #[cfg(feature = "qa")]
            0x31 => {
                let mut clock = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut clock) } != 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                let micros = clock.tv_sec * 1_000_000 + clock.tv_nsec / 1_000;
                stream.write_all(&micros.to_be_bytes())?;
            }
            0x30 => {
                let mut nonce = [0u8; 8];
                stream.read_exact(&mut nonce)?;
                stream.write_all(&nonce)?;
            }
            0x20 => {
                let mut off = [0u8];
                stream.read_exact(&mut off)?;
                let result = framework.panel_power(env, off[0] != 0);
                if let Err(error) = &result {
                    eprintln!("Display power unavailable: {error:#}");
                    env.exception_describe()?;
                    env.exception_clear()?;
                }
                stream.write_all(&[0x20, if result.is_ok() { off[0] } else { 0xff }])?;
            }
            0x10 => {
                stream.read_exact(&mut packet)?;
                let action = packet[0];
                let pointer = u64::from_be_bytes(packet[1..9].try_into()?);
                let x = i32::from_be_bytes(packet[9..13].try_into()?);
                let y = i32::from_be_bytes(packet[13..17].try_into()?);
                let w = u16::from_be_bytes(packet[17..19].try_into()?) as u32;
                let h = u16::from_be_bytes(packet[19..21].try_into()?) as u32;
                let desired = geometry.load(Ordering::Acquire);
                if desired != (w as u64) << 32 | h as u64 {
                    input.cancel(env)?;
                    continue;
                }
                input.touch(
                    env,
                    action,
                    pointer,
                    x.clamp(0, w as i32 - 1),
                    y.clamp(0, h as i32 - 1),
                )?;
            }
            _ => bail!("Unknown input message"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn arguments() -> Vec<String> {
        [
            "0123456789abcdef0123456789abcdef",
            "1920",
            "8000000",
            "120",
            "h264",
            "auto",
            "0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }
    #[test]
    fn configuration_selects_hardware_encoder_automatically() {
        assert!(Options::parse(&arguments()).is_ok_and(|o| o.encoder.is_empty()));
    }
    #[test]
    fn configuration_rejects_shell_metacharacters_in_encoder() {
        let mut args = arguments();
        args[5] = "c2.encoder;reboot".into();
        assert!(Options::parse(&args).is_err());
    }
    #[test]
    fn configuration_rejects_oversized_stream() {
        let mut args = arguments();
        args[1] = "9000".into();
        assert!(Options::parse(&args).is_err());
    }
    #[test]
    fn configuration_rejects_invalid_audio_mode() {
        let mut args = arguments();
        args[6] = "3".into();
        assert!(Options::parse(&args).is_err());
    }
    #[test]
    fn configuration_rejects_missing_fields() {
        assert!(Options::parse(&arguments()[..5]).is_err());
    }
    #[test]
    fn abstract_socket_rejects_names_that_exceed_address_capacity() {
        assert!(listener(&"x".repeat(108)).is_err());
    }
}
