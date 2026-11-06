//! Alerts: webhook notification (JSON format compatible with Slack, Mattermost, Teams via Workflows)
//! when a run fails or its checks degrade.

use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

fn default_on() -> Vec<String> {
    vec!["fail".into(), "error".into()]
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlertsConfig {
    pub webhook_url: Option<String>,
    /// Environment variable holding the URL (webhook URLs often contain a secret).
    pub webhook_url_env: Option<String>,
    /// Triggering events: `fail`, `warn`, `error` (run failure).
    #[serde(default = "default_on")]
    pub on: Vec<String>,
}

impl AlertsConfig {
    pub fn url(&self) -> Option<String> {
        self.webhook_url
            .clone()
            .or_else(|| self.webhook_url_env.as_ref().and_then(|v| std::env::var(v).ok()))
    }
}

pub struct Event<'a> {
    pub pipeline: &'a str,
    pub run_id: Option<i64>,
    /// `fail`, `warn`, `pass` or `error`.
    pub kind: &'a str,
    pub detail: &'a str,
    pub triggered_by: &'a str,
    pub link: Option<String>,
}

/// Sends the alert if the event is selected. Returns `Ok(false)` if nothing is sent.
pub fn notify(cfg: &AlertsConfig, e: &Event) -> Result<bool, String> {
    if !cfg.on.iter().any(|k| k == e.kind) {
        return Ok(false);
    }
    let Some(url) = cfg.url() else { return Ok(false) };
    let what = match e.kind {
        "fail" => "checks failed",
        "warn" => "checks with warnings",
        "error" => "run failed",
        other => other,
    };
    let run = e.run_id.map(|i| format!(" #{i}")).unwrap_or_default();
    let mut text = format!(
        "ModelFlow — {}{run}: {what} (triggered by {}).",
        e.pipeline, e.triggered_by
    );
    if !e.detail.is_empty() {
        text.push_str(&format!(" {}", e.detail));
    }
    if let Some(l) = &e.link {
        text.push_str(&format!(" {l}"));
    }
    let body = json!({"text": text, "pipeline": e.pipeline, "run_id": e.run_id, "status": e.kind,
                      "triggered_by": e.triggered_by, "link": e.link});
    post(&url, &body)?;
    Ok(true)
}

/// Sends an alert raised by a monitoring rule (`[[alert]]` in the pipeline): the rules are chosen
/// in the pipeline, so they are sent whatever `on` lists. Returns `Ok(false)` without a URL.
pub fn notify_rule(
    cfg: &AlertsConfig,
    pipeline: &str,
    run_id: i64,
    state: &mf_runtime::monitor::AlertState,
    episode: &mf_runtime::monitor::Episode,
    link: Option<String>,
) -> Result<bool, String> {
    let Some(url) = cfg.url() else { return Ok(false) };
    let mut text = format!(
        "ModelFlow — {pipeline} #{run_id}: {}",
        mf_runtime::monitor::line(state, episode)
    );
    if let Some(l) = &link {
        text.push_str(&format!(" {l}"));
    }
    let body = json!({"text": text, "pipeline": pipeline, "run_id": run_id, "status": "alert",
                      "alert": state.rule.name, "condition": state.condition, "date": episode.raised,
                      "since": episode.start, "value": episode.value, "link": link});
    post(&url, &body)?;
    Ok(true)
}

/// Escalation of a validation finding past its remediation deadline (sent once per deadline).
pub fn notify_finding(
    cfg: &AlertsConfig,
    f: &crate::governance::Finding,
    days_overdue: i64,
    link: Option<String>,
) -> Result<bool, String> {
    let Some(url) = cfg.url() else { return Ok(false) };
    let mut text = format!(
        "ModelFlow — finding #{} on {} ({}) is {days_overdue} days overdue: {} (due {}, owner {}).",
        f.id, f.model, f.severity, f.title, f.due_date, f.owner
    );
    if let Some(l) = &link {
        text.push_str(&format!(" {l}"));
    }
    let body = json!({"text": text, "status": "finding_overdue", "finding_id": f.id, "model": f.model,
                      "severity": f.severity, "due_date": f.due_date, "owner": f.owner,
                      "days_overdue": days_overdue, "link": link});
    post(&url, &body)?;
    Ok(true)
}

fn post(url: &str, body: &serde_json::Value) -> Result<(), String> {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build()
        .post(url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|err| format!("webhook: {err}"))?;
    Ok(())
}
