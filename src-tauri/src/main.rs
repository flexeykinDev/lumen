// Release builds are a tray app: no console window should ever flash.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    lumen_lib::run();
}
