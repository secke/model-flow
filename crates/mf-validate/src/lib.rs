//! Tests de validation de modèles : risque de marché (VaR), risque de crédit (PD), dérive des données.
//! Chaque résultat porte un `status` (`pass` / `warn` / `fail`).

use serde::{Deserialize, Serialize};

pub mod credit;
pub mod drift;
pub mod stats;
pub mod var;

pub use credit::{calibration, discrimination, Calibration, Discrimination, GradeCalibration};
pub use drift::{psi, Psi};
pub use var::{
    christoffersen, historical_var, kupiec_pof, rolling_backtest, traffic_light, var_backtest, Christoffersen, Kupiec,
    TrafficLight, VarBacktest,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
}

impl Status {
    /// Statut d'un test d'hypothèse : rejet à 1 % = échec, à 5 % = alerte.
    pub fn from_p_value(p: f64) -> Self {
        if p < 0.01 {
            Status::Fail
        } else if p < 0.05 {
            Status::Warn
        } else {
            Status::Pass
        }
    }
}
