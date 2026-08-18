// SPDX-License-Identifier: Apache-2.0
//! What a compiled program actually computes.
//!
//! These tests run Structured Text through the whole pipeline — lex, parse,
//! check, compile, scan — and assert on the values the runtime holds
//! afterwards. They are written against behaviour an engineer can see in a
//! watch window, not against any stage's internals, because the failure this
//! project exists to prevent is a wrong number on a running plant rather than
//! an untidy intermediate representation.
//!
//! Reading a slot by its dotted name is the same thing a watch list does, and
//! it is the only part of the runtime these tests know about.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_core::value::Value;
use salman_lang::dialect::Dialect;
use salman_vm::clock::Clock;
use salman_vm::memory::SlotId;
use salman_vm::project::{Build, build};
use salman_vm::task::Runtime;

/// Builds one source file, insisting that it compiled cleanly.
fn built(source: &str) -> Build {
    let build = build("t.st", source, &Dialect::generic()).expect("not too large");
    assert!(
        !build.diagnostics.has_errors(),
        "expected this to compile:\n{}",
        build.render_diagnostics()
    );
    build
}

/// A program that has been run, and can be asked what its variables hold.
struct Ran {
    runtime: Runtime,
}

impl Ran {
    /// The value of a variable, by the dotted name a watch list would use.
    fn get(&self, name: &str) -> Value {
        let slot = self
            .runtime
            .program()
            .slot_index(name)
            .unwrap_or_else(|| panic!("no slot called {name}; {}", self.slots()));
        self.runtime
            .memory()
            .read_slot(slot)
            .cloned()
            .unwrap_or_else(|| panic!("slot {name} is outside memory"))
    }

    /// Every slot name, for a failure message.
    fn slots(&self) -> String {
        self.runtime.program().slot_names.join(", ")
    }

    /// The slot a name occupies, which is what a layout test asks about.
    fn slot_of(&self, name: &str) -> SlotId {
        self.runtime
            .program()
            .slot_index(name)
            .unwrap_or_else(|| panic!("no slot called {name}; {}", self.slots()))
    }
}

/// Compiles a source file and runs it for `scans` scans.
fn run_scans(source: &str, scans: u64) -> Ran {
    let build = built(source);
    let compiled = build.compiled.expect("a compiled program");
    let mut runtime = Runtime::new(
        compiled.program,
        compiled.memory,
        Clock::virtual_default(),
        compiled.tasks,
    );
    runtime.run_scans(scans);
    assert!(
        !runtime.has_faulted(),
        "the program faulted: {:?}",
        runtime.faults()
    );
    Ran { runtime }
}

/// Compiles a source file and runs one scan.
fn run(source: &str) -> Ran {
    run_scans(source, 1)
}

/// Wraps declarations and statements in a `PROGRAM Main`.
fn program(declarations: &str, body: &str) -> String {
    format!("PROGRAM Main\nVAR\n{declarations}\nEND_VAR\n{body}\nEND_PROGRAM\n")
}

/// Runs one program body and returns what `Main.X` holds afterwards.
fn one(declarations: &str, body: &str) -> Ran {
    run(&program(declarations, body))
}

#[test]
fn probe_self_containing() {
    let source = "\
PROGRAM Main
VAR
  A1 : Looper;
  N : DINT;
END_VAR
  N := 5;
END_PROGRAM

FUNCTION_BLOCK Looper
VAR Inner : Looper; V : DINT; END_VAR
  V := 1;
END_FUNCTION_BLOCK
";
    let build = build("t.st", source, &Dialect::generic()).expect("not too large");
    println!("{}", build.render_diagnostics());
    println!("errors: {}", build.diagnostics.has_errors());
    if let Some(compiled) = &build.compiled {
        println!("slots: {}", compiled.program.slot_names.join(", "));
    }
}

#[test]
fn probe_nesting() {
    let source = "\
PROGRAM Main
VAR
  A1 : Outer;
  N : DINT;
END_VAR
  N := 5;
  A1(V := 9);
END_PROGRAM

FUNCTION_BLOCK Outer
VAR_INPUT V : DINT; END_VAR
VAR M : Middle; END_VAR
  M(V := V);
END_FUNCTION_BLOCK

FUNCTION_BLOCK Middle
VAR_INPUT V : DINT; END_VAR
VAR I : Inner; END_VAR
  I(V := V);
END_FUNCTION_BLOCK

FUNCTION_BLOCK Inner
VAR_INPUT V : DINT; END_VAR
VAR Seen : DINT; END_VAR
  Seen := V;
END_FUNCTION_BLOCK
";
    let r = run(source);
    println!("slots: {}", r.slots());
    println!("N slot {:?}", r.slot_of("Main.N"));
    assert_eq!(r.get("Main.N"), Value::Dint(5));
    assert_eq!(r.get("Main.A1.M.I.Seen"), Value::Dint(9));
}
