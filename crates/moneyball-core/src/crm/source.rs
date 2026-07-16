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
pub struct SourceSpec {
    /// Display name, e.g. "leadsquared" or "acme-crm".
    pub name: String,
    pub request: RequestSpec,
    #[serde(default)]
    pub paging: PagingSpec,
    pub map: MapSpec,
}

#[derive(Debug, Deserialize)]
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
    toml::from_str(spec).map_err(|e| Error::Config(format!("crm.toml: {}", e)))
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
    fn records_root_paths() {
        let resp = serde_json::json!({ "data": { "leads": [1, 2] } });
        assert_eq!(records(&resp, "data.leads").unwrap().len(), 2);
        assert!(records(&resp, "data.nope").is_err());
        let arr = serde_json::json!([1]);
        assert_eq!(records(&arr, "").unwrap().len(), 1);
    }
}
