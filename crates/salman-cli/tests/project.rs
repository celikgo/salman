// SPDX-License-Identifier: Apache-2.0
//! A project spanning several source files.
//!
//! For the whole of v0.1 `salman` compiled exactly one file and said so, rather
//! than silently compiling the first of several. The blocker was node identity:
//! every side table downstream — resolved types, resolutions, folded constants
//! — is a `Vec` indexed by `NodeId`, and two files parsed independently both
//! started at zero, so joining them would have made two different nodes read
//! and write the same entry. That is a wrong answer, not a crash, which is the
//! kind salman is built to avoid.
//!
//! The fix is to hand each file a disjoint range at parse time, so the units
//! are disjoint by construction rather than by a renumbering pass that is wrong
//! the first time a node is missed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_core::value::Value;
use salman_lang::dialect::Dialect;
use salman_vm::clock::Clock;
use salman_vm::memory::SlotId;
use salman_vm::project::build_all;
use salman_vm::task::Runtime;

fn built(files: &[(&str, &str)]) -> salman_vm::project::Build {
    build_all(files, &Dialect::generic()).expect("not too large")
}

fn compiled(files: &[(&str, &str)]) -> salman_vm::compile::Compiled {
    let built = built(files);
    let rendered = built.render_diagnostics();
    let Some(compiled) = built.compiled else {
        panic!("expected this to compile:\n{rendered}")
    };
    compiled
}

const COUNTER_FB: &str = "FUNCTION_BLOCK Counter\n\
     VAR_INPUT Amount : INT; END_VAR\n\
     VAR_OUTPUT Total : INT; END_VAR\n\
       Total := Total + Amount;\n\
     END_FUNCTION_BLOCK\n";

const MAIN: &str = "PROGRAM Main\n\
     VAR C : Counter; Result : INT; END_VAR\n\
       C(Amount := 2, Total => Result);\n\
     END_PROGRAM\n";

#[test]
fn a_program_can_call_a_function_block_declared_in_another_file() {
    let compiled = compiled(&[("fb.st", COUNTER_FB), ("main.st", MAIN)]);
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    runtime.run_scans(3);
    let slot = compiled
        .program
        .slot_index("Main.Result")
        .expect("the program has a Result");
    assert_eq!(runtime.memory().read_slot(slot), Some(&Value::Int(6)));
}

/// Runs a project and returns the fingerprint of `Main.Result` over three scans.
fn trace_of(files: &[(&str, &str)]) -> String {
    let compiled = compiled(files);
    let slot = compiled
        .program
        .slot_index("Main.Result")
        .expect("the program has a Result");
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    runtime.record(vec![salman_vm::trace::Signal::slot(slot, "Result")]);
    runtime.run_scans(3);
    runtime.trace().expect("recording").fingerprint_hex()
}

#[test]
fn declaration_order_across_files_does_not_matter() {
    // Names resolve across the whole project, not from the top of the first
    // file downwards. Slot layout follows the order the files were given, so
    // what has to match is the behaviour rather than the addresses.
    assert_eq!(
        trace_of(&[("fb.st", COUNTER_FB), ("main.st", MAIN)]),
        trace_of(&[("main.st", MAIN), ("fb.st", COUNTER_FB)]),
    );
}

#[test]
fn a_name_declared_in_two_files_is_reported_as_a_duplicate() {
    // The reason to join before checking rather than check each file alone:
    // neither file is wrong by itself.
    let built = built(&[("a.st", COUNTER_FB), ("b.st", COUNTER_FB)]);
    assert!(built.diagnostics.has_errors());
    let rendered = built.render_diagnostics();
    assert!(rendered.contains("Counter"), "{rendered}");
}

#[test]
fn a_diagnostic_points_at_the_file_it_came_from() {
    // Every span carries its own file, so joining must not make the second
    // file's errors read as if they were in the first.
    let built = built(&[
        ("good.st", COUNTER_FB),
        (
            "bad.st",
            "PROGRAM P\nVAR X : INT; END_VAR\n  X := TRUE;\nEND_PROGRAM\n",
        ),
    ]);
    let rendered = built.render_diagnostics();
    assert!(rendered.contains("bad.st"), "{rendered}");
    assert!(!rendered.contains("good.st"), "{rendered}");
}

#[test]
fn three_files_build_as_one_program() {
    // That the ids stay distinct is checked directly in salman-lang, where the
    // node walker lives; this is the end-to-end shape.
    let built = built(&[
        ("fb.st", COUNTER_FB),
        ("main.st", MAIN),
        (
            "more.st",
            "PROGRAM Q\nVAR Y : INT; END_VAR\n  Y := 1 + 2;\nEND_PROGRAM\n",
        ),
    ]);
    assert!(
        !built.diagnostics.has_errors(),
        "{}",
        built.render_diagnostics()
    );
    assert_eq!(built.files.len(), 3);
}

#[test]
fn a_project_with_no_files_says_there_is_nothing_to_run() {
    let built = built(&[]);
    assert!(built.compiled.is_none());
    let rendered = built.render_diagnostics();
    assert!(rendered.contains("nothing to run"), "{rendered}");
}

#[test]
fn one_file_through_build_all_is_the_same_program_as_through_build() {
    let single =
        salman_vm::project::build("t.st", MAIN, &Dialect::generic()).expect("not too large");
    let through_all = built(&[("t.st", MAIN)]);
    assert_eq!(
        single.render_diagnostics(),
        through_all.render_diagnostics()
    );
}

#[test]
fn a_function_in_one_file_is_callable_from_another() {
    let compiled = compiled(&[
        (
            "lib.st",
            "FUNCTION Double : INT\nVAR_INPUT N : INT; END_VAR\n  Double := N * 2;\nEND_FUNCTION\n",
        ),
        (
            "main.st",
            "PROGRAM Main\nVAR R : INT; END_VAR\n  R := Double(21);\nEND_PROGRAM\n",
        ),
    ]);
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    runtime.run_scans(1);
    let slot: SlotId = compiled.program.slot_index("Main.R").expect("R exists");
    assert_eq!(runtime.memory().read_slot(slot), Some(&Value::Int(42)));
}
