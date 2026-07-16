// Round-trip the REAL keychain via the same code path the wizard uses.
fn main() {
    let probe = "kc-selftest-value-12345";
    match moneyball_core::secrets::store_meta_token(probe) {
        Ok(()) => println!("write: OK"),
        Err(e) => {
            println!("write FAILED: {}", e);
            return;
        }
    }
    match moneyball_core::secrets::load_meta_token() {
        Some(v) if v == probe => println!("read-back: OK (matches)"),
        Some(_) => println!("read-back: MISMATCH"),
        None => println!("read-back: NOT FOUND (the user's bug)"),
    }
    match moneyball_core::secrets::clear_meta_token() {
        Ok(()) => println!("cleanup: OK"),
        Err(e) => println!("cleanup failed: {}", e),
    }
}
