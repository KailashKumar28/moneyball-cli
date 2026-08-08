//! Product identity helpers - resolving typed/suggested product names
//! against the workspace. Split from config.rs (size cap).

use crate::config::Product;

/// Resolve a user- or model-typed product name against the workspace:
/// exact, else case-insensitive, else unique substring. Err carries the
/// candidate list so every surface can print an actionable message
/// (live QA 2026-08-08: the advisor suggested "/funnel Namma Mane" for
/// the product "Disha Namma Mane" - typed suggestions must always run).
pub fn resolve_product<'a>(
    input: &str,
    products: &'a [Product],
) -> std::result::Result<&'a str, String> {
    let want = input.trim();
    let names: Vec<&str> = products.iter().map(|p| p.name.as_str()).collect();
    if let Some(n) = names.iter().find(|n| **n == want) {
        return Ok(n);
    }
    let lower = want.to_lowercase();
    let ci: Vec<&&str> = names.iter().filter(|n| n.to_lowercase() == lower).collect();
    if ci.len() == 1 {
        return Ok(ci[0]);
    }
    let sub: Vec<&&str> = names
        .iter()
        .filter(|n| n.to_lowercase().contains(&lower))
        .collect();
    match (sub.len(), want.is_empty()) {
        (1, false) => Ok(sub[0]),
        _ => Err(format!(
            "unknown product \"{}\" - configured: {}",
            want,
            names.join(", ")
        )),
    }
}

#[cfg(test)]
mod product_match_tests {
    use super::*;

    fn prods() -> Vec<Product> {
        [
            "Disha Namma Mane",
            "Cityville by Fincity",
            "Purva Sparkling Spring by Fincity",
        ]
        .iter()
        .map(|n| Product {
            name: n.to_string(),
            ad_account: "act_1".into(),
        })
        .collect()
    }

    #[test]
    fn exact_case_insensitive_and_unique_substring_match() {
        let p = prods();
        assert_eq!(
            resolve_product("Disha Namma Mane", &p).unwrap(),
            "Disha Namma Mane"
        );
        assert_eq!(
            resolve_product("disha namma mane", &p).unwrap(),
            "Disha Namma Mane"
        );
        // The exact live failure: the model's shorthand now resolves.
        assert_eq!(
            resolve_product("Namma Mane", &p).unwrap(),
            "Disha Namma Mane"
        );
        assert_eq!(
            resolve_product("cityville", &p).unwrap(),
            "Cityville by Fincity"
        );
    }

    #[test]
    fn ambiguous_or_unknown_errors_with_candidates() {
        let p = prods();
        // "Fincity" matches two products - must not guess.
        let e = resolve_product("Fincity", &p).unwrap_err();
        assert!(e.contains("configured:"), "{}", e);
        assert!(resolve_product("Nope", &p).is_err());
        assert!(resolve_product("", &p).is_err());
    }
}
