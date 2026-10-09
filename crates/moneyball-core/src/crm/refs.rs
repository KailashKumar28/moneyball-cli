//! A CRM request's refs - `secret:<name>` and `env:<VAR>` values, and
//! `{secret:<name>}` inside a body - resolved for one request, and every
//! secret it used masked back out of what anyone sees: an error snippet,
//! the sample the model drafts from, the data written. A CRM that echoes
//! the request must not hand the key back in clear.

use serde_json::Value;

use crate::error::{Error, Result};

/// Shorter values are not masked: replacing them would garble data.
const MIN_MASKED: usize = 6;

/// The secret a value refers to: `secret:<name>`, braces optional.
pub(super) fn secret_name(value: &str) -> Option<&str> {
    let bare = value
        .strip_prefix('{')
        .and_then(|v| v.strip_suffix('}'))
        .unwrap_or(value);
    bare.strip_prefix("secret:")
}

/// The `{secret:<name>}` refs in a body, in order.
pub(super) fn body_refs(body: &str) -> Vec<String> {
    body.split("{secret:")
        .skip(1)
        .filter_map(|rest| rest.split_once('}').map(|(name, _)| name.to_string()))
        .collect()
}

/// A secret's value by name.
type Lookup<'a> = Box<dyn Fn(&str) -> Option<String> + 'a>;

pub(super) struct Refs<'a> {
    /// The workspace's CRM keys.
    secret: Lookup<'a>,
    /// Each value handed out, with the ref that stands for it.
    used: Vec<(String, String)>,
}

impl<'a> Refs<'a> {
    pub(super) fn of(workspace: Option<&'a str>) -> Self {
        Refs::with(move |name| crate::secrets::load_crm_key(workspace, name))
    }

    fn with(secret: impl Fn(&str) -> Option<String> + 'a) -> Self {
        Refs {
            secret: Box::new(secret),
            used: Vec::new(),
        }
    }

    /// A header or query value: a secret from the workspace's keys, an
    /// env var, anything else literal.
    pub(super) fn value(&mut self, value: &str) -> Result<String> {
        if let Some(name) = secret_name(value) {
            let got = (self.secret)(name).ok_or_else(|| {
                Error::Secrets(format!(
                    "no CRM secret \"{}\" - store it with: moneyball crm secret {}",
                    name, name
                ))
            })?;
            self.used.push((got.clone(), format!("secret:{}", name)));
            return Ok(got);
        }
        if let Some(var) = value.strip_prefix("env:") {
            let got = std::env::var(var)
                .map_err(|_| Error::Secrets(format!("env var {} is not set", var)))?;
            self.used.push((got.clone(), value.to_string()));
            return Ok(got);
        }
        Ok(value.to_string())
    }

    /// A body with its `{secret:<name>}` refs filled, each JSON-escaped:
    /// a ref sits inside a JSON string.
    pub(super) fn body(&mut self, body: &str) -> Result<String> {
        let mut out = body.to_string();
        for name in body_refs(body) {
            let value = self.value(&format!("secret:{}", name))?;
            let quoted = serde_json::to_string(&value)?;
            let escaped = quoted
                .strip_prefix('"')
                .and_then(|q| q.strip_suffix('"'))
                .unwrap_or(&quoted);
            out = out.replace(&format!("{{secret:{}}}", name), escaped);
        }
        Ok(out)
    }

    /// `text` with every value this request used shown as its ref.
    pub(super) fn mask(&self, text: &str) -> String {
        self.used
            .iter()
            .filter(|(value, _)| value.chars().count() >= MIN_MASKED)
            .fold(text.to_string(), |t, (value, r)| {
                t.replace(value.as_str(), r)
            })
    }

    /// The same, over every string in a response.
    pub(super) fn mask_json(&self, v: &mut Value) {
        if self.used.is_empty() {
            return;
        }
        match v {
            Value::String(s) => *s = self.mask(s),
            Value::Array(a) => a.iter_mut().for_each(|v| self.mask_json(v)),
            Value::Object(o) => o.values_mut().for_each(|v| self.mask_json(v)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_resolve_and_mask_back_to_their_refs() {
        let mut refs = Refs::with(|name| (name == "x_key").then(|| r#"pa"ss\word"#.to_string()));
        assert_eq!(refs.value("{secret:x_key}").unwrap(), r#"pa"ss\word"#);
        assert_eq!(refs.value("{page}").unwrap(), "{page}");
        assert!(refs.value("secret:other").is_err());
        std::env::set_var("MB_CRM_TEST_KEY", "k-123456");
        assert_eq!(refs.value("env:MB_CRM_TEST_KEY").unwrap(), "k-123456");
        std::env::remove_var("MB_CRM_TEST_KEY");
        assert!(refs.value("env:MB_CRM_TEST_KEY").is_err());

        // A body ref is filled JSON-escaped; templates stay for expand().
        let body = refs
            .body(r#"{"k": "{secret:x_key}", "from": "{from_date}"}"#)
            .unwrap();
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["k"], r#"pa"ss\word"#);
        assert_eq!(v["from"], "{from_date}");

        // Whatever the CRM echoes comes back as the ref.
        assert_eq!(
            refs.mask(r#"bad key pa"ss\word for k-123456"#),
            "bad key secret:x_key for env:MB_CRM_TEST_KEY"
        );
        let mut echoed = serde_json::json!({"rows": [{"note": r#"key pa"ss\word"#}]});
        refs.mask_json(&mut echoed);
        assert_eq!(echoed["rows"][0]["note"], "key secret:x_key");
    }
}
