//! Is there a newer Lumen than this one?
//!
//! A plain text file in the repository, fetched once per launch, compared, and
//! shown in the About tab. Deliberately not an updater: nothing is downloaded,
//! nothing is replaced, and the answer is a sentence and a link. A portable exe
//! that rewrites itself is a different product with a different threat model.
//!
//! The version lives in `VERSION` at the repository root rather than in the
//! releases API, because the API needs a token to be reliable, rate-limits by
//! IP, and returns two kilobytes of JSON to answer a question that fits in six
//! bytes.

use serde::Serialize;

/// Where the published version number is kept.
const HOST: &str = "raw.githubusercontent.com";
const PATH: &str = "/flexeykinDev/lumen/master/VERSION";

/// Where a person goes when the answer is "yes".
pub const RELEASES: &str = "https://github.com/flexeykinDev/lumen/releases/latest";

/// The answer, as the settings window wants it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// What is running.
    pub current: String,
    /// What is published, when the check succeeded.
    pub latest: Option<String>,
    /// True only when `latest` is genuinely newer than `current`.
    pub newer: bool,
    /// Why the check produced nothing, for the one line the UI shows.
    pub error: Option<String>,
}

/// Ask the repository what the current release is.
///
/// Never returns an error: a failed update check is not a failure of anything
/// the user was doing, and an error dialog for "the network is down" is noise.
pub fn check() -> Status {
    let current = env!("CARGO_PKG_VERSION").to_owned();

    let latest = match crate::net::get(HOST, PATH) {
        Ok(Some(body)) => {
            let text = String::from_utf8_lossy(&body).trim().to_owned();
            // A version file that has become an HTML error page, a redirect, or
            // someone's stray note must not be shown as a version number.
            if is_version(&text) {
                Some(text)
            } else {
                return Status {
                    current,
                    latest: None,
                    newer: false,
                    error: Some("the published version could not be read".into()),
                };
            }
        }
        Ok(None) => {
            return Status {
                current,
                latest: None,
                newer: false,
                error: Some("no version file was found".into()),
            };
        }
        Err(e) => {
            tracing::debug!("update check failed: {e:#}");
            return Status {
                current,
                latest: None,
                newer: false,
                error: Some("could not reach GitHub".into()),
            };
        }
    };

    let newer = latest.as_deref().is_some_and(|l| is_newer(l, &current));
    Status { current, latest, newer, error: None }
}

/// A plausible `major.minor.patch`, and nothing else.
fn is_version(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 32
        && text.split('.').count() == 3
        && text.split('.').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

/// Whether `candidate` is a later version than `running`.
///
/// Compared field by field as numbers. String comparison would put 0.10.0
/// before 0.9.0, which is exactly the version where it would first matter.
fn is_newer(candidate: &str, running: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> { v.split('.').filter_map(|p| p.parse().ok()).collect() };
    let (a, b) = (parse(candidate), parse(running));
    if a.len() != 3 || b.len() != 3 {
        return false;
    }
    a > b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_version_is_newer() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.1", "0.1.0"));
    }

    #[test]
    fn the_same_or_older_is_not() {
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("0.9.9", "1.0.0"));
    }

    #[test]
    fn versions_compare_as_numbers_not_as_text() {
        // The comparison that string ordering gets wrong, and the first one
        // that will actually happen.
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.9.0", "0.10.0"));
        assert!(is_newer("0.2.10", "0.2.9"));
    }

    #[test]
    fn nonsense_is_never_newer() {
        for candidate in ["", "latest", "v0.2.0", "0.2", "0.2.0.1", "<!DOCTYPE html>"] {
            assert!(!is_newer(candidate, "0.1.0"), "{candidate:?} was treated as a version");
        }
    }

    #[test]
    fn only_a_bare_three_part_number_counts_as_a_version() {
        assert!(is_version("0.1.0"));
        assert!(is_version("12.34.56"));

        for text in [
            "",
            "v1.0.0",
            "1.0",
            "1.0.0-beta",
            "<!DOCTYPE html><html>404</html>",
            "0.1.0 (see the release notes)",
        ] {
            assert!(!is_version(text), "{text:?} was accepted as a version");
        }
    }

    /// Hits the network. Run it when the release process changes:
    ///   cargo test --lib update -- --ignored --nocapture
    #[test]
    #[ignore = "needs the network"]
    fn the_published_version_file_is_readable() {
        let status = check();
        println!("{status:?}");
        assert!(status.error.is_none(), "{:?}", status.error);
        assert!(status.latest.is_some());
    }
}
