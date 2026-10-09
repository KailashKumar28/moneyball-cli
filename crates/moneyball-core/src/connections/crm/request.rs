//! A pasted request's credentials become secret refs before anything
//! leaves the machine: every header value, and the query params and JSON
//! body fields that look like one (asked). crm.toml and the model see
//! only `secret:<name>` (`{secret:<name>}` inside a body).

use std::collections::HashMap;

use serde_json::Value;

use crate::crm::connect::ConnectInput;
use crate::error::Result;

use super::{Done, Prompter};

/// Some(done) when the user stopped the flow.
pub(super) fn secretize(
    ws: &str,
    name: &str,
    input: &mut ConnectInput,
    p: &mut dyn Prompter,
) -> Result<Option<Done>> {
    // One name, one value: a second credential under the same field name
    // (the query and the body both say apiKey) gets a name of its own.
    let mut taken: HashMap<String, String> = HashMap::new();
    let mut store = |k: &str, v: &str, p: &mut dyn Prompter, place: &str| -> Result<String> {
        let base = format!("{}_{}", name, k.to_lowercase().replace('-', "_"));
        let sname = free_name(&base, v, &taken);
        taken.insert(sname.clone(), v.to_string());
        crate::secrets::store_crm_key(ws, &sname, v)?;
        p.show(&format!("  {} {} stored as secret:{}", place, k, sname));
        Ok(sname)
    };
    for (k, v) in input.headers.iter_mut() {
        *v = format!("secret:{}", store(k, v, p, "header")?);
    }
    // Query params carry credentials in many CRMs (LeadSquared puts
    // accessKey/secretKey there). Secretize per param - dates and flags
    // must stay literal so the model can turn them into templates.
    for (k, v) in input.query.iter_mut() {
        let likely = looks_credential(k);
        if yes(
            &answer!(p, question("query param", k, likely), false),
            likely,
        ) {
            *v = format!("secret:{}", store(k, v, p, "query")?);
        }
    }
    let Some(body) = input.body.as_deref() else {
        return Ok(None);
    };
    let Ok(mut json) = serde_json::from_str::<Value>(body) else {
        // A form body cannot carry refs.
        return keep_anyway(p, &form_credentials(body));
    };
    let (strings, others) = credentials(&json);
    // (field, value) -> the secret its value became; asked once per pair.
    let mut refs = HashMap::new();
    for (k, v) in strings {
        if v.is_empty() || refs.contains_key(&(k.clone(), v.clone())) {
            continue;
        }
        if yes(&answer!(p, question("body field", &k, true), false), true) {
            let sname = store(&k, &v, p, "body")?;
            refs.insert((k, v), format!("{{secret:{}}}", sname));
        }
    }
    if !refs.is_empty() {
        fill(&mut json, &refs);
        input.body = Some(json.to_string());
    }
    // A credential that is a number or a list cannot become a ref.
    keep_anyway(p, &others)
}

/// Credentials a body keeps as written go to the model and into
/// crm.toml: say so, and let the user stop. Some(done) when they do.
fn keep_anyway(p: &mut dyn Prompter, carried: &[String]) -> Result<Option<Done>> {
    if carried.is_empty() {
        return Ok(None);
    }
    let go = answer!(
        p,
        format!(
            "the request body carries {} - it is sent to the model and saved in crm.toml as is. continue? [y/N]: ",
            carried.join(", ")
        ),
        false
    );
    if go.eq_ignore_ascii_case("y") {
        return Ok(None);
    }
    p.show("nothing saved - take the credential out of the body, or pick a listed CRM");
    Ok(Some(Done::NotSaved))
}

/// `base`, else `base_2`, `base_3`...: the first name not already holding
/// a different value.
fn free_name(base: &str, value: &str, taken: &HashMap<String, String>) -> String {
    (1..)
        .map(|i| match i {
            1 => base.to_string(),
            i => format!("{}_{}", base, i),
        })
        .find(|n| taken.get(n).is_none_or(|had| had == value))
        .expect("an unbounded range always finds a free name")
}

/// "store its value as a secret?", defaulting to `likely`.
fn question(what: &str, k: &str, likely: bool) -> String {
    let hint = if likely { "[Y/n]" } else { "[y/N]" };
    format!(
        "{} \"{}\" - store its value as a secret? {}: ",
        what, k, hint
    )
}

fn yes(answer: &str, likely: bool) -> bool {
    if answer.is_empty() {
        likely
    } else {
        answer.eq_ignore_ascii_case("y")
    }
}

/// Every credential-looking field in a JSON body: the ones holding a
/// string (which can become refs), and the names of the ones holding
/// anything else but an object (whose fields are looked at in turn).
fn credentials(v: &Value) -> (Vec<(String, String)>, Vec<String>) {
    let (mut strings, mut others) = (Vec::new(), Vec::new());
    let mut take = |(s, o): (Vec<_>, Vec<_>)| {
        strings.extend(s);
        others.extend(o);
    };
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                match v {
                    Value::String(s) if looks_credential(k) => {
                        take((vec![(k.clone(), s.clone())], vec![]))
                    }
                    Value::Object(_) => take(credentials(v)),
                    _ if looks_credential(k) => take((vec![], vec![k.clone()])),
                    v => take(credentials(v)),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|v| take(credentials(v))),
        _ => {}
    }
    (strings, others)
}

/// Replace each secretized field's value with its ref.
fn fill(v: &mut Value, refs: &HashMap<(String, String), String>) {
    match v {
        Value::Object(m) => {
            for (k, v) in m.iter_mut() {
                let key = match v {
                    Value::String(s) => Some((k.clone(), s.clone())),
                    _ => None,
                };
                match key.and_then(|key| refs.get(&key)) {
                    Some(r) => *v = Value::String(r.clone()),
                    None => fill(v, refs),
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|v| fill(v, refs)),
        _ => {}
    }
}

/// The credential-looking field names in a form body.
fn form_credentials(body: &str) -> Vec<String> {
    body.split('&')
        .filter_map(|kv| kv.split_once('=').map(|(k, _)| k.to_string()))
        .filter(|k| looks_credential(k))
        .collect()
}

/// Does this field name look like it carries a credential? The query
/// prompts' default, and which body fields are asked about at all.
fn looks_credential(key: &str) -> bool {
    let k = key.to_lowercase();
    ["key", "token", "secret", "auth", "sig", "pass", "pwd"]
        .iter()
        .any(|n| k.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_fields_anywhere_in_a_body_become_refs() {
        let mut body = serde_json::json!({
            "auth": {"apiKey": "k1", "page": 1},
            "filters": [{"token": "t1"}],
            "stage": "x",
            "password": "",
            "api_key": 12345678,
            "tokens": ["a", "b"]
        });
        let (strings, others) = credentials(&body);
        assert_eq!(
            strings,
            vec![
                ("apiKey".to_string(), "k1".to_string()),
                ("token".to_string(), "t1".to_string()),
                ("password".to_string(), String::new()),
            ]
        );
        assert_eq!(others, vec!["api_key", "tokens"]);
        let refs = HashMap::from([(
            ("apiKey".to_string(), "k1".to_string()),
            "{secret:acme_apikey}".to_string(),
        )]);
        fill(&mut body, &refs);
        assert_eq!(body["auth"]["apiKey"], "{secret:acme_apikey}");
        assert_eq!(body["filters"][0]["token"], "t1");
        assert_eq!(form_credentials("q=1&access_token=t"), vec!["access_token"]);
        assert_eq!(
            credentials(&serde_json::json!({"page": 1})),
            (vec![], vec![])
        );
    }

    #[test]
    fn a_name_holding_another_value_is_never_reused() {
        let taken = HashMap::from([
            ("acme_apikey".to_string(), "QK1".to_string()),
            ("acme_apikey_2".to_string(), "BK1".to_string()),
        ]);
        assert_eq!(free_name("acme_apikey", "QK1", &taken), "acme_apikey");
        assert_eq!(free_name("acme_apikey", "BK1", &taken), "acme_apikey_2");
        assert_eq!(free_name("acme_apikey", "ZK9", &taken), "acme_apikey_3");
        assert_eq!(free_name("acme_token", "T", &taken), "acme_token");
    }
}
