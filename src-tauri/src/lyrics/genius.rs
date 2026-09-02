//! Plain lyrics from Genius, for tracks LRCLIB has no timings for.
//!
//! # This is a scraper, and that matters
//!
//! Genius has no lyrics API. `/songs/:id` returns metadata only, and their API
//! terms prohibit redistributing lyrics through it — the words exist solely in
//! the page HTML. So this reads their website: it depends on markup that can
//! change without notice, and it will break sometimes. That is a property of
//! the approach, not a bug to be fixed later, which is why it is a separate
//! switch and why every failure here is a shrug rather than an error.
//!
//! The result never has real timings. `timing::distribute` guesses them and
//! marks them estimated, so nothing downstream mistakes a guess for a measurement.

use anyhow::Context;

use crate::net as http;

/// Lyrics live inside one or more of these on a Genius song page.
const CONTAINER: &str = "data-lyrics-container";

/// Find a song page path for `artist` and `title`, e.g. `/Artist-title-lyrics`.
pub fn find_song_path(artist: &str, title: &str) -> anyhow::Result<Option<String>> {
    let query = format!(
        "/api/search/multi?per_page=5&q={}",
        http::encode(&format!("{artist} {title}"))
    );
    let Some(body) = http::get("genius.com", &query)? else {
        return Ok(None);
    };

    let value: serde_json::Value = serde_json::from_slice(&body).context("search was not JSON")?;
    let sections = value
        .get("response")
        .and_then(|r| r.get("sections"))
        .and_then(|s| s.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();

    for section in sections {
        // Only the "song" section carries lyric pages; the others are artists,
        // albums and user-written annotations.
        if section.get("type").and_then(|t| t.as_str()) != Some("song") {
            continue;
        }
        let hits = section.get("hits").and_then(|h| h.as_array()).map(Vec::as_slice).unwrap_or_default();
        for hit in hits {
            if let Some(path) = hit.get("result").and_then(|r| r.get("path")).and_then(|p| p.as_str())
            {
                return Ok(Some(path.to_owned()));
            }
        }
    }
    Ok(None)
}

/// Fetch a song page and pull the lyric text out of it.
pub fn fetch_lyrics(path: &str) -> anyhow::Result<Option<String>> {
    let Some(body) = http::get("genius.com", path)? else {
        return Ok(None);
    };
    let html = String::from_utf8_lossy(&body);
    let text = extract(&html);
    Ok(if text.trim().is_empty() { None } else { Some(text) })
}

/// Pull the lyric text out of a Genius song page.
///
/// Deliberately not a real HTML parser: this looks for the lyric containers by
/// attribute and walks their tags. A DOM crate would be a large dependency for
/// one known shape of one page, and would not survive their markup changing any
/// better than this does.
pub fn extract(html: &str) -> String {
    let mut out = String::new();

    let mut rest = html;
    while let Some(at) = rest.find(CONTAINER) {
        // Step to the end of the opening tag, then take everything up to the
        // matching close. Nested divs are counted so a container holding markup
        // is not cut short at the first `</div>`.
        let after_attr = &rest[at..];
        let Some(open_end) = after_attr.find('>') else { break };
        let body_start = at + open_end + 1;
        let body = &rest[body_start..];

        let mut depth = 1usize;
        let mut cursor = 0usize;
        let mut end = body.len();
        while let Some(next) = body[cursor..].find("<div").map(|i| cursor + i).into_iter().chain(
            body[cursor..].find("</div").map(|i| cursor + i),
        ).min() {
            if body[next..].starts_with("<div") {
                depth += 1;
                cursor = next + 4;
            } else {
                depth -= 1;
                if depth == 0 {
                    end = next;
                    break;
                }
                cursor = next + 5;
            }
        }

        out.push_str(&strip_tags(&body[..end]));
        out.push('\n');
        rest = &body[end.min(body.len())..];
    }

    out
}

/// Turn a fragment of Genius lyric markup into plain text.
///
/// `<br>` is the line separator; everything else is inline formatting or links
/// to annotations and carries no meaning here.
fn strip_tags(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut rest = fragment;

    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        // A `<br>` in any of its spellings is a newline.
        let lower_head: String =
            tail.chars().take(4).flat_map(char::to_lowercase).collect();
        if lower_head.starts_with("<br") {
            out.push('\n');
        }
        match tail.find('>') {
            Some(close) => rest = &tail[close + 1..],
            // Truncated markup: keep what is left rather than losing the verse.
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);

    decode_entities(&out)
}

/// Decode the handful of entities Genius actually emits.
fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
<html><body>
<div class="Lyrics__Container" data-lyrics-container="true">[Verse 1]<br/>
First line here<br/>
Second &amp; line<br/></div>
<div data-lyrics-container="true">[Chorus]<br/><a href="/x"><span>Third line</span></a><br/>
Fourth line&#x27;s words<br/></div>
</body></html>
"#;

    #[test]
    fn extracts_lines_from_every_container() {
        let text = extract(PAGE);
        let lines: Vec<&str> =
            text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        assert_eq!(
            lines,
            vec![
                "[Verse 1]",
                "First line here",
                "Second & line",
                "[Chorus]",
                "Third line",
                "Fourth line's words",
            ]
        );
    }

    /// Lyric lines are wrapped in links and spans for annotations; the words are
    /// what matters, the markup is not.
    #[test]
    fn strips_inline_markup_but_keeps_its_text() {
        assert_eq!(strip_tags("<a href=\"/x\"><b>hi</b></a>"), "hi");
        assert_eq!(strip_tags("plain"), "plain");
    }

    #[test]
    fn treats_every_spelling_of_br_as_a_newline() {
        for br in ["<br>", "<br/>", "<br />", "<BR>"] {
            assert_eq!(strip_tags(&format!("a{br}b")), "a\nb", "failed for {br}");
        }
    }

    #[test]
    fn decodes_the_entities_genius_emits() {
        assert_eq!(decode_entities("a &amp; b"), "a & b");
        assert_eq!(decode_entities("don&#x27;t"), "don't");
        assert_eq!(decode_entities("don&#39;t"), "don't");
        assert_eq!(decode_entities("&quot;q&quot;"), "\"q\"");
        assert_eq!(decode_entities("&lt;tag&gt;"), "<tag>");
    }

    /// A container holding nested divs must not be cut off at the first close.
    #[test]
    fn handles_nested_divs_inside_a_container() {
        let html = r#"<div data-lyrics-container="true">one<br/><div class="x">two</div><br/>three</div>"#;
        let text = extract(html);
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        // The nested `</div>` must not end the container, and the `<br/>` after
        // it still separates the lines around it.
        assert_eq!(lines, vec!["one", "two", "three"]);
    }

    /// The page shape will change one day. That must produce nothing, not a
    /// panic and not a page of navigation text.
    #[test]
    fn unknown_markup_yields_nothing() {
        assert!(extract("<html><body>no lyrics here</body></html>").trim().is_empty());
        assert!(extract("").trim().is_empty());
    }

    /// Truncated HTML is a real outcome of a capped read.
    #[test]
    fn survives_an_unclosed_tag() {
        assert_eq!(strip_tags("good <span"), "good ");
    }

    /// Hits the real site, so it is not part of the normal run. It is also the
    /// test most likely to start failing without anything here changing — that
    /// is what scraping someone else's markup means, and is exactly why it is
    /// worth being able to check on demand:
    ///
    /// ```text
    /// cargo test -- --ignored genius
    /// ```
    #[test]
    #[ignore = "requires network access to genius.com"]
    fn live_lookup_finds_and_extracts_lyrics() {
        let path = find_song_path("Radiohead", "Creep")
            .expect("search must not error")
            .expect("genius should know this song");
        assert!(path.contains("Creep") || path.contains("creep"), "unexpected path {path}");

        let text = fetch_lyrics(&path)
            .expect("page fetch must not error")
            .expect("the page should carry lyrics");
        let sung: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !super::super::timing::is_section_header(l))
            .collect();
        assert!(sung.len() > 10, "expected a full lyric, got {} line(s)", sung.len());
        // Markup must not survive extraction.
        assert!(!text.contains('<'), "tags leaked into the lyric");
    }
}
