# HANDOFF.md - moneyball-cli

Running notes. Prune aggressively; this file ages in days, not months. Each
entry: 1-3 lines, dated, with the decision and the why. Decisions the code,
ARCHITECTURE.md or TODO.md already record do not belong here.

## 2026-07-14 - no target cost per qualified in setup

Setup (5 steps: workspace, Meta token, products, goals, model) never asks
for `target_rs_per_q`: it varies per product and industry (~Rs.100
e-commerce, ~Rs.2,500 real estate). The field stays None unless edited in
config.json; the funnel's kill table then uses Rs.2,500.

(Pruned 2026-10-22: the July notes on paste handling, session files, the
brief layout, Esc keys and /ask had all been superseded by the code.)
