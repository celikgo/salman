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

#[test]
fn probe_fb_initial_values() {
    let source = "\
PROGRAM Main
VAR
  A1 : Holder;
  N : DINT;
END_VAR
  N := A1.Out;
  A1();
END_PROGRAM

FUNCTION_BLOCK Holder
VAR_OUTPUT Out : DINT; END_VAR
VAR Seed : DINT := 42; END_VAR
  Out := Seed;
END_FUNCTION_BLOCK
";
    let r = run(source);
    println!("slots: {}", r.slots());
    println!("Seed = {:?}", r.get("Main.A1.Seed"));
    println!("Out  = {:?}", r.get("Main.A1.Out"));
}

#[test]
fn probe_multidim() {
    let source = "\
PROGRAM Main
VAR
  G : ARRAY [1..2, 1..3] OF DINT;
  I : DINT;
  J : DINT;
  N : DINT;
END_VAR
  FOR I := 1 TO 2 DO
    FOR J := 1 TO 3 DO
      G[I, J] := I * 10 + J;
    END_FOR;
  END_FOR;
  N := G[2, 3];
END_PROGRAM
";
    let r = run(source);
    println!("slots: {}", r.slots());
    for name in ["Main.G[1,1]","Main.G[1,2]","Main.G[1,3]","Main.G[2,1]","Main.G[2,2]","Main.G[2,3]"] {
        println!("{name} = {:?}", r.get(name));
    }
    println!("N = {:?}", r.get("Main.N"));
}

#[test]
fn probe_function_twice_and_state() {
    let source = "\
PROGRAM Main
VAR A : DINT; B : DINT; C : DINT; END_VAR
  A := Twice(3) + Twice(4);
  B := Counter();
  C := Counter();
END_PROGRAM

FUNCTION Twice : DINT
VAR_INPUT X : DINT; END_VAR
  Twice := X * 2;
END_FUNCTION

FUNCTION Counter : DINT
VAR N : DINT; END_VAR
  N := N + 1;
  Counter := N;
END_FUNCTION
";
    let r = run(source);
    println!("A={:?} B={:?} C={:?}", r.get("Main.A"), r.get("Main.B"), r.get("Main.C"));
}

#[test]
fn probe_for_negative_and_case() {
    let source = "\
PROGRAM Main
VAR I : DINT; N : DINT; S : DINT; K : DINT; END_VAR
  N := 0;
  FOR I := 5 TO 1 BY -1 DO
    N := N + I;
  END_FOR;
  S := 0;
  FOR I := 1 TO 10 BY 3 DO
    S := S + 1;
  END_FOR;
  K := 0;
  CASE S OF
    1: K := 100;
    4: CASE N OF
         15: K := 7;
       ELSE
         K := 8;
       END_CASE;
  ELSE
    K := 99;
  END_CASE;
END_PROGRAM
";
    let r = run(source);
    println!("N={:?} S={:?} K={:?} I={:?}", r.get("Main.N"), r.get("Main.S"), r.get("Main.K"), r.get("Main.I"));
}

#[test]
fn probe_mixed_arith() {
    let source = "\
PROGRAM Main
VAR
  A : INT; B : DINT; C : DINT; D : REAL; E : LREAL; F : SINT; G : UINT; H : DINT;
END_VAR
  A := 300;
  B := 1000000;
  C := A * B;
  D := 1.5;
  E := D + 1;
  F := 100;
  G := 40000;
  H := G + 1;
END_PROGRAM
";
    let r = run(source);
    println!("C={:?} E={:?} H={:?}", r.get("Main.C"), r.get("Main.E"), r.get("Main.H"));
}

#[test]
fn probe_order_dependence() {
    let source = "\
FUNCTION_BLOCK Inner
VAR_INPUT V : DINT; END_VAR
VAR Seen : DINT; END_VAR
  Seen := V;
END_FUNCTION_BLOCK

FUNCTION_BLOCK Middle
VAR_INPUT V : DINT; END_VAR
VAR I : Inner; END_VAR
  I(V := V);
END_FUNCTION_BLOCK

FUNCTION_BLOCK Outer
VAR_INPUT V : DINT; END_VAR
VAR M : Middle; END_VAR
  M(V := V);
END_FUNCTION_BLOCK

PROGRAM Main
VAR
  A1 : Outer;
  N : DINT;
END_VAR
  N := 5;
  A1(V := 9);
END_PROGRAM
";
    let r = run(source);
    println!("reordered: N={:?} Seen={:?}", r.get("Main.N"), r.get("Main.A1.M.I.Seen"));
}

#[test]
fn probe_arith2() {
    let source = "\
PROGRAM Main
VAR
  A : INT; B : DINT; C : DINT; R1 : REAL; L1 : LREAL; S1 : SINT; U1 : UINT; H : DINT;
  T1 : TIME; T2 : TIME; Q1 : BOOL; W1 : WORD; W2 : WORD;
END_VAR
  A := 300;
  B := 1000000;
  C := A * B;
  R1 := 1.5;
  L1 := R1 + 1;
  S1 := 100;
  U1 := 40000;
  H := U1 + 1;
  T1 := T#1s;
  T2 := T1 * 3;
  Q1 := T2 > T#2s;
  W1 := 16#00FF;
  W2 := W1 AND 16#0F0F;
END_PROGRAM
";
    let r = run(source);
    println!("C={:?} L1={:?} H={:?} T2={:?} Q1={:?} W2={:?}",
        r.get("Main.C"), r.get("Main.L1"), r.get("Main.H"), r.get("Main.T2"), r.get("Main.Q1"), r.get("Main.W2"));
}

#[test]
fn probe_struct_and_global() {
    let source = "\
TYPE Point : STRUCT X : DINT; Y : DINT; END_STRUCT; END_TYPE

VAR_GLOBAL
  Total : DINT := 3;
  Pt : Point;
END_VAR

PROGRAM Main
VAR
  Local : Point;
  N : DINT;
END_VAR
  Local.X := 4;
  Local.Y := Local.X + 1;
  Pt.Y := 9;
  N := Total + Pt.Y + Local.Y;
END_PROGRAM
";
    let r = run(source);
    println!("slots: {}", r.slots());
    println!("N={:?} LocalY={:?} PtY={:?}", r.get("Main.N"), r.get("Main.Local.Y"), r.get("Pt.Y"));
}

#[test]
fn probe_array_of_struct_and_fb_output() {
    let source = "\
PROGRAM Main
VAR
  T1 : TON;
  Started : BOOL;
  Elapsed : TIME;
  Go : BOOL;
END_VAR
  Go := TRUE;
  T1(IN := Go, PT := T#10ms, Q => Started, ET => Elapsed);
END_PROGRAM
";
    let r = run_scans(source, 5);
    println!("slots: {}", r.slots());
    println!("Started={:?} Elapsed={:?} Q={:?}", r.get("Main.Started"), r.get("Main.Elapsed"), r.get("Main.T1.Q"));
}

#[test]
fn probe_image() {
    let source = "\
PROGRAM Main
VAR N : DINT; B : BOOL; END_VAR
  %QX0.0 := TRUE;
  B := %QX0.0;
  %MB1 := 16#7F;
  IF %MX1.0 THEN N := 1; ELSE N := 2; END_IF;
END_PROGRAM
";
    let r = run(source);
    println!("N={:?} B={:?}", r.get("Main.N"), r.get("Main.B"));
}
