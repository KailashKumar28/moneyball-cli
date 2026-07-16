# CLAUDE.md - moneyball-cli

Read these before writing code; they are binding:

1. [AGENTS.md](AGENTS.md) - process + domain rules (network boundary,
   definition of done, Don'ts). When docs conflict, AGENTS.md wins.
2. [ARCHITECTURE.md](ARCHITECTURE.md) - structure & style contract,
   including §6b Agent core (history model, tool loop, cancellation,
   sessions - researched from codex-rs and pi-mono source).
3. [TODO.md](TODO.md) - the backlog; pick from the top, check items off
   in the same commit that ships them.

Hard rules that bite most often:

- Reference architectures are openai/codex (codex-rs) and badlogic/pi-mono
  ONLY. hermes-agent is explicitly excluded (AGENTS.md Don'ts).
- E2E first: reproduce bugs by driving the installed binary (tmux for the
  TUI) before fixing; a fix that isn't `cargo install`ed isn't shipped.
- Never slice strings by byte index (`&s[..n]`) - lead names and error
  bodies contain multibyte chars; use char-boundary walks.
- Never advertise unimplemented commands in COMMANDS or system prompts -
  the LLM will steer users into dead ends.
- Tool/LLM failures become messages in the loop, never exceptions or
  dead turns. Secrets never appear in errors, logs, or specs.
- ASCII only in TUI-facing strings we author (model output is exempt;
  the markdown renderer handles it).
