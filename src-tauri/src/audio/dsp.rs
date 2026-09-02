//! The sample maths behind volume boost and bass boost.
//!
//! Kept apart from the WASAPI plumbing because it is the half that can be
//! proven: every function here is pure, and the tests below check the things
//! that are actually easy to get wrong — a shelf that changes the treble, a
//! filter that explodes, a gain that clips into distortion.
//!
//! Everything works in `f32` at whatever sample rate the endpoint runs at.

use std::f32::consts::PI;

/// Loudest sample we will emit.
///
/// Slightly under full scale: the last fraction of a decibel buys nothing and
/// leaves no room for the intersample peaks that reconstruction can produce.
const CEILING: f32 = 0.985;

/// A one-pole-per-stage biquad, direct form I.
///
/// Direct form I rather than II because it is the form that behaves at the
/// large gains this module exists to apply — form II's single delay line can
/// overflow internally on a signal the output never shows.
#[derive(Debug, Clone, Copy, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    /// A low shelf: everything below `freq` is lifted by `gain_db`, everything
    /// well above it is left alone.
    ///
    /// This is what "bass boost" should mean. A low-*pass* would remove the
    /// rest of the music instead, and a peaking filter would leave the lowest
    /// octave — where most of the weight of a kick drum is — untouched.
    ///
    /// Coefficients are the RBJ audio EQ cookbook's, with `S = 1` (the widest
    /// shelf that does not overshoot).
    pub fn low_shelf(sample_rate: f32, freq: f32, gain_db: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * (freq / sample_rate);
        let (sin_w0, cos_w0) = w0.sin_cos();
        // The cookbook writes alpha as `sin(w0)/2 * sqrt((A + 1/A)(1/S - 1) + 2)`.
        // At S = 1 — the widest shelf that does not overshoot — the `(1/S - 1)`
        // factor is zero, so the whole first term drops out and what is left is
        // sqrt(2), independent of the gain.
        let alpha = sin_w0 / 2.0 * std::f32::consts::SQRT_2;
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            ..Self::default()
        }
    }

    /// Pass one sample through, advancing the filter's memory.
    #[inline]
    pub fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// Forget the past. Used when playback stops, so a silent gap does not end
    /// with the tail of whatever was playing before it.
    pub fn reset(&mut self) {
        *self = Self { x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0, ..*self };
    }
}

/// Soft-knee limiter.
///
/// Boosting past 100% guarantees samples above full scale, and what happens to
/// them decides whether this feature sounds loud or sounds broken. Hard
/// clipping squares off every peak and turns bass into a buzz. This bends the
/// top of the range instead: below the knee nothing is touched at all, above it
/// the curve compresses smoothly onto the ceiling and never crosses it.
#[derive(Debug, Clone, Copy)]
pub struct Limiter {
    knee: f32,
}

impl Default for Limiter {
    fn default() -> Self {
        // Two thirds of the way up. Music sits below this most of the time, so
        // most samples pass through untouched and the sound stays clean.
        Self { knee: 0.66 }
    }
}

impl Limiter {
    #[inline]
    pub fn run(&self, x: f32) -> f32 {
        let magnitude = x.abs();
        if magnitude <= self.knee {
            return x;
        }
        // Map [knee, ∞) onto [knee, CEILING) with a curve that starts at the
        // knee with slope 1 and flattens from there — tanh, scaled to land on
        // the ceiling rather than on 1.0.
        let over = (magnitude - self.knee) / (CEILING - self.knee);
        let shaped = self.knee + (CEILING - self.knee) * over.tanh();
        shaped.copysign(x)
    }
}

/// Gain and tone, applied to an interleaved buffer.
///
/// One filter per channel: sharing a single biquad across an interleaved stream
/// would feed the left channel's history into the right and produce a filter
/// running at half the intended frequency on a mangled signal.
#[derive(Debug)]
pub struct Processor {
    gain: f32,
    filters: Vec<Biquad>,
    limiter: Limiter,
    /// Whether the shelf does anything, so it can be skipped entirely at 0 dB.
    shelving: bool,
}

/// Where the bass shelf sits, in Hz.
///
/// Low enough to leave voices alone — a shelf up at 250 Hz makes everything
/// boomy and muddy — and high enough to cover the fundamentals of a kick and a
/// bass guitar.
pub const BASS_HZ: f32 = 120.0;

impl Processor {
    pub fn new(sample_rate: u32, channels: usize, gain: f32, bass_db: f32) -> Self {
        let shelving = bass_db.abs() > 0.01;
        let filters = if shelving {
            vec![Biquad::low_shelf(sample_rate as f32, BASS_HZ, bass_db); channels.max(1)]
        } else {
            Vec::new()
        };
        Self { gain, filters, limiter: Limiter::default(), shelving }
    }

    /// Process interleaved frames in place.
    pub fn run(&mut self, samples: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        for (i, sample) in samples.iter_mut().enumerate() {
            let mut value = *sample;
            if self.shelving && let Some(filter) = self.filters.get_mut(i % channels) {
                value = filter.run(value);
            }
            *sample = self.limiter.run(value * self.gain);
        }
    }

    pub fn reset(&mut self) {
        for filter in &mut self.filters {
            filter.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steady-state amplitude of a sine at `freq` after passing through.
    fn response(filter: &mut Biquad, sample_rate: f32, freq: f32) -> f32 {
        let total = (sample_rate * 2.0) as usize;
        let mut peak: f32 = 0.0;
        for n in 0..total {
            let x = (2.0 * PI * freq * n as f32 / sample_rate).sin();
            let y = filter.run(x);
            // Ignore the first half: the filter has to settle first.
            if n > total / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn the_bass_shelf_lifts_bass_and_leaves_treble_alone() {
        let rate = 48_000.0;
        let mut filter = Biquad::low_shelf(rate, BASS_HZ, 9.0);
        let low = response(&mut filter, rate, 40.0);
        filter.reset();
        let high = response(&mut filter, rate, 6_000.0);

        // 9 dB is a factor of ~2.82; well below the corner it should be close.
        assert!(low > 2.5, "40 Hz should be lifted, got {low}");
        // Above the shelf the signal must come out as it went in. This is the
        // difference between bass boost and simply turning everything up.
        assert!((high - 1.0).abs() < 0.05, "6 kHz should be untouched, got {high}");
    }

    #[test]
    fn a_zero_db_shelf_is_a_wire() {
        let rate = 48_000.0;
        let mut filter = Biquad::low_shelf(rate, BASS_HZ, 0.0);
        for freq in [30.0, 120.0, 1_000.0, 10_000.0] {
            let out = response(&mut filter, rate, freq);
            assert!((out - 1.0).abs() < 0.01, "{freq} Hz changed at 0 dB: {out}");
            filter.reset();
        }
    }

    #[test]
    fn the_filter_stays_stable_at_the_extremes() {
        for gain_db in [-12.0, 0.0, 6.0, 12.0, 24.0] {
            let mut filter = Biquad::low_shelf(48_000.0, BASS_HZ, gain_db);
            let mut last = 0.0;
            for n in 0..48_000 {
                last = filter.run((2.0 * PI * 50.0 * n as f32 / 48_000.0).sin());
                assert!(last.is_finite(), "{gain_db} dB produced {last}");
            }
            // A filter that has gone unstable grows without bound rather than
            // settling; anything this side of absurd is fine.
            assert!(last.abs() < 100.0, "{gain_db} dB ran away to {last}");
        }
    }

    #[test]
    fn the_limiter_never_exceeds_the_ceiling() {
        let limiter = Limiter::default();
        for raw in [-40.0, -3.0, -1.0, -0.5, 0.0, 0.5, 1.0, 3.0, 40.0f32] {
            let out = limiter.run(raw);
            assert!(out.abs() <= CEILING + 1e-6, "{raw} became {out}");
            // Sign is the waveform; inverting it would be a different sound.
            assert!(raw == 0.0 || out.signum() == raw.signum());
        }
    }

    #[test]
    fn quiet_audio_passes_through_the_limiter_untouched() {
        let limiter = Limiter::default();
        for raw in [0.0, 0.1, 0.3, 0.6f32] {
            assert_eq!(limiter.run(raw), raw, "the limiter must not colour quiet audio");
        }
    }

    #[test]
    fn boosting_raises_level_without_clipping() {
        let mut processor = Processor::new(48_000, 2, 2.5, 0.0);
        let mut samples: Vec<f32> = (0..960)
            .map(|n| 0.35 * (2.0 * PI * 440.0 * n as f32 / 48_000.0).sin())
            .collect();
        let before = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));

        processor.run(&mut samples, 2);

        let after = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(after > before * 2.0, "2.5x should be clearly louder: {before} → {after}");
        assert!(after <= CEILING + 1e-6, "output must stay under the ceiling, got {after}");
    }

    #[test]
    fn an_empty_buffer_is_not_a_special_case() {
        // WASAPI hands over zero-frame packets during a stream transition, and
        // the render path passes whatever it got straight through.
        let mut processor = Processor::new(48_000, 2, 2.0, 6.0);
        let mut nothing: Vec<f32> = Vec::new();
        processor.run(&mut nothing, 2);
        assert!(nothing.is_empty());
    }

    #[test]
    fn a_partial_frame_does_not_panic() {
        // Defensive: an odd sample count against a stereo layout should be
        // processed, not indexed off the end of the filter list.
        let mut processor = Processor::new(48_000, 2, 1.5, 3.0);
        let mut odd = vec![0.2f32; 7];
        processor.run(&mut odd, 2);
        assert!(odd.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn a_zero_channel_count_is_treated_as_mono() {
        // `channels` comes from a device mix format; a zero would divide by zero
        // in the channel rotation.
        let mut processor = Processor::new(48_000, 0, 2.0, 0.0);
        let mut block = vec![0.1f32; 8];
        processor.run(&mut block, 0);
        assert!(block.iter().all(|s| (*s - 0.2).abs() < 1e-6));
    }

    #[test]
    fn silence_in_is_silence_out_at_any_gain() {
        // A shelf with a non-zero history could otherwise ring on into a gap.
        let mut processor = Processor::new(48_000, 2, 3.0, 12.0);
        let mut block = vec![0.0f32; 480];
        processor.run(&mut block, 2);
        assert!(block.iter().all(|s| *s == 0.0), "boost invented sound from silence");
    }

    #[test]
    fn extreme_input_stays_bounded_through_the_whole_chain() {
        // Full-scale square wave at maximum gain and maximum bass: the worst
        // case a real signal can present, and the output still has to be audio.
        let mut processor = Processor::new(48_000, 2, 3.0, 12.0);
        let mut block: Vec<f32> =
            (0..4800).map(|n| if (n / 24) % 2 == 0 { 1.0 } else { -1.0 }).collect();
        processor.run(&mut block, 2);
        assert!(block.iter().all(|s| s.is_finite() && s.abs() <= CEILING + 1e-6));
    }

    #[test]
    fn resetting_clears_the_filter_history() {
        let mut processor = Processor::new(48_000, 1, 1.0, 12.0);
        let mut loud = vec![0.9f32; 128];
        processor.run(&mut loud, 1);
        processor.reset();

        let mut quiet = vec![0.0f32; 128];
        processor.run(&mut quiet, 1);
        assert!(quiet.iter().all(|s| *s == 0.0), "the previous block rang into the next");
    }

    #[test]
    fn a_low_sample_rate_still_produces_a_stable_filter() {
        // 8 kHz puts 120 Hz much closer to Nyquist; the coefficients must still
        // behave rather than blowing up.
        for rate in [8_000, 22_050, 44_100, 48_000, 96_000, 192_000] {
            let mut filter = Biquad::low_shelf(rate as f32, BASS_HZ, 9.0);
            let mut last = 0.0;
            for n in 0..rate {
                last = filter.run((2.0 * PI * 60.0 * n as f32 / rate as f32).sin());
                assert!(last.is_finite(), "{rate} Hz produced {last}");
            }
            assert!(last.abs() < 100.0, "{rate} Hz ran away to {last}");
        }
    }

    #[test]
    fn channels_are_filtered_independently() {
        // Silence on the right must stay silent no matter what the left does —
        // one shared filter would bleed one into the other.
        let mut processor = Processor::new(48_000, 2, 1.0, 12.0);
        let mut samples = vec![0.0f32; 480];
        for (i, sample) in samples.iter_mut().enumerate() {
            if i % 2 == 0 {
                *sample = (2.0 * PI * 60.0 * (i / 2) as f32 / 48_000.0).sin();
            }
        }

        processor.run(&mut samples, 2);

        for (i, sample) in samples.iter().enumerate() {
            if i % 2 == 1 {
                assert_eq!(*sample, 0.0, "channel bleed at index {i}");
            }
        }
    }
}
