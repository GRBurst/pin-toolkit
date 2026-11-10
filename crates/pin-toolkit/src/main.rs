//! The `pin-toolkit` binary (and every argv[0] alias): a thin shell over
//! [`pin_toolkit::main_with`].

use std::io::{self, Write};
use std::process::ExitCode;

use pin_toolkit::{Output, Process, main_with};

fn main() -> ExitCode {
    let out = match Process::from_os() {
        Ok(p) => main_with(p),
        Err(e) => Output {
            stderr: format!("pin-toolkit: cannot read the current directory: {e}\n"),
            code: 2,
            ..Output::default()
        },
    };
    // No print!: a closed pipe (`| head`) must not panic.
    let _ = io::stdout().write_all(out.stdout.as_bytes());
    let _ = io::stderr().write_all(out.stderr.as_bytes());
    ExitCode::from(out.code)
}
