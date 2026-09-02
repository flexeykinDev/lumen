//! Album art → accent palette.
//!
//! Dominant-colour extraction alone produces mud: most album art is dominated by
//! near-black or near-white background, and a literal "most common colour" accent
//! looks broken. We quantize with MMCQ, then *score* the palette for vibrancy and
//! correct the winner in HSL so the result is always usable as a UI accent.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

/// An sRGB triple that travels over IPC as `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0[0], self.0[1], self.0[2])
    }

    /// WCAG relative luminance, used to pick a readable foreground.
    fn luminance(self) -> f64 {
        let f = |c: u8| {
            let c = c as f64 / 255.0;
            if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(self.0[0]) + 0.7152 * f(self.0[1]) + 0.0722 * f(self.0[2])
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let h = s.strip_prefix('#').unwrap_or(&s);
        if h.len() != 6 {
            return Err(D::Error::custom("expected #rrggbb"));
        }
        let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map_err(D::Error::custom);
        Ok(Rgb([p(0)?, p(2)?, p(4)?]))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accent {
    /// Vibrancy-corrected dominant colour. Drives borders, the progress fill and
    /// the play-state glyph.
    pub base: Rgb,
    /// Readable against `base` — used for text sitting *on* the accent.
    pub fg: Rgb,
    /// Darker, desaturated companion for the ambient bloom behind the artwork.
    pub glow: Rgb,
}

/// The accent used before any artwork arrives, and when extraction fails.
impl Default for Accent {
    fn default() -> Self {
        Self {
            base: Rgb([0x7A, 0x8C, 0xFF]),
            fg: Rgb([0x0B, 0x0D, 0x14]),
            glow: Rgb([0x2A, 0x30, 0x52]),
        }
    }
}

/// Longest edge we quantize at. MMCQ cost is linear in pixel count and the
/// palette is indistinguishable above ~96 px, so this keeps a track change
/// under ~4 ms instead of ~120 ms for a 1400 px cover.
const SAMPLE_EDGE: u32 = 96;

/// Decode `bytes`, quantize, and pick a UI-usable accent.
///
/// Returns `None` only when the image cannot be decoded at all; a decodable but
/// colourless image still yields a (grey-corrected) accent.
pub fn extract(bytes: &[u8]) -> Option<Accent> {
    let img = image::load_from_memory(bytes)
        .inspect_err(|e| tracing::debug!("album art decode failed: {e}"))
        .ok()?;

    let thumb = img.thumbnail(SAMPLE_EDGE, SAMPLE_EDGE).to_rgb8();
    let (w, h) = thumb.dimensions();
    if w == 0 || h == 0 {
        return None;
    }

    // quality=1 means "consider every pixel"; at 96x96 that is 9k samples.
    let palette = color_thief::get_palette(thumb.as_raw(), color_thief::ColorFormat::Rgb, 1, 8)
        .inspect_err(|e| tracing::debug!("quantization failed: {e:?}"))
        .ok()?;

    if palette.is_empty() {
        return None;
    }

    let winner = palette
        .iter()
        .enumerate()
        .max_by(|a, b| {
            score(a.1, a.0).partial_cmp(&score(b.1, b.0)).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, c)| Rgb([c.r, c.g, c.b]))?;

    Some(build(winner))
}

/// How suitable a quantized colour is as a UI accent.
///
/// Rewards saturation, punishes the extremes of lightness (near-black and
/// near-white read as "no accent"), and decays with palette rank so that an
/// equally vibrant but rarer colour does not beat the dominant one.
fn score(c: &color_thief::Color, rank: usize) -> f64 {
    let (_, s, l) = to_hsl(Rgb([c.r, c.g, c.b]));

    let lightness_fit = 1.0 - ((l - 0.55).abs() * 1.6).min(1.0);
    let rank_decay = 1.0 / (1.0 + rank as f64 * 0.35);

    // The floor keeps a monochrome cover from scoring zero across the board,
    // which would make the choice arbitrary.
    (0.08 + s * lightness_fit) * rank_decay
}

/// Correct the winning colour into a guaranteed-usable triple.
fn build(base: Rgb) -> Accent {
    let (h, s, l) = to_hsl(base);

    // Floor the saturation so grey covers still tint the UI instead of
    // producing an invisible accent; cap lightness so it never blows out.
    let s = s.clamp(0.52, 0.92);
    let l = l.clamp(0.46, 0.68);
    let base = from_hsl(h, s, l);

    let fg = if base.luminance() > 0.45 { Rgb([0x0B, 0x0D, 0x14]) } else { Rgb([0xF4, 0xF6, 0xFF]) };
    let glow = from_hsl(h, s * 0.78, (l * 0.42).clamp(0.10, 0.30));

    Accent { base, fg, glow }
}

fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (c.0[0] as f64 / 255.0, c.0[1] as f64 / 255.0, c.0[2] as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;

    if d.abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }

    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    if s <= f64::EPSILON {
        let v = (l * 255.0).round().clamp(0.0, 255.0) as u8;
        return Rgb([v, v, v]);
    }

    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0
        }
        if t > 1.0 {
            t -= 1.0
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    Rgb([f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_roundtrips_within_one_step() {
        for c in [Rgb([200, 30, 90]), Rgb([12, 200, 180]), Rgb([255, 255, 255]), Rgb([0, 0, 0])] {
            let (h, s, l) = to_hsl(c);
            let back = from_hsl(h, s, l);
            for i in 0..3 {
                assert!(
                    (back.0[i] as i16 - c.0[i] as i16).abs() <= 1,
                    "{c:?} -> {back:?} at channel {i}"
                );
            }
        }
    }

    #[test]
    fn corrects_extremes_into_a_usable_accent() {
        // Pure black and pure white must both come back visible and legible.
        for input in [Rgb([0, 0, 0]), Rgb([255, 255, 255])] {
            let a = build(input);
            let l = to_hsl(a.base).2;
            assert!((0.45..=0.69).contains(&l), "lightness {l} out of usable band for {input:?}");
            let contrast = (a.base.luminance() - a.fg.luminance()).abs();
            assert!(contrast > 0.15, "foreground not readable for {input:?}");
        }
    }

    /// A PNG that is hostile to naive dominant-colour extraction: 90% near-black
    /// with a small vivid patch. "Most common colour" returns near-black here,
    /// which is exactly the mud this module exists to avoid, and real album art
    /// has the same shape — a dark sleeve with one bright element.
    ///
    /// Built rather than loaded. This test used to read `assets/icon-source.png`
    /// and assert the result was violet, which quietly encoded the *artwork* into
    /// the test: replacing the app icon with one that has a warm landscape in it
    /// broke the assertion, though nothing about the extraction had changed.
    /// Constructing the image keeps the property under test independent of any
    /// asset someone may swap.
    fn hostile_png(patch: [u8; 3]) -> Vec<u8> {
        use image::{ImageEncoder, codecs::png::PngEncoder};

        let (w, h) = (64u32, 64u32);
        let mut pixels = vec![0u8; (w * h * 3) as usize];
        for (i, chunk) in pixels.chunks_exact_mut(3).enumerate() {
            let (x, y) = (i as u32 % w, i as u32 / w);
            // A 20x20 patch in the corner: about 10% of the image.
            chunk.copy_from_slice(if x < 20 && y < 20 { &patch } else { &[8, 8, 10] });
        }

        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(&pixels, w, h, image::ExtendedColorType::Rgb8)
            .expect("encoding a test image must succeed");
        out
    }

    #[test]
    fn extracts_a_vivid_accent_from_a_mostly_dark_image() {
        // Violet-blue, the classic "one bright element on a dark sleeve".
        let accent = extract(&hostile_png([124, 92, 220])).expect("the test image must decode");

        let (_, s, l) = to_hsl(accent.base);
        assert!(s >= 0.5, "accent came out desaturated: {accent:?} (s={s})");
        assert!(
            (0.45..=0.69).contains(&l),
            "accent escaped the usable lightness band: {accent:?} (l={l})"
        );

        // Near-grey or green would mean the scoring picked the background or a
        // compression artifact rather than the patch.
        let [r, g, b] = accent.base.0;
        assert!(b > g, "expected a blue-leaning accent, got {accent:?}");
        assert!(r > g, "expected a violet-leaning accent, got {accent:?}");

        // The glow must be clearly darker than the base or the bloom washes out.
        assert!(
            accent.glow.luminance() < accent.base.luminance(),
            "glow is not darker than base: {accent:?}"
        );
    }

    /// The same shape with a warm patch, so the test cannot pass by accident on
    /// something that always leans blue.
    #[test]
    fn follows_the_hue_of_the_vivid_patch() {
        let accent = extract(&hostile_png([220, 140, 60])).expect("the test image must decode");
        let [r, _g, b] = accent.base.0;
        assert!(r > b, "expected a warm accent, got {accent:?}");
        let (_, s, _) = to_hsl(accent.base);
        assert!(s >= 0.4, "warm accent came out desaturated: {accent:?}");
    }

    /// The real app icon still has to produce *something* usable, whatever it
    /// happens to depict. No hue assertion: that is the artwork's business.
    #[test]
    fn the_shipped_app_icon_yields_a_usable_accent() {
        let accent =
            extract(include_bytes!("../../../assets/icon-source.png")).expect("icon must decode");
        let (_, s, l) = to_hsl(accent.base);
        assert!(s >= 0.3, "app icon accent is nearly grey: {accent:?} (s={s})");
        assert!(
            (0.35..=0.75).contains(&l),
            "app icon accent escaped the usable band: {accent:?} (l={l})"
        );
        assert!(
            accent.glow.luminance() < accent.base.luminance(),
            "glow is not darker than base: {accent:?}"
        );
    }

    #[test]
    fn survives_garbage_input() {
        assert!(extract(b"not an image").is_none());
        assert!(extract(&[]).is_none());
    }

    #[test]
    fn prefers_vibrancy_over_rank() {
        let dull = color_thief::Color { r: 20, g: 20, b: 22 };
        let vivid = color_thief::Color { r: 220, g: 60, b: 120 };
        assert!(score(&vivid, 1) > score(&dull, 0), "a vivid rank-1 must beat a dead rank-0");
    }
}
