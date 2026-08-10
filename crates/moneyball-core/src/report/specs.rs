//! Adset spec summaries (geo/age/learning) + archetype naming for the
//! targeting section. Split from targeting.rs (cap).

use crate::schema::TargetingSpecs;
use crate::snapshot::Snapshot;

/// python _archetype, name-based.
pub(super) fn archetype(name: &str) -> String {
    let n = name.to_lowercase();
    if n.contains("pincode") || n.contains("pin code") {
        "Pincode"
    } else if n.contains("lookalike") || n.contains("lal") {
        "Lookalike"
    } else if n.contains("income") || n.contains("broad") || n.contains("advantage") {
        "Broad-Income"
    } else if n.contains("detailed") || n.contains("interest") || n.contains("nri") {
        "Detailed"
    } else {
        "Other"
    }
    .into()
}

/// Geo/age/learning summary from adsets.json for the targeting's adsets.
pub(super) fn specs_for(snap: &Snapshot, adset_ids: &[String]) -> Option<TargetingSpecs> {
    let a = adset_ids
        .iter()
        .find_map(|id| snap.adsets.get(id).filter(|v| v.is_object()))?;
    let t = a.get("targeting");
    let geo = t.and_then(|t| t.get("geo_locations")).map(|g| {
        let mut names: Vec<String> = Vec::new();
        for key in ["cities", "regions", "custom_locations"] {
            for e in g.get(key).and_then(|v| v.as_array()).into_iter().flatten() {
                if let Some(n) = e.get("name").and_then(|v| v.as_str()) {
                    let radius = e
                        .get("radius")
                        .and_then(|r| r.as_f64())
                        .map(|r| format!(" {}km", r))
                        .unwrap_or_default();
                    names.push(format!("{}{}", n, radius));
                }
            }
        }
        if names.is_empty() {
            for c in g
                .get("countries")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                if let Some(n) = c.as_str() {
                    names.push(n.to_string());
                }
            }
        }
        match names.len() {
            0 => "-".to_string(),
            1 | 2 => names.join(", "),
            n => format!("{} +{} more", names[..2].join(", "), n - 2),
        }
    });
    let age = t.and_then(|t| {
        let lo = t.get("age_min").and_then(|v| v.as_u64())?;
        let hi = t.get("age_max").and_then(|v| v.as_u64()).unwrap_or(65);
        Some(format!("{}-{}", lo, hi))
    });
    let genders = t
        .and_then(|t| t.get("genders"))
        .and_then(|g| g.as_array())
        .map(|g| match g.first().and_then(|v| v.as_u64()) {
            Some(1) if g.len() == 1 => "men".to_string(),
            Some(2) if g.len() == 1 => "women".to_string(),
            _ => "all".to_string(),
        });
    let learning = a
        .pointer("/learning_stage_info/status")
        .and_then(|v| v.as_str())
        .map(String::from);
    let optimization_goal = a
        .get("optimization_goal")
        .and_then(|v| v.as_str())
        .map(String::from);
    Some(TargetingSpecs {
        geo,
        age,
        genders,
        learning,
        optimization_goal,
    })
}
