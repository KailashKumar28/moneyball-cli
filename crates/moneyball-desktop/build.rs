//! Tauri's build step, plus a stand-in page when the frontend has not
//! been built: `generate_context!` embeds `ui/dist` at compile time, so
//! without one a fresh checkout could not run `cargo test --workspace`.
//! `pnpm build` in ui/ replaces the stand-in; it is never committed.

use std::path::Path;

const STAND_IN: &str = "<!doctype html><meta charset=\"utf-8\"><title>Moneyball</title>\
<p>The interface is not built yet. Run <code>pnpm install &amp;&amp; pnpm build</code> in \
crates/moneyball-desktop/ui, then build again.</p>\n";

fn main() {
    let dist = Path::new("ui/dist");
    if !dist.join("index.html").is_file() {
        std::fs::create_dir_all(dist).expect("create ui/dist for the stand-in page");
        std::fs::write(dist.join("index.html"), STAND_IN).expect("write the stand-in page");
    }
    tauri_build::build();
}
