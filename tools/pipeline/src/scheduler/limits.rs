//! When a role that hit a usage limit (exit status 75, `pipeline role`)
//! may run again, from the agent's message.
//!
//! Claude Code says things like `Claude AI usage limit reached|1760000000`
//! (a Unix time), `5-hour limit reached ∙ resets 3pm`, `You've hit your
//! limit · resets 2am (Europe/Berlin)`; providers say `try again in 20
//! minutes` or `retry after 60 seconds`. A clock time is resolved by the
//! caller (`date -d`, in the time zone the message names, else the
//! server's). Without any hint the loop waits `loop.limit_pause_secs`.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reset {
    /// Seconds since the epoch.
    At(u64),
    /// Seconds from now.
    In(u64),
    /// A clock time such as `3pm` or `14:30`, in a time zone if one was named.
    Clock { time: String, tz: Option<String> },
}

/// The longest pause the loop believes (the weekly window plus a margin).
pub const MAX_PAUSE: u64 = 8 * 24 * 3600;
/// Added to a reset time so the retry lands after the window opens.
pub const MARGIN: u64 = 120;

/// The last reset hint in `output`, if any.
pub fn parse(output: &str) -> Option<Reset> {
    let mut found = None;
    for line in output.lines() {
        let lower = line.to_lowercase();
        if let Some(r) = epoch_after_bar(&lower).or_else(|| relative(&lower)).or_else(|| clock(line, &lower)) {
            found = Some(r);
        }
    }
    found
}

/// `...limit reached|1760000000`
fn epoch_after_bar(lower: &str) -> Option<Reset> {
    let (_, rest) = lower.split_once("limit reached|")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (digits.len() >= 9).then(|| digits.parse().ok().map(Reset::At)).flatten()
}

/// `try again in 20 minutes`, `retry after 60 seconds`, `resets in 2 hours`
fn relative(lower: &str) -> Option<Reset> {
    for key in ["try again in ", "retry after ", "resets in ", "reset in "] {
        let Some((_, rest)) = lower.split_once(key) else { continue };
        let mut words = rest.split_whitespace();
        let n: u64 = words.next()?.trim_end_matches(['s', 'm', 'h']).parse().ok()?;
        let unit = words.next().unwrap_or("seconds");
        let secs = match unit.chars().next() {
            Some('h') => n * 3600,
            Some('m') => n * 60,
            _ => n,
        };
        return Some(Reset::In(secs));
    }
    None
}

/// `resets 3pm`, `resets at 14:30`, `resets 2am (Europe/Berlin)`
fn clock(line: &str, lower: &str) -> Option<Reset> {
    let i = lower.find("resets ")?;
    let rest = lower[i + 7..].trim_start();
    let rest = rest.strip_prefix("at ").unwrap_or(rest);
    let time: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == ':').collect();
    if time.is_empty() {
        return None;
    }
    let after = &rest[time.len()..];
    let suffix = if after.starts_with("am") {
        "am"
    } else if after.starts_with("pm") {
        "pm"
    } else {
        ""
    };
    if suffix.is_empty() && !time.contains(':') {
        return None;
    }
    // A time zone in parentheses keeps its case: "Europe/Berlin".
    let tz = line.rsplit_once('(').and_then(|(_, r)| r.split_once(')')).map(|(tz, _)| tz.trim().to_string());
    let tz = tz.filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c)));
    Some(Reset::Clock { time: format!("{time}{suffix}"), tz })
}

/// The time to retry: the reset (resolved) plus [`MARGIN`], or the
/// default pause, never more than [`MAX_PAUSE`] away and at least a minute.
pub fn retry_time(now: u64, resolved: Option<u64>, default_pause: u64) -> u64 {
    let at = match resolved {
        Some(t) if t > now => t + MARGIN,
        _ => now + default_pause,
    };
    at.clamp(now + 60, now + MAX_PAUSE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints() {
        assert_eq!(parse("Claude AI usage limit reached|1760000000\n"), Some(Reset::At(1_760_000_000)));
        assert_eq!(parse("Error: 429 Too Many Requests, try again in 20 minutes"), Some(Reset::In(1200)));
        assert_eq!(parse("rate limited; retry after 60 seconds"), Some(Reset::In(60)));
        assert_eq!(parse("5-hour limit reached ∙ resets 3pm"), Some(Reset::Clock { time: "3pm".into(), tz: None }));
        assert_eq!(
            parse("You've hit your limit · resets 2am (Europe/Berlin)"),
            Some(Reset::Clock { time: "2am".into(), tz: Some("Europe/Berlin".into()) })
        );
        assert_eq!(
            parse("weekly limit reached, resets at 14:30"),
            Some(Reset::Clock { time: "14:30".into(), tz: None })
        );
        assert_eq!(parse("usage limit reached"), None);
        assert_eq!(parse("resets 3 things"), None);
    }

    #[test]
    fn retry_times() {
        assert_eq!(retry_time(1000, Some(5000), 1800), 5000 + MARGIN);
        assert_eq!(retry_time(1000, Some(500), 1800), 2800, "a reset in the past: default pause");
        assert_eq!(retry_time(1000, None, 1800), 2800);
        assert_eq!(retry_time(1000, Some(1000 + 30 * 24 * 3600), 1800), 1000 + MAX_PAUSE);
        assert_eq!(retry_time(1000, None, 0), 1060);
    }
}
