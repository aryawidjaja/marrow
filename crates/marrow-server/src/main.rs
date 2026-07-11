//! The `marrow-server` binary: run the served backbone.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut port, mut data_dir, mut token, mut insecure) = (None, None, None, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().and_then(|s| s.parse().ok()),
            "--data-dir" => data_dir = args.next().map(PathBuf::from),
            "--token" => token = args.next(),
            "--insecure" => insecure = true,
            "-h" | "--help" => return help(),
            _ => {}
        }
    }
    let port = port
        .or_else(|| std::env::var("PORT").ok().and_then(|s| s.parse().ok()))
        .unwrap_or(8787);
    let cfg = marrow_server::config_from(data_dir, token);
    // Without a token every request is authorized, so never expose that to the network by default:
    // bind loopback unless a token is set (or --insecure is passed for a trusted network).
    let host = if cfg.token.is_some() || insecure {
        "0.0.0.0"
    } else {
        eprintln!(
            "marrow-server: no MARROW_TOKEN set — binding 127.0.0.1 only. Set a token to serve other devices, or pass --insecure to force 0.0.0.0."
        );
        "127.0.0.1"
    };
    if let Err(e) = marrow_server::serve(cfg, &format!("{host}:{port}")) {
        eprintln!("marrow-server: {e}");
        std::process::exit(1);
    }
}

fn help() {
    println!(
        "marrow-server — the shared Marrow backbone across devices.\n\n\
         Usage: marrow-server [--port N] [--data-dir DIR] [--token SECRET]\n\n\
         Env: PORT, MARROW_DATA, MARROW_TOKEN (a set token is required on every request but /health).\n\
         Devices connect by pointing their agent's Marrow at this URL (MARROW_REMOTE)."
    );
}
