# moneyball test harness

Visual TDD harness for moneyball-cli. Generic, project-agnostic.

## What it does

1. **Spawn** the CLI (or any TUI) in a managed PTY via `pilotty`
2. **Drive** it with key/type/click/wait-for (pilotty primitives)
3. **Capture** a PNG screenshot via the ANSI stream + pyte + Pillow
4. **Review** actual vs expected via `desired-state/index.html`

## Files

```
.test-harness/
├── features.toml                  # Feature inventory + desired state (per-feature)
├── desired-state/
│   └── index.html                 # Browser-based viewer (open in any browser)
├── screenshots/
│   └── <feature-id>/latest.png    # Latest capture per feature
└── README.md                      # This file
```

Generic scripts (reusable across projects) live at
`~/.pi/agent/harness/tui-test/`:

- `capture.sh <session> <command...> [out.png]` — spawn + capture
- `render.py` — ANSI stream → PNG via pyte + Pillow

## Workflow

```bash
# 1. Capture a feature's screenshot (generic, works for any TUI)
~/.pi/agent/harness/tui-test/capture.sh \
    brief-command \
    moneyball brief --data-root /Users/Kailash/DEV/MOD_AI/fin_campaign_analysis \
    .test-harness/screenshots/brief-command/latest.png

# 2. Open the viewer
open .test-harness/desired-state/index.html

# 3. Update features.toml: status = passing | failing | blocked, add notes
```

## Loop (when run inside pi)

For each pending feature:

1. Capture current state
2. Compare to `expected` in features.toml
3. If visual/behavioral defect:
   - Diagnose root cause (read source, check parity vs mb.py)
   - If behavior unclear → ASK the user
   - Fix in source
   - `cargo build --workspace` → `cargo test` →
     `cargo install --path crates/moneyball --locked --quiet`
   - Re-capture
4. Mark passing in features.toml

## Substitutions / known limits

- `₹` (U+20B9) renders as `Rs` in PNGs (macOS fonts exposed via PIL don't
  include it; real terminal shows the correct glyph)
- Box-drawing chars (`┌─┐│└┘`) render via Menlo + Arial Unicode fallback
- Pilotty appends a `retention: format=...` line to the ANSI stream which is
  stripped by capture.sh
- `pilotty snapshot` returns plain text only; for visual review we use the ANSI
  history (`pilotty output --ansi`) and replay it through pyte
