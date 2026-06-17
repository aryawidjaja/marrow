//! Time, duration, and decay helpers.

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Current UTC time as an RFC3339 string.
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Parse an RFC3339 timestamp to a unix second count.
pub fn to_unix(ts: &str) -> Option<i64> {
    OffsetDateTime::parse(ts, &Rfc3339)
        .ok()
        .map(|t| t.unix_timestamp())
}

/// Parse a duration string like `"30d"`, `"12h"`, `"45m"`, `"10s"`, `"2w"` to seconds.
pub fn parse_duration_secs(s: &str) -> Option<i64> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| c.is_alphabetic())?);
    let n: i64 = num.trim().parse().ok()?;
    let mult = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return None,
    };
    Some(n * mult)
}

/// True if `expires_at` (RFC3339) is in the past relative to `now_unix`.
pub fn is_expired(expires_at: &str, now_unix: i64) -> bool {
    to_unix(expires_at).is_some_and(|e| e < now_unix)
}

/// Confidence after exponential decay: `base * 0.5^(age / half_life)`.
pub fn decayed_confidence(base: f64, created_at: &str, half_life: &str, now_unix: i64) -> f64 {
    let (Some(created), Some(hl)) = (to_unix(created_at), parse_duration_secs(half_life)) else {
        return base;
    };
    if hl <= 0 {
        return base;
    }
    let age = (now_unix - created).max(0) as f64;
    base * 0.5_f64.powf(age / hl as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration_secs("30d"), Some(2_592_000));
        assert_eq!(parse_duration_secs("12h"), Some(43_200));
        assert_eq!(parse_duration_secs("2w"), Some(1_209_600));
        assert_eq!(parse_duration_secs("nope"), None);
    }

    #[test]
    fn detects_expiry() {
        // 2000-01-01 is well before 2026.
        let now = to_unix("2026-06-06T00:00:00Z").unwrap();
        assert!(is_expired("2000-01-01T00:00:00Z", now));
        assert!(!is_expired("2099-01-01T00:00:00Z", now));
    }

    #[test]
    fn decays_by_half_life() {
        let now = to_unix("2026-02-01T00:00:00Z").unwrap(); // ~31 days later
        let c = decayed_confidence(1.0, "2026-01-01T00:00:00Z", "31d", now);
        assert!((c - 0.5).abs() < 0.02, "got {c}");
    }
}
