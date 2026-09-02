//! A radix-2 FFT, and the band folding on top of it.
//!
//! Hand-written rather than pulled in. `rustfft` is excellent and much faster,
//! but this transforms 1024 real samples about twenty times a second — roughly
//! 10k butterflies per transform, which is microseconds — and the binary has a
//! 10 MB budget. Speed is not the constraint here; size is.

use std::f32::consts::PI;

/// How many bars the UI draws. Enough to read as a spectrum, few enough that the
/// payload stays tiny.
pub const BANDS: usize = 16;

/// In-place iterative radix-2 Cooley-Tukey, on interleaved (re, im) pairs.
///
/// `re` and `im` must be the same length and a power of two.
pub fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert_eq!(n, im.len());
    debug_assert!(n.is_power_of_two());
    if n < 2 {
        return;
    }

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Butterflies, doubling the transform length each pass.
    let mut len = 2usize;
    while len <= n {
        let angle = -2.0 * PI / len as f32;
        let (wr, wi) = (angle.cos(), angle.sin());
        let mut i = 0usize;
        while i < n {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let a = i + k;
                let b = i + k + len / 2;
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next_cr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next_cr;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// A Hann window.
///
/// Without one, a tone that does not land exactly on a bin smears across the
/// whole spectrum — the bars then react to everything at once and the display
/// looks like noise rather than music.
pub fn hann(n: usize) -> Vec<f32> {
    (0..n).map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / n as f32).cos())).collect()
}

/// Fold an FFT magnitude spectrum into `BANDS` logarithmically-spaced bars.
///
/// Logarithmic because pitch is: linear bins would put nearly every musical
/// note in the leftmost bar and leave the right half showing cymbals.
pub fn to_bands(re: &[f32], im: &[f32], sample_rate: f32) -> [f32; BANDS] {
    let n = re.len();
    let mut out = [0.0f32; BANDS];
    if n < 4 {
        return out;
    }

    let nyquist = sample_rate / 2.0;
    let low = 40.0f32;
    let high = nyquist.min(16_000.0);
    let ratio = (high / low).powf(1.0 / BANDS as f32);

    let bin_of = |freq: f32| ((freq / nyquist) * (n / 2) as f32) as usize;

    let mut edge = low;
    for band in out.iter_mut() {
        let next = edge * ratio;
        let (from, to) = (bin_of(edge).max(1), bin_of(next).min(n / 2));
        // Peak rather than mean: an average across a wide high band washes a
        // real transient out against the quiet bins beside it.
        let mut peak = 0.0f32;
        for k in from..to.max(from + 1) {
            let mag = (re[k] * re[k] + im[k] * im[k]).sqrt();
            peak = peak.max(mag);
        }
        *band = peak;
        edge = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pure tone must land in one bin, which is the whole contract.
    #[test]
    fn transforms_a_sine_into_a_single_peak() {
        const N: usize = 256;
        let bin = 8usize;
        let mut re: Vec<f32> =
            (0..N).map(|i| (2.0 * PI * bin as f32 * i as f32 / N as f32).sin()).collect();
        let mut im = vec![0.0f32; N];
        fft(&mut re, &mut im);

        let mag = |k: usize| (re[k] * re[k] + im[k] * im[k]).sqrt();
        let peak = mag(bin);
        assert!(peak > 100.0, "expected a strong peak, got {peak}");
        for k in 1..N / 2 {
            if k != bin {
                assert!(mag(k) < peak * 0.05, "bin {k} leaked: {} vs {peak}", mag(k));
            }
        }
    }

    /// Silence in, silence out — the bars must not idle at some floor.
    #[test]
    fn silence_produces_no_energy() {
        let mut re = vec![0.0f32; 128];
        let mut im = vec![0.0f32; 128];
        fft(&mut re, &mut im);
        assert!(re.iter().chain(im.iter()).all(|v| v.abs() < 1e-6));
    }

    /// Constant input is entirely DC: all the energy in bin 0 and none above it.
    #[test]
    fn constant_input_is_all_dc() {
        const N: usize = 64;
        let mut re = vec![1.0f32; N];
        let mut im = vec![0.0f32; N];
        fft(&mut re, &mut im);
        assert!((re[0] - N as f32).abs() < 0.01, "dc bin was {}", re[0]);
        for k in 1..N {
            let mag = (re[k] * re[k] + im[k] * im[k]).sqrt();
            assert!(mag < 0.01, "bin {k} had {mag}");
        }
    }

    #[test]
    fn hann_window_is_zero_at_the_edges_and_one_in_the_middle() {
        let w = hann(64);
        assert!(w[0].abs() < 1e-6);
        assert!((w[32] - 1.0).abs() < 1e-6);
        assert_eq!(w.len(), 64);
    }

    /// A low tone must light a low bar, and a high tone a high one. This is the
    /// property that makes the display mean anything.
    #[test]
    fn band_folding_puts_low_and_high_tones_in_different_bars() {
        const N: usize = 1024;
        const RATE: f32 = 48_000.0;

        let bands_for = |freq: f32| {
            let mut re: Vec<f32> =
                (0..N).map(|i| (2.0 * PI * freq * i as f32 / RATE).sin()).collect();
            let mut im = vec![0.0f32; N];
            fft(&mut re, &mut im);
            to_bands(&re, &im, RATE)
        };

        let loudest = |b: [f32; BANDS]| {
            b.iter()
                .enumerate()
                .max_by(|a, c| a.1.partial_cmp(c.1).unwrap())
                .map(|(i, _)| i)
                .unwrap()
        };

        let low = loudest(bands_for(80.0));
        let high = loudest(bands_for(8000.0));
        assert!(low < 4, "80 Hz landed in band {low}");
        assert!(high > BANDS - 5, "8 kHz landed in band {high}");
    }

    #[test]
    fn band_folding_of_silence_is_flat() {
        let re = vec![0.0f32; 512];
        let im = vec![0.0f32; 512];
        assert!(to_bands(&re, &im, 48_000.0).iter().all(|v| *v == 0.0));
    }
}
