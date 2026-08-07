//! Pins the persisted-artifact contracts (docs/CLOUD_PLAN.md, slice A0).
//!
//! The serde structs in `moneyball_core::schema` are the source of
//! truth; this test generates their JSON Schemas and asserts they match
//! the committed files in `docs/schemas/`. A wire-shape edit therefore
//! fails CI until the schema file is consciously regenerated and the
//! diff reviewed:
//!
//!     MB_UPDATE_SCHEMAS=1 cargo test -p moneyball-core --test schema_contract

use std::path::{Path, PathBuf};

use moneyball_core::schema::{CreativeReport, CreativesFile};

fn schemas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/schemas")
}

fn generated(name: &str) -> (PathBuf, String) {
    let schema = match name {
        "creatives.v1" => schemars::schema_for!(CreativesFile),
        "creative_report.v1" => schemars::schema_for!(CreativeReport),
        _ => unreachable!(),
    };
    let json = serde_json::to_string_pretty(&schema).expect("schema serializes");
    (schemas_dir().join(format!("{}.schema.json", name)), json)
}

#[test]
fn committed_schemas_match_the_structs() {
    let update = std::env::var_os("MB_UPDATE_SCHEMAS").is_some();
    let mut stale = Vec::new();
    for name in ["creatives.v1", "creative_report.v1"] {
        let (path, json) = generated(name);
        if update {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("{}\n", json)).unwrap();
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(committed) if committed.trim_end() == json => {}
            Ok(_) => stale.push(format!("{} (differs)", path.display())),
            Err(_) => stale.push(format!("{} (missing)", path.display())),
        }
    }
    assert!(
        stale.is_empty(),
        "artifact structs changed without re-committing their schemas.\n\
         Review the wire-shape change (additive-only within a major!), then:\n\
         MB_UPDATE_SCHEMAS=1 cargo test -p moneyball-core --test schema_contract\n\
         and commit the diff under docs/schemas/.\n{}",
        stale.join("\n")
    );
}

/// The committed fixture parses through the typed structs - the same
/// forward-compat path the snapshot loader will use in slice A1.
#[test]
fn fixture_creatives_round_trips() {
    let p =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/snap/2026-08-06/creatives.json");
    let raw = std::fs::read_to_string(&p).expect("fixture present");
    let f: CreativesFile = serde_json::from_str(&raw).expect("fixture parses");
    assert_eq!(f.schema, moneyball_core::schema::CREATIVES_SCHEMA);
    assert_eq!(f.rows.len(), 3);

    // Image ad: has a hash + cached asset.
    let img = &f.rows[0];
    assert_eq!(img.image_hash.as_deref(), Some("a3f9c2d1e0b84756"));
    assert!(!img.is_video);
    let asset = img.asset.as_ref().expect("cached");
    assert_eq!(asset.sha256.len(), 64);

    // Video ad: video identity, no image hash.
    let vid = &f.rows[1];
    assert!(vid.is_video && vid.image_hash.is_none());
    assert_eq!(vid.video_id.as_deref(), Some("893471002316584"));

    // Failed download: asset null renders a placeholder, never an error.
    assert!(f.rows[2].asset.is_none());

    // Round trip preserves the wire shape (serialize -> reparse -> eq
    // on the JSON value, since the structs don't derive PartialEq).
    let v1: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let v2 = serde_json::to_value(&f).unwrap();
    for (i, row) in v1["rows"].as_array().unwrap().iter().enumerate() {
        for (k, val) in row.as_object().unwrap() {
            assert_eq!(&v2["rows"][i][k], val, "row {} field {}", i, k);
        }
    }
}
