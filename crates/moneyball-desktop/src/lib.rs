//! Moneyball for Mac: the Tauri 2 shell over moneyball-core. Commands in,
//! channel events out (plans/desktop/IPC.md is the contract). This file
//! is the table of contents and the app's wiring; every command lives in
//! the module of its concern.

pub mod action;
pub mod brief;
pub mod clipboard;
pub mod error;
pub mod front_end;
pub mod funnel;
pub mod refresh;
pub mod refresh_end;
pub mod report;
pub mod run_view;
pub mod session;
pub mod state;
pub mod today;
pub mod turn;
pub mod workspace;

#[cfg(test)]
mod fixtures;

use tauri::Manager;

/// Build and run the app; returns when the last window closes.
pub fn run() {
    let app = tauri::Builder::default()
        // First, as the plugin requires: a second launch focuses this
        // window instead of opening another writer of the same sessions.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            workspace::workspace_current,
            workspace::workspace_pick,
            today::today,
            brief::brief,
            funnel::funnel,
            refresh::refresh_start,
            refresh::refresh_cancel,
            refresh::refresh_state,
            report::report_days,
            report::report_for,
            report::report_generate,
            report::report_open_in_browser,
            session::sessions_list,
            session::session_current,
            session::session_open,
            session::session_new,
            turn::turn_start,
            turn::turn_cancel,
            clipboard::copy_text,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = app {
        eprintln!("Moneyball could not start: {}", e);
        std::process::exit(1);
    }
}
