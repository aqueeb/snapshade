//! Encoder round-trip tests: every format must decode (via the same `image`
//! crate) back to the input dimensions, lossless formats to the input pixels,
//! and malformed buffers must be rejected before encoding.

use snapshade_core::encode::{encode, EncodeError, ImageFormat};

/// 4x3 RGBA gradient with a non-opaque alpha so PNG/BMP alpha survives too.
fn gradient_4x3() -> Vec<u8> {
    let (w, h) = (4u32, 3u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[
                (x * 60) as u8,
                (y * 100) as u8,
                ((x + y) * 30) as u8,
                200 + (x as u8),
            ]);
        }
    }
    px
}

/// Deterministic 64x64 noise (LCG) so the JPEG size comparison is reproducible.
fn noise_64x64() -> Vec<u8> {
    let mut state: u32 = 0x1234_5678;
    (0..64 * 64 * 4)
        .map(|i| {
            if i % 4 == 3 {
                255
            } else {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            }
        })
        .collect()
}

fn decode(bytes: &[u8]) -> image::DynamicImage {
    image::load_from_memory(bytes).expect("encoder output decodes")
}

#[test]
fn png_round_trips_pixels_exactly() {
    let input = gradient_4x3();
    let bytes = encode(&input, 4, 3, ImageFormat::Png).expect("png encodes");
    assert!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "PNG signature missing"
    );
    let img = decode(&bytes);
    assert_eq!((img.width(), img.height()), (4, 3));
    assert_eq!(img.to_rgba8().into_raw(), input);
}

#[test]
fn bmp_round_trips_pixels_exactly() {
    let input = gradient_4x3();
    let bytes = encode(&input, 4, 3, ImageFormat::Bmp).expect("bmp encodes");
    assert!(bytes.starts_with(b"BM"), "BMP signature missing");
    let img = decode(&bytes);
    assert_eq!((img.width(), img.height()), (4, 3));
    assert_eq!(img.to_rgba8().into_raw(), input);
}

#[test]
fn jpeg_decodes_to_the_input_dimensions() {
    let input = gradient_4x3();
    let bytes = encode(&input, 4, 3, ImageFormat::Jpeg { quality: 90 }).expect("jpeg encodes");
    assert!(bytes.starts_with(b"\xff\xd8"), "JPEG SOI marker missing");
    let img = decode(&bytes);
    assert_eq!((img.width(), img.height()), (4, 3));
}

#[test]
fn jpeg_quality_is_honoured() {
    let input = noise_64x64();
    let hi = encode(&input, 64, 64, ImageFormat::Jpeg { quality: 90 }).expect("q90 encodes");
    let lo = encode(&input, 64, 64, ImageFormat::Jpeg { quality: 10 }).expect("q10 encodes");
    assert!(
        lo.len() <= hi.len(),
        "quality 10 ({} bytes) should not exceed quality 90 ({} bytes)",
        lo.len(),
        hi.len()
    );
    assert!(
        lo.len() < hi.len(),
        "quality must change the output size for a noisy image"
    );
}

#[test]
fn zero_width_is_rejected() {
    assert_eq!(
        encode(&[], 0, 3, ImageFormat::Png),
        Err(EncodeError::InvalidDimensions)
    );
    assert_eq!(
        encode(&[], 4, 0, ImageFormat::Bmp),
        Err(EncodeError::InvalidDimensions)
    );
}

#[test]
fn length_mismatch_is_rejected() {
    let input = gradient_4x3();
    assert_eq!(
        encode(&input[..input.len() - 1], 4, 3, ImageFormat::Png),
        Err(EncodeError::InvalidDimensions)
    );
    assert_eq!(
        encode(&input, 4, 4, ImageFormat::Jpeg { quality: 90 }),
        Err(EncodeError::InvalidDimensions)
    );
}
