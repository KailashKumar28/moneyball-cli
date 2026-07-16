//! CRM connector presets - the keys-only connect catalog.
//!
//! The pattern every embedded-integration product converged on (Nango,
//! Fivetran, Merge): the user picks their CRM and pastes CREDENTIALS
//! ONLY; the catalog carries endpoint/paging/field-map, and a live
//! fetch test gates the save. Only presets grounded in real evidence
//! ship here: a wrong guess would fail the test step, but a vetted
//! preset is the product promise.

/// One credential the user must paste, with where-to-find help.
pub struct SecretPrompt {
    /// Suffix of the stored secret name: `<preset_id>_<key>`.
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
}

pub struct Preset {
    pub id: &'static str,
    pub display: &'static str,
    /// crm.toml template; `{secret:<key>}` placeholders become
    /// `secret:<preset_id>_<key>` refs at render time.
    pub toml: &'static str,
    pub secrets: &'static [SecretPrompt],
    /// One-line caveat shown before the test run.
    pub note: &'static str,
}

impl Preset {
    /// Final crm.toml with secret refs resolved to stored names.
    pub fn render(&self) -> String {
        let mut out = self.toml.to_string();
        for s in self.secrets {
            out = out.replace(
                &format!("{{secret:{}}}", s.key),
                &format!("secret:{}_{}", self.id, s.key),
            );
        }
        out
    }

    /// Stored secret name for a prompt (what `crm secret` would use).
    pub fn secret_name(&self, s: &SecretPrompt) -> String {
        format!("{}_{}", self.id, s.key)
    }
}

/// Verified presets. LeadZump's shape is proven by a production
/// pipeline; LeadSquared follows its public API docs and is gated by
/// the live test at connect time.
pub fn catalog() -> Vec<Preset> {
    vec![
        Preset {
            id: "leadzump",
            display: "LeadZump",
            toml: r#"name = "leadzump"

[request]
url = "https://leadzump.ai/api/entity/processor/tickets/eager/query?fetchOwners=false"
method = "POST"
body = '{"condition": null, "eager": true, "size": 200, "page": {page}}'

[request.headers]
"appcode" = "leadzump"
"clientcode" = "SYSTEM"
"authorization" = "{secret:token}"

[paging]
mode = "page"
param = "page"
start = 0
size = 200

[map]
root = "content"
ad_id = "adId.adId"
stage = "stage.name"
delivery = "delivery"
funnel = "status.funnelStage"

[map.stage_map]
"Non Contactable" = "NonContactable"
"#,
            secrets: &[SecretPrompt {
                key: "token",
                label: "API token",
                help: "the LeadZump auth token (same value your team uses as LEADZUMP_TOKEN)",
            }],
            note: "pulls all tickets; pages of 200",
        },
        Preset {
            id: "leadsquared",
            display: "LeadSquared",
            toml: r#"name = "leadsquared"

[request]
url = "https://api-in21.leadsquared.com/v2/LeadManagement.svc/Leads.Get"

[request.query]
accessKey = "{secret:access_key}"
secretKey = "{secret:secret_key}"
fromDate = "{from_date}"
toDate = "{to_date}"

[paging]
mode = "page"
param = "pageIndex"
start = 1
size_param = "pageSize"
size = 500

[map]
root = ""
ad_id = "mx_Ad_Id"
stage = "ProspectStage"
delivery = "mx_Delivery_Time"

[map.stage_map]
"Qualified" = "Contactable"
"Site Visit" = "Visit"
"Won" = "Booking"
"#,
            secrets: &[
                SecretPrompt {
                    key: "access_key",
                    label: "Access Key",
                    help: "LeadSquared: My Profile > Settings > API and Webhooks > Access Key",
                },
                SecretPrompt {
                    key: "secret_key",
                    label: "Secret Key",
                    help: "shown next to the Access Key on the same page",
                },
            ],
            note: "assumes the in21 region host and mx_* ad-attribution fields - the test \
                   run verifies both; if it fails, edit .moneyball/crm.toml (host/fields)",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_renders_to_a_parseable_spec_with_no_placeholders() {
        for p in catalog() {
            let rendered = p.render();
            assert!(
                !rendered.contains("{secret:"),
                "{}: unresolved placeholder",
                p.id
            );
            let spec = super::super::source::parse(&rendered)
                .unwrap_or_else(|e| panic!("{}: {}", p.id, e));
            assert_eq!(spec.name, p.id);
            for s in p.secrets {
                assert!(
                    rendered.contains(&format!("secret:{}", p.secret_name(s))),
                    "{}: secret ref {} missing",
                    p.id,
                    s.key
                );
            }
        }
    }
}
