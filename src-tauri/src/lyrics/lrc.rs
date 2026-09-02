//! Parsing the LRC format.
//!
//! A line is one or more timestamps followed by text:
//!
//! ```text
//! [00:12.34]Some words
//! [00:20.10][01:04.55]A repeated chorus line
//! [ar:Artist]            <- metadata, not a lyric
//! ```
//!
//! Repeated timestamps on one line are how choruses are written, so each one
//! produces its own entry. Metadata tags look identical to a timestamp at a
//! glance and are the main thing a naive parser gets wrong: `[ar:Artist]` would
//! otherwise become a lyric at some arbitrary time.

use serde::Serialize;

/// One lyric line, at the moment it should appear.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub at_sec: f64,
    pub text: String,
}

/// Parse an LRC document into timed lines, ascending.
///
/// Anything unparseable is skipped rather than failing the document: these files
/// are community-written and a single malformed line should not cost the track
/// its lyrics.
pub fn parse(lrc: &str) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();

    for raw in lrc.lines() {
        let mut rest = raw.trim();
        let mut stamps: Vec<f64> = Vec::new();

        // Consume every leading [..] group; the last one is followed by text.
        // `close` is the index of `]` *within the text after `[`*, so in `rest`
        // the bracket content is `1..=close` and the next tag starts at
        // `close + 2`. Both offsets are byte indices, which is safe because the
        // brackets themselves are ASCII even when the content is not.
        while let Some(close) = rest.strip_prefix('[').and_then(|r| r.find(']')) {
            match timestamp(&rest[1..=close]) {
                Some(sec) => stamps.push(sec),
                // A metadata tag such as `[ar:Artist]`. Not a timestamp, and not
                // a lyric either — stop reading tags and let the rest be text.
                None => break,
            }
            rest = rest[close + 2..].trim_start();
        }

        if stamps.is_empty() {
            continue;
        }
        let text = rest.trim().to_owned();
        // Blank timed lines are real: they mark instrumental gaps, and dropping
        // them would leave the previous line on screen through the whole break.
        for at_sec in stamps {
            out.push(Line { at_sec, text: text.clone() });
        }
    }

    out.sort_by(|a, b| a.at_sec.partial_cmp(&b.at_sec).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// `mm:ss`, `mm:ss.xx` or `mm:ss.xxx` to seconds.
fn timestamp(text: &str) -> Option<f64> {
    let (minutes, rest) = text.split_once(':')?;
    let minutes: f64 = minutes.trim().parse().ok()?;
    // Only fractions of a second may follow; `ti:Some Title` must not parse.
    let seconds: f64 = rest.trim().parse().ok()?;
    if !(0.0..60.0).contains(&seconds) || minutes < 0.0 {
        return None;
    }
    Some(minutes * 60.0 + seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_timed_lines() {
        let lines = parse("[00:12.34]hello\n[01:02.50]world");
        assert_eq!(lines.len(), 2);
        assert!((lines[0].at_sec - 12.34).abs() < 0.001);
        assert_eq!(lines[0].text, "hello");
        assert!((lines[1].at_sec - 62.50).abs() < 0.001);
    }

    /// How choruses are written: one line, several times.
    #[test]
    fn expands_repeated_timestamps() {
        let lines = parse("[00:10.00][01:10.00][02:10.00]chorus");
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| l.text == "chorus"));
        assert!((lines[2].at_sec - 130.0).abs() < 0.001);
    }

    /// The trap: metadata tags are bracketed exactly like timestamps, and a
    /// naive parser turns the artist's name into a lyric.
    #[test]
    fn ignores_metadata_tags() {
        let lines = parse("[ar:Some Artist]\n[ti:Some Title]\n[00:05.00]real line");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "real line");
    }

    /// Instrumental gaps are marked with an empty timed line; dropping them
    /// leaves the previous line on screen through the whole break.
    #[test]
    fn keeps_blank_timed_lines() {
        let lines = parse("[00:01.00]words\n[00:30.00]\n[01:00.00]more");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].text, "");
    }

    #[test]
    fn sorts_by_time() {
        let lines = parse("[02:00.00]late\n[00:30.00]early");
        assert_eq!(lines[0].text, "early");
        assert_eq!(lines[1].text, "late");
    }

    #[test]
    fn accepts_two_and_three_digit_fractions() {
        assert!((parse("[00:01.5]x")[0].at_sec - 1.5).abs() < 0.001);
        assert!((parse("[00:01.250]x")[0].at_sec - 1.25).abs() < 0.001);
        assert!((parse("[00:01]x")[0].at_sec - 1.0).abs() < 0.001);
    }

    /// Community files contain junk; one bad line must not cost the whole track.
    #[test]
    fn skips_unparseable_lines_without_losing_the_rest() {
        let lines = parse("not a lyric\n[bad]\n[00:09.00]good");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "good");
    }

    #[test]
    fn rejects_impossible_times() {
        assert!(parse("[00:75.00]x").is_empty(), "seconds must be under 60");
        assert!(parse("[-1:10.00]x").is_empty(), "minutes must not be negative");
    }

    #[test]
    fn empty_input_is_empty_output() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n").is_empty());
    }
}
