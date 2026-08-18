// SPDX-License-Identifier: Apache-2.0
//! The `salman` command line interface.
//!
//! Every salman capability is reachable headless. That is not a convenience:
//! it is what makes `salman test` runnable in a container on every commit,
//! which is the point of the project.
//!
//! # Posture
//!
//! salman is read-only by default. At this version the question does not
//! arise — there is no code path in the whole program that opens a socket or
//! writes to a device — and `salman run` says so rather than leaving the reader
//! to wonder. See `docs/adr/ADR-0002-read-only-by-default.md`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use salman_core::capability;
use salman_core::posture::Posture;
use salman_core::time::Duration;
use salman_lang::dialect::{Dialect, DialectId};
use salman_vm::clock::Clock;
use salman_vm::compile::Compiled;
use salman_vm::memory::SlotId;
use salman_vm::task::Runtime;
use salman_vm::trace::Signal;

/// Exit code when something the user asked about is wrong: a program with
/// errors, or a test that failed.
const EXIT_PROBLEM: u8 = 1;
/// Exit code when salman itself could not do the job: a missing file, a
/// directory it cannot read.
const EXIT_CANNOT_RUN: u8 = 2;

/// Vendor-neutral, text-first IEC 61131-3 engineering workbench.
///
/// Not a safety tool and not certified. See LEGAL.md.
#[derive(Debug, Parser)]
#[command(name = "salman", version = salman_core::VERSION, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the version.
    Version,
    /// Print what salman can do, and how far each capability is tested.
    Status {
        /// Emit the generated Markdown that `docs/STATUS.md` holds.
        #[arg(long)]
        markdown: bool,
    },
    /// Parse and type-check a source file.
    Check {
        /// The Structured Text file.
        path: PathBuf,
        /// Which dialect to apply.
        #[arg(long, default_value = "generic")]
        dialect: String,
    },
    /// Compile and run a source file on the simulation runtime.
    Run {
        /// The Structured Text file.
        path: PathBuf,
        /// How many scans to run.
        #[arg(long)]
        scans: Option<u64>,
        /// How far to advance the simulation clock, as a duration literal.
        #[arg(long)]
        until: Option<String>,
        /// Variables to record, comma separated.
        #[arg(long, value_delimiter = ',')]
        record: Vec<String>,
        /// Where to write the recorded trace.
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Which dialect to apply.
        #[arg(long, default_value = "generic")]
        dialect: String,
    },
    /// Run declarative tests against a source file.
    Test {
        /// The Structured Text file.
        path: PathBuf,
        /// The test file, or a directory of `.salman-test.yaml` files.
        tests: PathBuf,
        /// Where to write a JUnit XML report.
        #[arg(long)]
        junit: Option<PathBuf>,
        /// Rewrite golden trace files from this run.
        #[arg(long)]
        update_golden: bool,
        /// Which dialect to apply.
        #[arg(long, default_value = "generic")]
        dialect: String,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("salman: {message}");
            ExitCode::from(EXIT_CANNOT_RUN)
        }
    }
}

fn run(cli: Cli) -> Result<u8, String> {
    match cli.command {
        Command::Version => {
            println!("salman {}", salman_core::VERSION);
            println!("posture: {} (read-only)", Posture::default());
            Ok(0)
        }
        Command::Status { markdown } => {
            if markdown {
                print!("{}", capability::render_markdown());
            } else {
                print!("{}", status_table());
            }
            Ok(0)
        }
        Command::Check { path, dialect } => check(&path, &dialect),
        Command::Run {
            path,
            scans,
            until,
            record,
            trace,
            dialect,
        } => run_program(
            &path,
            scans,
            until.as_deref(),
            &record,
            trace.as_deref(),
            &dialect,
        ),
        Command::Test {
            path,
            tests,
            junit,
            update_golden,
            dialect,
        } => run_tests(&path, &tests, junit.as_deref(), update_golden, &dialect),
    }
}

fn dialect_for(name: &str) -> Result<Dialect, String> {
    DialectId::from_name(name)
        .map(Dialect::for_id)
        .ok_or_else(|| {
            let known: Vec<&str> = DialectId::all().iter().map(|d| d.name()).collect();
            format!(
                "unknown dialect {name:?}. salman implements: {}",
                known.join(", ")
            )
        })
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn build(path: &Path, dialect: &str) -> Result<salman_vm::project::Build, String> {
    let dialect = dialect_for(dialect)?;
    let text = read(path)?;
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    salman_vm::project::build(&name, &text, &dialect).map_err(|e| e.to_string())
}

fn check(path: &Path, dialect: &str) -> Result<u8, String> {
    let built = build(path, dialect)?;
    let rendered = built.render_diagnostics();
    if !rendered.is_empty() {
        print!("{rendered}");
    }
    if built.diagnostics.has_errors() {
        println!("{} errors", built.diagnostics.error_count());
        return Ok(EXIT_PROBLEM);
    }
    println!("{}: no errors", path.display());
    Ok(0)
}

fn run_program(
    path: &Path,
    scans: Option<u64>,
    until: Option<&str>,
    record: &[String],
    trace_path: Option<&Path>,
    dialect: &str,
) -> Result<u8, String> {
    let built = build(path, dialect)?;
    let rendered = built.render_diagnostics();
    if !rendered.is_empty() {
        print!("{rendered}");
    }
    let Some(compiled) = built.compiled else {
        return Ok(EXIT_PROBLEM);
    };

    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );

    if !record.is_empty() {
        let mut signals = Vec::new();
        for name in record {
            let slot = find_slot(&compiled, name)?;
            signals.push(Signal {
                slot,
                name: name.clone(),
            });
        }
        runtime.record(signals);
    }

    println!(
        "posture: {} — salman 0.0.1 has no code path that writes to a device",
        Posture::default()
    );
    for task in runtime.tasks() {
        println!(
            "task {} priority {} ({:?})",
            task.name, task.priority, task.trigger
        );
    }

    if let Some(text) = until {
        let duration = parse_duration(text)?;
        runtime.run_until(duration);
    } else {
        runtime.run_scans(scans.unwrap_or(1));
    }

    println!(
        "{} scans, simulation time {}",
        runtime.scan_count(),
        runtime.clock().elapsed().to_iec_literal()
    );
    for (index, stats) in runtime.stats().iter().enumerate() {
        let name = runtime.tasks().get(index).map_or("?", |t| t.name.as_str());
        println!(
            "  {name}: {} scans, {} overruns, {}..{} instructions",
            stats.scans, stats.overruns, stats.min_instructions, stats.max_instructions
        );
    }
    for fault in runtime.faults() {
        println!("fault: {fault}");
    }

    if let Some(trace) = runtime.trace() {
        let text = trace.render();
        match trace_path {
            Some(path) => {
                std::fs::write(path, &text)
                    .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                println!(
                    "trace written to {} ({})",
                    path.display(),
                    trace.fingerprint_hex()
                );
            }
            None => print!("{text}"),
        }
    }

    Ok(if runtime.has_faulted() {
        EXIT_PROBLEM
    } else {
        0
    })
}

fn run_tests(
    path: &Path,
    tests: &Path,
    junit: Option<&Path>,
    update_golden: bool,
    dialect: &str,
) -> Result<u8, String> {
    let built = build(path, dialect)?;
    let rendered = built.render_diagnostics();
    if !rendered.is_empty() {
        print!("{rendered}");
    }
    let Some(compiled) = built.compiled else {
        return Ok(EXIT_PROBLEM);
    };

    let files = test_files(tests)?;
    if files.is_empty() {
        return Err(format!("no test files found at {}", tests.display()));
    }

    let mut outcomes = Vec::new();
    for file in &files {
        let text = read(file)?;
        let cases =
            salman_test::spec::parse(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut file_outcomes = salman_test::run_all(&compiled, &cases);
        let directory = file.parent().unwrap_or(Path::new("."));
        for outcome in &mut file_outcomes {
            compare_golden(outcome, directory, update_golden)?;
        }
        outcomes.extend(file_outcomes);
    }

    print!("{}", salman_test::render_text(&outcomes));

    if let Some(path) = junit {
        let name = tests.file_stem().map_or_else(
            || "salman".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let xml = salman_test::render_junit(&name, &outcomes);
        std::fs::write(path, xml).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }

    Ok(salman_test::Summary::of(&outcomes).exit_code())
}

/// Compares a recorded trace against its golden file, or rewrites it.
fn compare_golden(
    outcome: &mut salman_test::Outcome,
    directory: &Path,
    update: bool,
) -> Result<(), String> {
    let (Some(golden), Some(trace)) = (outcome.golden.clone(), outcome.trace.clone()) else {
        return Ok(());
    };
    let path = directory.join(&golden);
    let rendered = trace.render();

    if update {
        std::fs::write(&path, &rendered)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        return Ok(());
    }

    let Ok(expected) = std::fs::read_to_string(&path) else {
        outcome.status = salman_test::Status::Failed;
        outcome.problems.push(salman_test::runner::Problem {
            step: None,
            message: format!(
                "{} does not exist. Run with --update-golden to create it, then read it \
                 before committing",
                path.display()
            ),
        });
        return Ok(());
    };

    if expected != rendered {
        outcome.status = salman_test::Status::Failed;
        outcome.problems.push(salman_test::runner::Problem {
            step: None,
            message: format!(
                "the trace does not match {}:\n{}",
                path.display(),
                first_difference(&expected, &rendered)
            ),
        });
    }
    Ok(())
}

/// The first line where two traces differ, with a little context.
fn first_difference(expected: &str, found: &str) -> String {
    for (number, (a, b)) in expected.lines().zip(found.lines()).enumerate() {
        if a != b {
            return format!("  line {}\n  expected: {a}\n  found:    {b}", number + 1);
        }
    }
    let (a, b) = (expected.lines().count(), found.lines().count());
    format!("  expected {a} lines, found {b}")
}

fn test_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let entries =
        std::fs::read_dir(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .extension()
                .is_some_and(|e| e == "yaml" || e == "yml")
        })
        .collect();
    // read_dir order is platform-dependent and can change between calls. Sort,
    // or the same suite reports its tests in a different order on every machine.
    files.sort();
    Ok(files)
}

fn find_slot(compiled: &Compiled, name: &str) -> Result<SlotId, String> {
    compiled
        .program
        .slot_index(name)
        .or_else(|| {
            compiled
                .program
                .slot_names
                .iter()
                .position(|candidate| {
                    candidate
                        .rsplit('.')
                        .next()
                        .is_some_and(|tail| tail.eq_ignore_ascii_case(name))
                })
                .and_then(|index| u32::try_from(index).ok())
                .map(SlotId)
        })
        .ok_or_else(|| format!("no variable called {name}"))
}

fn parse_duration(text: &str) -> Result<Duration, String> {
    salman_test::ValueSpec::Text(text.to_string())
        .to_value(salman_core::value::ElementaryType::Time)
        .map_err(|e| e.to_string())?
        .as_duration()
        .ok_or_else(|| format!("{text} is not a duration"))
}

/// The status table, as a person reads it in a terminal.
fn status_table() -> String {
    let mut out = String::new();
    out.push_str("salman ");
    out.push_str(salman_core::VERSION);
    out.push_str("\n\n");
    for area in capability::areas() {
        out.push_str(area);
        out.push('\n');
        for entry in capability::in_area(area) {
            let _ = writeln!(
                out,
                "  {} {:<58} {}",
                entry.status.marker(),
                entry.title,
                entry.status
            );
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn an_unknown_dialect_lists_the_ones_that_exist() {
        let error = dialect_for("codesys").unwrap_err();
        assert!(error.contains("generic"), "{error}");
        assert!(error.contains("codesys"), "{error}");
    }

    #[test]
    fn every_implemented_dialect_can_be_selected_by_name() {
        for id in DialectId::all() {
            assert!(dialect_for(id.name()).is_ok());
        }
    }

    #[test]
    fn the_status_table_lists_every_registered_capability() {
        let table = status_table();
        for entry in capability::REGISTRY {
            assert!(
                table.contains(entry.title),
                "{} is missing from the table",
                entry.id
            );
        }
    }

    #[test]
    fn a_duration_argument_is_an_iec_literal() {
        assert_eq!(
            parse_duration("T#5s").map(salman_core::Duration::nanos),
            Ok(5_000_000_000)
        );
        assert!(parse_duration("five seconds").is_err());
    }
}
