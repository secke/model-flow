//! Validation findings: raised by a validator on a model (and optionally on the run that shows
//! them), remediated by the model owner with a run as evidence, closed by a validator. Overdue
//! findings are escalated once; open critical and high findings block the model's approval.
//!
//! Lifecycle: `open` → (`remediate`, owner) → `remediated` → (`close`, validator) → `closed`, or
//! back to `open` (`reopen`, validator). A validator can also accept the risk (`risk_accepted`)
//! with a justification, and extend the deadline. Every step is an event of the finding and an
//! entry of the audit log.

use crate::governance::{Finding, Gov, FINDING_STATUSES, SEVERITIES};
use crate::{
    alerts, authorize, bad_request, blocking, find_model, locate, verify_run, ApiError, ApiResult, AppState, Auth, Role,
};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use mf_runtime::spec;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Default remediation deadline, in days after the finding is raised.
pub fn default_due_days(severity: &str) -> i64 {
    match severity {
        "critical" => 30,
        "high" => 90,
        "medium" => 180,
        _ => 365,
    }
}

/// Severities that block the approval of a model while the finding is not closed.
pub fn blocks_approval(f: &Finding) -> bool {
    ["critical", "high"].contains(&f.severity.as_str()) && is_active(f)
}

/// Open or remediated (awaiting the closure review).
pub fn is_active(f: &Finding) -> bool {
    f.status == "open" || f.status == "remediated"
}

/// Days past the deadline of an open finding (`None` if not overdue). A remediated finding waits
/// for the validator, not for the owner: it is not overdue.
pub fn days_overdue(f: &Finding, today: &str) -> Option<i64> {
    if f.status != "open" || f.due_date.as_str() >= today {
        return None;
    }
    let (d, t) = (date(&f.due_date)?, date(today)?);
    Some((t - d).num_days())
}

fn date(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

pub fn today() -> String {
    chrono::Utc::now().date_naive().format("%Y-%m-%d").to_string()
}

fn with_status(f: &Finding, today: &str) -> Value {
    let mut v = serde_json::to_value(f).unwrap();
    v["overdue_days"] = json!(days_overdue(f, today));
    v["blocks_approval"] = json!(blocks_approval(f));
    v
}

/// Counts of active findings for one model: active, overdue, blocking the approval.
pub fn model_counts(findings: &[Finding], model: &str, today: &str) -> Value {
    let mine: Vec<&Finding> = findings.iter().filter(|f| f.model == model).collect();
    json!({
        "active": mine.iter().filter(|f| is_active(f)).count(),
        "overdue": mine.iter().filter(|f| days_overdue(f, today).is_some()).count(),
        "blocking": mine.iter().filter(|f| blocks_approval(f)).count(),
    })
}

// ---------- Governance report ----------

/// Governance summary of the findings at `today`: counts by status and severity, active findings
/// (overdue first), closures of the last 90 days, by model.
pub fn report(findings: &[Finding], today: &str, now_ts: i64) -> Value {
    let mut counts: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    for st in FINDING_STATUSES {
        counts.insert(st, SEVERITIES.iter().map(|s| (*s, 0)).collect());
    }
    for f in findings {
        if let Some(c) = counts
            .get_mut(f.status.as_str())
            .and_then(|m| m.get_mut(f.severity.as_str()))
        {
            *c += 1;
        }
    }
    let rank = |s: &str| SEVERITIES.iter().position(|x| *x == s).unwrap_or(9);
    let mut active: Vec<&Finding> = findings.iter().filter(|f| is_active(f)).collect();
    active.sort_by_key(|f| {
        (
            days_overdue(f, today).is_none(),
            rank(&f.severity),
            f.due_date.clone(),
            f.id,
        )
    });
    let age = |f: &Finding| (now_ts - f.created_at) / 86_400;
    let recent: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.closed_at.is_some_and(|c| now_ts - c <= 90 * 86_400))
        .collect();
    let mean_days = |v: &[&Finding]| {
        (!v.is_empty()).then(|| {
            v.iter()
                .map(|f| (f.closed_at.unwrap() - f.created_at) as f64 / 86_400.0)
                .sum::<f64>()
                / v.len() as f64
        })
    };
    let mut by_model: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    for f in &active {
        *by_model.entry(&f.model).or_default().entry(&f.severity).or_default() += 1;
    }
    json!({
        "as_of": today,
        "counts": counts,
        "active": active.iter().map(|f| json!({
            "id": f.id, "model": f.model, "severity": f.severity, "status": f.status, "title": f.title,
            "owner": f.owner, "due_date": f.due_date, "overdue_days": days_overdue(f, today),
            "age_days": age(f), "escalated": f.escalated_at.is_some(), "blocks_approval": blocks_approval(f),
        })).collect::<Vec<_>>(),
        "overdue": active.iter().filter(|f| days_overdue(f, today).is_some()).count(),
        "awaiting_review": active.iter().filter(|f| f.status == "remediated").count(),
        "closed_last_90_days": recent.len(),
        "mean_days_to_close_last_90_days": mean_days(&recent),
        "active_by_model": by_model,
    })
}

pub fn report_markdown(r: &Value) -> String {
    let mut out = format!(
        "# Validation findings — status at {}\n\n",
        r["as_of"].as_str().unwrap_or("")
    );
    out += &format!(
        "{} active findings, {} overdue, {} awaiting the closure review. {} closed or accepted in the last 90 days",
        r["active"].as_array().map_or(0, Vec::len),
        r["overdue"],
        r["awaiting_review"],
        r["closed_last_90_days"]
    );
    match r["mean_days_to_close_last_90_days"].as_f64() {
        Some(d) => out += &format!(", in {d:.0} days on average.\n\n"),
        None => out += ".\n\n",
    }
    out += "| Status | Critical | High | Medium | Low |\n|---|---:|---:|---:|---:|\n";
    for st in FINDING_STATUSES {
        let c = &r["counts"][*st];
        out += &format!(
            "| {st} | {} | {} | {} | {} |\n",
            c["critical"], c["high"], c["medium"], c["low"]
        );
    }
    out += "\n## Active findings\n\n";
    let active = r["active"].as_array().cloned().unwrap_or_default();
    if active.is_empty() {
        out += "None.\n";
    } else {
        out +=
            "| # | Model | Severity | Status | Title | Owner | Due | Overdue (days) | Age (days) | Blocks approval |\n";
        out += "|---:|---|---|---|---|---|---|---:|---:|---|\n";
        for f in active {
            out += &format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                f["id"],
                f["model"].as_str().unwrap_or(""),
                f["severity"].as_str().unwrap_or(""),
                f["status"].as_str().unwrap_or(""),
                f["title"].as_str().unwrap_or("").replace('|', "/"),
                f["owner"].as_str().unwrap_or(""),
                f["due_date"].as_str().unwrap_or(""),
                f["overdue_days"].as_i64().map(|d| d.to_string()).unwrap_or_default(),
                f["age_days"],
                if f["blocks_approval"].as_bool() == Some(true) {
                    "yes"
                } else {
                    ""
                },
            );
        }
    }
    out
}

// ---------- Escalation ----------

/// Escalates every open finding past its deadline that has not been escalated yet: webhook (when
/// configured), audit entry and finding event. Returns the escalated ids.
pub fn escalate_overdue(st: &AppState, today: &str) -> Result<Vec<i64>, String> {
    let gov = st.gov().map_err(|e| e.1)?;
    let mut done = vec![];
    for mut f in gov.findings()? {
        let Some(days) = days_overdue(&f, today) else { continue };
        if f.escalated_at.is_some() {
            continue;
        }
        let link =
            st.0.public_url
                .as_ref()
                .map(|u| format!("{}/#/findings/{}", u.trim_end_matches('/'), f.id));
        let outcome = match &st.0.alerts {
            Some(cfg) => match alerts::notify_finding(cfg, &f, days, link) {
                Ok(true) => "notified".to_string(),
                Ok(false) => "no webhook".to_string(),
                Err(e) => format!("error: {e}"),
            },
            None => "no webhook".to_string(),
        };
        f.escalated_at = Some(chrono::Utc::now().timestamp());
        gov.update_finding(&f)?;
        // The event is visible to every user: no transport detail, which stays in the audit log.
        let sent = if outcome.starts_with("error") {
            "notification failed"
        } else {
            outcome.as_str()
        };
        let note = format!("{days} days overdue (due {}), {sent}", f.due_date);
        gov.finding_event(f.id, "system", "escalate", &f.status, &note)?;
        gov.audit(
            "system",
            Role::Admin,
            "finding.escalate",
            &format!("finding#{}", f.id),
            &outcome,
        )?;
        done.push(f.id);
    }
    Ok(done)
}

// ---------- HTTP ----------

#[derive(Deserialize)]
pub struct ListQuery {
    model: Option<String>,
    status: Option<String>,
    severity: Option<String>,
    #[serde(default)]
    overdue: bool,
}

pub async fn list(State(st): State<AppState>, _auth: Auth, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    blocking(move || {
        let t = today();
        let out: Vec<Value> = st
            .gov()?
            .findings()
            .map_err(ApiError::internal)?
            .iter()
            .filter(|f| q.model.as_ref().is_none_or(|m| &f.model == m))
            .filter(|f| {
                q.status
                    .as_ref()
                    .is_none_or(|s| &f.status == s || (s == "active" && is_active(f)))
            })
            .filter(|f| q.severity.as_ref().is_none_or(|s| &f.severity == s))
            .filter(|f| !q.overdue || days_overdue(f, &t).is_some())
            .map(|f| with_status(f, &t))
            .collect();
        Ok(Json(json!({"as_of": t, "findings": out})))
    })
    .await
}

#[derive(Deserialize)]
pub struct NewFinding {
    model: String,
    title: String,
    description: String,
    severity: String,
    pipeline: Option<String>,
    run_id: Option<i64>,
    step: Option<String>,
    due_date: Option<String>,
    owner: Option<String>,
}

pub async fn create(State(st): State<AppState>, auth: Auth, Json(b): Json<NewFinding>) -> ApiResult<Response> {
    authorize(&st, &auth, &[Role::Validator], "finding.raise", &b.model).await?;
    if b.title.trim().is_empty() || b.description.trim().is_empty() {
        return Err(bad_request("`title` and `description` are required"));
    }
    if !SEVERITIES.contains(&b.severity.as_str()) {
        return Err(bad_request(format!("`severity`: {}", SEVERITIES.join(", "))));
    }
    if b.run_id.is_some() && b.pipeline.is_none() {
        return Err(bad_request("`run_id` needs `pipeline`"));
    }
    let due = match &b.due_date {
        Some(d) => {
            date(d).ok_or_else(|| bad_request("`due_date`: date YYYY-MM-DD"))?;
            d.clone()
        }
        None => (chrono::Utc::now().date_naive() + chrono::Duration::days(default_due_days(&b.severity)))
            .format("%Y-%m-%d")
            .to_string(),
    };
    blocking(move || {
        let gov = st.gov()?;
        let m = find_model(&gov, &b.model)?;
        if let Some(p) = &b.pipeline {
            let toml = locate(&st, p)?;
            if let Some(id) = b.run_id {
                st.store_for(&toml)?
                    .info(p, id)
                    .map_err(ApiError::internal)?
                    .ok_or_else(|| ApiError::not_found(format!("run #{id} not found")))?;
            }
        }
        let f = Finding {
            id: 0,
            model: m.id.clone(),
            pipeline: b.pipeline,
            run_id: b.run_id,
            step: b.step.filter(|s| !s.trim().is_empty()),
            title: b.title.trim().into(),
            description: b.description.trim().into(),
            severity: b.severity,
            status: "open".into(),
            owner: b.owner.filter(|o| !o.trim().is_empty()).unwrap_or(m.owner),
            raised_by: auth.name.clone(),
            due_date: due,
            remediation: String::new(),
            evidence: None,
            remediated_by: None,
            closed_by: None,
            closed_at: None,
            escalated_at: None,
            created_at: 0,
            updated_at: 0,
        };
        let id = gov.create_finding(&f, &auth.name).map_err(ApiError::internal)?;
        gov.audit(
            &auth.name,
            auth.role,
            "finding.raise",
            &format!("finding#{id}"),
            &f.severity,
        )
        .map_err(ApiError::internal)?;
        let f = gov.finding(id).map_err(ApiError::internal)?.unwrap();
        Ok((StatusCode::CREATED, Json(with_status(&f, &today()))).into_response())
    })
    .await
}

fn load(gov: &Gov, id: i64) -> ApiResult<Finding> {
    gov.finding(id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found(format!("finding #{id} not found")))
}

pub async fn get(State(st): State<AppState>, _auth: Auth, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    blocking(move || {
        let gov = st.gov()?;
        let f = load(&gov, id)?;
        let mut v = with_status(&f, &today());
        v["events"] = json!(gov.finding_events(id).map_err(ApiError::internal)?);
        Ok(Json(v))
    })
    .await
}

#[derive(Deserialize)]
pub struct ActionBody {
    action: String,
    #[serde(default)]
    comment: String,
    due_date: Option<String>,
    /// `pipeline#run`: the run that evidences a remediation.
    evidence: Option<String>,
}

pub const ACTIONS: &[&str] = &["comment", "plan", "remediate", "reopen", "close", "accept", "extend"];

pub async fn act(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(b): Json<ActionBody>,
) -> ApiResult<Json<Value>> {
    let target = format!("finding#{id}");
    let roles: &[Role] = match b.action.as_str() {
        "comment" => &[Role::Validator, Role::ModelOwner, Role::Admin],
        "plan" | "remediate" => &[Role::ModelOwner, Role::Admin],
        "reopen" | "close" | "accept" | "extend" => &[Role::Validator],
        _ => return Err(bad_request(format!("`action`: {}", ACTIONS.join(", ")))),
    };
    authorize(&st, &auth, roles, &format!("finding.{}", b.action), &target).await?;
    if b.comment.trim().is_empty() {
        return Err(bad_request("a `comment` is required"));
    }
    blocking(move || {
        let gov = st.gov()?;
        let mut f = load(&gov, id)?;
        let action = format!("finding.{}", b.action);
        let refuse = |code: StatusCode, why: String| {
            let _ = gov.audit(&auth.name, auth.role, &action, &target, &format!("refused: {why}"));
            Err(ApiError::new(code, why))
        };
        let need = |states: &[&str]| -> Result<(), String> {
            if states.contains(&f.status.as_str()) {
                Ok(())
            } else {
                Err(format!("`{}` is not possible: the finding is {}", b.action, f.status))
            }
        };
        let states: &[&str] = match b.action.as_str() {
            "comment" => FINDING_STATUSES,
            "plan" | "remediate" => &["open"],
            "reopen" | "close" => &["remediated"],
            _ => &["open", "remediated"],
        };
        if let Err(why) = need(states) {
            return refuse(StatusCode::CONFLICT, why);
        }
        // Only the owner of the remediation (or an administrator) remediates.
        if ["plan", "remediate"].contains(&b.action.as_str()) && auth.role != Role::Admin && f.owner != auth.name {
            return refuse(
                StatusCode::FORBIDDEN,
                format!("only {} (the remediation owner) can do this", f.owner),
            );
        }
        match b.action.as_str() {
            "plan" => f.remediation = b.comment.trim().into(),
            "remediate" => {
                if let Some(e) = &b.evidence {
                    let Some((p, run)) = e
                        .split_once('#')
                        .and_then(|(p, r)| r.parse::<i64>().ok().map(|r| (p, r)))
                    else {
                        return refuse(StatusCode::BAD_REQUEST, "`evidence`: `pipeline#run`".into());
                    };
                    let toml = locate(&st, p)?;
                    let declares = spec::load(&toml).ok().and_then(|(sp, _)| sp.model) == Some(f.model.clone());
                    if !declares {
                        return refuse(
                            StatusCode::CONFLICT,
                            format!("pipeline {p} does not declare model {}", f.model),
                        );
                    }
                    if st.store_for(&toml)?.info(p, run).map_err(ApiError::internal)?.is_none() {
                        return Err(ApiError::not_found(format!("run #{run} not found")));
                    }
                    if !verify_run(&st, &toml, p, run)?.is_empty() {
                        return refuse(
                            StatusCode::CONFLICT,
                            format!("{e} does not replay identically: not evidence"),
                        );
                    }
                    f.evidence = Some(e.clone());
                }
                f.status = "remediated".into();
                f.remediated_by = Some(auth.name.clone());
            }
            "reopen" => {
                f.status = "open".into();
                f.evidence = None;
                f.remediated_by = None;
            }
            "close" | "accept" => {
                if f.remediated_by.as_deref() == Some(auth.name.as_str()) {
                    return refuse(
                        StatusCode::CONFLICT,
                        "segregation of duties: you remediated this finding".into(),
                    );
                }
                f.status = if b.action == "close" { "closed" } else { "risk_accepted" }.into();
                f.closed_by = Some(auth.name.clone());
                f.closed_at = Some(chrono::Utc::now().timestamp());
            }
            "extend" => {
                let Some(d) = b.due_date.as_deref().filter(|d| date(d).is_some()) else {
                    return refuse(
                        StatusCode::BAD_REQUEST,
                        "`due_date`: the new deadline, YYYY-MM-DD".into(),
                    );
                };
                if d <= f.due_date.as_str() {
                    return refuse(
                        StatusCode::BAD_REQUEST,
                        format!("the new deadline must be after {}", f.due_date),
                    );
                }
                f.due_date = d.into();
                f.escalated_at = None; // a new deadline is escalated again if missed
            }
            _ => {}
        }
        let comment = match (&b.action[..], &b.evidence, &b.due_date) {
            ("remediate", Some(e), _) => format!("{} (evidence: {e})", b.comment.trim()),
            ("extend", _, Some(d)) => format!("{} (new deadline {d})", b.comment.trim()),
            _ => b.comment.trim().to_string(),
        };
        if b.action != "comment" {
            gov.update_finding(&f).map_err(ApiError::internal)?;
        }
        gov.finding_event(id, &auth.name, &b.action, &f.status, &comment)
            .map_err(ApiError::internal)?;
        gov.audit(&auth.name, auth.role, &action, &target, &f.status)
            .map_err(ApiError::internal)?;
        let mut v = with_status(&load(&gov, id)?, &today());
        v["events"] = json!(gov.finding_events(id).map_err(ApiError::internal)?);
        Ok(Json(v))
    })
    .await
}

#[derive(Deserialize)]
pub struct ReportQuery {
    format: Option<String>,
}

pub async fn governance_report(
    State(st): State<AppState>,
    _auth: Auth,
    Query(q): Query<ReportQuery>,
) -> ApiResult<Response> {
    blocking(move || {
        let all = st.gov()?.findings().map_err(ApiError::internal)?;
        let r = report(&all, &today(), chrono::Utc::now().timestamp());
        Ok(match q.format.as_deref() {
            Some("md") | Some("markdown") => (
                [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
                report_markdown(&r),
            )
                .into_response(),
            _ => Json(r).into_response(),
        })
    })
    .await
}

/// Active critical and high findings of a model: blockers of its approval.
pub fn approval_blockers(gov: &Gov, model: &str) -> ApiResult<Vec<String>> {
    Ok(gov
        .findings()
        .map_err(ApiError::internal)?
        .iter()
        .filter(|f| f.model == model && blocks_approval(f))
        .map(|f| format!("finding #{} ({}, {}): {}", f.id, f.severity, f.status, f.title))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(id: i64, severity: &str, status: &str, due: &str) -> Finding {
        Finding {
            id,
            model: "M1".into(),
            pipeline: None,
            run_id: None,
            step: None,
            title: format!("finding {id}"),
            description: "d".into(),
            severity: severity.into(),
            status: status.into(),
            owner: "alice".into(),
            raised_by: "bob".into(),
            due_date: due.into(),
            remediation: String::new(),
            evidence: None,
            remediated_by: None,
            closed_by: None,
            closed_at: (status == "closed").then_some(86_400 * 30),
            escalated_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn overdue_blocking_and_report() {
        let fs = [
            finding(1, "high", "open", "2026-01-10"),
            finding(2, "low", "open", "2026-06-01"),
            finding(3, "critical", "remediated", "2026-01-01"),
            finding(4, "medium", "closed", "2026-01-01"),
        ];
        let today = "2026-02-09";
        assert_eq!(days_overdue(&fs[0], today), Some(30));
        assert_eq!(days_overdue(&fs[1], today), None);
        // Remediated: waiting for the validator, not overdue for the owner.
        assert_eq!(days_overdue(&fs[2], today), None);
        assert_eq!(
            fs.iter().map(blocks_approval).collect::<Vec<_>>(),
            [true, false, true, false]
        );
        let c = model_counts(&fs, "M1", today);
        assert_eq!(
            (c["active"].as_u64(), c["overdue"].as_u64(), c["blocking"].as_u64()),
            (Some(3), Some(1), Some(2))
        );
        let r = report(&fs, today, 86_400 * 60);
        assert_eq!(r["counts"]["open"]["high"], 1);
        assert_eq!(r["counts"]["closed"]["medium"], 1);
        // Overdue first, then by severity.
        let order: Vec<i64> = r["active"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["id"].as_i64().unwrap())
            .collect();
        assert_eq!(order, [1, 3, 2]);
        assert_eq!(
            (r["overdue"].as_u64(), r["awaiting_review"].as_u64()),
            (Some(1), Some(1))
        );
        assert_eq!(r["closed_last_90_days"], 1);
        assert_eq!(r["mean_days_to_close_last_90_days"], 30.0);
        let md = report_markdown(&r);
        assert!(md.contains("3 active findings, 1 overdue, 1 awaiting the closure review"));
        assert!(md.contains("| 1 | M1 | high | open | finding 1 | alice | 2026-01-10 | 30 |"));
        assert_eq!(default_due_days("critical"), 30);
    }
}
