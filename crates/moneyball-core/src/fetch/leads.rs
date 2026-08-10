//! Per-lead Meta records for the fetched window (lead segmentation -
//! the Diff breakdown). GET {ad}/leads for every ad that delivered;
//! extract name/phone/email from field_data. RAW PII by explicit user
//! decision 2026-08-10 (local raw ok, revisit at Postgres): leads.json
//! is chmod 0600 and never syncs.

use std::path::Path;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::schema::{LeadRow, LeadsFile, LEADS_SCHEMA};

/// Pull leads for every distinct ad in `ads_daily_rows` and write
/// `<snap_dir>/leads.json` (0600). Returns the row count. Best-effort
/// by contract: the caller treats an Err as a warning, never a lost
/// snapshot (a token without leads_retrieval must not break /fetch).
pub fn fetch_and_write(
    client: &reqwest::blocking::Client,
    token: &str,
    ads_daily_rows: &[Value],
    snap_dir: &Path,
) -> Result<usize> {
    let mut ad_ids: Vec<&str> = ads_daily_rows
        .iter()
        .filter_map(|r| r.get("ad_id").and_then(|v| v.as_str()))
        .collect();
    ad_ids.sort_unstable();
    ad_ids.dedup();

    let mut rows: Vec<LeadRow> = Vec::new();
    for ad_id in ad_ids {
        let mut url = format!("{}/{}/leads", super::META_GRAPH_BASE, ad_id);
        let mut first = true;
        loop {
            let req = if first {
                client.get(&url).query(&[
                    ("access_token", token),
                    ("fields", "id,created_time,field_data"),
                    ("limit", "300"),
                ])
            } else {
                client.get(&url) // paging.next carries everything
            };
            let resp = req
                .send()
                .map_err(|e| Error::Meta(format!("leads network: {}", e)))?;
            let status = resp.status();
            let body: Value = resp
                .json()
                .map_err(|e| Error::Meta(format!("leads json: {}", e)))?;
            if !status.is_success() {
                let msg = body
                    .pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                return Err(Error::Meta(format!("Meta {} for leads: {}", status, msg)));
            }
            for l in body
                .get("data")
                .and_then(|d| d.as_array())
                .into_iter()
                .flatten()
            {
                rows.push(row_from_lead(ad_id, l));
            }
            match body.pointer("/paging/next").and_then(|n| n.as_str()) {
                Some(next) => {
                    url = next.to_string();
                    first = false;
                }
                None => break,
            }
        }
    }

    let file = LeadsFile {
        schema: LEADS_SCHEMA.into(),
        fetched_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        rows,
    };
    write_leads(snap_dir, &file)?;
    Ok(file.rows.len())
}

/// field_data is `[{name, values: [..]}, ..]`; match by substring like
/// the python `_lead_field` (form field names vary per campaign).
fn lead_field(l: &Value, needles: &[&str]) -> Option<String> {
    let fields = l.get("field_data")?.as_array()?;
    for needle in needles {
        for f in fields {
            let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name.to_lowercase().contains(needle) {
                if let Some(v) = f
                    .pointer("/values/0")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn row_from_lead(ad_id: &str, l: &Value) -> LeadRow {
    LeadRow {
        lead_id: l
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        ad_id: ad_id.to_string(),
        created_time: l
            .get("created_time")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        name: lead_field(l, &["full_name", "full name", "name"]),
        phone: lead_field(l, &["phone"]),
        email: lead_field(l, &["email"]),
    }
}

/// Temp-then-rename, then 0600: raw contact PII must not be
/// world-readable (same posture as ~/.moneyball/auth.json).
fn write_leads(snap_dir: &Path, file: &LeadsFile) -> Result<()> {
    let final_path = snap_dir.join("leads.json");
    let tmp = snap_dir.join("leads.json.tmp");
    let body = serde_json::to_string_pretty(file)
        .map_err(|e| Error::Config(format!("serialize leads: {}", e)))?;
    std::fs::write(&tmp, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, &final_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn field_data_matching_is_substring_and_ordered() {
        let l = json!({
            "id": "lg1", "created_time": "2026-08-09T10:00:00+0530",
            "field_data": [
                {"name": "your_full_name", "values": ["Asha K"]},
                {"name": "phone_number", "values": ["+91 98765 43210"]},
                {"name": "work_email", "values": ["asha@example.com"]}
            ]
        });
        let r = row_from_lead("a1", &l);
        assert_eq!(r.lead_id, "lg1");
        assert_eq!(r.name.as_deref(), Some("Asha K"));
        assert_eq!(r.phone.as_deref(), Some("+91 98765 43210"));
        assert_eq!(r.email.as_deref(), Some("asha@example.com"));
    }

    #[test]
    fn missing_fields_stay_none_never_panic() {
        let r = row_from_lead("a1", &json!({"id": "lg2"}));
        assert!(r.name.is_none() && r.phone.is_none() && r.email.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn leads_file_is_written_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mb-leads-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = LeadsFile {
            schema: LEADS_SCHEMA.into(),
            fetched_at: String::new(),
            rows: vec![],
        };
        write_leads(&dir, &file).unwrap();
        let mode = std::fs::metadata(dir.join("leads.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "raw PII must be owner-only");
        std::fs::remove_dir_all(&dir).ok();
    }
}
