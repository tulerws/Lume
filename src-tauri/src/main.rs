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
    // No Wayland, o renderizador DMABUF do WebKitGTK faz commit de um buffer com
    // sincronização explícita sem acquire point em algumas combinações de
    // compositor/GPU (ex.: KWin com NVIDIA), e o servidor derruba a conexão com
    // "Error 71 (Protocol error)" antes de a cápsula abrir. Desligá-lo apenas no
    // Wayland evita o crash sem tocar no desempenho do X11, e respeita um valor
    // que o usuário já tenha escolhido.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
        && std::env::var("XDG_SESSION_TYPE").ok().as_deref() == Some("wayland")
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    lume_lib::run()
}
