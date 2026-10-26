//! Exécutions planifiées : expression cron à 5 champs (UTC) déclarée dans `pipeline.toml`.
//!
//! Chaque créneau est réservé en base (comparer-et-échanger) : avec plusieurs instances sur
//! PostgreSQL, une seule lance l'exécution. Après une interruption, seul le dernier créneau
//! manqué est rattrapé. À la première découverte d'un pipeline, rien n'est lancé : le compteur
//! démarre à l'instant présent.

use chrono::{TimeZone, Utc};
use croner::Cron;

pub fn parse(expr: &str) -> Result<Cron, String> {
    if expr.split_whitespace().count() != 5 {
        return Err(format!(
            "planification `{expr}` : 5 champs attendus (minute heure jour mois jour-semaine)"
        ));
    }
    Cron::new(expr)
        .parse()
        .map_err(|e| format!("planification `{expr}` : {e}"))
}

/// Prochain créneau strictement après `ts` (secondes Unix).
pub fn next_after(cron: &Cron, ts: i64) -> Option<i64> {
    let t = Utc.timestamp_opt(ts, 0).single()?;
    cron.find_next_occurrence(&t, false).ok().map(|d| d.timestamp())
}

/// Dernier créneau dans `]after, now]`, s'il y en a un.
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
        // Mardi 2026-01-13 07:00 UTC -> prochain créneau mercredi 06:00.
        let tue = Utc.with_ymd_and_hms(2026, 1, 13, 7, 0, 0).unwrap().timestamp();
        let wed = Utc.with_ymd_and_hms(2026, 1, 14, 6, 0, 0).unwrap().timestamp();
        assert_eq!(next_after(&c, tue), Some(wed));
        // Du vendredi soir au lundi 07:00 : seul le lundi 06:00 est dû (week-end exclu).
        let fri = Utc.with_ymd_and_hms(2026, 1, 16, 20, 0, 0).unwrap().timestamp();
        let mon7 = Utc.with_ymd_and_hms(2026, 1, 19, 7, 0, 0).unwrap().timestamp();
        let mon6 = Utc.with_ymd_and_hms(2026, 1, 19, 6, 0, 0).unwrap().timestamp();
        assert_eq!(latest_due(&c, fri, mon7), Some(mon6));
        assert_eq!(latest_due(&c, mon6, mon7), None);
        assert!(parse("0 6 * *").is_err());
        assert!(parse("99 6 * * *").is_err());
    }
}
