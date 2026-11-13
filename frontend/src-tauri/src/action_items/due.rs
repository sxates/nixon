//! Due-date resolution shared with the TypeScript quick-add parser (specs/0088).
//!
//! The rules are defined by `frontend/src/lib/__fixtures__/due-dates.json`, which both
//! this module's tests and the TypeScript tests read.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

const WEEKDAYS: [(&str, &str, Weekday); 7] = [
    ("monday", "mon", Weekday::Mon),
    ("tuesday", "tue", Weekday::Tue),
    ("wednesday", "wed", Weekday::Wed),
    ("thursday", "thu", Weekday::Thu),
    ("friday", "fri", Weekday::Fri),
    ("saturday", "sat", Weekday::Sat),
    ("sunday", "sun", Weekday::Sun),
];

const MONTHS: [(&str, &str); 12] = [
    ("january", "jan"),
    ("february", "feb"),
    ("march", "mar"),
    ("april", "apr"),
    ("may", "may"),
    ("june", "jun"),
    ("july", "jul"),
    ("august", "aug"),
    ("september", "sep"),
    ("october", "oct"),
    ("november", "nov"),
    ("december", "dec"),
];

/// Resolves the text inside a due-date parenthesis (trimmed, case-insensitive). The same
/// weekday as `today` means today; `this <weekday>` is the bare weekday; `next <weekday>`
/// is that weekday in the next Monday-start week; `next week` is the Monday after `today`;
/// a month-day (US order, `M/D`) before `today` rolls to next year. Anything else is
/// `None`.
pub fn resolve_due_phrase(phrase: &str, today: NaiveDate) -> Option<NaiveDate> {
    let lower = phrase.trim().to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let norm = words.join(" ");

    match norm.as_str() {
        "today" => return Some(today),
        "tomorrow" => return today.succ_opt(),
        "next week" => {
            let ahead = 7 - i64::from(today.weekday().num_days_from_monday());
            return today.checked_add_signed(Duration::days(ahead));
        }
        _ => {}
    }

    // `this <weekday>` = the bare weekday; `next <weekday>` = that weekday in the next
    // (Monday-start) calendar week.
    let (which, wd_text) = match words.as_slice() {
        [q @ ("this" | "next"), w] => (Some(*q), *w),
        _ => (None, norm.as_str()),
    };
    if let Some((_, _, wd)) = WEEKDAYS
        .iter()
        .find(|(full, abbr, _)| wd_text == *full || wd_text == *abbr)
    {
        let wd = i64::from(wd.num_days_from_monday());
        let now = i64::from(today.weekday().num_days_from_monday());
        let ahead = if which == Some("next") {
            7 - now + wd
        } else {
            (wd - now).rem_euclid(7)
        };
        return today.checked_add_signed(Duration::days(ahead));
    }

    if let Ok(d) = NaiveDate::parse_from_str(&norm, "%Y-%m-%d") {
        // chrono accepts non-padded fields; the contract is exactly YYYY-MM-DD.
        return (norm.len() == 10).then_some(d);
    }

    if let [m, day] = words.as_slice() {
        let month = MONTHS
            .iter()
            .position(|(full, abbr)| m == full || m == abbr)? as u32
            + 1;
        return month_day(month, parse_day(day)?, today);
    }

    if let Some((m, day)) = norm.split_once('/') {
        return month_day(parse_day(m)?, parse_day(day)?, today);
    }

    None
}

fn parse_day(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 2 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// This year, or next year when the date has already passed. An impossible date (Feb 30,
/// month 13) is `None`.
fn month_day(month: u32, day: u32, today: NaiveDate) -> Option<NaiveDate> {
    if let Some(d) = NaiveDate::from_ymd_opt(today.year(), month, day).filter(|d| *d >= today) {
        return Some(d);
    }
    // Passed already (or Feb 29 in a non-leap year): try next year (None if still invalid).
    NaiveDate::from_ymd_opt(today.year() + 1, month, day)
}

/// Resolves an extraction `due` hint ("by Friday", "(Nov 20)", "2026-07-10") against the
/// meeting's local date, returning an ISO `YYYY-MM-DD` string.
pub fn resolve_due_hint(hint: &str, anchor: NaiveDate) -> Option<String> {
    let mut h = hint.trim();
    if let Some(inner) = h.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        h = inner.trim();
    }
    let lower = h.to_lowercase();
    for prefix in ["by ", "on ", "due ", "before "] {
        if lower.starts_with(prefix) {
            h = h[prefix.len()..].trim();
            break;
        }
    }
    resolve_due_phrase(h, anchor).map(|d| d.format("%Y-%m-%d").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../src/lib/__fixtures__/due-dates.json");

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn fixture_phrases() {
        let v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        let cases = v["phrases"].as_array().unwrap();
        assert!(cases.len() > 20);
        for c in cases {
            let text = c["text"].as_str().unwrap();
            let today = d(c["today"].as_str().unwrap());
            let expect = c["expect"].as_str().map(d);
            assert_eq!(
                resolve_due_phrase(text, today),
                expect,
                "phrase {text:?} on {today}"
            );
        }
    }

    #[test]
    fn hint_strips_by() {
        let a = d("2026-11-12");
        assert_eq!(
            resolve_due_hint("by Friday", a).as_deref(),
            Some("2026-11-13")
        );
        assert_eq!(
            resolve_due_hint("(on tue)", a).as_deref(),
            Some("2026-11-17")
        );
        assert_eq!(
            resolve_due_hint("Due 2026-07-10", a).as_deref(),
            Some("2026-07-10")
        );
        assert_eq!(
            resolve_due_hint("before Nov 20", a).as_deref(),
            Some("2026-11-20")
        );
        assert_eq!(resolve_due_hint("by end of Q3", a), None);
        assert_eq!(resolve_due_hint("  ", a), None);
    }
}
