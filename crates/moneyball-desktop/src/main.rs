//! Moneyball for Mac. Everything lives in the library; this is the door.

// No console window on Windows release builds (harmless on macOS).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    moneyball_desktop::run();
}
