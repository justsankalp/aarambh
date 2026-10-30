//! Pure helpers for syncing with the GitHub Contents API.

use crate::time::Day;

/// Retries after the first attempt when a write hits a conflict (409/422).
pub const MAX_RETRIES: u32 = 4;

/// Days loaded on start; older days load on request.
pub const INITIAL_DAYS: i64 = 14;

/// Delay before retry `n` (1-based): doubling from 400 ms, with "equal
/// jitter" so two devices that collided don't retry in lockstep.
/// `jitter` is in `[0, 1)`.
pub fn backoff_ms(retry: u32, jitter: f64) -> u32 {
    let base = 400u32.saturating_mul(1 << retry.clamp(1, 8).saturating_sub(1));
    let half = base / 2;
    half + (f64::from(half) * jitter.clamp(0.0, 1.0)) as u32
}

/// A GitHub repository, `owner/name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    /// Accepts `owner/name`, or a pasted `https://github.com/owner/name(.git)`.
    pub fn parse(input: &str) -> Option<RepoRef> {
        let s = input.trim().trim_end_matches('/');
        let s = s
            .strip_prefix("https://")
            .or_else(|| s.strip_prefix("http://"))
            .unwrap_or(s);
        let s = s
            .strip_prefix("github.com/")
            .or_else(|| s.strip_prefix("www.github.com/"))
            .unwrap_or(s);
        let s = s.strip_suffix(".git").unwrap_or(s);
        let (owner, name) = s.split_once('/')?;
        let owner_ok = (1..=39).contains(&owner.len())
            && owner
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            && !owner.starts_with('-');
        let name_ok = (1..=100).contains(&name.len())
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
            && name != "."
            && name != "..";
        (owner_ok && name_ok).then(|| RepoRef {
            owner: owner.into(),
            name: name.into(),
        })
    }

    pub fn api_base(&self) -> String {
        format!("https://api.github.com/repos/{}/{}", self.owner, self.name)
    }
}

impl std::fmt::Display for RepoRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// `2026-10-30.json` → that day. Other names in `data/events/` are ignored.
pub fn day_from_events_file(name: &str) -> Option<Day> {
    name.strip_suffix(".json")?.parse().ok()
}

pub fn commit_message(author: &str, day: Day, count: usize) -> String {
    let who: String = author
        .chars()
        .filter(|c| !c.is_control())
        .take(40)
        .collect();
    let noun = if count == 1 { "event" } else { "events" };
    format!("notes: {count} {noun} from {who} ({day})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_with_bounded_jitter() {
        assert_eq!(backoff_ms(1, 0.0), 200);
        assert_eq!(backoff_ms(1, 0.999_999), 399);
        assert_eq!(backoff_ms(2, 0.0), 400);
        assert_eq!(backoff_ms(4, 0.0), 1600);
        assert_eq!(backoff_ms(4, 1.0), 3200);
        assert_eq!(backoff_ms(99, 2.0), backoff_ms(8, 1.0), "clamped");
        let total: u32 = (1..=MAX_RETRIES).map(|n| backoff_ms(n, 1.0)).sum();
        assert!(total <= 6000, "worst case wait stays short: {total} ms");
    }

    #[test]
    fn parses_repo_names_and_urls() {
        let want = Some(RepoRef {
            owner: "justsankalp".into(),
            name: "aarambh".into(),
        });
        for s in [
            "justsankalp/aarambh",
            "  justsankalp/aarambh  ",
            "https://github.com/justsankalp/aarambh",
            "https://github.com/justsankalp/aarambh/",
            "github.com/justsankalp/aarambh.git",
        ] {
            assert_eq!(RepoRef::parse(s), want, "{s}");
        }
        assert_eq!(
            want.unwrap().api_base(),
            "https://api.github.com/repos/justsankalp/aarambh"
        );
    }

    #[test]
    fn rejects_bad_repo_names() {
        for s in [
            "",
            "aarambh",
            "a/b/c",
            "-bad/repo",
            "own er/repo",
            "owner/..",
            "owner/re?po",
            "../../etc/passwd",
        ] {
            assert_eq!(RepoRef::parse(s), None, "{s}");
        }
    }

    #[test]
    fn reads_event_file_names() {
        assert_eq!(
            day_from_events_file("2026-10-30.json").unwrap().to_string(),
            "2026-10-30"
        );
        assert_eq!(day_from_events_file(".gitkeep"), None);
        assert_eq!(day_from_events_file("2026-10-30.json.bak"), None);
        assert_eq!(day_from_events_file("notes.json"), None);
    }

    #[test]
    fn commit_messages_are_single_line() {
        let day: Day = "2026-10-30".parse().unwrap();
        assert_eq!(
            commit_message("Sankalp", day, 1),
            "notes: 1 event from Sankalp (2026-10-30)"
        );
        assert_eq!(
            commit_message("A\nB", day, 3),
            "notes: 3 events from AB (2026-10-30)"
        );
    }
}
