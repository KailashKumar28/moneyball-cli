//! Workspace id contract: minted exactly once, at load, and persisted -
//! so report generation (and any in-memory clone of the config) only
//! ever reads the one tenant key.

use moneyball_core::config::WorkspaceConfig;

#[test]
fn workspace_id_is_minted_once_at_load_and_persisted() {
    let root = std::env::temp_dir().join(format!("mb-wsid-{}", std::process::id()));
    let dot = root.join(".moneyball");
    std::fs::create_dir_all(&dot).unwrap();
    std::fs::write(dot.join("config.json"), r#"{"products": []}"#).unwrap();

    let first = WorkspaceConfig::load(&root).unwrap().workspace_id.unwrap();
    let second = WorkspaceConfig::load(&root).unwrap().workspace_id.unwrap();
    assert_eq!(first, second, "a second load must not mint again");
    let on_disk = std::fs::read_to_string(dot.join("config.json")).unwrap();
    assert!(on_disk.contains(&first), "id persisted: {}", on_disk);
    std::fs::remove_dir_all(&root).ok();
}
