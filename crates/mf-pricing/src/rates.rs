//! Courbes de taux et swaps de taux (mono-courbe).
//!
//! Bootstrap séquentiel sur dépôts (taux simple) et swaps de taux par (jambe fixe annualisée par
//! `fixed_frequency`), interpolation log-linéaire des facteurs d'actualisation (forwards constants par
//! morceaux), extrapolation au dernier forward. Les temps sont en années et les fractions d'année des
//! coupons valent 1/fréquence (pas de calendrier ni de convention de jours).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CurveInstrument {
    Deposit {
        maturity: f64,
        rate: f64,
    },
    Swap {
        maturity: f64,
        rate: f64,
        fixed_frequency: u32,
    },
}

impl CurveInstrument {
    pub fn maturity(&self) -> f64 {
        match self {
            CurveInstrument::Deposit { maturity, .. } | CurveInstrument::Swap { maturity, .. } => *maturity,
        }
    }
    pub fn quote(&self) -> f64 {
        match self {
            CurveInstrument::Deposit { rate, .. } | CurveInstrument::Swap { rate, .. } => *rate,
        }
    }
    pub fn label(&self) -> String {
        match self {
            CurveInstrument::Deposit { maturity, .. } => format!("DEP {maturity}Y"),
            CurveInstrument::Swap { maturity, .. } => format!("IRS {maturity}Y"),
        }
    }
    fn bumped(&self, d: f64) -> Self {
        let mut x = self.clone();
        match &mut x {
            CurveInstrument::Deposit { rate, .. } | CurveInstrument::Swap { rate, .. } => *rate += d,
        }
        x
    }
}

fn two() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CurveSpec {
    pub instruments: Vec<CurveInstrument>,
    /// Fréquence de la jambe variable (informative en mono-courbe).
    #[serde(default = "two")]
    pub float_frequency: u32,
}

#[derive(Debug, Clone)]
pub struct Curve {
    times: Vec<f64>,
    log_df: Vec<f64>,
    pub spec: CurveSpec,
}

/// Nombre entier de périodes `t × f`.
pub fn periods(t: f64, f: u32) -> Result<usize, String> {
    if f == 0 {
        return Err("fréquence nulle".into());
    }
    let n = (t * f as f64).round();
    if n < 1.0 || (n - t * f as f64).abs() > 1e-9 {
        return Err(format!("maturité {t} non multiple de la période 1/{f}"));
    }
    Ok(n as usize)
}

impl Curve {
    pub fn discount(&self, t: f64) -> f64 {
        if t <= 0.0 {
            return 1.0;
        }
        let n = self.times.len();
        let i = self.times.partition_point(|x| *x < t).clamp(1, n - 1);
        let (t0, t1, l0, l1) = (self.times[i - 1], self.times[i], self.log_df[i - 1], self.log_df[i]);
        (l0 + (l1 - l0) * (t - t0) / (t1 - t0)).exp()
    }

    /// Taux zéro-coupon continu.
    pub fn zero_rate(&self, t: f64) -> f64 {
        if t <= 0.0 {
            return self.forward_rate(0.0, 1e-6);
        }
        -self.discount(t).ln() / t
    }

    /// Taux forward continu entre `t1` et `t2`.
    pub fn forward_rate(&self, t1: f64, t2: f64) -> f64 {
        (self.discount(t1).ln() - self.discount(t2).ln()) / (t2 - t1)
    }

    /// Annuité Σ αᵢ P(tᵢ) d'une jambe fixe de fréquence `f` jusqu'à `t`.
    pub fn annuity(&self, t: f64, f: u32) -> Result<f64, String> {
        let n = periods(t, f)?;
        Ok((1..=n).map(|i| self.discount(i as f64 / f as f64)).sum::<f64>() / f as f64)
    }

    /// Taux de swap par (départ immédiat, mono-courbe).
    pub fn par_swap_rate(&self, t: f64, f: u32) -> Result<f64, String> {
        Ok((1.0 - self.discount(t)) / self.annuity(t, f)?)
    }

    fn model_rate(&self, inst: &CurveInstrument) -> Result<f64, String> {
        match inst {
            CurveInstrument::Deposit { maturity, .. } => Ok((1.0 / self.discount(*maturity) - 1.0) / maturity),
            CurveInstrument::Swap {
                maturity,
                fixed_frequency,
                ..
            } => self.par_swap_rate(*maturity, *fixed_frequency),
        }
    }

    pub fn bootstrap(spec: &CurveSpec) -> Result<Self, String> {
        let mut inst = spec.instruments.clone();
        if inst.is_empty() {
            return Err("courbe sans instrument".into());
        }
        inst.sort_by(|a, b| a.maturity().partial_cmp(&b.maturity()).unwrap());
        let mut c = Curve {
            times: vec![0.0],
            log_df: vec![0.0],
            spec: spec.clone(),
        };
        for k in &inst {
            let t = k.maturity();
            if !(t.is_finite() && t > *c.times.last().unwrap()) {
                return Err(format!("{}: maturités non strictement croissantes", k.label()));
            }
            if let CurveInstrument::Swap { fixed_frequency, .. } = k {
                periods(t, *fixed_frequency)?;
            }
            c.times.push(t);
            c.log_df.push(0.0);
            // Dichotomie sur le taux zéro du nouveau pilier : le taux modèle est croissant en z.
            let (mut lo, mut hi) = (-0.5f64, 1.0f64);
            let err_at = |c: &mut Curve, z: f64| -> Result<f64, String> {
                *c.log_df.last_mut().unwrap() = -z * t;
                Ok(c.model_rate(k)? - k.quote())
            };
            if err_at(&mut c, lo)? > 0.0 || err_at(&mut c, hi)? < 0.0 {
                return Err(format!("{}: cotation hors de l'intervalle résoluble", k.label()));
            }
            for _ in 0..200 {
                let mid = 0.5 * (lo + hi);
                if err_at(&mut c, mid)? > 0.0 {
                    hi = mid
                } else {
                    lo = mid
                }
                if hi - lo < 1e-16 {
                    break;
                }
            }
            err_at(&mut c, 0.5 * (lo + hi))?;
        }
        Ok(c)
    }

    /// Courbe reconstruite après déplacement de toutes les cotations de `bp` points de base.
    pub fn bumped(&self, bp: f64) -> Result<Self, String> {
        let spec = CurveSpec {
            instruments: self.spec.instruments.iter().map(|i| i.bumped(bp * 1e-4)).collect(),
            float_frequency: self.spec.float_frequency,
        };
        Self::bootstrap(&spec)
    }

    /// Re-pricing des instruments d'entrée : (instrument, maturité, cotation, taux modèle).
    pub fn repricing(&self) -> Result<Vec<(String, f64, f64, f64)>, String> {
        self.spec
            .instruments
            .iter()
            .map(|k| Ok((k.label(), k.maturity(), k.quote(), self.model_rate(k)?)))
            .collect()
    }

    pub fn pillars(&self) -> &[f64] {
        &self.times[1..]
    }
}

/// Courbes de marché nommées.
#[derive(Debug, Clone, Default)]
pub struct Market {
    pub curves: BTreeMap<String, Curve>,
}

impl Market {
    /// `{"curves": {"<nom>": {"instruments": [...], "float_frequency": 2}}}`.
    pub fn from_value(v: &Value) -> Result<Self, String> {
        let specs: BTreeMap<String, CurveSpec> =
            serde_json::from_value(v.get("curves").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("marché invalide: {e}"))?;
        let curves = specs
            .into_iter()
            .map(|(n, s)| {
                Curve::bootstrap(&s)
                    .map(|c| (n.clone(), c))
                    .map_err(|e| format!("courbe {n}: {e}"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { curves })
    }

    pub fn curve(&self, name: &str) -> Result<&Curve, String> {
        self.curves
            .get(name)
            .ok_or_else(|| format!("courbe `{name}` absente du marché"))
    }
}

/// NPV d'un swap de taux vanille au départ immédiat, côté payeur ou receveur du fixe.
pub fn swap_npv(
    c: &Curve,
    notional: f64,
    fixed_rate: f64,
    maturity: f64,
    f: u32,
    pay_fixed: bool,
) -> Result<f64, String> {
    let float_leg = notional * (1.0 - c.discount(maturity));
    let fixed_leg = notional * fixed_rate * c.annuity(maturity, f)?;
    Ok(if pay_fixed {
        float_leg - fixed_leg
    } else {
        fixed_leg - float_leg
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> CurveSpec {
        serde_json::from_value(serde_json::json!({"instruments": [
            {"type": "deposit", "maturity": 0.25, "rate": 0.031},
            {"type": "deposit", "maturity": 0.5, "rate": 0.0315},
            {"type": "swap", "maturity": 1, "rate": 0.0322, "fixed_frequency": 1},
            {"type": "swap", "maturity": 2, "rate": 0.0330, "fixed_frequency": 1},
            {"type": "swap", "maturity": 5, "rate": 0.0345, "fixed_frequency": 1},
            {"type": "swap", "maturity": 10, "rate": 0.0362, "fixed_frequency": 1},
            {"type": "swap", "maturity": 30, "rate": 0.0371, "fixed_frequency": 1}
        ]}))
        .unwrap()
    }

    #[test]
    fn bootstrap_reprices_inputs() {
        let c = Curve::bootstrap(&spec()).unwrap();
        for (label, _, q, m) in c.repricing().unwrap() {
            assert!((q - m).abs() < 1e-13, "{label}: {q} vs {m}");
        }
        assert_eq!(c.pillars().len(), 7);
    }

    #[test]
    fn par_swap_has_zero_npv_and_dv01_sign() {
        let c = Curve::bootstrap(&spec()).unwrap();
        let k = c.par_swap_rate(7.0, 1).unwrap();
        assert!(swap_npv(&c, 1e7, k, 7.0, 1, true).unwrap().abs() < 1e-6);
        let up = c.bumped(1.0).unwrap();
        let dv01 = swap_npv(&up, 1e7, k, 7.0, 1, true).unwrap() - swap_npv(&c, 1e7, k, 7.0, 1, true).unwrap();
        // Payeur du fixe : gagne quand les taux montent, ~ notionnel × annuité × 1 pb.
        let approx = 1e7 * c.annuity(7.0, 1).unwrap() * 1e-4;
        assert!(dv01 > 0.0 && (dv01 / approx - 1.0).abs() < 0.05, "{dv01} vs {approx}");
    }

    #[test]
    fn flat_curve_closed_form() {
        // Dépôt 1 an à r simple puis swaps annuels au même taux : courbe plate en composition annuelle.
        let r: f64 = 0.04;
        let s: CurveSpec = serde_json::from_value(serde_json::json!({"instruments": [
            {"type": "deposit", "maturity": 1, "rate": r},
            {"type": "swap", "maturity": 2, "rate": r, "fixed_frequency": 1},
            {"type": "swap", "maturity": 3, "rate": r, "fixed_frequency": 1}
        ]}))
        .unwrap();
        let c = Curve::bootstrap(&s).unwrap();
        for t in [1.0, 2.0, 3.0] {
            assert!((c.discount(t) - (1.0 + r).powf(-t)).abs() < 1e-14);
        }
        // Interpolation log-linéaire = forward constant entre piliers.
        assert!((c.discount(1.5) - (1.0 + r).powf(-1.5)).abs() < 1e-14);
    }

    #[test]
    fn rejects_bad_curves() {
        let bad = |j| Curve::bootstrap(&serde_json::from_value::<CurveSpec>(j).unwrap()).is_err();
        assert!(bad(serde_json::json!({"instruments": []})));
        assert!(bad(serde_json::json!({"instruments": [
            {"type": "deposit", "maturity": 1, "rate": 0.03}, {"type": "deposit", "maturity": 1, "rate": 0.03}]})));
        assert!(bad(
            serde_json::json!({"instruments": [{"type": "swap", "maturity": 1.3, "rate": 0.03, "fixed_frequency": 1}]})
        ));
    }
}
