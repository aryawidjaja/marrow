//! The `marrow-server` binary: run the served backbone.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut port, mut data_dir, mut token) = (None, None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().and_then(|s| s.parse().ok()),
            "--data-dir" => data_dir = args.next().map(PathBuf::from),
            "--token" => token = args.next(),
            "-h" | "--help" => return help(),
            _ => {}
        }
    }
    let port = port
        .or_else(|| std::env::var("PORT").ok().and_then(|s| s.parse().ok()))
        .unwrap_or(8787);
    let cfg = marrow_server::config_from(data_dir, token);
    if let Err(e) = marrow_server::serve(cfg, &format!("0.0.0.0:{port}")) {
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
