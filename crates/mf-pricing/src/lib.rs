//! Bibliothèque de pricing de référence : formules fermées et arbres, testées sur des valeurs publiées
//! (Hull, Haug). Sert d'étalon pour valider des pricers de production (QuantLib, bibliothèques internes).

use mf_validate::stats::gamma_q;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::f64::consts::PI;

pub mod benchmark;
pub mod rates;

use rates::{swap_npv, Market};

/// Fonction de répartition de la loi normale centrée réduite.
pub fn norm_cdf(x: f64) -> f64 {
    let q = 0.5 * gamma_q(0.5, x * x / 2.0);
    if x >= 0.0 {
        1.0 - q
    } else {
        q
    }
}

pub fn norm_pdf(x: f64) -> f64 {
    (-0.5 * x * x).exp() / (2.0 * PI).sqrt()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OptionType {
    Call,
    Put,
}

impl OptionType {
    fn sign(self) -> f64 {
        match self {
            OptionType::Call => 1.0,
            OptionType::Put => -1.0,
        }
    }
}

fn zero() -> f64 {
    0.0
}
fn default_steps() -> usize {
    1000
}
fn default_face() -> f64 {
    100.0
}
fn yes() -> bool {
    true
}

/// Instruments pris en charge (taux continus, maturités en années).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "product", rename_all = "snake_case")]
pub enum Instrument {
    EuropeanOption {
        option_type: OptionType,
        spot: f64,
        strike: f64,
        rate: f64,
        #[serde(default = "zero")]
        dividend: f64,
        vol: f64,
        maturity: f64,
    },
    AmericanOption {
        option_type: OptionType,
        spot: f64,
        strike: f64,
        rate: f64,
        #[serde(default = "zero")]
        dividend: f64,
        vol: f64,
        maturity: f64,
        #[serde(default = "default_steps")]
        steps: usize,
    },
    /// Option sur future / forward (modèle de Black 1976).
    Black76 {
        option_type: OptionType,
        forward: f64,
        strike: f64,
        rate: f64,
        vol: f64,
        maturity: f64,
    },
    /// Obligation à taux fixe, rendement actuariel composé `frequency` fois par an.
    FixedRateBond {
        #[serde(default = "default_face")]
        face: f64,
        coupon_rate: f64,
        maturity: f64,
        frequency: u32,
        yield_rate: f64,
    },
    /// Swap de taux vanille au départ immédiat, actualisé et projeté sur la courbe `curve`.
    InterestRateSwap {
        notional: f64,
        fixed_rate: f64,
        maturity: f64,
        fixed_frequency: u32,
        #[serde(default = "yes")]
        pay_fixed: bool,
        curve: String,
    },
    /// Zéro-coupon actualisé sur la courbe `curve`.
    ZeroCouponBond {
        #[serde(default = "default_face")]
        face: f64,
        maturity: f64,
        curve: String,
    },
}

impl Instrument {
    pub fn product(&self) -> &'static str {
        match self {
            Instrument::EuropeanOption { .. } => "european_option",
            Instrument::AmericanOption { .. } => "american_option",
            Instrument::Black76 { .. } => "black76",
            Instrument::FixedRateBond { .. } => "fixed_rate_bond",
            Instrument::InterestRateSwap { .. } => "interest_rate_swap",
            Instrument::ZeroCouponBond { .. } => "zero_coupon_bond",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Trade {
    pub id: String,
    #[serde(flatten)]
    pub instrument: Instrument,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Valuation {
    pub id: String,
    pub product: String,
    pub price: f64,
    /// Sensibilités et mesures (delta, gamma, vega, theta, rho, duration…).
    pub measures: BTreeMap<String, f64>,
}

fn positive(name: &str, v: f64) -> Result<(), String> {
    if v.is_finite() && v > 0.0 {
        Ok(())
    } else {
        Err(format!("`{name}` doit être strictement positif (reçu {v})"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Greeks {
    pub price: f64,
    pub delta: f64,
    pub gamma: f64,
    /// Pour une variation de 1,00 de volatilité (×0,01 pour un point).
    pub vega: f64,
    /// Par année.
    pub theta: f64,
    pub rho: f64,
}

/// Black-Scholes-Merton avec dividende continu `q`.
pub fn bsm(t: OptionType, s: f64, k: f64, r: f64, q: f64, vol: f64, mat: f64) -> Result<Greeks, String> {
    for (n, v) in [("spot", s), ("strike", k), ("vol", vol), ("maturity", mat)] {
        positive(n, v)?;
    }
    let sq = vol * mat.sqrt();
    let d1 = ((s / k).ln() + (r - q + 0.5 * vol * vol) * mat) / sq;
    let d2 = d1 - sq;
    let (dq, dr, w) = ((-q * mat).exp(), (-r * mat).exp(), t.sign());
    let price = w * (s * dq * norm_cdf(w * d1) - k * dr * norm_cdf(w * d2));
    let delta = w * dq * norm_cdf(w * d1);
    let gamma = dq * norm_pdf(d1) / (s * sq);
    let vega = s * dq * norm_pdf(d1) * mat.sqrt();
    let theta = -s * dq * norm_pdf(d1) * vol / (2.0 * mat.sqrt()) - w * r * k * dr * norm_cdf(w * d2)
        + w * q * s * dq * norm_cdf(w * d1);
    let rho = w * k * mat * dr * norm_cdf(w * d2);
    Ok(Greeks {
        price,
        delta,
        gamma,
        vega,
        theta,
        rho,
    })
}

/// Modèle de Black (1976) sur un forward `f`.
pub fn black76(t: OptionType, f: f64, k: f64, r: f64, vol: f64, mat: f64) -> Result<Greeks, String> {
    for (n, v) in [("forward", f), ("strike", k), ("vol", vol), ("maturity", mat)] {
        positive(n, v)?;
    }
    let sq = vol * mat.sqrt();
    let d1 = ((f / k).ln() + 0.5 * vol * vol * mat) / sq;
    let d2 = d1 - sq;
    let (dr, w) = ((-r * mat).exp(), t.sign());
    let price = dr * w * (f * norm_cdf(w * d1) - k * norm_cdf(w * d2));
    Ok(Greeks {
        price,
        delta: dr * w * norm_cdf(w * d1),
        gamma: dr * norm_pdf(d1) / (f * sq),
        vega: dr * f * norm_pdf(d1) * mat.sqrt(),
        theta: r * price - dr * f * norm_pdf(d1) * vol / (2.0 * mat.sqrt()),
        rho: -mat * price,
    })
}

/// Arbre binomial de Cox-Ross-Rubinstein ; exercice anticipé si `american`.
#[allow(clippy::too_many_arguments)]
pub fn crr(
    t: OptionType,
    s: f64,
    k: f64,
    r: f64,
    q: f64,
    vol: f64,
    mat: f64,
    steps: usize,
    american: bool,
) -> Result<f64, String> {
    for (n, v) in [("spot", s), ("strike", k), ("vol", vol), ("maturity", mat)] {
        positive(n, v)?;
    }
    if steps == 0 {
        return Err("`steps` doit être ≥ 1".into());
    }
    let dt = mat / steps as f64;
    let u = (vol * dt.sqrt()).exp();
    let d = 1.0 / u;
    let p = (((r - q) * dt).exp() - d) / (u - d);
    if !(0.0..=1.0).contains(&p) {
        return Err(format!(
            "probabilité risque-neutre hors de [0, 1] ({p}) : augmenter `steps`"
        ));
    }
    let disc = (-r * dt).exp();
    let w = t.sign();
    let payoff = |x: f64| (w * (x - k)).max(0.0);
    let mut v: Vec<f64> = (0..=steps)
        .map(|j| payoff(s * u.powi(j as i32) * d.powi((steps - j) as i32)))
        .collect();
    for i in (0..steps).rev() {
        for j in 0..=i {
            let cont = disc * (p * v[j + 1] + (1.0 - p) * v[j]);
            v[j] = if american {
                cont.max(payoff(s * u.powi(j as i32) * d.powi((i - j) as i32)))
            } else {
                cont
            };
        }
    }
    Ok(v[0])
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BondMeasures {
    pub price: f64,
    pub macaulay_duration: f64,
    pub modified_duration: f64,
    pub convexity: f64,
}

/// Obligation à taux fixe ; la maturité doit correspondre à un nombre entier de coupons.
pub fn fixed_rate_bond(face: f64, coupon: f64, mat: f64, freq: u32, y: f64) -> Result<BondMeasures, String> {
    positive("face", face)?;
    positive("maturity", mat)?;
    if freq == 0 {
        return Err("`frequency` doit être ≥ 1".into());
    }
    let f = freq as f64;
    let n = (mat * f).round();
    if (n - mat * f).abs() > 1e-9 {
        return Err(format!(
            "maturité {mat} non multiple de la période de coupon (1/{freq})"
        ));
    }
    let yp = y / f;
    if yp <= -1.0 {
        return Err("rendement invalide".into());
    }
    let (mut price, mut dur, mut conv) = (0.0, 0.0, 0.0);
    for i in 1..=n as u32 {
        let t = i as f64 / f;
        let cf = face * coupon / f + if i == n as u32 { face } else { 0.0 };
        let df = (1.0 + yp).powi(-(i as i32));
        price += cf * df;
        dur += t * cf * df;
        conv += cf * df * t * (t + 1.0 / f);
    }
    let mac = dur / price;
    Ok(BondMeasures {
        price,
        macaulay_duration: mac,
        modified_duration: mac / (1.0 + yp),
        convexity: conv / (price * (1.0 + yp).powi(2)),
    })
}

/// Volatilité implicite Black-Scholes-Merton (Newton, repli par dichotomie).
pub fn implied_vol(t: OptionType, price: f64, s: f64, k: f64, r: f64, q: f64, mat: f64) -> Result<f64, String> {
    let (mut lo, mut hi, mut x) = (1e-6, 5.0, 0.2);
    let f = |v: f64| bsm(t, s, k, r, q, v, mat).map(|g| (g.price - price, g.vega));
    let (flo, fhi) = (f(lo)?.0, f(hi)?.0);
    if flo > 0.0 || fhi < 0.0 {
        return Err("prix hors des bornes d'arbitrage".into());
    }
    for _ in 0..200 {
        let (fx, vega) = f(x)?;
        if fx.abs() < 1e-12 {
            return Ok(x);
        }
        if fx > 0.0 {
            hi = x
        } else {
            lo = x
        }
        let newton = x - fx / vega;
        x = if vega > 1e-12 && newton > lo && newton < hi {
            newton
        } else {
            0.5 * (lo + hi)
        };
        if hi - lo < 1e-14 {
            return Ok(x);
        }
    }
    Ok(x)
}

fn greeks_map(g: &Greeks) -> BTreeMap<String, f64> {
    [
        ("delta", g.delta),
        ("gamma", g.gamma),
        ("vega", g.vega),
        ("theta", g.theta),
        ("rho", g.rho),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// Valorisation d'une transaction sans données de marché (instruments auto-suffisants).
pub fn value(trade: &Trade) -> Result<Valuation, String> {
    value_in(trade, &Market::default())
}

fn map<const N: usize>(xs: [(&str, f64); N]) -> BTreeMap<String, f64> {
    xs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// Valorisation d'une transaction par le pricer de référence, avec les courbes de `market`.
pub fn value_in(trade: &Trade, market: &Market) -> Result<Valuation, String> {
    let (price, measures) = match trade.instrument {
        Instrument::EuropeanOption {
            option_type,
            spot,
            strike,
            rate,
            dividend,
            vol,
            maturity,
        } => {
            let g = bsm(option_type, spot, strike, rate, dividend, vol, maturity)?;
            (g.price, greeks_map(&g))
        }
        Instrument::AmericanOption {
            option_type,
            spot,
            strike,
            rate,
            dividend,
            vol,
            maturity,
            steps,
        } => {
            let p = crr(option_type, spot, strike, rate, dividend, vol, maturity, steps, true)?;
            let euro = bsm(option_type, spot, strike, rate, dividend, vol, maturity)?.price;
            (p, BTreeMap::from([("early_exercise_premium".to_string(), p - euro)]))
        }
        Instrument::Black76 {
            option_type,
            forward,
            strike,
            rate,
            vol,
            maturity,
        } => {
            let g = black76(option_type, forward, strike, rate, vol, maturity)?;
            (g.price, greeks_map(&g))
        }
        Instrument::FixedRateBond {
            face,
            coupon_rate,
            maturity,
            frequency,
            yield_rate,
        } => {
            let b = fixed_rate_bond(face, coupon_rate, maturity, frequency, yield_rate)?;
            let m = [
                ("macaulay_duration", b.macaulay_duration),
                ("modified_duration", b.modified_duration),
                ("convexity", b.convexity),
            ];
            (b.price, m.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
        }
        Instrument::InterestRateSwap {
            notional,
            fixed_rate,
            maturity,
            fixed_frequency,
            pay_fixed,
            ref curve,
        } => {
            let c = market.curve(curve)?;
            let npv = swap_npv(c, notional, fixed_rate, maturity, fixed_frequency, pay_fixed)?;
            let up = swap_npv(
                &c.bumped(1.0)?,
                notional,
                fixed_rate,
                maturity,
                fixed_frequency,
                pay_fixed,
            )?;
            let annuity = c.annuity(maturity, fixed_frequency)?;
            let m = map([
                ("par_rate", c.par_swap_rate(maturity, fixed_frequency)?),
                ("annuity", notional * annuity),
                ("fixed_leg_pv", notional * fixed_rate * annuity),
                ("float_leg_pv", notional * (1.0 - c.discount(maturity))),
                ("dv01", up - npv),
            ]);
            (npv, m)
        }
        Instrument::ZeroCouponBond {
            face,
            maturity,
            ref curve,
        } => {
            positive("maturity", maturity)?;
            let c = market.curve(curve)?;
            let p = face * c.discount(maturity);
            let up = face * c.bumped(1.0)?.discount(maturity);
            (p, map([("zero_rate", c.zero_rate(maturity)), ("dv01", up - p)]))
        }
    };
    Ok(Valuation {
        id: trade.id.clone(),
        product: trade.instrument.product().into(),
        price,
        measures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use OptionType::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn normal_cdf_reference_values() {
        assert!(close(norm_cdf(0.0), 0.5, 1e-15));
        assert!(close(norm_cdf(1.96), 0.975_002_104_851_780, 1e-13));
        assert!(close(norm_cdf(-3.0), 0.001_349_898_031_630_094_6, 1e-15));
    }

    #[test]
    fn bsm_hull_example() {
        // Hull, Options, Futures and Other Derivatives : S=42, K=40, r=10 %, σ=20 %, T=0,5.
        assert!(close(
            bsm(Call, 42.0, 40.0, 0.1, 0.0, 0.2, 0.5).unwrap().price,
            4.7594,
            1e-4
        ));
        assert!(close(
            bsm(Put, 42.0, 40.0, 0.1, 0.0, 0.2, 0.5).unwrap().price,
            0.8086,
            1e-4
        ));
    }

    #[test]
    fn bsm_haug_dividend_example() {
        // Haug, The Complete Guide to Option Pricing Formulas : S=100, K=95, T=0,5, r=10 %, b=5 %, σ=20 %.
        assert!(close(
            bsm(Put, 100.0, 95.0, 0.1, 0.05, 0.2, 0.5).unwrap().price,
            2.4648,
            1e-4
        ));
    }

    #[test]
    fn black76_haug_example() {
        // Haug : F=19, K=19, T=0,75, r=10 %, σ=28 % -> call = put = 1,7011.
        assert!(close(
            black76(Call, 19.0, 19.0, 0.1, 0.28, 0.75).unwrap().price,
            1.7011,
            1e-4
        ));
        assert!(close(
            black76(Put, 19.0, 19.0, 0.1, 0.28, 0.75).unwrap().price,
            1.7011,
            1e-4
        ));
    }

    #[test]
    fn crr_hull_american_put() {
        // Hull : arbre à 5 pas, S=K=50, r=10 %, σ=40 %, T=5/12 -> 4,49.
        assert!(close(
            crr(Put, 50.0, 50.0, 0.1, 0.0, 0.4, 5.0 / 12.0, 5, true).unwrap(),
            4.49,
            5e-3
        ));
    }

    #[test]
    fn crr_european_converges_to_bsm() {
        let tree = crr(Call, 100.0, 105.0, 0.03, 0.01, 0.25, 1.0, 4000, false).unwrap();
        let bs = bsm(Call, 100.0, 105.0, 0.03, 0.01, 0.25, 1.0).unwrap().price;
        assert!(close(tree, bs, 2e-3));
    }

    #[test]
    fn put_call_parity() {
        let (s, k, r, q, v, t) = (100.0, 90.0, 0.04, 0.02, 0.3, 2.0);
        let c = bsm(Call, s, k, r, q, v, t).unwrap().price;
        let p = bsm(Put, s, k, r, q, v, t).unwrap().price;
        assert!(close(c - p, s * (-q * t).exp() - k * (-r * t).exp(), 1e-12));
    }

    #[test]
    fn greeks_match_finite_differences() {
        let g = bsm(Call, 100.0, 100.0, 0.05, 0.0, 0.2, 1.0).unwrap();
        let h = 1e-4;
        let up = bsm(Call, 100.0 + h, 100.0, 0.05, 0.0, 0.2, 1.0).unwrap().price;
        let dn = bsm(Call, 100.0 - h, 100.0, 0.05, 0.0, 0.2, 1.0).unwrap().price;
        assert!(close((up - dn) / (2.0 * h), g.delta, 1e-7));
        let vu = bsm(Call, 100.0, 100.0, 0.05, 0.0, 0.2 + h, 1.0).unwrap().price;
        let vd = bsm(Call, 100.0, 100.0, 0.05, 0.0, 0.2 - h, 1.0).unwrap().price;
        assert!(close((vu - vd) / (2.0 * h), g.vega, 1e-6));
    }

    #[test]
    fn bond_par_and_zero_coupon() {
        assert!(close(
            fixed_rate_bond(100.0, 0.05, 10.0, 2, 0.05).unwrap().price,
            100.0,
            1e-10
        ));
        let z = fixed_rate_bond(100.0, 0.0, 5.0, 1, 0.04).unwrap();
        assert!(close(z.price, 100.0 / 1.04f64.powi(5), 1e-10) && close(z.macaulay_duration, 5.0, 1e-12));
        assert!(fixed_rate_bond(100.0, 0.05, 2.3, 2, 0.05).is_err());
    }

    #[test]
    fn implied_vol_roundtrip() {
        let p = bsm(Put, 80.0, 95.0, 0.02, 0.0, 0.37, 0.75).unwrap().price;
        assert!(close(
            implied_vol(Put, p, 80.0, 95.0, 0.02, 0.0, 0.75).unwrap(),
            0.37,
            1e-9
        ));
    }

    #[test]
    fn trade_json_roundtrip() {
        let t: Trade = serde_json::from_str(
            r#"{"id":"T1","product":"european_option","option_type":"call","spot":42,"strike":40,"rate":0.1,"vol":0.2,"maturity":0.5}"#,
        )
        .unwrap();
        assert!(close(value(&t).unwrap().price, 4.7594, 1e-4));
    }
}
