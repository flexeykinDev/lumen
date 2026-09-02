//! Estimated timings for lyrics that arrived without any.
//!
//! # What this is, and what it is not
//!
//! Plain lyrics have no timestamps, so the only thing available is the track
//! duration and a count of lines. Spreading the lines evenly across the song is
//! a guess, and on real music it is a poor one: verses are dense, choruses
//! repeat, instrumental breaks are silent, and nothing about a line's length
//! tells you how long it is sung for.
//!
//! It is still worth doing — an approximately-scrolling lyric beats a blank
//! panel — but the result is labelled `estimated` all the way to the UI so it
//! can be shown with less confidence than a real `.lrc`. Presenting a guess with
//! the same authority as a measurement is the actual failure mode here.

use super::lrc::Line;

/// Fraction of the track assumed to be intro before the first line.
///
/// Songs open with a bar or two of music far more often than they open on a
/// vocal, and starting the first line at 0:00 makes the whole lyric run early
/// for its entire length.
const INTRO: f64 = 0.06;
/// Fraction assumed to be outro after the last line.
const OUTRO: f64 = 0.08;

/// A section header such as `[Chorus]` or `[Verse 2]`.
///
/// These are annotations, not words anyone sings. Timing them as lines both
/// shows markup on screen and steals time from the lines around them.
pub fn is_section_header(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 2 && t.starts_with('[') && t.ends_with(']')
}

/// Weight a line by roughly how long it takes to sing.
///
/// Equal spacing treats "Yeah" and a fourteen-word verse line as the same
/// length, which is wrong in a way that accumulates: every short line hands time
/// to the next one, and by the middle of a song the words are visibly behind.
/// Character count is a crude proxy for syllables but a far better one than
/// nothing, and the floor keeps a one-word ad-lib from collapsing to no time at
/// all.
fn weight(line: &str) -> f64 {
    (line.chars().count() as f64).max(6.0)
}

/// Spread `lines` across a track of `duration_sec`, in proportion to length.
///
/// Blank lines and section headers are dropped first, so the spacing is
/// computed over the lines that are actually sung.
///
/// `offset_sec` shifts everything, for dialling in a lyric that runs early or
/// late. Estimated timings drift by their nature; a nudge is the only honest fix
/// available without real per-line data.
pub fn distribute(text: &str, duration_sec: f64, offset_sec: f64) -> Vec<Line> {
    let sung: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !is_section_header(l))
        .collect();

    if sung.is_empty() {
        return Vec::new();
    }

    // Without a duration there is nothing to spread across. Returning nothing is
    // right: a lyric pinned at 0:00 would flash the whole song at once.
    if !(duration_sec.is_finite() && duration_sec > 1.0) {
        return Vec::new();
    }

    let start = duration_sec * INTRO;
    let end = duration_sec * (1.0 - OUTRO);
    let span = (end - start).max(0.0);

    // One line has no interval to divide; put it where the vocal probably is.
    if sung.len() == 1 {
        return vec![Line { at_sec: (start + offset_sec).max(0.0), text: sung[0].to_owned() }];
    }

    // Each line begins where the previous one's share of the time ends, so the
    // last line starts at the end of the span and runs to the outro.
    let total: f64 = sung.iter().map(|l| weight(l)).sum();
    let mut elapsed = 0.0;
    sung.iter()
        .map(|text| {
            let at = start + span * (elapsed / total) + offset_sec;
            elapsed += weight(text);
            Line { at_sec: at.max(0.0), text: (*text).to_owned() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each line *starts* where the previous one's share of the time ends, so
    /// every line has a duration to sweep across. The last one therefore starts
    /// before the end of the span and runs on into the outro.
    #[test]
    fn spreads_lines_across_the_middle_of_the_track() {
        let lines = distribute("one\ntwo\nthree", 100.0, 0.0);
        assert_eq!(lines.len(), 3);
        // Begins after the intro.
        assert!((lines[0].at_sec - 6.0).abs() < 0.001, "{:?}", lines[0]);
        // These three are all below the length floor, so their weights are
        // equal and the spacing stays uniform.
        let a = lines[1].at_sec - lines[0].at_sec;
        let b = lines[2].at_sec - lines[1].at_sec;
        assert!((a - b).abs() < 0.001, "{a} vs {b}");
        assert!(lines[2].at_sec < 92.0, "last line must leave room to be sung");
    }

    /// The reason weighting exists: a long line takes longer to sing than a
    /// one-word ad-lib, and treating them equally makes the lyric drift further
    /// behind with every short line.
    #[test]
    fn longer_lines_are_given_more_time() {
        let lines = distribute(
            "hey\nthis line is considerably longer than the others\nbye",
            300.0,
            0.0,
        );
        assert_eq!(lines.len(), 3);
        let short_first = lines[1].at_sec - lines[0].at_sec;
        let long_middle = lines[2].at_sec - lines[1].at_sec;
        assert!(
            long_middle > short_first * 2.0,
            "long line got {long_middle}s, short line got {short_first}s"
        );
    }

    /// Estimated timings drift; the offset is the only correction available
    /// without real per-line data.
    #[test]
    fn offset_shifts_every_line() {
        let base = distribute("one\ntwo\nthree", 100.0, 0.0);
        let late = distribute("one\ntwo\nthree", 100.0, 2.5);
        for (b, l) in base.iter().zip(late.iter()) {
            assert!((l.at_sec - b.at_sec - 2.5).abs() < 0.001);
        }
    }

    /// A negative offset larger than the intro must not produce a time before
    /// the track starts.
    #[test]
    fn offset_never_goes_negative() {
        let lines = distribute("one\ntwo", 100.0, -30.0);
        assert!(lines.iter().all(|l| l.at_sec >= 0.0), "{lines:?}");
    }

    /// Section markers are annotations, not sung words. Timing them puts
    /// "[Chorus]" on screen and steals a slot from a real line.
    #[test]
    fn drops_section_headers_and_blanks() {
        let lines = distribute("[Verse 1]\none\n\n[Chorus]\ntwo\n", 60.0, 0.0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "one");
        assert_eq!(lines[1].text, "two");
    }

    #[test]
    fn timings_ascend() {
        let text = (1..=20).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
        let lines = distribute(&text, 210.0, 0.0);
        assert_eq!(lines.len(), 20);
        for pair in lines.windows(2) {
            assert!(pair[1].at_sec > pair[0].at_sec, "not ascending at {pair:?}");
        }
        assert!(lines.last().unwrap().at_sec < 210.0);
    }

    /// A live stream reports no duration, and a lyric with every line at zero
    /// would flash the entire song at once.
    #[test]
    fn refuses_without_a_usable_duration() {
        assert!(distribute("one\ntwo", 0.0, 0.0).is_empty());
        assert!(distribute("one\ntwo", f64::NAN, 0.0).is_empty());
        assert!(distribute("one\ntwo", -5.0, 0.0).is_empty());
    }

    #[test]
    fn single_line_is_placed_not_divided() {
        let lines = distribute("only", 100.0, 0.0);
        assert_eq!(lines.len(), 1);
        assert!((lines[0].at_sec - 6.0).abs() < 0.001);
    }

    #[test]
    fn nothing_sung_produces_nothing() {
        assert!(distribute("", 100.0, 0.0).is_empty());
        assert!(distribute("[Intro]\n\n[Outro]\n", 100.0, 0.0).is_empty());
    }

    #[test]
    fn recognises_section_headers() {
        assert!(is_section_header("[Chorus]"));
        assert!(is_section_header("  [Verse 2: Artist]  "));
        assert!(!is_section_header("[not closed"));
        assert!(!is_section_header("a [Chorus] b"));
        assert!(!is_section_header(""));
    }
}
