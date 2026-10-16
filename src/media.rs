//! Narrow ownership wrappers around upstream native codec and renderer APIs.
use crate::geometry::{aspect_fit, Geometry};
use anyhow::{bail, Context, Result};
use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::NonNull,
};

unsafe extern "C" {
    fn pp_init(dir: *const u16) -> c_int;
    fn pp_error() -> *const c_char;
    fn pp_decoder_new(codec: *const c_char, hardware: c_int, device: *mut c_void) -> *mut c_void;
    fn pp_decoder_free(decoder: *mut c_void);
    fn pp_decoder_push(
        decoder: *mut c_void,
        bytes: *const u8,
        length: c_int,
        pts: i64,
        frame: *mut *mut c_void,
    ) -> c_int;
    fn pp_decoder_hardware(decoder: *mut c_void) -> c_int;
    fn pp_frame_free(frame: *mut c_void);
    fn pp_frame_width(frame: *mut c_void) -> c_int;
    fn pp_frame_height(frame: *mut c_void) -> c_int;
    #[cfg(feature = "qa")]
    fn pp_frame_pts(frame: *mut c_void) -> i64;
    #[cfg(feature = "qa")]
    fn pp_clock_us() -> i64;
    fn pp_viewer_new(width: c_int, height: c_int, fullscreen: c_int, vsync: c_int) -> *mut c_void;
    fn pp_viewer_free(viewer: *mut c_void);
    fn pp_viewer_poll(viewer: *mut c_void) -> c_int;
    #[cfg(feature = "qa")]
    fn pp_viewer_feedback(viewer: *mut c_void, sample: *mut i64) -> c_int;
    fn pp_viewer_geometry(viewer: *mut c_void, w: c_int, h: c_int) -> c_int;
    fn pp_viewer_output_size(viewer: *mut c_void, w: *mut c_int, h: *mut c_int) -> c_int;
    fn pp_viewer_present(viewer: *mut c_void, frame: *mut c_void, rectangles: *const f64) -> c_int;
    fn pp_viewer_renderer(viewer: *mut c_void) -> *const c_char;
    fn pp_viewer_vsync(viewer: *mut c_void) -> c_int;
    fn pp_viewer_device(viewer: *mut c_void) -> *mut c_void;
    fn pp_viewer_icon(viewer: *mut c_void, pixels: *const u8, w: c_int, h: c_int);
    fn pp_audio_new(buffer_ms: c_int) -> *mut c_void;
    fn pp_audio_free(audio: *mut c_void);
    fn pp_audio_push(audio: *mut c_void, bytes: *const u8, length: c_int) -> c_int;
    fn pp_audio_stats(audio: *mut c_void, values: *mut f64) -> c_int;
}
fn error() -> anyhow::Error {
    // SAFETY: the shim returns a terminated thread-local diagnostic buffer.
    anyhow::anyhow!(unsafe { CStr::from_ptr(pp_error()) }
        .to_string_lossy()
        .into_owned())
}
pub fn initialize(dir: &Path) -> Result<()> {
    let dir: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { pp_init(dir.as_ptr()) } != 0 {
        return Err(error());
    }
    Ok(())
}
pub struct Decoder(NonNull<c_void>);
// SAFETY: the owned codec context is moved once after initialization and then
// used exclusively by the decode worker. Its shared D3D11 context has the
// multithread protection installed by Viewer; Decoder is deliberately not Sync.
unsafe impl Send for Decoder {}
impl Decoder {
    pub fn new(codec: &str, hardware: &str, device: usize) -> Result<Self> {
        let codec = CString::new(if codec == "h265" { "hevc" } else { codec })?;
        let hardware = match hardware {
            "d3d11va" => 2,
            "auto" => 1,
            _ => 0,
        };
        let handle = unsafe { pp_decoder_new(codec.as_ptr(), hardware, device as *mut c_void) };
        Ok(Self(NonNull::new(handle).ok_or_else(error)?))
    }
    pub fn decode(&mut self, bytes: &[u8], pts: i64) -> Result<Option<Frame>> {
        if bytes.len() > 16 * 1024 * 1024 {
            bail!("Encoded packet exceeds limit");
        }
        let mut frame = std::ptr::null_mut();
        let result = unsafe {
            pp_decoder_push(
                self.0.as_ptr(),
                bytes.as_ptr(),
                bytes.len() as c_int,
                pts,
                &mut frame,
            )
        };
        if result < 0 {
            return Err(error());
        }
        Ok(NonNull::new(frame).map(Frame))
    }
    pub fn hardware(&self) -> bool {
        unsafe { pp_decoder_hardware(self.0.as_ptr()) != 0 }
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            pp_decoder_free(self.0.as_ptr());
        }
    }
}
pub struct Frame(NonNull<c_void>);
// SAFETY: each Frame owns an independent AVFrame reference. After decode it is
// immutable, moved to one presenter, and freed once. FFmpeg buffer refs are atomic.
unsafe impl Send for Frame {}
impl Frame {
    #[cfg(feature = "qa")]
    pub fn timestamp(&self) -> i64 {
        unsafe { pp_frame_pts(self.0.as_ptr()) }
    }
    pub fn size(&self) -> Result<(u16, u16)> {
        let w = unsafe { pp_frame_width(self.0.as_ptr()) };
        let h = unsafe { pp_frame_height(self.0.as_ptr()) };
        Ok((
            u16::try_from(w).context("Invalid frame width")?,
            u16::try_from(h).context("Invalid frame height")?,
        ))
    }
}
#[cfg(feature = "qa")]
pub fn clock_micros() -> i64 {
    unsafe { pp_clock_us() }
}
impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            pp_frame_free(self.0.as_ptr());
        }
    }
}
pub struct Viewer(NonNull<c_void>);
impl Viewer {
    pub fn new(size: (u16, u16), fullscreen: bool, vsync: bool) -> Result<Self> {
        let viewer = Self(
            NonNull::new(unsafe {
                pp_viewer_new(
                    size.0.into(),
                    size.1.into(),
                    fullscreen.into(),
                    vsync.into(),
                )
            })
            .ok_or_else(error)?,
        );
        if let Ok(icon) = image::load_from_memory(include_bytes!("../assets/pocketplay.png")) {
            let icon = icon.into_rgba8();
            if icon.width() <= 512 && icon.height() <= 512 {
                unsafe {
                    pp_viewer_icon(
                        viewer.0.as_ptr(),
                        icon.as_ptr(),
                        icon.width() as c_int,
                        icon.height() as c_int,
                    );
                }
            }
        }
        Ok(viewer)
    }
    pub fn renderer(&self) -> String {
        unsafe { CStr::from_ptr(pp_viewer_renderer(self.0.as_ptr())) }
            .to_string_lossy()
            .into_owned()
    }
    pub fn device(&self) -> usize {
        unsafe { pp_viewer_device(self.0.as_ptr()) as usize }
    }
    pub fn vsync(&self) -> bool {
        unsafe { pp_viewer_vsync(self.0.as_ptr()) != 0 }
    }
    #[cfg(feature = "qa")]
    pub fn presentation_feedback(&self) -> Option<[i64; 6]> {
        let mut sample = [0; 6];
        // SAFETY: the main thread owns the viewer; the shim writes exactly six i64s.
        (unsafe { pp_viewer_feedback(self.0.as_ptr(), sample.as_mut_ptr()) } == 1).then_some(sample)
    }
    pub fn open(&self) -> Result<bool> {
        match unsafe { pp_viewer_poll(self.0.as_ptr()) } {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(error()),
        }
    }
    pub fn present(&mut self, frame: &Frame, geometry: Geometry) -> Result<bool> {
        if unsafe {
            pp_viewer_geometry(
                self.0.as_ptr(),
                geometry.display.0.into(),
                geometry.display.1.into(),
            )
        } != 0
        {
            return Err(error());
        }
        let (mut w, mut h) = (0, 0);
        let output = unsafe { pp_viewer_output_size(self.0.as_ptr(), &mut w, &mut h) };
        if output == 1 {
            return Ok(false);
        }
        if output != 0 {
            return Err(error());
        }
        let source = aspect_fit(
            (geometry.frame.0.into(), geometry.frame.1.into()),
            geometry.display,
        )
        .context("Invalid capture viewport")?;
        let destination = aspect_fit((f64::from(w), f64::from(h)), geometry.display)
            .context("Invalid render viewport")?;
        let mut rectangles = [0.; 8];
        rectangles[..4].copy_from_slice(&source);
        rectangles[4..].copy_from_slice(&destination);
        let result =
            unsafe { pp_viewer_present(self.0.as_ptr(), frame.0.as_ptr(), rectangles.as_ptr()) };
        if result < 0 {
            return Err(error());
        }
        Ok(result == 0)
    }
}
impl Drop for Viewer {
    fn drop(&mut self) {
        unsafe {
            pp_viewer_free(self.0.as_ptr());
        }
    }
}
pub struct Audio(NonNull<c_void>);
impl Audio {
    pub fn new(buffer_ms: u32) -> Result<Self> {
        Ok(Self(
            NonNull::new(unsafe { pp_audio_new(buffer_ms.min(200) as c_int) }).ok_or_else(error)?,
        ))
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > 65536 {
            bail!("PCM packet exceeds limit");
        }
        if unsafe { pp_audio_push(self.0.as_ptr(), bytes.as_ptr(), bytes.len() as c_int) } != 0 {
            return Err(error());
        }
        Ok(())
    }
    pub fn stats(&self) -> Result<[f64; 2]> {
        let mut values = [0.; 2];
        // SAFETY: the owned audio stream is live; the shim writes exactly two doubles.
        if unsafe { pp_audio_stats(self.0.as_ptr(), values.as_mut_ptr()) } != 0 {
            return Err(error());
        }
        Ok(values)
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        unsafe {
            pp_audio_free(self.0.as_ptr());
        }
    }
}
