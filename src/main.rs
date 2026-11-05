use clap::Parser;
use std::path::PathBuf;

use pin_ci::{PinOptions, client_from_env, exit_code, pin_files};

/// Pin container image references in GitLab CI files to immutable digests.
#[derive(Parser)]
#[command(name = "pin-ci", version)]
struct Cli {
    /// CI files to pin
    #[arg(default_value = "ci/.gitlab-ci.yml")]
    files: Vec<String>,

    /// Resolve and report without writing files
    #[arg(long)]
    dry_run: bool,

    /// Print the report as JSON instead of human-readable text
    #[arg(long)]
    json: bool,

    /// Max concurrent digest lookups per file
    #[arg(long, default_value_t = 8)]
    concurrency: usize,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let opts = PinOptions {
        dry_run: cli.dry_run,
        json: cli.json,
        concurrency: cli.concurrency,
        // The `--update`/`--update-major` CLI flags land in M2 Task 4; the
        // library default is off.
        update: false,
        update_major: false,
    };
    let client = client_from_env();
    let paths: Vec<PathBuf> = cli.files.iter().map(PathBuf::from).collect();
    match pin_files(&paths, &opts, &client).await {
        Ok(report) => {
            let out = if cli.json {
                report.to_json()
            } else {
                report.render_human()
            };
            println!("{out}");
            std::process::exit(exit_code(&report));
        }
        Err(e) => {
            eprintln!("pin-ci: {e}");
            std::process::exit(1);
        }
    }
}
