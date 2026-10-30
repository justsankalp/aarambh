//! The week row: seven dots for the last seven days, showing whether both
//! of you, one of you, or nobody wrote. No streaks, no penalties.

use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    Nobody,
    One,
    Both,
}

/// `entries` are (local day number, author) for each note; `today` is
/// today's local day number. Returns the last seven days, oldest first.
/// "Both" means two different people wrote that day.
pub fn week<'a>(entries: impl IntoIterator<Item = (i64, &'a str)>, today: i64) -> [Dot; 7] {
    let mut who: [BTreeSet<&str>; 7] = Default::default();
    for (day, author) in entries {
        let back = today - day;
        if (0..7).contains(&back) && !author.trim().is_empty() {
            who[(6 - back) as usize].insert(author);
        }
    }
    who.map(|w| match w.len() {
        0 => Dot::Nobody,
        1 => Dot::One,
        _ => Dot::Both,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_days_oldest_first() {
        let today = 20_000;
        let dots = week(
            [
                (today, "Asha"),
                (today, "Sankalp"),
                (today, "Asha"),
                (today - 2, "Sankalp"),
                (today - 6, "Asha"),
                (today - 6, "Asha"),
                (today - 7, "Asha"),
                (today - 7, "Sankalp"),
                (today + 1, "Asha"),
                (today - 3, "  "),
            ],
            today,
        );
        use Dot::*;
        assert_eq!(dots, [One, Nobody, Nobody, Nobody, One, Nobody, Both]);
    }

    #[test]
    fn an_empty_week() {
        assert_eq!(week([], 5), [Dot::Nobody; 7]);
    }
}
