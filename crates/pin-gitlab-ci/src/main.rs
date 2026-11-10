use clap::Parser;
use std::path::PathBuf;

use pin_gitlab_ci::{PinOptions, client_from_env, gitlab_from_env, outcome, pin_files};

/// Pin container image references in GitLab CI files to immutable digests.
#[derive(Parser)]
#[command(name = "pin-ci", version)]
struct Cli {
    /// CI files to pin
    #[arg(default_value = "ci/.gitlab-ci.yml")]
    files: Vec<String>,

    /// Bump unpinned, versioned image refs (any registry host) to the
    /// latest compatible stable tag and pin the bumped tag's digest
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

    /// GitLab instance host for component/include resolution (default gitlab.com)
    #[arg(long, default_value = "gitlab.com")]
    gitlab_host: String,

    /// with --update: skip bump candidates younger than N days (Docker Hub
    /// refs only; 0 = off; recommended 14)
    #[arg(long, default_value_t = 0)]
    min_age_days: u64,
}

fn build_opts(cli: &Cli) -> PinOptions {
    PinOptions {
        dry_run: cli.dry_run,
        json: cli.json,
        concurrency: cli.concurrency,
        update: cli.update,
        update_major: cli.update_major,
        gitlab_host: cli.gitlab_host.clone(),
        min_age_days: cli.min_age_days,
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let opts = build_opts(&cli);
    let client = client_from_env();
    let gitlab = gitlab_from_env(&opts.gitlab_host);
    let paths: Vec<PathBuf> = cli.files.iter().map(PathBuf::from).collect();
    let out = outcome(pin_files(&paths, &opts, &client, &gitlab).await, cli.json);
    if let Some(stdout) = &out.stdout {
        println!("{stdout}");
    }
    if let Some(stderr) = &out.stderr {
        eprintln!("{stderr}");
    }
    std::process::exit(out.code);
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

    #[test]
    fn cli_gitlab_host_parsed() {
        let cli = Cli::try_parse_from(["pin-ci"]).unwrap();
        let opts = build_opts(&cli);
        assert_eq!(opts.gitlab_host, "gitlab.com");

        let cli = Cli::try_parse_from(["pin-ci", "--gitlab-host", "gitlab.example.com"]).unwrap();
        let opts = build_opts(&cli);
        assert_eq!(opts.gitlab_host, "gitlab.example.com");
    }

    #[test]
    fn flag_default_zero() {
        let cli = Cli::try_parse_from(["pin-ci"]).unwrap();
        let opts = build_opts(&cli);
        assert_eq!(opts.min_age_days, 0);

        let cli = Cli::try_parse_from(["pin-ci", "--min-age-days", "7"]).unwrap();
        let opts = build_opts(&cli);
        assert_eq!(opts.min_age_days, 7);
    }
}
