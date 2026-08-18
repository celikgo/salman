// SPDX-License-Identifier: Apache-2.0
//! The worked example, end to end.
//!
//! This is the test that says salman is a tool rather than a collection of
//! crates: it takes the Structured Text in `examples/conveyor/`, lexes, parses,
//! checks and compiles it, runs the declarative tests against the simulation
//! runtime, and compares a recorded trace with a committed golden file.
//!
//! It is also the determinism gate's smallest honest form: the same project run
//! twice produces the same trace fingerprint. The cross-platform half of that
//! promise is what `.github/workflows/determinism.yml` is for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use salman_lang::dialect::Dialect;
use salman_vm::project::{Build, build};

fn repository_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <root>/crates/salman-cli.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate is two levels below the repository root")
        .to_path_buf()
}

fn example_dir() -> PathBuf {
    repository_root().join("examples/conveyor")
}

fn built() -> Build {
    let path = example_dir().join("conveyor.st");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    build("conveyor.st", &text, &Dialect::generic()).expect("the example is not too large")
}

#[test]
fn the_example_compiles_with_no_diagnostics_at_all() {
    let build = built();
    assert!(
        build.diagnostics.is_empty(),
        "the worked example must be clean, warnings included:\n{}",
        build.render_diagnostics()
    );
    assert!(build.is_ok(), "the example did not compile");
}

#[test]
fn the_example_declares_the_variables_the_tests_name() {
    let build = built();
    let compiled = build.compiled.expect("compiled");
    for name in [
        "Conveyor.Motor",
        "Conveyor.Jam_Lamp",
        "Conveyor.Parts.CV",
        "Conveyor.Starter.Run_Off.ET",
        "Conveyor.State",
    ] {
        assert!(
            compiled.program.slot_index(name).is_some(),
            "no slot called {name}; the program has {} slots",
            compiled.program.slot_names.len()
        );
    }
}

#[test]
fn every_test_in_the_example_passes() {
    let build = built();
    let compiled = build.compiled.expect("compiled");
    let path = example_dir().join("conveyor.salman-test.yaml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let cases = salman_test::spec::parse(&text).expect("the test file parses");
    assert!(
        cases.len() >= 8,
        "the example should carry a real suite, found {}",
        cases.len()
    );

    let outcomes = salman_test::run_all(&compiled, &cases);
    let report = salman_test::render_text(&outcomes);
    let summary = salman_test::Summary::of(&outcomes);
    assert_eq!(
        summary.failed + summary.errored,
        0,
        "the worked example must be green:\n{report}"
    );
    assert_eq!(
        summary.skipped, 0,
        "the example should skip nothing:\n{report}"
    );
}

#[test]
fn the_recorded_trace_matches_the_committed_golden_file() {
    let build = built();
    let compiled = build.compiled.expect("compiled");
    let path = example_dir().join("conveyor.salman-test.yaml");
    let text = std::fs::read_to_string(&path).expect("the test file exists");
    let cases = salman_test::spec::parse(&text).expect("the test file parses");

    let golden_case = cases
        .iter()
        .find(|case| case.golden.is_some())
        .expect("the example carries a golden-trace test");
    let outcome = salman_test::run(&compiled, golden_case);
    let trace = outcome.trace.expect("the golden test records a trace");

    let golden_path = example_dir().join(golden_case.golden.clone().expect("a golden file"));
    let expected = std::fs::read_to_string(&golden_path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}. Create it with `salman test ... --update-golden`, then read \
             it before committing",
            golden_path.display()
        )
    });
    assert_eq!(
        trace.render(),
        expected,
        "the recorded trace does not match {}",
        golden_path.display()
    );
}

#[test]
fn a_golden_trace_file_contains_no_carriage_returns() {
    // Rust never translates line endings, so the only way a CR reaches a golden
    // file is git rewriting it. .gitattributes prevents that; this test makes
    // the guarantee live in the suite rather than in YAML a contributor can
    // bypass locally.
    let path = example_dir().join("conveyor.trace");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert!(
        !bytes.contains(&b'\r'),
        "{} contains a carriage return; git has rewritten it",
        path.display()
    );
}

#[test]
fn the_same_project_run_twice_produces_the_same_fingerprint() {
    let fingerprint = || {
        let build = built();
        let compiled = build.compiled.expect("compiled");
        let path = example_dir().join("conveyor.salman-test.yaml");
        let text = std::fs::read_to_string(&path).expect("the test file exists");
        let cases = salman_test::spec::parse(&text).expect("the test file parses");
        let case = cases
            .iter()
            .find(|c| c.golden.is_some())
            .expect("a golden test");
        salman_test::run(&compiled, case)
            .trace
            .expect("a trace")
            .fingerprint_hex()
    };
    assert_eq!(fingerprint(), fingerprint());
}

#[test]
fn the_compiled_program_is_byte_identical_across_two_compilations() {
    // Determinism starts before the runtime: two compilations of the same
    // source must produce the same bytecode, or the trace comparison is
    // comparing two different programs.
    let a = built().compiled.expect("compiled");
    let b = built().compiled.expect("compiled");
    assert_eq!(a.program, b.program);
    assert_eq!(a.tasks, b.tasks);
}
