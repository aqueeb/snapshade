//! Pixel-format helpers shared by the capture backends (pure; compiled and tested on every
//! host).

/// Convert `height` rows of 32-bit BGRA pixels, `pitch` bytes apart in `src`, into tightly
/// packed RGBA (alpha forced to 255) in `dst` (`width * height * 4` bytes). Rows are split
/// into bands converted on separate threads for frames of 4K and above: the conversion is
/// memory-bound and a single thread takes ~25 ms for a 33 MB frame at `opt-level = "s"`.
///
/// Panics if `src` is shorter than `pitch * (height - 1) + width * 4` or `dst` shorter than
/// `width * height * 4` — a programming error, never a runtime condition (the callers size
/// both from the same texture description).
// Only the Windows capture backends call it; the unit tests below run on every host.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn bgra_rows_to_rgba(src: &[u8], pitch: usize, width: usize, height: usize, dst: &mut [u8]) {
    let row_bytes = width * 4;
    assert!(
        dst.len() >= row_bytes * height,
        "rgba destination too small"
    );
    if height > 0 {
        assert!(
            src.len() >= pitch * (height - 1) + row_bytes,
            "bgra source too small"
        );
    }
    let convert = |rows: std::ops::Range<usize>, out: &mut [u8]| {
        for (i, row) in rows.enumerate() {
            let s = &src[row * pitch..row * pitch + row_bytes];
            let d = &mut out[i * row_bytes..(i + 1) * row_bytes];
            let (dp, _) = d.as_chunks_mut::<4>();
            let (sp, _) = s.as_chunks::<4>();
            for (dst_px, px) in dp.iter_mut().zip(sp) {
                dst_px[0] = px[2];
                dst_px[1] = px[1];
                dst_px[2] = px[0];
                dst_px[3] = 255;
            }
        }
    };
    const BANDS: usize = 4;
    const MIN_PIXELS_FOR_BANDS: usize = 1920 * 1080;
    if width * height < MIN_PIXELS_FOR_BANDS || height < BANDS {
        convert(0..height, &mut dst[..row_bytes * height]);
        return;
    }
    let band_rows = height.div_ceil(BANDS);
    std::thread::scope(|scope| {
        for (band, out) in dst[..row_bytes * height]
            .chunks_mut(band_rows * row_bytes)
            .enumerate()
        {
            let start = band * band_rows;
            let end = (start + band_rows).min(height);
            let convert = &convert;
            scope.spawn(move || convert(start..end, out));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::bgra_rows_to_rgba;

    #[test]
    fn swaps_channels_forces_alpha_and_honours_the_pitch() {
        // 2x2 BGRA with a 12-byte pitch (4 bytes of padding per row).
        let src = [
            1, 2, 3, 9, 4, 5, 6, 9, 0xAA, 0xAA, 0xAA, 0xAA, // row 0 + padding
            7, 8, 9, 9, 10, 11, 12, 9, 0xBB, 0xBB, 0xBB, 0xBB, // row 1 + padding
        ];
        let mut dst = [0u8; 16];
        bgra_rows_to_rgba(&src, 12, 2, 2, &mut dst);
        assert_eq!(
            dst,
            [3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, 12, 11, 10, 255]
        );
    }

    #[test]
    fn banded_conversion_matches_the_single_thread_result() {
        // Large enough to take the banded path; a height that does not divide by 4.
        let (w, h) = (1920usize, 1083usize);
        let pitch = w * 4 + 64;
        let src: Vec<u8> = (0..pitch * h).map(|i| (i % 251) as u8).collect();
        let mut banded = vec![0u8; w * h * 4];
        bgra_rows_to_rgba(&src, pitch, w, h, &mut banded);
        let mut expected = vec![0u8; w * h * 4];
        for row in 0..h {
            for col in 0..w {
                let s = row * pitch + col * 4;
                let d = (row * w + col) * 4;
                expected[d] = src[s + 2];
                expected[d + 1] = src[s + 1];
                expected[d + 2] = src[s];
                expected[d + 3] = 255;
            }
        }
        assert!(banded == expected);
    }

    #[test]
    fn zero_height_is_a_no_op() {
        let mut dst = [7u8; 4];
        bgra_rows_to_rgba(&[], 0, 1, 0, &mut dst);
        assert_eq!(dst, [7, 7, 7, 7]);
    }
}
