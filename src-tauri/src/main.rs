// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("hook") {
        let provider = args.get(2).map(String::as_str).unwrap_or("");
        std::process::exit(lume_lib::run_hook_client(provider));
    }
    if args.get(1).map(String::as_str) == Some("terminal-run") {
        let payload = args.get(2).map(String::as_str).unwrap_or("");
        std::process::exit(lume_lib::run_terminal_payload(payload));
    }
    if args.get(1).map(String::as_str) == Some("ingest") {
        std::process::exit(lume_lib::run_ingest_client());
    }
    lume_lib::run()
}
