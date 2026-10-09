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

impl RequestSpec {
    /// The secret names the request references (`secret:<name>` in a
    /// header or query value, `{secret:<name>}` in the body): what the
    /// user must store, and what an auth error must name.
    pub fn secret_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .headers
            .values()
            .chain(self.query.values())
            .filter_map(|v| v.trim_matches(['{', '}']).strip_prefix("secret:"))
            .map(String::from)
            .chain(self.body.as_deref().map(body_refs).unwrap_or_default())
            .collect();
        names.sort();
        names.dedup();
        names
    }
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
    /// Optional contact paths (lead segmentation). Empty = not mapped.
    #[serde(default)]
    pub lead_id: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub email: String,
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

/// Resolve a spec value: `secret:<name>` from the workspace's CRM keys in
/// auth.json, `env:<VAR>` from the environment, anything else literal.
pub fn resolve_ref(value: &str, workspace: Option<&str>) -> Result<String> {
    if let Some(name) = value.strip_prefix("secret:") {
        return crate::secrets::load_crm_key(workspace, name).ok_or_else(|| {
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

/// The `{secret:<name>}` refs in a body, in order.
fn body_refs(body: &str) -> Vec<String> {
    body.split("{secret:")
        .skip(1)
        .filter_map(|rest| rest.split_once('}').map(|(name, _)| name.to_string()))
        .collect()
}

/// A body with its `{secret:<name>}` refs filled from the workspace's
/// keys, each JSON-escaped: a ref sits inside a JSON string.
pub fn fill_secrets(body: &str, workspace: Option<&str>) -> Result<String> {
    fill_refs(body, |name| {
        resolve_ref(&format!("secret:{}", name), workspace)
    })
}

fn fill_refs(body: &str, value_of: impl Fn(&str) -> Result<String>) -> Result<String> {
    let mut out = body.to_string();
    for name in body_refs(body) {
        let value = value_of(&name)?;
        let quoted = serde_json::to_string(&value)?;
        let escaped = quoted
            .strip_prefix('"')
            .and_then(|q| q.strip_suffix('"'))
            .unwrap_or(&quoted);
        out = out.replace(&format!("{{secret:{}}}", name), escaped);
    }
    Ok(out)
}

pub use super::transform::{csv_records, organic_contacts, transform};

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

/// Dot-path lookup: `a.b.c` descends nested objects. Exposed because
/// the page loop uses it to compute each record's delivery epoch for
/// the client-side date cutover (`fetch_crm`).
pub fn get_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// Ids and stages must end up as strings whatever JSON type the CRM used.
pub(crate) fn scalar_string(v: &Value) -> String {
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
        assert_eq!(resolve_ref("env:MB_CRM_TEST_KEY", None).unwrap(), "k-123");
        std::env::remove_var("MB_CRM_TEST_KEY");
        assert!(resolve_ref("env:MB_CRM_TEST_KEY", None).is_err());
        assert_eq!(resolve_ref("plain", None).unwrap(), "plain");
    }

    #[test]
    fn transform_maps_stages_and_stringifies_ids() {
        let recs = vec![serde_json::json!({
            "mx_Ad_Id": 120211,
            "ProspectStage": "Site Visit",
            "mx_Delivery_Time": "2026-07-15T09:30:00+05:30"
        })];
        let spec = parse(SPEC).unwrap();
        let (tickets, dropped) = transform(&recs, &spec.map);
        assert_eq!(dropped, 0);
        assert_eq!(tickets[0]["ad_id"], "120211");
        assert_eq!(tickets[0]["stage"], "Visit");
        assert_eq!(tickets[0]["delivery"], "2026-07-15T09:30:00+05:30");
    }

    #[test]
    fn transform_drops_and_counts_records_without_ad_id() {
        let recs = vec![
            serde_json::json!({"mx_Ad_Id": 120211, "ProspectStage": "Won",
                               "mx_Delivery_Time": "2026-07-15T09:30:00+05:30"}),
            serde_json::json!({"ProspectStage": "Won",
                               "mx_Delivery_Time": "2026-07-15T10:00:00+05:30"}),
            serde_json::json!({"mx_Ad_Id": "", "ProspectStage": "Won",
                               "mx_Delivery_Time": "2026-07-15T11:00:00+05:30"}),
            // Placeholder ids are non-empty and KEPT (join rule).
            serde_json::json!({"mx_Ad_Id": "Stattic Ad", "ProspectStage": "Won",
                               "mx_Delivery_Time": "2026-07-15T12:00:00+05:30"}),
        ];
        let spec = parse(SPEC).unwrap();
        let (tickets, dropped) = transform(&recs, &spec.map);
        assert_eq!(dropped, 2);
        assert_eq!(tickets.len(), 2);
        assert_eq!(tickets[1]["ad_id"], "Stattic Ad");
    }

    #[test]
    fn body_refs_are_named_and_filled_json_escaped() {
        let spec = parse(
            "name = \"x\"\n[request]\nurl = \"https://x\"\n\
             body = '{\"k\": \"{secret:x_key}\", \"from\": \"{from_date}\"}'\n\
             [map]\nad_id = \"a\"\nstage = \"s\"\ndelivery = \"d\"\n",
        )
        .unwrap();
        let request = spec.request.unwrap();
        assert_eq!(request.secret_names(), vec!["x_key"]);
        let body = request.body.unwrap();
        let filled = fill_refs(&body, |_| Ok(r#"pa"ss\"#.into())).unwrap();
        let v: Value = serde_json::from_str(&filled).unwrap();
        assert_eq!(v["k"], r#"pa"ss\"#);
        assert_eq!(v["from"], "{from_date}");
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
