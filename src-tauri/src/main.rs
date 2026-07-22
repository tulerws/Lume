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
    // O renderizador DMABUF do WebKitGTK quebra em alguns compositores Wayland
    // com sincronização explícita (ex.: KWin), derrubando a conexão com
    // "Error 71 (Protocol error)" antes de a cápsula aparecer. Desligá-lo usa um
    // caminho de composição compatível; num overlay pequeno o custo é
    // irrelevante. Só definimos quando o usuário ainda não escolheu um valor.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    lume_lib::run()
}
