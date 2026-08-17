// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Disable DMABUF renderer in WebKitGTK — fixes black video frames on Linux.
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    kakazuma_desk_lib::run()
}
