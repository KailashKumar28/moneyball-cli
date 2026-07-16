You are working in the `moneyball-cli` Rust workspace. Read `AGENTS.md` first -
its "Definition of done" and "Don'ts" are binding. moneyball is a GENERIC,
read-only Meta-ads advisor: it works against any account's snapshot files and
must NEVER call Meta/CRM APIs from core. Tests are hermetic (fixtures under
`crates/*/tests/`) - you never need a real account or the network.

TASK: {{task}}

Run this build-test loop until the task is done and the workspace is green.
Do NOT stop while the build is broken or tests are red.

1. PLAN - state the ONE smallest shippable slice you'll do now. One slice at a time.
2. IMPLEMENT - edit only what the slice needs. No `unwrap()` in library code;
   errors via `thiserror` in core, `anyhow` only at the binary edge.
3. BUILD - `cargo build --workspace`. On failure, read the errors, fix, rebuild.
   Loop until clean.
4. TEST - `cargo test --workspace`. If red, fix the CODE (not the test, unless the
   test is provably wrong) and rerun. Loop until green.
5. SEE IT - render the affected surface headlessly via TestBackend and READ the
   output. This is how you "see the difference" without a TTY:
     cargo run --example render_app   -p moneyball-tui
     cargo run --example render_views -p moneyball-tui -- welcome
     cargo run --example render_views -p moneyball-tui -- setup 1 1
   If the UI is wrong (placeholders, clipped text, mis-aligned columns, bad
   colors), fix it and re-render. UI placeholders are bugs.
6. DIFF - `git --no-pager diff --stat`, then summarize what changed and why.
7. REPORT - paste the final `cargo test` summary + the relevant rendered view,
   then STOP for sign-off (AGENTS.md: one fix at a time, then wait).

Hard stops - if a step genuinely needs a real Meta account, the network, or data
you don't have, STOP and say so. Never fake output or weaken a test to pass.
