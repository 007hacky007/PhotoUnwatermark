//! Binary mask helpers. A [`Mask`] is a grayscale image where 255 marks the
//! watermark area to remove and 0 is background.

use image::{GrayImage, Luma, imageops};
use imageproc::rect::Rect;
use imageproc::region_labelling::{Connectivity, connected_components};

/// Grayscale mask, 255 = watermark area.
pub type Mask = GrayImage;

/// One 8-connected region of a mask.
#[derive(Debug, Clone)]
pub struct Component {
    /// Bounding box in the source mask.
    pub bbox: Rect,
    /// Number of set pixels.
    pub pixels: u32,
    /// Only this component's pixels, cropped to `bbox`.
    pub mask: Mask,
}

/// Grow radius in pixels for an image of `w` x `h`: `max(7, round(max(w,h) * frac))`.
pub fn grow_for(w: u32, h: u32, frac: f32) -> u32 {
    let r = (w.max(h) as f32 * frac).round();
    (r as u32).max(7)
}

/// Square-kernel dilation (kernel side `2r+1`). Any non-zero pixel counts as
/// set; the result contains only 0 and 255. Separable and O(pixels)
/// independent of `radius`.
pub fn dilate(m: &Mask, radius: u32) -> Mask {
    let (w, h) = m.dimensions();
    if w == 0 || h == 0 {
        return m.clone();
    }
    let (wu, hu, r) = (w as usize, h as usize, radius as usize);
    let src = m.as_raw();

    // Horizontal pass: sliding count of set pixels per row.
    let mut tmp = vec![0u8; wu * hu];
    for y in 0..hu {
        let row = &src[y * wu..(y + 1) * wu];
        let out = &mut tmp[y * wu..(y + 1) * wu];
        let mut count = row[..(r + 1).min(wu)].iter().filter(|&&v| v != 0).count();
        for x in 0..wu {
            out[x] = if count > 0 { 255 } else { 0 };
            let add = x + r + 1;
            if add < wu && row[add] != 0 {
                count += 1;
            }
            if x >= r && row[x - r] != 0 {
                count -= 1;
            }
        }
    }

    // Vertical pass: per-column counts, advanced row by row (cache friendly).
    let mut out = vec![0u8; wu * hu];
    let mut counts = vec![0u32; wu];
    for y in 0..(r + 1).min(hu) {
        for (c, &v) in counts.iter_mut().zip(&tmp[y * wu..(y + 1) * wu]) {
            *c += (v != 0) as u32;
        }
    }
    for y in 0..hu {
        for (o, &c) in out[y * wu..(y + 1) * wu].iter_mut().zip(&counts) {
            *o = if c > 0 { 255 } else { 0 };
        }
        let add = y + r + 1;
        if add < hu {
            for (c, &v) in counts.iter_mut().zip(&tmp[add * wu..(add + 1) * wu]) {
                *c += (v != 0) as u32;
            }
        }
        if y >= r {
            let rem = y - r;
            for (c, &v) in counts.iter_mut().zip(&tmp[rem * wu..(rem + 1) * wu]) {
                *c -= (v != 0) as u32;
            }
        }
    }
    GrayImage::from_raw(w, h, out).expect("buffer size matches dimensions")
}

/// Pixel-wise maximum. Both masks must have the same size; the result has the
/// size of `a` (checked with `debug_assert_eq!`).
pub fn union(a: &Mask, b: &Mask) -> Mask {
    debug_assert_eq!(a.dimensions(), b.dimensions(), "mask sizes must match");
    let mut out = a.clone();
    for (o, p) in out.pixels_mut().zip(b.pixels()) {
        o.0[0] = o.0[0].max(p.0[0]);
    }
    out
}

/// Bilinear resize, then threshold at > 127 to 0/255.
pub fn resize(m: &Mask, w: u32, h: u32) -> Mask {
    if w == 0 || h == 0 {
        return GrayImage::new(w, h);
    }
    let mut out = imageops::resize(m, w, h, imageops::FilterType::Triangle);
    for p in out.pixels_mut() {
        *p = Luma([if p.0[0] > 127 { 255 } else { 0 }]);
    }
    out
}

/// Fraction (0..1) of pixels that are non-zero. 0.0 for an empty mask.
pub fn coverage(m: &Mask) -> f32 {
    let total = m.width() as u64 * m.height() as u64;
    if total == 0 {
        return 0.0;
    }
    let set = m.as_raw().iter().filter(|&&v| v != 0).count() as u64;
    (set as f64 / total as f64) as f32
}

/// Bounding box of all non-zero pixels, or `None` if the mask is empty.
pub fn bbox(m: &Mask) -> Option<Rect> {
    let (w, h) = m.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut any = false;
    for y in 0..h {
        let row = &m.as_raw()[(y * w) as usize..((y + 1) * w) as usize];
        if let Some(first) = row.iter().position(|&v| v != 0) {
            let last = row.iter().rposition(|&v| v != 0).unwrap();
            any = true;
            x0 = x0.min(first as u32);
            x1 = x1.max(last as u32);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
    }
    any.then(|| Rect::at(x0 as i32, y0 as i32).of_size(x1 - x0 + 1, y1 - y0 + 1))
}

/// 8-connected components of the non-zero pixels, ordered by the raster
/// position of each component's first pixel.
pub fn components(m: &Mask) -> Vec<Component> {
    let (w, h) = m.dimensions();
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let labels = connected_components(m, Connectivity::Eight, Luma([0u8]));
    // Labels are assigned in raster order of first pixel, but do not rely on
    // them being contiguous: collect per label in first-seen order.
    struct Acc {
        x0: u32,
        y0: u32,
        x1: u32,
        y1: u32,
        pixels: u32,
    }
    let mut order: Vec<u32> = Vec::new();
    let mut accs: std::collections::HashMap<u32, Acc> = std::collections::HashMap::new();
    for (x, y, p) in labels.enumerate_pixels() {
        let l = p.0[0];
        if l == 0 {
            continue;
        }
        accs.entry(l)
            .and_modify(|a| {
                a.x0 = a.x0.min(x);
                a.x1 = a.x1.max(x);
                a.y1 = y;
                a.pixels += 1;
            })
            .or_insert_with(|| {
                order.push(l);
                Acc {
                    x0: x,
                    y0: y,
                    x1: x,
                    y1: y,
                    pixels: 1,
                }
            });
    }
    order
        .into_iter()
        .map(|l| {
            let a = &accs[&l];
            let (bw, bh) = (a.x1 - a.x0 + 1, a.y1 - a.y0 + 1);
            let mut cm = GrayImage::new(bw, bh);
            for y in 0..bh {
                for x in 0..bw {
                    if labels.get_pixel(a.x0 + x, a.y0 + y).0[0] == l {
                        cm.put_pixel(x, y, Luma([255]));
                    }
                }
            }
            Component {
                bbox: Rect::at(a.x0 as i32, a.y0 as i32).of_size(bw, bh),
                pixels: a.pixels,
                mask: cm,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(w: u32, h: u32) -> Mask {
        GrayImage::new(w, h)
    }

    fn fill(m: &mut Mask, x0: u32, y0: u32, w: u32, h: u32) {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                m.put_pixel(x, y, Luma([255]));
            }
        }
    }

    #[test]
    fn dilate_single_pixel() {
        let mut m = blank(11, 11);
        m.put_pixel(5, 5, Luma([255]));
        let d = dilate(&m, 2);
        for y in 0..11 {
            for x in 0..11 {
                let inside = (3..=7).contains(&x) && (3..=7).contains(&y);
                assert_eq!(
                    d.get_pixel(x, y).0[0],
                    if inside { 255 } else { 0 },
                    "({x},{y})"
                );
            }
        }
        assert_eq!(coverage(&d), 25.0 / 121.0);
    }

    #[test]
    fn dilate_edges_and_degenerate() {
        let mut m = blank(5, 4);
        m.put_pixel(0, 0, Luma([255]));
        let d = dilate(&m, 1);
        assert_eq!(d.as_raw().iter().filter(|&&v| v == 255).count(), 4);
        // radius larger than the image fills everything reachable
        assert_eq!(coverage(&dilate(&m, 100)), 1.0);
        assert_eq!(dilate(&m, 0), m);
        assert_eq!(dilate(&blank(0, 0), 3).dimensions(), (0, 0));
        // matches a naive reference on a random-ish pattern
        let mut p = blank(23, 17);
        for i in 0..40u32 {
            p.put_pixel((i * 7) % 23, (i * 11) % 17, Luma([255]));
        }
        let r = 3i32;
        let d = dilate(&p, r as u32);
        for y in 0..17i32 {
            for x in 0..23i32 {
                let mut want = 0;
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (nx, ny) = (x + dx, y + dy);
                        if (0..23).contains(&nx)
                            && (0..17).contains(&ny)
                            && p.get_pixel(nx as u32, ny as u32).0[0] != 0
                        {
                            want = 255;
                        }
                    }
                }
                assert_eq!(d.get_pixel(x as u32, y as u32).0[0], want, "({x},{y})");
            }
        }
    }

    #[test]
    fn components_two_blobs() {
        let mut m = blank(20, 10);
        fill(&mut m, 12, 5, 3, 2); // second in raster order (starts at y=5)
        fill(&mut m, 2, 1, 4, 3); // first: starts at y=1
        m.put_pixel(6, 4, Luma([255])); // diagonal neighbour of first blob (8-connected)
        let c = components(&m);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].bbox, Rect::at(2, 1).of_size(5, 4));
        assert_eq!(c[0].pixels, 13);
        assert_eq!(c[0].mask.dimensions(), (5, 4));
        assert_eq!(c[0].mask.get_pixel(4, 3).0[0], 255);
        assert_eq!(c[0].mask.get_pixel(3, 0).0[0], 255);
        assert_eq!(c[0].mask.get_pixel(4, 0).0[0], 0);
        assert_eq!(c[0].mask.get_pixel(0, 3).0[0], 0);
        assert_eq!(c[1].bbox, Rect::at(12, 5).of_size(3, 2));
        assert_eq!(c[1].pixels, 6);
        assert!(components(&blank(4, 4)).is_empty());
    }

    #[test]
    fn grow_for_values() {
        assert_eq!(grow_for(7360, 4912, 0.004), 29);
        assert_eq!(grow_for(800, 600, 0.004), 7);
    }

    #[test]
    fn coverage_half() {
        let mut m = blank(10, 10);
        fill(&mut m, 0, 0, 10, 5);
        assert_eq!(coverage(&m), 0.5);
        assert_eq!(coverage(&blank(0, 0)), 0.0);
    }

    #[test]
    fn bbox_cases() {
        assert_eq!(bbox(&blank(8, 8)), None);
        let mut m = blank(8, 8);
        m.put_pixel(2, 3, Luma([255]));
        m.put_pixel(6, 5, Luma([255]));
        assert_eq!(bbox(&m), Some(Rect::at(2, 3).of_size(5, 3)));
    }

    #[test]
    fn union_and_resize() {
        let mut a = blank(4, 4);
        let mut b = blank(4, 4);
        a.put_pixel(0, 0, Luma([255]));
        b.put_pixel(3, 3, Luma([255]));
        let u = union(&a, &b);
        assert_eq!(u.get_pixel(0, 0).0[0], 255);
        assert_eq!(u.get_pixel(3, 3).0[0], 255);
        assert_eq!(coverage(&u), 2.0 / 16.0);

        let mut big = blank(8, 8);
        fill(&mut big, 0, 0, 4, 8);
        let small = resize(&big, 4, 4);
        assert_eq!(small.dimensions(), (4, 4));
        assert!(small.pixels().all(|p| p.0[0] == 0 || p.0[0] == 255));
        assert_eq!(small.get_pixel(0, 2).0[0], 255);
        assert_eq!(small.get_pixel(3, 2).0[0], 0);
    }

    #[test]
    #[ignore = "timing check, run with --release"]
    fn dilate_large_is_fast() {
        let mut m = blank(7360, 4912);
        fill(&mut m, 3000, 2000, 400, 100);
        m.put_pixel(0, 0, Luma([255]));
        let t = std::time::Instant::now();
        let d = dilate(&m, 29);
        let el = t.elapsed();
        eprintln!("dilate 7360x4912 r=29: {el:?}");
        assert!(el.as_secs_f32() < 2.0, "too slow: {el:?}");
        assert_eq!(d.get_pixel(29, 29).0[0], 255);
        assert_eq!(d.get_pixel(30, 30).0[0], 0);
    }
}
