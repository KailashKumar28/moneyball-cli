use std::path::PathBuf;

use super::*;
use crate::config::WorkspaceConfig;

/// A home of its own: `~/Moneyball/` and `~/.moneyball/` inside it.
fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mb-create-{}-{}", name, std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    workspace::canonical(&dir)
}

fn product(name: &str, account: &str) -> Product {
    Product {
        name: name.into(),
        ad_account: account.into(),
    }
}

#[test]
fn a_client_gets_its_folder_config_id_and_place_in_the_list() {
    let home = home("happy");
    let cfg = create_in(
        &home,
        None,
        " Fincity ",
        vec![product("Primus ", "act_405885579167395")],
    )
    .unwrap();
    let dir = home.join("Moneyball").join("Fincity");
    assert_eq!(cfg.data_root, dir);
    let w = cfg.workspace.as_ref().unwrap();
    assert_eq!(w.brand.as_deref(), Some("Fincity"));
    assert_eq!(w.products[0].name, "Primus");
    assert_eq!(w.products[0].ad_account, "405885579167395");
    let id = w.workspace_id.clone().expect("minted");
    let known = clients::list_in(&home.join(DOT_DIR));
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].data_root, dir);

    // A re-run replaces the products and keeps the id, the brand and the
    // CRM - read from disk, so a change made since is kept too.
    config::update(&dir, |w| {
        w.brand = Some("Fincity Homes".into());
        w.crm.name = "leadzump".into();
    })
    .unwrap();
    let again = create_in(
        &home,
        None,
        "Fincity",
        vec![product("Disha", "2087011578504572")],
    )
    .unwrap();
    let w = again.workspace.unwrap();
    assert_eq!(w.workspace_id.as_deref(), Some(id.as_str()));
    assert_eq!(w.brand.as_deref(), Some("Fincity Homes"));
    assert_eq!(w.crm.name, "leadzump");
    assert_eq!(w.product_names(), ["Disha"]);
    assert_eq!(clients::list_in(&home.join(DOT_DIR)).len(), 1);
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn a_name_another_client_has_is_refused_and_nothing_is_written() {
    let home = home("taken");
    create_in(&home, None, "Fincity", vec![product("P", "111111")]).unwrap();
    let elsewhere = home.join("clients").join("fincity-2");
    let err = create_in(
        &home,
        Some(&elsewhere),
        "fincity",
        vec![product("P", "222222")],
    )
    .unwrap_err();
    assert!(err.reason().contains("already called fincity"), "{}", err);
    assert!(!elsewhere.exists());
    assert_eq!(clients::list_in(&home.join(DOT_DIR)).len(), 1);
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn a_bad_name_or_products_are_refused_before_any_write() {
    let home = home("refused");
    let one = || vec![product("P", "111111")];
    for name in ["", "  ", ".hidden", "a/b", "a:b"] {
        assert!(create_in(&home, None, name, one()).is_err(), "{:?}", name);
    }
    for products in [
        vec![],
        vec![product(" ", "111111")],
        vec![product("P", "act_x1")],
        vec![product("P", "111111"), product("P", "222222")],
    ] {
        assert!(create_in(&home, None, "Fincity", products).is_err());
    }
    let entries: Vec<_> = std::fs::read_dir(&home).unwrap().collect();
    assert!(entries.is_empty(), "{:?}", entries);
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn a_failure_leaves_nothing_behind() {
    let home = home("rollback");
    // The folder holds a config that will not parse: refused before any
    // write, and the file is left as it was.
    let dir = home.join("Moneyball").join("Fincity");
    std::fs::create_dir_all(dir.join(DOT_DIR)).unwrap();
    std::fs::write(dir.join(DOT_DIR).join("config.json"), "{ not json").unwrap();
    assert!(create_in(&home, None, "Fincity", vec![product("P", "111111")]).is_err());
    let raw = std::fs::read_to_string(dir.join(DOT_DIR).join("config.json")).unwrap();
    assert_eq!(raw, "{ not json");
    let global = home.join(DOT_DIR);
    assert!(!global.exists(), "the client list is not touched");

    // The client list will not parse, found only once the folder is
    // made: the folder goes again.
    std::fs::create_dir_all(&global).unwrap();
    std::fs::write(global.join("config.json"), "{ not json").unwrap();
    assert!(create_in(&home, None, "Disha", vec![product("P", "111111")]).is_err());
    assert!(!home.join("Moneyball").join("Disha").exists());
    assert!(WorkspaceConfig::load(&home.join("Moneyball").join("Disha")).is_err());
    std::fs::remove_dir_all(&home).ok();
}
