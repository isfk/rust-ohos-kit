//! What the core makes of a photo's pixels: "is this shot bright?", "what colour
//! is it?", "where is the light?".
//!
//! Plain arithmetic on RGBA bytes, no decoding: the platform decodes (its image
//! decoder handles HEIF, which is what the camera writes) and the core does the
//! maths. That half is testable without a phone, which is the reason it is here.

use std::time::Instant;

use serde::{Deserialize, Serialize};

const LUMA_BUCKETS: usize = 8; // the histogram the UI draws as bars
const LUMA_GRID: usize = 8; // the coarse brightness map the UI draws as a square
const COLOR_BITS: u16 = 4; // per channel: 4096 buckets, enough to name a colour
const TOP_COLORS: usize = 3;
const MAX_SAMPLES: usize = 1 << 20;

/// One of the colours that make up the picture, with its share of the pixels
/// (0..1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorShare {
    pub hex: String,
    pub share: f64,
}

/// A picture, in numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub width: i32,
    pub height: i32,
    /// 0..100
    pub mean_luma: i32,
    /// 0..255 each
    pub mean_rgb: [i32; 3],
    /// `LUMA_BUCKETS` percentages, summing to 100
    pub histo: Vec<i32>,
    /// `LUMA_GRID`×`LUMA_GRID` brightness 0..100, row by row; a cell no sampled
    /// pixel lands in (only possible when the picture is smaller than the grid)
    /// reads 0
    pub grid: Vec<i32>,
    /// The `TOP_COLORS` most common colours, biggest share first
    pub colors: Vec<ColorShare>,
    /// How long the maths took
    pub micros: i64,
}

/// What a buffer that is not RGBA of the stated size gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadPixels;

impl std::fmt::Display for BadPixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("pixels: buffer doesn't match the size")
    }
}

impl std::error::Error for BadPixels {}

#[derive(Default, Clone, Copy)]
struct ColorBucket {
    count: i64,
    r: i64,
    g: i64,
    b: i64,
}

/// Reads RGBA_8888 pixels and describes them.
///
/// `stride` is the number of bytes in a row: the platform's buffers are padded,
/// and a padded row read as pixels is a picture sliced on the diagonal. Pass 0
/// for a tightly packed picture.
pub fn analyze_pixels(
    rgba: &[u8],
    width: i32,
    height: i32,
    stride: i32,
) -> Result<Analysis, BadPixels> {
    if width <= 0 || height <= 0 {
        return Err(BadPixels);
    }
    let width = width as usize;
    let height = height as usize;
    let stride = if stride <= 0 {
        width * 4
    } else {
        stride as usize
    };
    if stride < width * 4 || rgba.len() < stride * height {
        return Err(BadPixels);
    }
    let started = Instant::now();

    // A 12 MP photo through the bridge is fine, but there is no point doing the
    // maths on every pixel: sample about a million of them at most.
    let mut step = 1;
    while (width / step) * (height / step) > MAX_SAMPLES {
        step += 1;
    }

    let mut pixels: i64 = 0;
    let (mut sum_r, mut sum_g, mut sum_b, mut sum_luma) = (0i64, 0i64, 0i64, 0i64);
    let mut histo = [0i64; LUMA_BUCKETS];
    let mut cells = [0i64; LUMA_GRID * LUMA_GRID];
    let mut cell_count = [0i64; LUMA_GRID * LUMA_GRID];
    // One slot per colour the bucketing can produce, indexed by the key itself.
    // A map here would be a hash lookup per pixel, and this loop is the only
    // real arithmetic in the demo: 812 µs with the map, 398 µs with the array,
    // measured on a 240×320 photo.
    let mut buckets = [ColorBucket::default(); 1 << (3 * COLOR_BITS)];

    for y in (0..height).step_by(step) {
        let row = &rgba[y * stride..];
        let cell_y = y * LUMA_GRID / height;
        for x in (0..width).step_by(step) {
            let p = &row[x * 4..];
            let (r, g, b) = (i64::from(p[0]), i64::from(p[1]), i64::from(p[2]));

            // ITU-R BT.601 luma, in integers: what the eye sees, not an average.
            let luma = (299 * r + 587 * g + 114 * b) / 1000;

            pixels += 1;
            sum_r += r;
            sum_g += g;
            sum_b += b;
            sum_luma += luma;
            histo[(luma as usize * LUMA_BUCKETS / 256).min(LUMA_BUCKETS - 1)] += 1;
            let cell = cell_y * LUMA_GRID + x * LUMA_GRID / width;
            cells[cell] += luma;
            cell_count[cell] += 1;

            let key = ((r as u16 >> (8 - COLOR_BITS)) << (2 * COLOR_BITS)
                | (g as u16 >> (8 - COLOR_BITS)) << COLOR_BITS
                | (b as u16 >> (8 - COLOR_BITS))) as usize;
            let bucket = &mut buckets[key];
            bucket.count += 1;
            bucket.r += r;
            bucket.g += g;
            bucket.b += b;
        }
    }
    if pixels == 0 {
        return Err(BadPixels);
    }

    let mut histo_out: Vec<i32> = histo.iter().map(|n| (n * 100 / pixels) as i32).collect();
    // Rounding leaves a few percent unaccounted for; give them to the tallest
    // bar, so the bars the UI draws add up to the whole picture.
    let mut rest = 100;
    let mut tallest = 0;
    for (i, p) in histo_out.iter().enumerate() {
        rest -= p;
        if *p > histo_out[tallest] {
            tallest = i;
        }
    }
    histo_out[tallest] += rest;

    let mut grid = vec![0i32; LUMA_GRID * LUMA_GRID];
    for (i, cell) in grid.iter_mut().enumerate() {
        if cell_count[i] > 0 {
            *cell = (cells[i] * 100 / (cell_count[i] * 255)) as i32;
        }
    }

    // The three biggest buckets, without sorting all 4096: a photo with many
    // colours fills most of them, and sorting costs about as much as the
    // per-pixel loop it sorts for. Three slots and an insertion is enough.
    let mut top: [Option<usize>; TOP_COLORS] = [None; TOP_COLORS];
    for i in 0..buckets.len() {
        if buckets[i].count == 0 {
            continue;
        }
        for j in 0..TOP_COLORS {
            let better = match top[j] {
                None => true,
                Some(at) => buckets[i].count > buckets[at].count,
            };
            if better {
                // Shift the rest down: `top` stays ordered, biggest first.
                for k in (j + 1..TOP_COLORS).rev() {
                    top[k] = top[k - 1];
                }
                top[j] = Some(i);
                break;
            }
        }
    }
    let colors = top
        .iter()
        .flatten()
        .map(|&at| {
            let b = &buckets[at];
            ColorShare {
                hex: format!(
                    "#{:02x}{:02x}{:02x}",
                    b.r / b.count,
                    b.g / b.count,
                    b.b / b.count
                ),
                share: b.count as f64 / pixels as f64,
            }
        })
        .collect();

    Ok(Analysis {
        width: width as i32,
        height: height as i32,
        mean_luma: (sum_luma * 100 / (pixels * 255)) as i32, // 0..255 → 0..100
        mean_rgb: [
            (sum_r / pixels) as i32,
            (sum_g / pixels) as i32,
            (sum_b / pixels) as i32,
        ],
        histo: histo_out,
        grid,
        colors,
        micros: started.elapsed().as_micros() as i64,
    })
}

#[cfg(test)]
mod tests {
    //! The cases are the Go suite's, one for one: the port is only worth
    //! anything if the same pixels give the same numbers.

    use super::*;

    /// Builds width×height pixels, rows `stride` bytes long, with the padding
    /// filled by junk — which is what a platform buffer looks like.
    fn rgba(
        width: usize,
        height: usize,
        stride: usize,
        at: impl Fn(usize, usize) -> [u8; 3],
    ) -> Vec<u8> {
        let stride = if stride == 0 { width * 4 } else { stride };
        let mut buf = vec![0xA5u8; stride * height]; // junk: padding must not read as a pixel
        for y in 0..height {
            for x in 0..width {
                let c = at(x, y);
                let p = y * stride + x * 4;
                buf[p] = c[0];
                buf[p + 1] = c[1];
                buf[p + 2] = c[2];
                buf[p + 3] = 0xFF;
            }
        }
        buf
    }

    fn halves(
        width: usize,
        height: usize,
        stride: usize,
        left: [u8; 3],
        right: [u8; 3],
    ) -> Vec<u8> {
        rgba(width, height, stride, |x, _| {
            if x < width / 2 {
                left
            } else {
                right
            }
        })
    }

    const BLACK: [u8; 3] = [0, 0, 0];
    const WHITE: [u8; 3] = [255, 255, 255];

    #[test]
    fn black_and_white_halves() {
        // 16 wide, so every one of the 8 grid columns gets samples: a picture
        // smaller than the grid would leave cells with nothing to average.
        let a = analyze_pixels(&halves(16, 16, 0, BLACK, WHITE), 16, 16, 0).unwrap();
        assert_eq!(a.mean_luma, 50);
        assert_eq!((a.histo[0], a.histo[7]), (50, 50));
        assert_eq!(a.mean_rgb, [127, 127, 127]);
        // Left cells dark, right cells bright: the grid is a picture, not an average.
        for (i, v) in a.grid.iter().enumerate() {
            let want = if i % 8 < 4 { 0 } else { 100 };
            assert_eq!(*v, want, "grid[{i}]");
        }
    }

    #[test]
    fn names_the_colors() {
        let a = analyze_pixels(&halves(8, 4, 0, [255, 0, 0], [0, 0, 255]), 8, 4, 0).unwrap();
        assert_eq!(a.colors.len(), 2, "{:?}", a.colors);
        for want in ["#ff0000", "#0000ff"] {
            let found = a.colors.iter().find(|c| c.hex == want);
            assert_eq!(
                found.map(|c| c.share),
                Some(0.5),
                "{want} in {:?}",
                a.colors
            );
        }
    }

    #[test]
    fn merges_colours_in_the_same_bucket() {
        // The key keeps the top 4 bits of each channel, so two reds that differ
        // only below that are the same colour here — and the average the UI
        // shows has to be the average of both, not one of them.
        let a = analyze_pixels(&halves(8, 4, 0, [255, 0, 0], [240, 0, 0]), 8, 4, 0).unwrap();
        assert_eq!(a.colors.len(), 1, "{:?}", a.colors);
        assert_eq!(a.colors[0].hex, "#f70000"); // the average of 255 and 240
        assert_eq!(a.colors[0].share, 1.0);
    }

    #[test]
    fn keeps_colours_in_different_buckets() {
        // One step coarser than the test above: 224>>4 is 14, not 15, so these
        // are two colours. Guards the key arithmetic against an off-by-one in
        // the shift, which a per-pixel array index would happily hide.
        let a = analyze_pixels(&halves(8, 4, 0, [255, 0, 0], [224, 0, 0]), 8, 4, 0).unwrap();
        assert_eq!(a.colors.len(), 2, "{:?}", a.colors);
    }

    #[test]
    fn ignores_row_padding() {
        let packed = halves(4, 4, 0, BLACK, WHITE);
        let padded = halves(4, 4, 4 * 4 + 8, BLACK, WHITE);
        let a = analyze_pixels(&padded, 4, 4, 4 * 4 + 8).unwrap();
        let b = analyze_pixels(&packed, 4, 4, 0).unwrap();
        assert_eq!((a.mean_luma, a.mean_rgb), (b.mean_luma, b.mean_rgb));
    }

    #[test]
    fn mid_grey_is_one_bucket() {
        let grey = [128u8, 128, 128];
        let a = analyze_pixels(&halves(4, 4, 0, grey, grey), 4, 4, 0).unwrap();
        // 128/255 is 50%: one bucket, all of it.
        assert_eq!(a.mean_luma, 50);
        assert_eq!(a.histo[4], 100);
        assert_eq!(a.colors.len(), 1);
        assert_eq!(a.colors[0].share, 1.0);
    }

    #[test]
    fn sampling_keeps_big_pictures_honest() {
        let (w, h) = (1200, 1000); // more pixels than MAX_SAMPLES, so it samples
        let grey = [51u8, 51, 51];
        let a = analyze_pixels(&halves(w, h, 0, grey, grey), w as i32, h as i32, 0).unwrap();
        assert_eq!(a.mean_luma, 20); // 51/255: sampling changed the answer
        assert!(a.micros > 0);
    }

    #[test]
    fn rejects_bad_buffers() {
        assert_eq!(analyze_pixels(&[], 4, 4, 0), Err(BadPixels));
        assert_eq!(
            analyze_pixels(&[0; 4 * 4 * 4], 4, 4, 4 * 4 * 2),
            Err(BadPixels)
        );
        assert_eq!(analyze_pixels(&[0; 16], 0, 4, 0), Err(BadPixels));
        assert_eq!(analyze_pixels(&[0; 63], 4, 4, 0), Err(BadPixels));
    }
}
