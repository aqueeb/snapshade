//! Image encoding: RGBA8 → PNG / JPEG (quality) / BMP bytes via the `image` crate.
//!
//! Pure library code (D-01): no file or clipboard I/O here; later phases hand
//! the bytes to the exporters.

use std::io::Cursor;

use image::codecs::bmp::BmpEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};

/// Output image format; the choices on the Options → Formats tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    /// `quality` is 1..=100 (the JPEG quality slider in Options).
    Jpeg {
        quality: u8,
    },
    Bmp,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("image dimensions are invalid for the buffer length")]
    InvalidDimensions,
    #[error("I/O error: {0}")]
    Io(String),
}

impl ImageFormat {
    /// File extension (lower-case, no dot).
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg { .. } => "jpg",
            ImageFormat::Bmp => "bmp",
        }
    }

    /// Infer the format from a file extension; `jpeg_quality` fills the JPEG variant.
    /// Unknown or missing extensions return `None` (the caller falls back to its default).
    pub fn from_extension(ext: &str, jpeg_quality: u8) -> Option<ImageFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "png" => Some(ImageFormat::Png),
            "jpg" | "jpeg" => Some(ImageFormat::Jpeg {
                quality: jpeg_quality,
            }),
            "bmp" => Some(ImageFormat::Bmp),
            _ => None,
        }
    }
}

/// Encode a tightly packed RGBA8 buffer (`width * height * 4` bytes, row-major,
/// top-left origin) as `format`.
///
/// Rejects a zero dimension or a buffer whose length does not match before
/// allocating anything (T-01-10). JPEG has no alpha channel: the buffer is
/// converted to RGB first.
pub fn encode(
    rgba: &[u8],
    width: u32,
    height: u32,
    format: ImageFormat,
) -> Result<Vec<u8>, EncodeError> {
    encode_with(rgba, width, height, format, false)
}

/// [`encode`] with the "Optimize PNG files" switch: when `optimize_png`
/// is set a PNG uses the slowest / smallest zlib level and adaptive filtering (the
/// in-process stand-in for OptiPNG); other formats are unaffected.
pub fn encode_with(
    rgba: &[u8],
    width: u32,
    height: u32,
    format: ImageFormat,
    optimize_png: bool,
) -> Result<Vec<u8>, EncodeError> {
    if width == 0 || height == 0 {
        return Err(EncodeError::InvalidDimensions);
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or(EncodeError::InvalidDimensions)?;
    if rgba.len() != expected {
        return Err(EncodeError::InvalidDimensions);
    }
    let img =
        RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or(EncodeError::InvalidDimensions)?;

    let mut out = Cursor::new(Vec::new());
    let written =
        match format {
            ImageFormat::Png if optimize_png => img.write_with_encoder(
                PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive),
            ),
            ImageFormat::Png => img.write_with_encoder(PngEncoder::new(&mut out)),
            ImageFormat::Jpeg { quality } => {
                let rgb = DynamicImage::ImageRgba8(img).to_rgb8();
                rgb.write_with_encoder(JpegEncoder::new_with_quality(
                    &mut out,
                    quality.clamp(1, 100),
                ))
            }
            ImageFormat::Bmp => img.write_with_encoder(BmpEncoder::new(&mut out)),
        };
    written.map_err(|e| EncodeError::Io(e.to_string()))?;
    Ok(out.into_inner())
}
