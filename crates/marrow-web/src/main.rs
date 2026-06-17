use std::path::PathBuf;

fn main() {
    let (root, port) = parse_args();
    let addr = format!("127.0.0.1:{port}");
    if let Err(e) = marrow_web::serve(&root, &addr) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// Parse `--root <path>` (default `.`) and `--port <n>` (default 8088).
fn parse_args() -> (PathBuf, u16) {
    let mut root = PathBuf::from(".");
    let mut port = 8088u16;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                if let Some(p) = args.next() {
                    root = PathBuf::from(p);
                }
            }
            "--port" => {
                if let Some(p) = args.next().and_then(|s| s.parse().ok()) {
                    port = p;
                }
            }
            _ => {}
        }
    }
    (root, port)
}
