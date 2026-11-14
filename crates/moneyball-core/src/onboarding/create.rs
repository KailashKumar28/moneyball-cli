//! Making a client (plan Step 2): its folder, its config, its place in
//! the client list - all of it, or nothing.

use std::collections::HashSet;
use std::path::Path;

use super::clients;
use crate::config::{self, home_dir, AppConfig, Product, WorkspaceConfig, DOT_DIR};
use crate::error::{Error, Result};
use crate::{meta, text, workspace};

/// Where a client goes unless the user puts it elsewhere (D3).
const PARENT: &str = "Moneyball";

/// Make the client `name` from `products`: its folder (`~/Moneyball/<name>`
/// unless `folder` says otherwise), its config with those products, the
/// name as its brand and a minted workspace id, and its place in the client
/// list; returns it opened. On a folder that already holds a workspace the
/// products are replaced and the rest is kept (the id, the brand, the CRM).
/// Refused with nothing written for a blank name or one no folder can
/// have, no products or two of one name, or a name another known client
/// has; a write that fails takes back what was written before it.
pub fn create_client(
    folder: Option<&Path>,
    name: &str,
    products: Vec<Product>,
) -> Result<AppConfig> {
    let home = home_dir()
        .ok_or_else(|| Error::Config("no home directory to keep the client in".into()))?;
    create_in(&home, folder, name, products)
}

fn create_in(
    home: &Path,
    folder: Option<&Path>,
    name: &str,
    products: Vec<Product>,
) -> Result<AppConfig> {
    let name = name.trim();
    check_name(name)?;
    let products = check_products(name, products)?;
    let global = home.join(DOT_DIR);
    let dir = folder.map_or_else(|| home.join(PARENT).join(name), Path::to_path_buf);
    let here = workspace::canonical(&dir);
    let lower = name.to_lowercase();
    let known = clients::list_in(&global);
    if let Some(other) = known
        .iter()
        .find(|c| c.data_root != here && clients::name(c).to_lowercase() == lower)
    {
        return Err(Error::Config(format!(
            "Another client is already called {} (in {}). Pick another name.",
            name,
            text::tilde(&other.data_root)
        )));
    }
    // A config already there that will not read stops it before any write.
    match WorkspaceConfig::load(&dir) {
        Ok(_) | Err(Error::NoWorkspaceConfig { .. }) => {}
        Err(e) => return Err(e),
    }
    // What a later failure takes back: the topmost folder this call makes,
    // and the list entry when it is new.
    let fresh = [dir.clone(), dir.join(DOT_DIR)]
        .into_iter()
        .find(|p| !p.exists());
    let mut added = false;
    let made = (|| {
        std::fs::create_dir_all(&dir)?;
        added = clients::add_in(&global, &dir)?;
        config::update(&dir, |w| {
            w.products = products;
            w.brand.get_or_insert_with(|| name.to_string());
        })?;
        workspace::open(&dir, None)
    })();
    if made.is_err() {
        if added {
            let _ = clients::remove_in(&global, &dir);
        }
        if let Some(f) = fresh {
            let _ = std::fs::remove_dir_all(f);
        }
    }
    made
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::Config("Give the client a name.".into()));
    }
    if name.starts_with('.')
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
    {
        return Err(Error::Config(format!(
            "A folder cannot be called \"{}\": leave out / \\ : and a leading dot.",
            name
        )));
    }
    Ok(())
}

/// The products as they are stored: trimmed names, account ids without
/// "act_".
fn check_products(client: &str, products: Vec<Product>) -> Result<Vec<Product>> {
    if products.is_empty() {
        return Err(Error::Config(format!(
            "Pick at least one ad account for {}.",
            client
        )));
    }
    let mut seen = HashSet::new();
    products
        .into_iter()
        .map(|p| {
            let name = p.name.trim().to_string();
            let account = meta::account_id_for_storage(p.ad_account.trim());
            if name.is_empty() {
                return Err(Error::Config("Every product needs a name.".into()));
            }
            if account.is_empty() || !account.chars().all(|c| c.is_ascii_digit()) {
                return Err(Error::Config(format!(
                    "{}: \"{}\" is not an ad account id (digits, as Meta lists them).",
                    name, p.ad_account
                )));
            }
            if !seen.insert(name.clone()) {
                return Err(Error::Config(format!(
                    "Two products are called {}: give each its own name.",
                    name
                )));
            }
            Ok(Product {
                name,
                ad_account: account,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "create_tests.rs"]
mod tests;
