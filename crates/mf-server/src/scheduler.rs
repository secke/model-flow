//! Scheduled runs: 5-field cron expression (UTC) declared in `pipeline.toml`.
//!
//! Each slot is claimed in the database (compare-and-swap): with several instances on
//! PostgreSQL, only one starts the run. After an outage, only the latest missed slot
//! is caught up. When a pipeline is first discovered, nothing is started: the counter
//! starts at the present time.

use chrono::{TimeZone, Utc};
use croner::Cron;

pub fn parse(expr: &str) -> Result<Cron, String> {
    if expr.split_whitespace().count() != 5 {
        return Err(format!(
            "schedule `{expr}`: 5 fields expected (minute hour day month weekday)"
        ));
    }
    Cron::new(expr).parse().map_err(|e| format!("schedule `{expr}`: {e}"))
}

/// Next slot strictly after `ts` (Unix seconds).
pub fn next_after(cron: &Cron, ts: i64) -> Option<i64> {
    let t = Utc.timestamp_opt(ts, 0).single()?;
    cron.find_next_occurrence(&t, false).ok().map(|d| d.timestamp())
}

/// Latest slot in `]after, now]`, if any.
pub fn latest_due(cron: &Cron, after: i64, now: i64) -> Option<i64> {
    let mut slot = None;
    let mut cur = after;
    for _ in 0..100_000 {
        match next_after(cron, cur) {
            Some(n) if n <= now => {
                slot = Some(n);
                cur = n;
            }
            _ => break,
        }
    }
    slot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots() {
        let c = parse("0 6 * * 1-5").unwrap();
        // Tuesday 2026-01-13 07:00 UTC -> next slot Wednesday 06:00.
        let tue = Utc.with_ymd_and_hms(2026, 1, 13, 7, 0, 0).unwrap().timestamp();
        let wed = Utc.with_ymd_and_hms(2026, 1, 14, 6, 0, 0).unwrap().timestamp();
        assert_eq!(next_after(&c, tue), Some(wed));
        // From Friday evening to Monday 07:00: only Monday 06:00 is due (weekend excluded).
        let fri = Utc.with_ymd_and_hms(2026, 1, 16, 20, 0, 0).unwrap().timestamp();
        let mon7 = Utc.with_ymd_and_hms(2026, 1, 19, 7, 0, 0).unwrap().timestamp();
        let mon6 = Utc.with_ymd_and_hms(2026, 1, 19, 6, 0, 0).unwrap().timestamp();
        assert_eq!(latest_due(&c, fri, mon7), Some(mon6));
        assert_eq!(latest_due(&c, mon6, mon7), None);
        assert!(parse("0 6 * *").is_err());
        assert!(parse("99 6 * * *").is_err());
    }
}
