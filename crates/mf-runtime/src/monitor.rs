//! Monitoring rules: alerts on the series of a pipeline's results, date by date (dated pipelines)
//! or run by run. A rule holds when a value is one of a set of verdicts, above or below a
//! threshold, or changes; it is raised once it has held for `consecutive` dates in a row.

use crate::dated::{label, Series};
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn one() -> usize {
    1
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AlertRule {
    pub name: String,
    /// Output path, in diff notation (`backtest.latest_basel_window.zone`).
    pub path: String,
    /// The value is one of these (`["fail"]`, `["red", "yellow"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub one_of: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub above: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<f64>,
    /// The value differs from the previous date.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub changes: bool,
    /// Number of dates in a row the condition must hold before the alert is raised.
    #[serde(default = "one")]
    pub consecutive: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl AlertRule {
    pub fn check(&self) -> Result<(), String> {
        let conditions = [
            !self.one_of.is_empty(),
            self.above.is_some(),
            self.below.is_some(),
            self.changes,
        ];
        if conditions.iter().filter(|c| **c).count() != 1 {
            return Err(format!(
                "alert {}: give exactly one of `one_of`, `above`, `below`, `changes`",
                self.name
            ));
        }
        if self.consecutive == 0 {
            return Err(format!("alert {}: `consecutive` must be at least 1", self.name));
        }
        if self.changes && self.consecutive != 1 {
            return Err(format!(
                "alert {}: `consecutive` does not apply to `changes`",
                self.name
            ));
        }
        if self.path.contains(['*', '?']) {
            return Err(format!(
                "alert {}: the path names one output, without wildcards",
                self.name
            ));
        }
        Ok(())
    }

    pub fn describe(&self) -> String {
        let cond = if !self.one_of.is_empty() {
            format!("{} is {}", self.path, self.one_of.join(" or "))
        } else if let Some(x) = self.above {
            format!("{} > {x}", self.path)
        } else if let Some(x) = self.below {
            format!("{} < {x}", self.path)
        } else {
            format!("{} changes", self.path)
        };
        match self.consecutive {
            1 => cond,
            n => format!("{cond}, {n} dates in a row"),
        }
    }

    fn holds(&self, v: Option<&Value>, prev: Option<&Value>) -> bool {
        let Some(v) = v else { return false };
        if self.changes {
            return prev.is_some_and(|p| p != v);
        }
        if !self.one_of.is_empty() {
            let text = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            return self.one_of.contains(&text);
        }
        match (v.as_f64(), self.above, self.below) {
            (Some(x), Some(t), _) => x > t,
            (Some(x), _, Some(t)) => x < t,
            _ => false,
        }
    }
}

pub fn check_rules(rules: &[AlertRule]) -> Result<(), String> {
    for (i, r) in rules.iter().enumerate() {
        r.check()?;
        if rules[..i].iter().any(|o| o.name == r.name) {
            return Err(format!("alert {}: duplicate name", r.name));
        }
    }
    Ok(())
}

/// A period during which a rule held long enough to be raised.
#[derive(Debug, Clone, Serialize)]
pub struct Episode {
    /// First date of the period.
    pub start: String,
    /// Date the alert was raised (the `consecutive`-th date of the period).
    pub raised: String,
    /// Last date the condition held.
    pub last: String,
    pub dates: usize,
    /// The condition still holds at the latest date.
    pub ongoing: bool,
    /// Value on the date the alert was raised (and the value before it, for `changes`).
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertState {
    pub rule: AlertRule,
    pub condition: String,
    /// The rule holds at the latest date and has been raised.
    pub active: bool,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertReport {
    pub pipeline: String,
    pub dates: usize,
    pub latest: Option<String>,
    pub alerts: Vec<AlertState>,
}

impl AlertReport {
    pub fn active(&self) -> usize {
        self.alerts.iter().filter(|a| a.active).count()
    }

    /// Alerts raised on `date` (an as-of date, or `#ID` for undated pipelines).
    pub fn raised_on<'a>(&'a self, date: &'a str) -> impl Iterator<Item = (&'a AlertState, &'a Episode)> + 'a {
        self.alerts
            .iter()
            .flat_map(move |a| a.episodes.iter().filter(move |e| e.raised == date).map(move |e| (a, e)))
    }
}

/// Paths a set of rules reads, in rule order.
pub fn paths(rules: &[AlertRule]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for r in rules {
        if !out.contains(&r.path) {
            out.push(r.path.clone());
        }
    }
    out
}

/// Evaluates the rules on a series that contains their paths (`dated::series(.., &paths(rules))`).
pub fn evaluate(rules: &[AlertRule], s: &Series) -> AlertReport {
    let alerts = rules
        .iter()
        .map(|rule| {
            let j = s.columns.iter().position(|c| *c == rule.path);
            let value = |i: usize| j.and_then(|j| s.rows[i].values[j].as_ref());
            let mut episodes: Vec<Episode> = vec![];
            let mut streak: Option<usize> = None; // first row of the current period
            for i in 0..s.rows.len() {
                let prev = if i > 0 { value(i - 1) } else { None };
                if !rule.holds(value(i), prev) {
                    streak = None;
                    continue;
                }
                let start = *streak.get_or_insert(i);
                let n = i - start + 1;
                if n == rule.consecutive {
                    episodes.push(Episode {
                        start: label(&s.rows[start]),
                        raised: label(&s.rows[i]),
                        last: label(&s.rows[i]),
                        dates: n,
                        ongoing: false,
                        value: value(i).cloned(),
                        previous: if rule.changes { prev.cloned() } else { None },
                    });
                    if rule.changes {
                        streak = None; // every change is an alert of its own
                    }
                } else if n > rule.consecutive {
                    let e = episodes.last_mut().unwrap();
                    e.last = label(&s.rows[i]);
                    e.dates = n;
                }
            }
            let last = s.rows.len().checked_sub(1);
            let active = match (episodes.last_mut(), last) {
                (Some(e), Some(l)) if e.last == label(&s.rows[l]) => {
                    e.ongoing = true;
                    true
                }
                _ => false,
            };
            AlertState {
                rule: rule.clone(),
                condition: rule.describe(),
                active,
                episodes,
            }
        })
        .collect();
    AlertReport {
        pipeline: s.pipeline.clone(),
        dates: s.rows.len(),
        latest: s.rows.last().map(label),
        alerts,
    }
}

fn short(v: &Option<Value>) -> String {
    match v {
        None => "-".into(),
        Some(Value::String(t)) => t.clone(),
        Some(Value::Number(n)) => n.as_f64().map(crate::diff::num).unwrap_or_else(|| n.to_string()),
        Some(other) => other.to_string(),
    }
}

fn period(e: &Episode) -> String {
    let end = if e.ongoing {
        "ongoing".to_string()
    } else {
        e.last.clone()
    };
    let raised = if e.raised != e.start {
        format!(", raised {}", e.raised)
    } else {
        String::new()
    };
    let value = match &e.previous {
        Some(p) => format!("{} → {}", short(&Some(p.clone())), short(&e.value)),
        None => short(&e.value),
    };
    if e.start == e.last && !e.ongoing {
        format!("{}  {value}", e.start)
    } else {
        format!("{} → {end} ({} dates{raised})  {value}", e.start, e.dates)
    }
}

/// One line per alert raised on `date`, for the output of `mf run` and the webhooks.
pub fn line(a: &AlertState, e: &Episode) -> String {
    // The value, unless the condition already names it (`zone is red`).
    let what = match &e.previous {
        Some(p) => format!(" ({} → {})", short(&Some(p.clone())), short(&e.value)),
        None if a.rule.one_of.len() == 1 => String::new(),
        None => format!(" ({})", short(&e.value)),
    };
    let msg = a.rule.message.as_deref().map(|m| format!(" — {m}")).unwrap_or_default();
    format!("ALERT {} on {}: {}{what}{msg}", a.rule.name, e.raised, a.condition)
}

pub fn render_text(r: &AlertReport) -> String {
    let mut o = format!(
        "{}: {} alert rule{}, {} date{}{}\n",
        r.pipeline,
        r.alerts.len(),
        if r.alerts.len() == 1 { "" } else { "s" },
        r.dates,
        if r.dates == 1 { "" } else { "s" },
        r.latest.as_deref().map(|d| format!(", latest {d}")).unwrap_or_default()
    );
    for a in &r.alerts {
        let state = if a.active { "ACTIVE" } else { "ok" };
        o += &format!(
            "\n{state:<7}{}  {}  ({} episode{})\n",
            a.rule.name,
            a.condition,
            a.episodes.len(),
            if a.episodes.len() == 1 { "" } else { "s" }
        );
        if let Some(m) = &a.rule.message {
            o += &format!("       {m}\n");
        }
        for e in &a.episodes {
            o += &format!("       {}\n", period(e));
        }
    }
    o += &format!(
        "\n{}\n",
        match r.active() {
            0 => "no active alert".to_string(),
            1 => "1 active alert".to_string(),
            n => format!("{n} active alerts"),
        }
    );
    o
}

pub fn render_markdown(r: &AlertReport) -> String {
    let mut o = format!(
        "**{}** — {} active alert(s), {} date(s){}\n\n| Alert | Condition | State | Episodes | Periods |\n|---|---|---|---:|---|\n",
        r.pipeline,
        r.active(),
        r.dates,
        r.latest.as_deref().map(|d| format!(", latest {d}")).unwrap_or_default()
    );
    for a in &r.alerts {
        let periods: Vec<String> = a.episodes.iter().map(period).collect();
        o += &format!(
            "| {} | `{}` | {} | {} | {} |\n",
            a.rule.name,
            a.condition.replace('|', "\\|"),
            if a.active { "**active**" } else { "ok" },
            a.episodes.len(),
            periods.join("<br>").replace('|', "\\|")
        );
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dated::SeriesRow;
    use serde_json::json;

    fn series(values: &[Value]) -> Series {
        Series {
            pipeline: "p".into(),
            columns: vec!["drift.status".into()],
            rows: values
                .iter()
                .enumerate()
                .map(|(i, v)| SeriesRow {
                    as_of: format!("2024-{:02}", i + 1),
                    run_id: i as i64 + 1,
                    status: None,
                    values: vec![(!v.is_null()).then(|| v.clone())],
                })
                .collect(),
        }
    }

    fn rule(toml_text: &str) -> AlertRule {
        let r: AlertRule = toml::from_str(&format!("name='a'\npath='drift.status'\n{toml_text}")).unwrap();
        r.check().unwrap();
        r
    }

    #[test]
    fn consecutive_dates() {
        let s = series(&[
            json!("fail"),
            json!("fail"),
            json!("warn"),
            json!("fail"),
            json!("fail"),
            json!("fail"),
            json!("fail"),
        ]);
        let r = evaluate(&[rule("one_of=['fail']\nconsecutive=3")], &s);
        let a = &r.alerts[0];
        assert!(a.active);
        assert_eq!(a.episodes.len(), 1);
        let e = &a.episodes[0];
        assert_eq!(
            (e.start.as_str(), e.raised.as_str(), e.dates),
            ("2024-04", "2024-06", 4)
        );
        assert!(e.ongoing);
        assert_eq!(r.raised_on("2024-06").count(), 1);
        assert_eq!(r.raised_on("2024-07").count(), 0);
        // The first two fails are too short to raise the alert.
        let once = evaluate(&[rule("one_of=['fail']")], &s);
        assert_eq!(once.alerts[0].episodes.len(), 2);
        assert_eq!(once.alerts[0].episodes[0].last, "2024-02");
    }

    #[test]
    fn thresholds_changes_and_gaps() {
        let s = series(&[json!(0.02), json!(0.06), Value::Null, json!(0.07), json!(0.03)]);
        let r = evaluate(&[rule("above=0.05")], &s);
        // A missing value ends the period.
        assert_eq!(r.alerts[0].episodes.len(), 2);
        assert!(!r.alerts[0].active);
        let s = series(&[json!("green"), json!("green"), json!("yellow"), json!("red")]);
        let r = evaluate(&[rule("changes=true")], &s);
        let e = &r.alerts[0].episodes;
        assert_eq!(e.len(), 2);
        assert_eq!(
            (e[0].start.as_str(), e[0].previous.clone()),
            ("2024-03", Some(json!("green")))
        );
        assert_eq!(
            (e[1].raised.as_str(), e[1].value.clone()),
            ("2024-04", Some(json!("red")))
        );
        assert!(r.alerts[0].active);
        assert!(render_text(&r).contains("ACTIVE"));
        let missing = evaluate(
            &[AlertRule {
                path: "nope.x".into(),
                ..rule("one_of=['fail']")
            }],
            &s,
        );
        assert!(missing.alerts[0].episodes.is_empty());
    }

    #[test]
    fn rejects_bad_rules() {
        let bad = |t: &str| {
            toml::from_str::<AlertRule>(&format!("name='a'\npath='x.y'\n{t}"))
                .map_err(|e| e.to_string())
                .and_then(|r| r.check())
                .is_err()
        };
        assert!(bad(""));
        assert!(bad("above=1\nbelow=0"));
        assert!(bad("one_of=['fail']\nconsecutive=0"));
        assert!(bad("changes=true\nconsecutive=2"));
        assert!(bad("above=1\nwindow=3"));
        assert!(!bad("below=0.5\nconsecutive=2"));
        let dup = vec![rule("above=1"), rule("below=1")];
        assert!(check_rules(&dup).unwrap_err().contains("duplicate"));
    }
}
