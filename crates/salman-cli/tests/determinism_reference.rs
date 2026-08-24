// SPDX-License-Identifier: Apache-2.0
//! The determinism reference, and the things about it that must not drift.
//!
//! `examples/determinism/hazards.st` exists to be recorded on three platforms
//! and compared — see `.github/workflows/determinism.yml`. That gate catches the
//! platforms disagreeing with *each other*. It cannot catch a change that alters
//! the trace on all three equally, because all three would still agree.
//!
//! The committed golden closes that. It also gives the fixture the property the
//! conveyor already has: a reviewable text file whose diff in a pull request
//! says what changed about the machine.
//!
//! The tests below check three separate things, and the second and third are the
//! ones that stop this from becoming decoration:
//!
//! 1. the recorded trace still equals the committed golden;
//! 2. the golden still describes the run the workflow actually performs — same
//!    scan count, same columns — because a golden that pins a different run than
//!    the gate compares is worse than none, and nothing else would notice;
//! 3. the trace still contains every hazard the fixture exists for, so that a
//!    column cannot be quietly deleted and leave a gate comparing less.

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
    repository_root().join("examples/determinism")
}

fn built() -> Build {
    let path = example_dir().join("hazards.st");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    build("hazards.st", &text, &Dialect::generic()).expect("the reference is not too large")
}

fn golden_case() -> salman_test::spec::TestCase {
    let path = example_dir().join("hazards.salman-test.yaml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let cases = salman_test::spec::parse(&text).expect("the test file parses");
    cases
        .iter()
        .find(|case| case.golden.is_some())
        .expect("the reference carries a golden-trace test")
        .clone()
}

#[test]
fn the_reference_trace_matches_the_committed_golden_file() {
    let build = built();
    let compiled = build.compiled.expect("the reference compiles");
    let case = golden_case();
    let outcome = salman_test::run(&compiled, &case);
    let trace = outcome.trace.expect("the golden test records a trace");

    let golden_path = example_dir().join(case.golden.clone().expect("a golden file"));
    let expected = std::fs::read_to_string(&golden_path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}. Create it with `salman test \
             examples/determinism/hazards.st examples/determinism/ --update-golden`, then read \
             it before committing",
            golden_path.display()
        )
    });
    assert_eq!(
        trace.render(),
        expected,
        "the recorded trace does not match {}. If the change is intended, regenerate with \
         `salman test examples/determinism/hazards.st examples/determinism/ --update-golden` \
         and read the diff",
        golden_path.display()
    );
}

#[test]
fn the_golden_trace_file_contains_no_carriage_returns() {
    // Rust never translates line endings, so the only way a CR reaches a golden
    // file is git rewriting it. .gitattributes prevents that; this test makes
    // the guarantee live in the suite rather than in YAML a contributor can
    // bypass locally.
    let path = example_dir().join("hazards.trace");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert!(
        !bytes.contains(&b'\r'),
        "{} contains a carriage return; git has rewritten it",
        path.display()
    );
}

#[test]
fn the_golden_pins_the_same_run_the_workflow_records() {
    // The gate uploads what `salman run` produces with the scan count and record
    // list in determinism.yml; the golden pins what the declarative harness
    // produces from hazards.salman-test.yaml. Those two agree today, byte for
    // byte, and nothing but this test would notice if one moved.
    //
    // A golden pinning a different run than the gate compares is worse than no
    // golden: it looks like coverage of the thing being gated and is coverage of
    // something else.
    let workflow_path = repository_root().join(".github/workflows/determinism.yml");
    let workflow = std::fs::read_to_string(&workflow_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", workflow_path.display()));

    let value_after = |key: &str| -> String {
        workflow
            .lines()
            .find_map(|line| line.trim().strip_prefix(key).map(str::trim))
            .unwrap_or_else(|| panic!("{} has no `{key}` line", workflow_path.display()))
            .trim_matches('"')
            .to_string()
    };

    let case = golden_case();

    let recorded = value_after("RECORD:");
    let expected: Vec<&str> = recorded.split(',').map(str::trim).collect();
    assert_eq!(
        case.record, expected,
        "hazards.salman-test.yaml records a different set of columns than \
         determinism.yml's RECORD. The golden and the gate must describe the same run"
    );

    let scans: u64 = value_after("SCANS:")
        .parse()
        .expect("determinism.yml's SCANS is a number");
    let total: u64 = case.steps.iter().filter_map(|step| step.scans).sum();
    assert_eq!(
        total, scans,
        "hazards.salman-test.yaml runs {total} scans and determinism.yml records {scans}. \
         The golden and the gate must describe the same run"
    );
}

#[test]
fn the_reference_trace_still_contains_every_hazard() {
    // The fixture's whole value is that three platforms have something to
    // disagree about. Deleting a column would leave the gate green and comparing
    // less, and no other test would fail. Each string below is a rendered value
    // that only appears if its hazard is still in the trace.
    let path = example_dir().join("hazards.trace");
    let trace = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    for (hazard, evidence) in [
        ("a canonicalised NaN", "NaN"),
        ("an infinity", "inf"),
        ("a preserved negative zero", "-0.0"),
        ("a duration rendered as an IEC literal", "T#"),
        // A REAL accumulating 0.1, which is exact in neither binary32 nor
        // decimal. If this string is gone, either the arithmetic changed or the
        // column did.
        ("a REAL carrying accumulated error", "0.10000000149011612"),
        // LREAL division. 1.0 / 3.0 rendered at full precision.
        ("an LREAL division", "0.3333333333333333"),
    ] {
        assert!(
            trace.contains(evidence),
            "{} no longer contains {hazard} (looked for `{evidence}`). If the fixture \
             deliberately lost a hazard, delete it from this list and from \
             examples/determinism/README.md in the same commit",
            path.display()
        );
    }

    // Two tasks, so that the row order at a tie is observable at all. Without a
    // second task every row reads `0` here and the ordering hazard is invisible.
    assert!(
        trace.lines().any(|line| {
            let mut fields = line.split('\t');
            fields.next();
            fields.next();
            fields.next() == Some("1")
        }),
        "{} has no rows from a second task, so the row order at a tie is not \
         being compared. examples/determinism/hazards.st should declare two PROGRAMs",
        path.display()
    );
}
