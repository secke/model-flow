//! Rapport de validation HTML autonome (une seule page, sans dépendance externe).

use mf_core::{Manifest, Mismatch};
use serde_json::Value;
use std::fmt::Write;

/// Décision de validation enregistrée (approbation ou rejet).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Decision {
    pub user: String,
    pub decision: String,
    pub comment: String,
    pub created_at: i64,
}

pub struct ReportMeta<'a> {
    pub pipeline: &'a str,
    pub run_id: i64,
    pub created_at: i64,
    pub generated_at: i64,
    /// `None` si le rejeu n'a pas été effectué.
    pub replay: Option<&'a [Mismatch]>,
    pub decisions: &'a [Decision],
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Date UTC lisible à partir d'un horodatage Unix (algorithme civil_from_days).
pub fn utc(ts: i64) -> String {
    let (days, secs) = (ts.div_euclid(86_400), ts.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", secs / 3600, secs % 3600 / 60)
}

fn status_of(v: &Value) -> Option<&str> {
    v.get("status").and_then(Value::as_str)
}

fn badge(s: &str) -> String {
    let (cls, label) = match s {
        "pass" | "green" => ("ok", s),
        "warn" | "yellow" => ("warn", s),
        "fail" | "red" => ("bad", s),
        _ => ("na", s),
    };
    format!("<span class=\"b {cls}\">{}</span>", esc(label))
}

fn num(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    if x.abs() >= 1e4 && x.abs() < 1e15 {
        // Montants : groupement des milliers, sans décimales.
        let r = x.round() as i64;
        let digits = r.unsigned_abs().to_string();
        let mut g = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                g.push('\u{202f}');
            }
            g.push(c);
        }
        return if r < 0 { format!("-{g}") } else { g };
    }
    if x == 0.0 || x.abs() >= 1e-3 {
        let s = format!("{x:.6}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s.is_empty() || s == "-" || s == "-0" {
            "0".into()
        } else {
            s.into()
        }
    } else {
        format!("{x:.3e}")
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Null => "—".into(),
        Value::Bool(b) => {
            if *b {
                "oui".into()
            } else {
                "non".into()
            }
        }
        Value::Number(n) => n.as_f64().map(num).unwrap_or_else(|| n.to_string()),
        Value::String(s) if matches!(s.as_str(), "pass" | "warn" | "fail" | "green" | "yellow" | "red") => badge(s),
        Value::String(s) => esc(s),
        _ => String::new(),
    }
}

fn numeric_summary(a: &[Value]) -> String {
    if a.is_empty() {
        return "<span class=\"muted\">aucun</span>".into();
    }
    let xs: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
    if xs.is_empty() {
        return format!("[{} éléments]", a.len());
    }
    let (min, max) = xs
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), x| (l.min(*x), h.max(*x)));
    let mean = xs.iter().sum::<f64>() / xs.len() as f64;
    if a.len() <= 12 {
        return xs.iter().map(|x| num(*x)).collect::<Vec<_>>().join(" · ");
    }
    format!(
        "<span class=\"muted\">{} valeurs — min {} · moy. {} · max {}</span>",
        a.len(),
        num(min),
        num(mean),
        num(max)
    )
}

/// Rendu générique : objet → tableau clé/valeur, tableau d'objets → tableau à colonnes.
fn render(v: &Value, depth: usize) -> String {
    match v {
        Value::Object(map) => {
            let mut s = String::from("<table class=\"kv\">");
            for (k, x) in map {
                let cell = match x {
                    Value::Object(_) if depth < 2 => render(x, depth + 1),
                    Value::Array(a) if a.first().is_some_and(Value::is_object) && depth < 2 => render(x, depth + 1),
                    Value::Array(a) => numeric_summary(a),
                    Value::Object(_) => "{…}".into(),
                    _ => scalar(x),
                };
                let _ = write!(s, "<tr><th>{}</th><td>{cell}</td></tr>", esc(k));
            }
            s + "</table>"
        }
        Value::Array(a) if a.first().is_some_and(Value::is_object) => {
            let rank = |k: &str| match k {
                "id" | "grade" | "name" | "t" | "instrument" => 0,
                "maturity" => 1,
                "product" => 1,
                "status" => 3,
                _ => 2,
            };
            let mut cols: Vec<&String> = a[0].as_object().unwrap().keys().collect();
            cols.sort_by_key(|k| rank(k));
            let mut s = String::from("<table class=\"grid\"><tr>");
            cols.iter().for_each(|c| {
                let _ = write!(s, "<th>{}</th>", esc(c));
            });
            s.push_str("</tr>");
            for row in a.iter().take(200) {
                s.push_str("<tr>");
                for c in &cols {
                    let _ = write!(s, "<td>{}</td>", row.get(c.as_str()).map(scalar).unwrap_or_default());
                }
                s.push_str("</tr>");
            }
            s + "</table>"
        }
        Value::Array(a) => numeric_summary(a),
        x => scalar(x),
    }
}

pub fn render_html(meta: &ReportMeta, m: &Manifest) -> String {
    let statuses: Vec<&str> = m.steps.iter().filter_map(|s| status_of(&s.output)).collect();
    let count = |k: &str| statuses.iter().filter(|s| **s == k).count();
    let (pass, warn, fail) = (count("pass"), count("warn"), count("fail"));
    let replay_ok = meta.replay.map(<[Mismatch]>::is_empty);
    let overall = if fail > 0 || replay_ok == Some(false) {
        "fail"
    } else if warn > 0 {
        "warn"
    } else {
        "pass"
    };
    let verdict = match overall {
        "fail" => "Non conforme — actions requises",
        "warn" => "Conforme avec réserves",
        _ => "Conforme",
    };

    let mut h = String::new();
    let _ = write!(
        h,
        r#"<!doctype html><html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Rapport de validation — {p}</title><style>
:root{{--bg:#f7f7f5;--card:#fff;--fg:#1c1c1a;--muted:#6b6b66;--line:#e3e2dd;--ok:#1f7a4d;--okb:#e3f3ea;--warn:#8a5a00;--warnb:#fbf0d9;--bad:#a8261b;--badb:#fbe4e1}}
@media (prefers-color-scheme:dark){{:root{{--bg:#151514;--card:#1e1e1c;--fg:#ecebe6;--muted:#9a9993;--line:#33322f;--ok:#6fd3a0;--okb:#173326;--warn:#f2c25c;--warnb:#3a2e12;--bad:#ff8a7d;--badb:#3d1c18}}}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--fg);font:14px/1.5 -apple-system,"Segoe UI",Inter,Roboto,sans-serif}}
main{{max-width:1040px;margin:0 auto;padding:32px 16px 64px}}h1{{font-size:24px;margin:0 0 4px}}h2{{font-size:17px;margin:32px 0 12px}}h3{{font-size:15px;margin:0}}
.muted{{color:var(--muted)}}.mono{{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px}}
.cards{{display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:12px;margin-top:20px}}
.card{{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:14px 16px}}.card .k{{color:var(--muted);font-size:12px}}.card .v{{font-size:20px;font-weight:600;margin-top:2px}}
.b{{display:inline-block;padding:1px 8px;border-radius:999px;font-size:12px;font-weight:600;text-transform:uppercase;letter-spacing:.02em}}
.ok{{color:var(--ok);background:var(--okb)}}.warn{{color:var(--warn);background:var(--warnb)}}.bad{{color:var(--bad);background:var(--badb)}}.na{{color:var(--muted);background:var(--line)}}
table{{border-collapse:collapse;width:100%}}.wrap,details{{max-width:100%;min-width:0}}details>div{{max-width:100%}}.wrap{{overflow-x:auto;background:var(--card);border:1px solid var(--line);border-radius:10px}}
.steps th,.steps td,.grid th,.grid td{{text-align:left;padding:8px 10px;border-bottom:1px solid var(--line);vertical-align:top}}
.grid td,.grid th{{white-space:nowrap}}.steps tr:last-child td{{border-bottom:0}}th{{font-weight:600;color:var(--muted);font-size:12px}}
.kv>tbody>tr>th{{width:34%;padding:4px 10px 4px 0;font-weight:500;text-align:left;vertical-align:top}}.kv>tbody>tr>td{{padding:4px 0}}.kv .kv>tbody>tr>th{{width:40%}}
details{{background:var(--card);border:1px solid var(--line);border-radius:10px;margin-bottom:10px}}summary{{cursor:pointer;padding:12px 16px;display:flex;gap:10px;align-items:center}}
details>div{{padding:0 16px 16px;overflow-x:auto}}.verdict{{font-size:16px;font-weight:600;margin-top:14px}}
footer{{margin-top:40px;color:var(--muted);font-size:12px;overflow-wrap:anywhere}}@media print{{details{{break-inside:avoid}}details>div{{display:block}}}}
</style></head><body><main>
<div class="muted">Rapport de validation de modèles · ModelFlow</div><h1>{p}</h1>
<div class="verdict">{b} {verdict}</div>
<div class="cards">
<div class="card"><div class="k">Exécution</div><div class="v">#{id}</div><div class="muted">{created}</div></div>
<div class="card"><div class="k">Tests conformes</div><div class="v" style="color:var(--ok)">{pass}</div></div>
<div class="card"><div class="k">Réserves</div><div class="v" style="color:var(--warn)">{warn}</div></div>
<div class="card"><div class="k">Non conformes</div><div class="v" style="color:var(--bad)">{fail}</div></div>
<div class="card"><div class="k">Reproductibilité</div><div class="v">{replay}</div></div>
</div>"#,
        p = esc(meta.pipeline),
        b = badge(overall),
        id = meta.run_id,
        created = utc(meta.created_at),
        replay = match replay_ok {
            Some(true) => badge("pass"),
            Some(false) => badge("fail"),
            None => "<span class=\"muted\">non vérifiée</span>".into(),
        },
    );

    if let Some(mm) = meta.replay.filter(|mm| !mm.is_empty()) {
        h.push_str(
            "<h2>Écarts au rejeu</h2><div class=\"wrap\"><table class=\"steps\"><tr><th>Étape</th><th>Écart</th></tr>",
        );
        for x in mm {
            let _ = write!(h, "<tr><td>{}</td><td>{}</td></tr>", esc(&x.step), esc(&x.reason));
        }
        h.push_str("</table></div>");
    }

    if !meta.decisions.is_empty() {
        h.push_str("<h2>Décisions de validation</h2><div class=\"wrap\"><table class=\"steps\"><tr><th>Date</th><th>Validateur</th><th>Décision</th><th>Commentaire</th></tr>");
        for d in meta.decisions {
            let b = if d.decision == "approved" {
                badge("pass")
            } else {
                badge("fail")
            };
            let label = match d.decision.as_str() {
                "approved" => "approuvée",
                "rejected" => "rejetée",
                other => other,
            };
            let _ = write!(
                h,
                "<tr><td>{}</td><td>{}</td><td>{b} {}</td><td>{}</td></tr>",
                utc(d.created_at),
                esc(&d.user),
                esc(label),
                esc(&d.comment)
            );
        }
        h.push_str("</table></div>");
    }

    h.push_str("<h2>Étapes</h2><div class=\"wrap\"><table class=\"steps\"><tr><th>Étape</th><th>Version</th><th>Statut</th><th>Empreinte sortie</th><th>Empreinte code</th></tr>");
    for s in &m.steps {
        let _ = write!(
            h,
            "<tr><td>{}</td><td class=\"mono\">{}</td><td>{}</td><td class=\"mono\">{}</td><td class=\"mono\">{}</td></tr>",
            esc(&s.name),
            esc(&s.version),
            status_of(&s.output).map(badge).unwrap_or_else(|| "<span class=\"muted\">donnée</span>".into()),
            &s.output_hash[..12],
            s.code_hash.as_deref().map(|c| &c[..12]).unwrap_or("—"),
        );
    }
    h.push_str("</table></div><h2>Détail des résultats</h2>");
    for s in m.steps.iter().filter(|s| status_of(&s.output).is_some()) {
        let open = if status_of(&s.output) == Some("pass") {
            ""
        } else {
            " open"
        };
        let _ = write!(
            h,
            "<details{open}><summary>{} <h3>{}</h3><span class=\"muted mono\">v{}</span></summary><div>{}</div></details>",
            badge(status_of(&s.output).unwrap()),
            esc(&s.name),
            esc(&s.version),
            render(&s.output, 0)
        );
    }
    let _ = write!(
        h,
        "<footer>run_hash <span class=\"mono\">{}</span><br>Rapport généré le {}. Les empreintes SHA-256 permettent de rejouer et d'auditer chaque résultat (<span class=\"mono\">mf verify</span>).</footer></main></body></html>",
        m.run_hash,
        utc(meta.generated_at)
    );
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formatting() {
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(1_709_210_096), "2024-02-29 12:34 UTC");
    }

    #[test]
    fn number_formatting() {
        assert_eq!(num(-2_614_755.4), "-2\u{202f}614\u{202f}755");
        assert_eq!(num(12_000.0), "12\u{202f}000");
        assert_eq!(num(0.871585), "0.871585");
        assert_eq!(num(-0.0000001), "-1.000e-7");
        assert_eq!(num(-0.0), "0");
    }

    #[test]
    fn escapes_html() {
        assert_eq!(esc("<a&\"b\">"), "&lt;a&amp;&quot;b&quot;&gt;");
    }
}
