// Prevents an extra console window from opening alongside the app on Windows
// in release builds. Do not remove.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    surd_desktop_lib::run()
}
