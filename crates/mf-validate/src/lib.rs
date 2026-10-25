//! Tests de validation de modèles de risque de marché.

use serde::{Deserialize, Serialize};

/// VaR historique (perte positive) au niveau de confiance `alpha` (ex. 0.99).
pub fn historical_var(returns: &[f64], alpha: f64) -> Option<f64> {
    if returns.is_empty() || !(0.0..1.0).contains(&alpha) { return None; }
    let mut r = returns.to_vec();
    r.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // k-ième pire rendement, k = ceil(n·(1-α)) (tolérance pour l'arrondi flottant).
    let k = ((1.0 - alpha) * r.len() as f64 - 1e-9).ceil().max(1.0) as usize;
    let idx = (k - 1).min(r.len() - 1);
    Some(-r[idx])
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Kupiec {
    pub observations: usize,
    pub exceptions: usize,
    pub expected_rate: f64,
    pub observed_rate: f64,
    pub lr_pof: f64,
    /// Seuil du χ²(1) à 95 %.
    pub critical_value: f64,
    pub passed: bool,
}

/// Test de Kupiec (proportion d'exceptions).
pub fn kupiec_pof(observations: usize, exceptions: usize, alpha: f64) -> Kupiec {
    let (n, x, p) = (observations as f64, exceptions as f64, 1.0 - alpha);
    let ll = |q: f64| -> f64 {
        let a = if n - x > 0.0 { (n - x) * (1.0 - q).ln() } else { 0.0 };
        let b = if x > 0.0 { x * q.ln() } else { 0.0 };
        a + b
    };
    let phat = x / n;
    let lr = -2.0 * (ll(p) - ll(phat));
    let critical = 3.841_458_820_694_124;
    Kupiec {
        observations,
        exceptions,
        expected_rate: p,
        observed_rate: phat,
        lr_pof: lr,
        critical_value: critical,
        passed: lr < critical,
    }
}

/// Backtest glissant : VaR estimée sur `window` jours, comparée au rendement suivant.
pub fn rolling_backtest(returns: &[f64], window: usize, alpha: f64) -> Option<Kupiec> {
    if returns.len() <= window { return None; }
    let mut exc = 0;
    for t in window..returns.len() {
        let var = historical_var(&returns[t - window..t], alpha)?;
        if returns[t] < -var { exc += 1; }
    }
    Some(kupiec_pof(returns.len() - window, exc, alpha))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn var_on_known_sample() {
        let r: Vec<f64> = (1..=100).map(|i| -(i as f64)).collect(); // -1 .. -100
        // 1 % le plus bas = -100 -> VaR = 100
        assert_eq!(historical_var(&r, 0.99), Some(100.0));
    }

    #[test]
    fn kupiec_accepts_expected_rate() {
        let k = kupiec_pof(1000, 10, 0.99);
        assert!(k.passed && k.lr_pof < 1e-9);
    }

    #[test]
    fn kupiec_rejects_too_many_exceptions() {
        assert!(!kupiec_pof(1000, 30, 0.99).passed);
    }

    #[test]
    fn kupiec_handles_zero_exceptions() {
        let k = kupiec_pof(250, 0, 0.99);
        assert!(k.lr_pof.is_finite());
    }
}
