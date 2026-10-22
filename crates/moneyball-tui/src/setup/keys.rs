//! Setup keys: the account picker's list keys, typing and backspace into
//! whichever field the step has. Split from setup/mod.rs (size cap).

use super::*;

pub(crate) fn handle_setup_key(app: &mut App, mut state: SetupState, k: KeyEvent) {
    // Substep 1 of step 1 is a list-selection mode with its own keymap.
    if state.step == 1 && state.meta_substep == 1 {
        handle_select_keys(&mut state, k);
        app.view = View::Setup(state);
        return;
    }
    // Step 4 picker substeps (provider pick, model pick) have their own
    // arrow-key navigation. Other keys (Enter, Esc, Char) fall through
    // to the default handler so Enter advances to the next substep.
    if state.step == 4
        && (state.llm_substep == 0 || state.llm_substep == 2)
        && handle_llm_picker_keys(&mut state, k)
    {
        app.view = View::Setup(state);
        return;
    }
    // Substep 2 of step 1 is the rename buffer. Esc goes back to substep 1.
    if state.step == 1 && state.meta_substep == 2 && k.code == KeyCode::Esc {
        state.meta_substep = 1;
        state.meta_rename_input.clear();
        state.error = None;
        app.view = View::Setup(state);
        return;
    }
    // Char / Enter / Backspace fall through to the default handler below.
    match k.code {
        KeyCode::Esc => {
            // Esc: clear the active input if any, else leave the wizard
            // for the chat - the footer advertises "esc exit setup".
            let cleared = match state.step {
                0 if !state.workspace_path.is_empty() => {
                    state.workspace_path.clear();
                    true
                }
                1 => match state.meta_substep {
                    0 if !state.meta_input.is_empty() => {
                        state.meta_input.clear();
                        true
                    }
                    _ => false,
                },
                2 if !state.product_input.is_empty() => {
                    state.product_input.clear();
                    true
                }
                3 if !state.goals_input.is_empty() => {
                    state.goals_input.clear();
                    true
                }
                // The custom provider's model: back to its URL, not out.
                4 if state.llm_substep == 3 && state.llm_url_entered => {
                    state.llm_url_entered = false;
                    true
                }
                _ => false,
            };
            if cleared {
                state.error = None;
            } else {
                // Esc leaves the wizard for the chat (Claude Code modal
                // behavior) - saved steps are kept; /setup resumes with
                // current values prefilled.
                app.view = View::Brief;
                app.status = Some("setup closed - /setup reopens it".into());
                return;
            }
        }
        KeyCode::Enter => {
            advance_setup(app, &mut state);
        }
        KeyCode::Backspace => {
            backspace_setup(&mut state);
        }
        KeyCode::Char(c) => {
            insert_setup(&mut state, c);
        }
        _ => {}
    }
    // advance_save may have transitioned us out to View::Brief. Don't clobber that.
    if app.view != View::Brief {
        app.view = View::Setup(state);
    }
}

/// Keyboard handler for the multi-account selection list (step 1 substep 1).
fn handle_select_keys(s: &mut SetupState, k: KeyEvent) {
    let n = s.meta_discovered.len();
    if n == 0 {
        return;
    }
    // Visible rows must match the renderer's visible_rows constant below.
    const VISIBLE_ROWS: usize = 12;
    match k.code {
        KeyCode::Up => {
            if s.meta_highlight > 0 {
                s.meta_highlight -= 1;
                if s.meta_highlight < s.meta_scroll {
                    s.meta_scroll = s.meta_highlight;
                }
            }
        }
        KeyCode::Down => {
            if s.meta_highlight + 1 < n {
                s.meta_highlight += 1;
                if s.meta_highlight >= s.meta_scroll + VISIBLE_ROWS {
                    s.meta_scroll = s.meta_highlight + 1 - VISIBLE_ROWS;
                }
            }
        }
        KeyCode::PageUp => {
            s.meta_highlight = s.meta_highlight.saturating_sub(VISIBLE_ROWS);
            s.meta_scroll = s.meta_scroll.saturating_sub(VISIBLE_ROWS);
        }
        KeyCode::PageDown => {
            s.meta_highlight = (s.meta_highlight + VISIBLE_ROWS).min(n - 1);
            s.meta_scroll = (s.meta_highlight + 1).saturating_sub(VISIBLE_ROWS);
        }
        KeyCode::Home => {
            s.meta_highlight = 0;
            s.meta_scroll = 0;
        }
        KeyCode::End => {
            s.meta_highlight = n - 1;
            s.meta_scroll = n.saturating_sub(VISIBLE_ROWS);
        }
        KeyCode::Char(' ') => {
            s.meta_selections[s.meta_highlight] = !s.meta_selections[s.meta_highlight];
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            let any = s.meta_selections.iter().any(|&b| b);
            for sel in s.meta_selections.iter_mut() {
                *sel = !any;
            }
        }
        KeyCode::Enter => {
            let chosen: Vec<usize> = (0..n).filter(|&i| s.meta_selections[i]).collect();
            if chosen.is_empty() {
                s.error = Some("select at least one account (Space to toggle, 'a' for all)".into());
                return;
            }
            s.meta_selected = chosen;
            s.meta_substep = 2;
            s.error = None;
        }
        KeyCode::Esc => {
            // "back" - return to substep 0 (token paste) WITHOUT nuking the
            // user's progress. The discovered account list and the per-row
            // checkbox state survive so the user doesn't have to re-select
            // after re-validating. The token itself was already cleared
            // from `meta_input` after validation, so we restore a masked
            // placeholder (N bullets of `meta_token_len`) so the input
            // box doesn't look empty.
            s.meta_input = "\u{2022}".repeat(s.meta_token_len);
            s.meta_rename_input.clear();
            s.meta_substep = 0;
            s.error = None;
        }
        _ => {}
    }
}

pub(crate) fn insert_setup(s: &mut SetupState, c: char) {
    match s.step {
        0 => {
            s.workspace_path.push(c);
        }
        1 => {
            meta_insert(s, c);
        }
        2 => {
            s.product_input.push(c);
        }
        3 => {
            s.goals_input.push(c);
        }
        4 => {
            // Step 4 substeps that accept free-text: paste-key (1) and
            // custom URL/model (3). Picker substeps (0, 2) ignore chars.
            match s.llm_substep {
                1 => s.llm_input.push(c),
                // Custom: the URL until enter, then the model.
                3 if s.llm_url_entered => s.llm_model.push(c),
                3 => s.llm_url.push(c),
                _ => {}
            }
        }
        _ => {}
    }
}

fn backspace_setup(s: &mut SetupState) {
    match s.step {
        0 => {
            s.workspace_path.pop();
        }
        1 => {
            meta_backspace(s);
        }
        2 => {
            s.product_input.pop();
        }
        3 => {
            s.goals_input.pop();
        }
        4 => match s.llm_substep {
            1 => {
                s.llm_input.pop();
            }
            3 if s.llm_url_entered => {
                s.llm_model.pop();
            }
            3 => {
                s.llm_url.pop();
            }
            _ => {}
        },
        _ => {}
    }
}
