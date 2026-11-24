//! Masked normalised cross-correlation (Padfield, "Masked object registration in the Fourier
//! domain", IEEE TIP 2012 / CVPR 2010), computed with FFTs.

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

/// Masked normalised cross-correlation of `tmpl` over `image`.
///
/// `image` is `iw` x `ih` and `tmpl`/`tmask` are `tw` x `th`, all row-major. `tmask` holds
/// per-pixel weights (0 = ignore, 1 = use; fractional values give a weighted correlation;
/// negative values count as 0), so only the template pixels under the mask take part.
///
/// Returns the score map for valid placements only (the template fully inside the image): it
/// is `(iw - tw + 1)` x `(ih - th + 1)`, row-major, and entry `y * (iw - tw + 1) + x` is the
/// score with the template's top-left corner at image pixel `(x, y)`. Scores are in -1..=1;
/// a placement where the image under the mask, or the template under the mask, has (numerically)
/// zero variance scores 0. The map is empty when the template is empty or larger than the image.
///
/// Panics when a slice length does not match its dimensions.
pub fn masked_ncc(
    image: &[f32],
    iw: usize,
    ih: usize,
    tmpl: &[f32],
    tmask: &[f32],
    tw: usize,
    th: usize,
) -> Vec<f32> {
    assert_eq!(image.len(), iw * ih, "image size");
    assert_eq!(tmpl.len(), tw * th, "template size");
    assert_eq!(tmask.len(), tw * th, "template mask size");
    if tw == 0 || th == 0 || tw > iw || th > ih {
        return Vec::new();
    }
    let (ow, oh) = (iw - tw + 1, ih - th + 1);
    let mut out = vec![0f32; ow * oh];

    // Template statistics under the mask. Centring the template makes sum(m * t') zero, so the
    // covariance is just the correlation of the image with m * t'.
    let (mut n, mut st) = (0f64, 0f64);
    for (&t, &m) in tmpl.iter().zip(tmask) {
        let m = m.max(0.0) as f64;
        n += m;
        st += m * t as f64;
    }
    if n <= 0.0 {
        return out;
    }
    let mean_t = st / n;
    let (mut var_t, mut stt) = (0f64, 0f64);
    for (&t, &m) in tmpl.iter().zip(tmask) {
        let (m, t) = (m.max(0.0) as f64, t as f64);
        var_t += m * (t - mean_t) * (t - mean_t);
        stt += m * t * t;
    }
    if var_t <= 1e-12 * stt {
        return out;
    }

    // Centring the image (any constant shift leaves the scores unchanged) keeps the f^2 sums
    // small, which matters for the cancellation in sum(f^2) - sum(f)^2 / n.
    let mean_f = image.iter().map(|&v| v as f64).sum::<f64>() / image.len() as f64;
    let fft = Fft2::new(fft_len(iw), fft_len(ih));
    let (fw, fh) = (fft.w, fft.h);
    let mut f = vec![Complex::default(); fw * fh];
    let mut f2 = vec![Complex::default(); fw * fh];
    let mut energy = 0f64;
    for y in 0..ih {
        for x in 0..iw {
            let v = image[y * iw + x] as f64 - mean_f;
            f[y * fw + x].re = v;
            f2[y * fw + x].re = v * v;
            energy += v * v;
        }
    }
    let mut a = vec![Complex::default(); fw * fh];
    let mut b = vec![Complex::default(); fw * fh];
    for y in 0..th {
        for x in 0..tw {
            let m = tmask[y * tw + x].max(0.0) as f64;
            a[y * fw + x].re = m * (tmpl[y * tw + x] as f64 - mean_t);
            b[y * fw + x].re = m;
        }
    }
    let (f, f2, a, b) = (
        fft.forward(f),
        fft.forward(f2),
        fft.forward(a),
        fft.forward(b),
    );
    // Correlation: corr(u) = sum_x f(x + u) k(x) = IFFT(F * conj(K))(u). The FFT size is at
    // least the image size, so valid placements never wrap around.
    let corr = |s: &[Complex<f64>], k: &[Complex<f64>]| {
        fft.inverse(s.iter().zip(k).map(|(s, k)| s * k.conj()).collect())
    };
    let (c_ft, c_f, c_ff) = (corr(&f, &a), corr(&f, &b), corr(&f2, &b));
    let norm = 1.0 / (fw * fh) as f64;
    // FFT round-off is relative to the whole image's energy; variances below this are noise.
    let eps = 1e-10 * energy;
    for y in 0..oh {
        for x in 0..ow {
            let i = y * fw + x;
            let (sft, sf, sff) = (c_ft[i].re * norm, c_f[i].re * norm, c_ff[i].re * norm);
            let var_f = sff - sf * sf / n;
            if var_f > eps && var_f > 1e-9 * sff {
                out[y * ow + x] = (sft / (var_f * var_t).sqrt()).clamp(-1.0, 1.0) as f32;
            }
        }
    }
    out
}

/// Smallest n' >= n whose prime factors are all 2, 3 or 5 (fast FFT sizes).
fn fft_len(n: usize) -> usize {
    let mut m = n.max(1);
    loop {
        let mut r = m;
        for p in [2, 3, 5] {
            while r.is_multiple_of(p) {
                r /= p;
            }
        }
        if r == 1 {
            return m;
        }
        m += 1;
    }
}

/// 2-D FFT of a row-major `w` x `h` buffer. The spectrum is kept transposed (column-major),
/// which is fine for element-wise products and saves two transposes per transform.
struct Fft2 {
    w: usize,
    h: usize,
    row_fwd: Arc<dyn Fft<f64>>,
    col_fwd: Arc<dyn Fft<f64>>,
    row_inv: Arc<dyn Fft<f64>>,
    col_inv: Arc<dyn Fft<f64>>,
}

impl Fft2 {
    fn new(w: usize, h: usize) -> Self {
        let mut p = FftPlanner::new();
        Self {
            w,
            h,
            row_fwd: p.plan_fft_forward(w),
            col_fwd: p.plan_fft_forward(h),
            row_inv: p.plan_fft_inverse(w),
            col_inv: p.plan_fft_inverse(h),
        }
    }

    fn forward(&self, mut buf: Vec<Complex<f64>>) -> Vec<Complex<f64>> {
        self.row_fwd.process(&mut buf);
        let mut t = transpose(&buf, self.w, self.h);
        self.col_fwd.process(&mut t);
        t
    }

    /// Inverse of [`Fft2::forward`], unnormalised (scaled by `w * h`).
    fn inverse(&self, mut t: Vec<Complex<f64>>) -> Vec<Complex<f64>> {
        self.col_inv.process(&mut t);
        let mut buf = transpose(&t, self.h, self.w);
        self.row_inv.process(&mut buf);
        buf
    }
}

/// Transpose a row-major `w` x `h` buffer into a row-major `h` x `w` one.
fn transpose(src: &[Complex<f64>], w: usize, h: usize) -> Vec<Complex<f64>> {
    const B: usize = 32;
    let mut dst = vec![Complex::default(); w * h];
    for y0 in (0..h).step_by(B) {
        for x0 in (0..w).step_by(B) {
            for y in y0..(y0 + B).min(h) {
                for x in x0..(x0 + B).min(w) {
                    dst[x * h + y] = src[y * w + x];
                }
            }
        }
    }
    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    /// Direct weighted NCC at every valid placement, in f64.
    fn brute(
        image: &[f32],
        iw: usize,
        ih: usize,
        tmpl: &[f32],
        tmask: &[f32],
        tw: usize,
        th: usize,
    ) -> Vec<f32> {
        if tw == 0 || th == 0 || tw > iw || th > ih {
            return Vec::new();
        }
        let (ow, oh) = (iw - tw + 1, ih - th + 1);
        let mut out = vec![0f32; ow * oh];
        for oy in 0..oh {
            for ox in 0..ow {
                let (mut n, mut sf, mut st) = (0f64, 0f64, 0f64);
                for y in 0..th {
                    for x in 0..tw {
                        let m = tmask[y * tw + x] as f64;
                        n += m;
                        sf += m * image[(oy + y) * iw + ox + x] as f64;
                        st += m * tmpl[y * tw + x] as f64;
                    }
                }
                if n <= 0.0 {
                    continue;
                }
                let (mf, mt) = (sf / n, st / n);
                let (mut cov, mut vf, mut vt) = (0f64, 0f64, 0f64);
                for y in 0..th {
                    for x in 0..tw {
                        let m = tmask[y * tw + x] as f64;
                        let f = image[(oy + y) * iw + ox + x] as f64 - mf;
                        let t = tmpl[y * tw + x] as f64 - mt;
                        cov += m * f * t;
                        vf += m * f * f;
                        vt += m * t * t;
                    }
                }
                if vf > 1e-9 && vt > 1e-9 {
                    out[oy * ow + ox] = (cov / (vf * vt).sqrt()) as f32;
                }
            }
        }
        out
    }

    #[test]
    fn matches_brute_force() {
        let mut rng = Lcg(7);
        for (iw, ih, tw, th, binary) in [
            (17, 13, 5, 4, true),
            (20, 9, 7, 9, false),
            (31, 24, 1, 1, true),
            (12, 12, 12, 12, true),
            (25, 18, 9, 6, false),
            (8, 30, 3, 11, true),
        ] {
            let image: Vec<f32> = (0..iw * ih).map(|_| rng.next() * 255.0).collect();
            let tmpl: Vec<f32> = (0..tw * th).map(|_| rng.next() * 255.0).collect();
            let tmask: Vec<f32> = (0..tw * th)
                .map(|_| {
                    let v = rng.next();
                    if binary { (v > 0.4) as u8 as f32 } else { v }
                })
                .collect();
            let fast = masked_ncc(&image, iw, ih, &tmpl, &tmask, tw, th);
            let slow = brute(&image, iw, ih, &tmpl, &tmask, tw, th);
            assert_eq!(fast.len(), (iw - tw + 1) * (ih - th + 1));
            assert_eq!(fast.len(), slow.len());
            let diff = fast
                .iter()
                .zip(&slow)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            assert!(diff < 1e-3, "{iw}x{ih} / {tw}x{th}: max diff {diff}");
        }
    }

    #[test]
    fn zero_variance_scores_zero() {
        // Left half flat, right half random; the template is random.
        let (iw, ih, tw, th) = (24, 10, 6, 6);
        let mut rng = Lcg(3);
        let image: Vec<f32> = (0..iw * ih)
            .map(|i| {
                if i % iw < 12 {
                    90.0
                } else {
                    rng.next() * 255.0
                }
            })
            .collect();
        let tmpl: Vec<f32> = (0..tw * th).map(|_| rng.next()).collect();
        let ones = vec![1f32; tw * th];
        let map = masked_ncc(&image, iw, ih, &tmpl, &ones, tw, th);
        let ow = iw - tw + 1;
        for y in 0..ih - th + 1 {
            for x in 0..=12 - tw {
                assert_eq!(map[y * ow + x], 0.0, "flat placement ({x},{y})");
            }
        }
        // A flat template scores 0 everywhere, as does an all-zero mask.
        let flat = vec![5f32; tw * th];
        assert!(
            masked_ncc(&image, iw, ih, &flat, &ones, tw, th)
                .iter()
                .all(|&v| v == 0.0)
        );
        let none = vec![0f32; tw * th];
        assert!(
            masked_ncc(&image, iw, ih, &tmpl, &none, tw, th)
                .iter()
                .all(|&v| v == 0.0)
        );
    }

    #[test]
    fn mask_excludes_pixels() {
        // Template matches the image only under the mask; the unmasked half is garbage.
        let (iw, ih, tw, th) = (30, 20, 8, 6);
        let mut rng = Lcg(11);
        let image: Vec<f32> = (0..iw * ih).map(|_| rng.next() * 100.0).collect();
        let (ox, oy) = (13, 7);
        let mut tmpl = vec![0f32; tw * th];
        let mut tmask = vec![0f32; tw * th];
        for y in 0..th {
            for x in 0..tw {
                if x < tw / 2 {
                    tmpl[y * tw + x] = image[(oy + y) * iw + ox + x] * 2.0 + 7.0;
                    tmask[y * tw + x] = 1.0;
                } else {
                    tmpl[y * tw + x] = rng.next() * 1000.0;
                }
            }
        }
        let map = masked_ncc(&image, iw, ih, &tmpl, &tmask, tw, th);
        let ow = iw - tw + 1;
        let best = (0..map.len())
            .max_by(|&a, &b| map[a].total_cmp(&map[b]))
            .unwrap();
        assert_eq!((best % ow, best / ow), (ox, oy));
        assert!(map[best] > 0.999, "{}", map[best]);
    }

    #[test]
    fn degenerate_sizes() {
        let image = vec![1f32; 6];
        assert!(masked_ncc(&image, 3, 2, &[1.0; 9], &[1.0; 9], 3, 3).is_empty());
        assert!(masked_ncc(&image, 3, 2, &[], &[], 0, 0).is_empty());
        assert!(masked_ncc(&[], 0, 0, &[1.0], &[1.0], 1, 1).is_empty());
    }
}
