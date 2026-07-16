//! Declarative CRM source spec - `crm.toml` parsing and the record ->
//! ticket transform. Offline: the network executor lives in `crm::fetch`.
//!
//! One spec connects any REST CRM: endpoint + auth refs + paging mode +
//! field paths + stage map. Secrets are referenced (`secret:<name>` from
//! `~/.moneyball/auth.json` crm_keys, or `env:<VAR>`), never stored in
//! the spec, so crm.toml is safe to commit.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};

/// Annotated starter spec written by `moneyball crm init`. The examples
/// are LeadSquared-shaped; every key is documented in place.
pub const TEMPLATE_TOML: &str = include_str!("template.crm.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpec {
    /// Display name, e.g. "leadsquared" or "acme-crm".
    pub name: String,
    /// Absent for CSV-only sources (`moneyball crm import`).
    pub request: Option<RequestSpec>,
    #[serde(default)]
    pub paging: PagingSpec,
    pub map: MapSpec,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestSpec {
    pub url: String,
    #[serde(default = "default_method")]
    pub method: String,
    /// Header values may be `secret:<name>`, `env:<VAR>`, or literal.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Query values support the same refs plus `{from_date}` `{to_date}`
    /// `{page}` `{page_size}` templates.
    #[serde(default)]
    pub query: HashMap<String, String>,
    /// JSON body template for POST endpoints; same template vars.
    #[serde(default)]
    pub body: Option<String>,
}

fn default_method() -> String {
    "GET".into()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PagingSpec {
    /// "none" (default) = single request; "page" = increment a page
    /// number in `param` until a page returns fewer than `size` records.
    #[serde(default)]
    pub mode: PagingMode,
    #[serde(default)]
    pub param: String,
    #[serde(default = "default_page_start")]
    pub start: u32,
    #[serde(default)]
    pub size_param: String,
    #[serde(default = "default_page_size")]
    pub size: u32,
}

fn default_page_start() -> u32 {
    1
}
fn default_page_size() -> u32 {
    500
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PagingMode {
    #[default]
    None,
    Page,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapSpec {
    /// Dot path to the record array in the response; "" when the
    /// response IS the array.
    #[serde(default)]
    pub root: String,
    pub ad_id: String,
    pub stage: String,
    pub delivery: String,
    #[serde(default)]
    pub funnel: String,
    /// Source stage name -> canonical stage name. Unmapped stages pass
    /// through as-is (the validator warns on unrecognized ones).
    #[serde(default)]
    pub stage_map: HashMap<String, String>,
}

pub fn parse(spec: &str) -> Result<SourceSpec> {
    let s: SourceSpec =
        toml::from_str(spec).map_err(|e| Error::Config(format!("crm.toml: {}", e)))?;
    // Semantic validation here, not at execution time, so the connect
    // wizard's LLM retry loop sees these failures too.
    if s.paging.mode == PagingMode::Page && s.paging.param.is_empty() {
        return Err(Error::Config(
            "crm.toml: paging.mode = \"page\" requires paging.param (the query \
             parameter that carries the page number)"
                .into(),
        ));
    }
    Ok(s)
}

/// Replace `{var}` templates. Unknown vars are left intact so typos show
/// up verbatim in the failing request rather than as silent blanks.
pub fn expand(template: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{}}}", k), v);
    }
    out
}

/// Resolve a spec value: `secret:<name>` from auth.json crm_keys,
/// `env:<VAR>` from the environment, anything else literal.
pub fn resolve_ref(value: &str) -> Result<String> {
    if let Some(name) = value.strip_prefix("secret:") {
        return crate::secrets::load_crm_key(name).ok_or_else(|| {
            Error::Secrets(format!(
                "no CRM secret \"{}\" - store it with: moneyball crm secret {}",
                name, name
            ))
        });
    }
    if let Some(var) = value.strip_prefix("env:") {
        return std::env::var(var)
            .map_err(|_| Error::Secrets(format!("env var {} is not set", var)));
    }
    Ok(value.to_string())
}

/// Pull the record array out of a response via the spec's `root` path.
pub fn records<'a>(resp: &'a Value, root: &str) -> Result<&'a Vec<Value>> {
    let node = if root.is_empty() {
        resp
    } else {
        get_path(resp, root).ok_or_else(|| {
            Error::Config(format!("response has nothing at map.root \"{}\"", root))
        })?
    };
    node.as_array().ok_or_else(|| {
        Error::Config(format!(
            "map.root \"{}\" is not a JSON array (got {})",
            root,
            type_name(node)
        ))
    })
}

/// Transform raw CRM records into contract tickets. Missing fields become
/// missing keys - the validator reports them per-row afterwards.
pub fn transform(records: &[Value], map: &MapSpec) -> Vec<Value> {
    records
        .iter()
        .map(|rec| {
            let mut t = serde_json::Map::new();
            if let Some(v) = get_path(rec, &map.ad_id) {
                t.insert("ad_id".into(), Value::String(scalar_string(v)));
            }
            if let Some(v) = get_path(rec, &map.stage).map(scalar_string) {
                let stage = map.stage_map.get(&v).cloned().unwrap_or(v);
                t.insert("stage".into(), Value::String(stage));
            }
            if let Some(v) = get_path(rec, &map.delivery) {
                t.insert("delivery".into(), v.clone());
            }
            if !map.funnel.is_empty() {
                if let Some(v) = get_path(rec, &map.funnel) {
                    t.insert("funnel".into(), Value::String(scalar_string(v)));
                }
            }
            Value::Object(t)
        })
        .collect()
}

/// CSV rows as records: header row names the fields, every value is a
/// string (parse_epoch and the validator handle coercion downstream).
/// For CSV, the spec's map paths are simply column names.
pub fn csv_records(raw: &str) -> Result<Vec<Value>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(raw.as_bytes());
    let headers = rdr
        .headers()
        .map_err(|e| Error::Config(format!("csv: {}", e)))?
        .clone();
    let mut out = Vec::new();
    for row in rdr.records() {
        let row = row.map_err(|e| Error::Config(format!("csv: {}", e)))?;
        let mut rec = serde_json::Map::new();
        for (h, v) in headers.iter().zip(row.iter()) {
            rec.insert(h.to_string(), Value::String(v.to_string()));
        }
        out.push(Value::Object(rec));
    }
    Ok(out)
}

/// Add stage mappings to a spec's [map.stage_map], preserving existing
/// entries (parse -> insert -> re-serialize; returns a valid spec).
pub fn add_stage_mappings(spec_toml: &str, pairs: &[(String, String)]) -> Result<String> {
    let mut doc: toml::Table =
        toml::from_str(spec_toml).map_err(|e| Error::Config(format!("crm.toml: {}", e)))?;
    let map = doc
        .entry("map")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or_else(|| Error::Config("crm.toml: [map] is not a table".into()))?;
    let sm = map
        .entry("stage_map")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or_else(|| Error::Config("crm.toml: [map.stage_map] is not a table".into()))?;
    for (from, to) in pairs {
        sm.insert(from.clone(), toml::Value::String(to.clone()));
    }
    let out = toml::to_string(&doc).map_err(|e| Error::Config(format!("serialize: {}", e)))?;
    parse(&out)?; // never write a spec that cannot be read back
    Ok(out)
}

/// Cut a string at (or just before) `cap` bytes, on a char boundary.
/// The byte-slice `&s[..n]` panics on multibyte text - lead names and
/// error bodies routinely contain it.
pub fn truncate_chars(s: &str, cap: usize) -> &str {
    let mut end = cap.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Dot-path lookup: `a.b.c` descends nested objects.
fn get_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// Ids and stages must end up as strings whatever JSON type the CRM used.
fn scalar_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = r#"
name = "leadsquared"

[request]
url = "https://api-in21.leadsquared.com/v2/LeadManagement.svc/Leads.Get"
[request.headers]
"x-LSQ-AccessKey" = "env:LSQ_ACCESS"
[request.query]
fromDate = "{from_date}"

[paging]
mode = "page"
param = "pageIndex"

[map]
root = "Leads"
ad_id = "mx_Ad_Id"
stage = "ProspectStage"
delivery = "mx_Delivery_Time"
[map.stage_map]
"Site Visit" = "Visit"
"#;

    #[test]
    fn parses_full_spec() {
        let s = parse(SPEC).unwrap();
        assert_eq!(s.name, "leadsquared");
        assert_eq!(s.paging.mode, PagingMode::Page);
        assert_eq!(s.paging.start, 1);
        assert_eq!(s.map.stage_map.get("Site Visit").unwrap(), "Visit");
    }

    #[test]
    fn template_toml_parses() {
        parse(TEMPLATE_TOML).unwrap();
    }

    #[test]
    fn expand_replaces_known_vars_keeps_unknown() {
        let vars = HashMap::from([("from_date", "2026-07-01".to_string())]);
        assert_eq!(expand("{from_date}..{typo}", &vars), "2026-07-01..{typo}");
    }

    #[test]
    fn resolve_ref_env_and_literal() {
        std::env::set_var("MB_CRM_TEST_KEY", "k-123");
        assert_eq!(resolve_ref("env:MB_CRM_TEST_KEY").unwrap(), "k-123");
        std::env::remove_var("MB_CRM_TEST_KEY");
        assert!(resolve_ref("env:MB_CRM_TEST_KEY").is_err());
        assert_eq!(resolve_ref("plain").unwrap(), "plain");
    }

    #[test]
    fn transform_maps_stages_and_stringifies_ids() {
        let recs = vec![serde_json::json!({
            "mx_Ad_Id": 120211,
            "ProspectStage": "Site Visit",
            "mx_Delivery_Time": "2026-07-15T09:30:00+05:30"
        })];
        let spec = parse(SPEC).unwrap();
        let tickets = transform(&recs, &spec.map);
        assert_eq!(tickets[0]["ad_id"], "120211");
        assert_eq!(tickets[0]["stage"], "Visit");
        assert_eq!(tickets[0]["delivery"], "2026-07-15T09:30:00+05:30");
    }

    #[test]
    fn csv_records_use_header_names() {
        let raw = "Ad Id,Stage,Delivered\n111,Site Visit,2026-07-15T09:30:00+05:30\n";
        let recs = csv_records(raw).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["Ad Id"], "111");
        assert_eq!(recs[0]["Stage"], "Site Visit");
    }

    #[test]
    fn spec_without_request_parses() {
        let s = parse("name = \"csv-crm\"\n[map]\nad_id = \"Ad Id\"\nstage = \"Stage\"\ndelivery = \"Delivered\"\n").unwrap();
        assert!(s.request.is_none());
    }

    #[test]
    fn records_root_paths() {
        let resp = serde_json::json!({ "data": { "leads": [1, 2] } });
        assert_eq!(records(&resp, "data.leads").unwrap().len(), 2);
        assert!(records(&resp, "data.nope").is_err());
        let arr = serde_json::json!([1]);
        assert_eq!(records(&arr, "").unwrap().len(), 1);
    }
}
