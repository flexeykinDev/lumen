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
    let mut offset = 0.0;

    for raw in lrc.lines() {
        let mut rest = raw.trim();
        let mut stamps: Vec<f64> = Vec::new();

        // Consume every leading [..] group; the last one is followed by text.
        // `close` is the index of `]` *within the text after `[`*, so in `rest`
        // the bracket content is `1..=close` and the next tag starts at
        // `close + 2`. Both offsets are byte indices, which is safe because the
        // brackets themselves are ASCII even when the content is not.
        while let Some(close) = rest.strip_prefix('[').and_then(|r| r.find(']')) {
            let tag = &rest[1..=close];
            match timestamp(tag) {
                Some(sec) => stamps.push(sec),
                // `[offset:...]` is the one metadata tag that changes the
                // timing rather than describing the song. Files that need it
                // are files whose timestamps are known to be wrong by a fixed
                // amount, so ignoring it is a guaranteed constant drift — the
                // exact symptom of lyrics that run early or late all the way
                // through a track.
                None if offset_tag(tag).is_some() => {
                    offset = offset_tag(tag).unwrap_or(0.0);
                    break;
                }
                // Any other metadata tag, such as `[ar:Artist]`. Not a
                // timestamp and not a lyric — stop reading tags and let the
                // rest be text.
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

    // Applied after the whole document is read, because the tag is allowed to
    // appear anywhere in it — including after the lines it corrects.
    if offset != 0.0 {
        for line in &mut out {
            line.at_sec = (line.at_sec - offset).max(0.0);
        }
    }

    out.sort_by(|a, b| a.at_sec.partial_cmp(&b.at_sec).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// `[offset:-500]` in milliseconds, as seconds to *subtract* from every stamp.
///
/// The sign follows the LRC convention rather than intuition: a positive offset
/// shifts the lyrics earlier, so it comes off the timestamps.
fn offset_tag(tag: &str) -> Option<f64> {
    let (key, value) = tag.split_once(':')?;
    if !key.trim().eq_ignore_ascii_case("offset") {
        return None;
    }
    let ms: f64 = value.trim().trim_start_matches('+').parse().ok()?;
    // A file claiming a minute of correction is a broken file, not a hint.
    (ms.abs() <= 60_000.0).then_some(ms / 1000.0)
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

    #[test]
    fn an_offset_tag_shifts_every_line() {
        // Positive shifts the words earlier, which is the LRC convention and
        // the opposite of what the sign looks like it should do.
        let lines = parse("[offset:+500]\n[00:10.00]a\n[00:20.00]b");
        assert_eq!(lines[0].at_sec, 9.5);
        assert_eq!(lines[1].at_sec, 19.5);

        let later = parse("[offset:-750]\n[00:10.00]a");
        assert_eq!(later[0].at_sec, 10.75);
    }

    #[test]
    fn an_offset_tag_after_the_lines_still_applies() {
        // The tag is allowed anywhere in the file, so it cannot be applied as
        // the lines are read.
        let lines = parse("[00:10.00]a\n[offset:+500]\n[00:20.00]b");
        assert_eq!(lines[0].at_sec, 9.5);
        assert_eq!(lines[1].at_sec, 19.5);
    }

    #[test]
    fn a_nonsense_offset_is_ignored() {
        // Neither a number nor a plausible correction; both would otherwise
        // move every line somewhere useless.
        assert_eq!(parse("[offset:soon]\n[00:10.00]a")[0].at_sec, 10.0);
        assert_eq!(parse("[offset:900000]\n[00:10.00]a")[0].at_sec, 10.0);
        // And a line may never be dragged before the start of the track.
        assert_eq!(parse("[offset:+5000]\n[00:01.00]a")[0].at_sec, 0.0);
    }

    #[test]
    fn hostile_input_produces_no_lines_rather_than_a_panic() {
        // Community-written files, fetched over the network. Every one of these
        // has to come back as "no lyrics" rather than take the app down.
        for input in [
            "",
            "


",
            "[",
            "[]",
            "[[[[[[",
            "[00:",
            "[99999999999999:00.00]x",
            "[-5:00.00]x",
            "[00:99.00]x",
            "[aa:bb.cc]x",
            "no timestamps at all",
            "[ti:Only metadata]",
        ] {
            let lines = parse(input);
            assert!(
                lines.iter().all(|l| l.at_sec.is_finite() && l.at_sec >= 0.0),
                "{input:?} produced a nonsense timestamp: {lines:?}"
            );
        }
    }

    #[test]
    fn a_multi_byte_line_is_split_on_character_boundaries() {
        // Byte indices into a string with Cyrillic or emoji in it will panic if
        // the arithmetic is wrong, and lyrics are full of both.
        let lines = parse("[00:12.00]Привет, мир 🎧
[00:15.00]日本語のテキスト");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Привет, мир 🎧");
        assert_eq!(lines[1].text, "日本語のテキスト");
    }

    #[test]
    fn a_very_long_document_is_still_ordered() {
        // Shuffled input, because LRC files are not required to be sorted and a
        // binary search over unsorted lines silently shows the wrong words.
        let mut doc = String::new();
        for n in (0..500).rev() {
            doc.push_str(&format!("[{:02}:{:02}.00]line {n}
", n / 60, n % 60));
        }
        let lines = parse(&doc);
        assert_eq!(lines.len(), 500);
        assert!(lines.windows(2).all(|w| w[0].at_sec <= w[1].at_sec), "output is not ascending");
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
