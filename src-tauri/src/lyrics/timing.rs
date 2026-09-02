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

/// Spread `lines` evenly across a track of `duration_sec`.
///
/// Blank lines and section headers are dropped first, so the spacing is
/// computed over the lines that are actually sung.
pub fn distribute(text: &str, duration_sec: f64) -> Vec<Line> {
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
        return vec![Line { at_sec: start, text: sung[0].to_owned() }];
    }

    let step = span / (sung.len() - 1) as f64;
    sung.iter()
        .enumerate()
        .map(|(i, text)| Line { at_sec: start + step * i as f64, text: (*text).to_owned() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreads_lines_across_the_middle_of_the_track() {
        let lines = distribute("one\ntwo\nthree", 100.0);
        assert_eq!(lines.len(), 3);
        // Starts after the intro, ends before the outro.
        assert!((lines[0].at_sec - 6.0).abs() < 0.001, "{:?}", lines[0]);
        assert!((lines[2].at_sec - 92.0).abs() < 0.001, "{:?}", lines[2]);
        // Evenly spaced.
        let a = lines[1].at_sec - lines[0].at_sec;
        let b = lines[2].at_sec - lines[1].at_sec;
        assert!((a - b).abs() < 0.001);
    }

    /// Section markers are annotations, not sung words. Timing them puts
    /// "[Chorus]" on screen and steals a slot from a real line.
    #[test]
    fn drops_section_headers_and_blanks() {
        let lines = distribute("[Verse 1]\none\n\n[Chorus]\ntwo\n", 60.0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "one");
        assert_eq!(lines[1].text, "two");
    }

    #[test]
    fn timings_ascend() {
        let text = (1..=20).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
        let lines = distribute(&text, 210.0);
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
        assert!(distribute("one\ntwo", 0.0).is_empty());
        assert!(distribute("one\ntwo", f64::NAN).is_empty());
        assert!(distribute("one\ntwo", -5.0).is_empty());
    }

    #[test]
    fn single_line_is_placed_not_divided() {
        let lines = distribute("only", 100.0);
        assert_eq!(lines.len(), 1);
        assert!((lines[0].at_sec - 6.0).abs() < 0.001);
    }

    #[test]
    fn nothing_sung_produces_nothing() {
        assert!(distribute("", 100.0).is_empty());
        assert!(distribute("[Intro]\n\n[Outro]\n", 100.0).is_empty());
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
