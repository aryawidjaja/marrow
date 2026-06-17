use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;
use marrow_cli::{run, Cli};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut stdout = io::stdout().lock();
    match run(cli, &mut stdout) {
        Ok(()) => {
            stdout.flush().ok();
            ExitCode::SUCCESS
        }
        Err(msg) => {
            writeln!(io::stderr(), "error: {msg}").ok();
            ExitCode::FAILURE
        }
    }
}
