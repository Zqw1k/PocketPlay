//! Resource limits for local imports and diagnostic reads.
use anyhow::{bail, Context, Result};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
};

pub const MAX_JSON_BYTES: u64 = 512 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_LOG_TAIL: u64 = 64 * 1024;
const MAX_IMAGE_EDGE: u32 = 8192;
const MAX_IMAGE_PIXELS: u64 = 16 * 1024 * 1024;

/// Reads no more than the limit plus one detection byte, even if the file grows.
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = File::open(path).context("Could not open input file")?;
    if file.metadata()?.len() > limit {
        bail!("Input file exceeds the supported size.");
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("Input file exceeds the supported size.");
    }
    Ok(bytes)
}

pub fn read_log_tail(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let offset = length.saturating_sub(MAX_LOG_TAIL);
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(MAX_LOG_TAIL).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    // A tail can begin in the middle of a line or UTF-8 character.
    Ok(if offset > 0 {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &text
    }
    .to_owned())
}

pub fn decode_screenshot(bytes: &[u8]) -> Result<image::RgbaImage> {
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        bail!("Screenshot file is too large.");
    }
    let reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let (width, height) = reader.into_dimensions()?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_EDGE
        || height > MAX_IMAGE_EDGE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        bail!("Screenshot exceeds the supported dimensions.");
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_IMAGE_EDGE);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode()?.into_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_file_rejects_oversized_input() {
        let path = std::env::temp_dir().join(format!("pocketplay-limit-{}", crate::config::id()));
        std::fs::write(&path, b"abcd").unwrap();
        assert!(read_bounded(&path, 3).is_err());
        assert_eq!(read_bounded(&path, 4).unwrap(), b"abcd");
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn screenshot_rejects_bad_input_and_accepts_bundled_icon() {
        assert!(decode_screenshot(b"not an image").is_err());
        let image = decode_screenshot(include_bytes!("../assets/pocketplay.png")).unwrap();
        assert!(image.width() > 0 && image.height() > 0);
    }
    #[test]
    fn screenshot_rejects_oversized_dimensions_before_decode() {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::new(MAX_IMAGE_EDGE + 1, 1));
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let error = decode_screenshot(encoded.get_ref()).unwrap_err();
        assert!(error.to_string().contains("dimensions"));
    }
}
