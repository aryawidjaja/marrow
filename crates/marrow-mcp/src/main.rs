use std::io::{self, BufReader};
use std::path::PathBuf;

fn main() -> io::Result<()> {
    let root = root_from_args();
    let stdin = io::stdin();
    let stdout = io::stdout();
    marrow_mcp::serve(&root, BufReader::new(stdin.lock()), stdout.lock())
}

/// Resolve the store root from `--root <path>`, defaulting to the current directory.
fn root_from_args() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--root" {
            if let Some(path) = args.next() {
                return PathBuf::from(path);
            }
        }
    }
    PathBuf::from(".")
}
