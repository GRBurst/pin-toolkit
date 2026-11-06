use clap::Parser;
use std::path::PathBuf;

use pin_ci::{PinOptions, client_from_env, exit_code, gitlab_from_env, pin_files};

/// Pin container image references in GitLab CI files to immutable digests.
#[derive(Parser)]
#[command(name = "pin-ci", version)]
struct Cli {
    /// CI files to pin
    #[arg(default_value = "ci/.gitlab-ci.yml")]
    files: Vec<String>,

    /// Bump unpinned, versioned Docker Hub tags to the latest compatible
    /// stable tag and pin the bumped tag's digest
    #[arg(long)]
    update: bool,

    /// With --update, also allow bumping to a higher major (meaningful only
    /// with --update)
    #[arg(long)]
    update_major: bool,

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

fn build_opts(cli: &Cli) -> PinOptions {
    PinOptions {
        dry_run: cli.dry_run,
        json: cli.json,
        concurrency: cli.concurrency,
        update: cli.update,
        update_major: cli.update_major,
        // The --gitlab-host flag is wired in M3 Task 5; day one defaults
        // to gitlab.com.
        gitlab_host: "gitlab.com".into(),
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let opts = build_opts(&cli);
    let client = client_from_env();
    let gitlab = gitlab_from_env(&opts.gitlab_host);
    let paths: Vec<PathBuf> = cli.files.iter().map(PathBuf::from).collect();
    match pin_files(&paths, &opts, &client, &gitlab).await {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_update_flags_parsed() {
        let cli =
            Cli::try_parse_from(["pin-ci", "ci/.gitlab-ci.yml", "--update", "--update-major"])
                .unwrap();
        let opts = build_opts(&cli);
        assert!(opts.update);
        assert!(opts.update_major);
        assert!(!opts.dry_run);
        assert!(!opts.json);
        assert_eq!(opts.concurrency, 8);

        let cli = Cli::try_parse_from(["pin-ci"]).unwrap();
        let opts = build_opts(&cli);
        assert!(!opts.update);
        assert!(!opts.update_major);
        assert_eq!(opts.concurrency, 8);
    }
}
