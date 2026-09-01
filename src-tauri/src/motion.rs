//! The single easing curve shared by the host and the renderer.
//!
//! MUST stay numerically identical to `src/lib/motion.ts`. The window region is
//! animated here in Rust while the panel contents are animated by CSS in the
//! WebView; the two never share a clock, only this curve. See ARCHITECTURE.md §4.

/// easeOutQuint-like control points: fast out, long settle.
pub const EASE: [f64; 4] = [0.22, 1.0, 0.36, 1.0];

/// A cubic-bezier timing curve, in CSS's `cubic-bezier(x1, y1, x2, y2)` terms.
#[derive(Debug, Clone, Copy)]
pub struct Curve {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// The shared curve used by every state transition. Mirrored in `motion.ts`.
pub const SHARED: Curve = Curve { x1: EASE[0], y1: EASE[1], x2: EASE[2], y2: EASE[3] };

/// Momentum decay, for a flick that has to continue an existing motion rather
/// than start from rest. Its very steep initial slope (y1/x1 = 10) is the point:
/// the glide begins fast and decays, so it can be matched to the speed the
/// pointer was already travelling at.
pub const MOMENTUM: Curve = Curve { x1: 0.1, y1: 1.0, x2: 0.2, y2: 1.0 };

impl Curve {
    /// Slope as the curve leaves the origin, i.e. its initial speed relative to
    /// `distance / duration`. Used to pick a duration whose opening velocity
    /// matches the flick that triggered it.
    pub fn initial_slope(self) -> f64 {
        if self.x1.abs() < f64::EPSILON { 1.0 } else { self.y1 / self.x1 }
    }
}

pub mod duration {
    use std::time::Duration;

    pub const EXPAND: Duration = Duration::from_millis(340);
    pub const COLLAPSE: Duration = Duration::from_millis(280);
    pub const REVEAL: Duration = Duration::from_millis(260);
    pub const CONCEAL: Duration = Duration::from_millis(220);
    /// Magnetic snap after a drag. Longer than a state transition: the capsule
    /// travels much further, and a fast glide over that distance reads as a jump.
    pub const DOCK_SNAP: Duration = Duration::from_millis(450);
}

/// Frame budget for the region animator. 60 Hz; DWM will coalesce past that.
pub const FRAME: std::time::Duration = std::time::Duration::from_micros(16_667);

fn bezier_axis(t: f64, a1: f64, a2: f64) -> f64 {
    let u = 1.0 - t;
    3.0 * u * u * t * a1 + 3.0 * u * t * t * a2 + t * t * t
}

/// Evaluate the cubic-bezier at `x` in 0..=1, matching CSS `cubic-bezier()`.
///
/// Newton refinement over the parametric form, with a bisection fallback for the
/// flat regions where the derivative collapses.
pub fn ease(x: f64) -> f64 {
    ease_with(SHARED, x)
}

/// Evaluate an arbitrary cubic-bezier at `x` in 0..=1.
pub fn ease_with(curve: Curve, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }

    let (x1, y1, x2, y2) = (curve.x1, curve.y1, curve.x2, curve.y2);

    let mut t = x;
    for _ in 0..8 {
        let err = bezier_axis(t, x1, x2) - x;
        if err.abs() < 1e-6 {
            return bezier_axis(t, y1, y2);
        }
        let u = 1.0 - t;
        let d = 3.0 * u * u * x1 + 6.0 * u * t * (x2 - x1) + 3.0 * t * t * (1.0 - x2);
        if d.abs() < 1e-6 {
            break;
        }
        t -= err / d;
    }

    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    t = x;
    for _ in 0..24 {
        let v = bezier_axis(t, x1, x2);
        if (v - x).abs() < 1e-6 {
            break;
        }
        if v > x { hi = t } else { lo = t }
        t = (lo + hi) / 2.0;
    }
    bezier_axis(t, y1, y2)
}

#[inline]
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

#[inline]
pub fn lerp_i32(a: i32, b: i32, t: f64) -> i32 {
    lerp(a as f64, b as f64, t).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression net for host/renderer drift.
    ///
    /// These are cubic-bezier(0.22, 1, 0.36, 1) solved by 200-step bisection —
    /// an independent method from the Newton solver under test. `motion.ts`
    /// asserts the same table in `npm run check:motion`, so a change to either
    /// side that breaks host/renderer sync fails a test rather than showing up
    /// as a visible seam between the window shape and its contents.
    #[test]
    fn matches_reference_curve_at_sample_points() {
        const EXPECTED: [f64; 11] = [
            0.000_000_000,
            0.401_096_896,
            0.673_978_074,
            0.832_192_057,
            0.917_146_858,
            0.961_382_548,
            0.983_675_387,
            0.994_193_844,
            0.998_524_422,
            0.999_839_526,
            1.000_000_000,
        ];
        for (i, want) in EXPECTED.iter().enumerate() {
            let got = ease(i as f64 / 10.0);
            assert!(
                (got - want).abs() < 1e-4,
                "ease({}) = {got}, expected {want}",
                i as f64 / 10.0
            );
        }
    }

    #[test]
    fn is_monotonic_and_clamped() {
        let mut prev = -1.0;
        for i in 0..=200 {
            let v = ease(i as f64 / 200.0);
            assert!((0.0..=1.0).contains(&v), "out of range at {i}: {v}");
            assert!(v >= prev - 1e-9, "non-monotonic at {i}");
            prev = v;
        }
    }
}
