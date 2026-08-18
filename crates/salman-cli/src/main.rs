//! The `salman` command line interface.
//!
//! Every salman capability is reachable headless. Nothing in this binary
//! writes to a physical device: at this version no such code path exists at
//! all. See `docs/adr/ADR-0002-read-only-by-default.md`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use clap::{Parser, Subcommand};

/// Vendor-neutral, text-first IEC 61131-3 engineering workbench.
///
/// Not a safety tool. Not certified. See the README safety boundary.
#[derive(Debug, Parser)]
#[command(name = "salman", version = salman_core::VERSION, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print version and build information.
    Version,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Version => {
            println!("salman {}", salman_core::VERSION);
            std::process::ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn cli_version_string_is_the_project_version() {
        let rendered = Cli::command().render_version();
        assert!(
            rendered.contains(salman_core::VERSION),
            "clap --version output {rendered:?} does not carry the VERSION file value"
        );
    }
}
