// SPDX-License-Identifier: Apache-2.0
//! What a compiled program actually computes.
//!
//! These run Structured Text through the whole pipeline — lex, parse, check,
//! compile, scan — and assert on the values the runtime holds afterwards. They
//! are written against behaviour an engineer can see in a watch window rather
//! than against any stage's internals, because the failure this project exists
//! to prevent is a wrong number on a running plant, not an untidy intermediate
//! representation.
//!
//! Reading a slot by its dotted name is what a watch list does, and it is the
//! only part of the runtime these tests know about.
//!
//! Several of these were written to demonstrate a bug in the compiler and kept
//! afterwards to stop it coming back. Each of those says so.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_core::time::Duration;
use salman_core::value::Value;
use salman_lang::address::{AddressLocation, AddressSize, DirectAddress};
use salman_lang::dialect::Dialect;
use salman_vm::clock::Clock;
use salman_vm::memory::Restart;
use salman_vm::project::{Build, build};
use salman_vm::task::Runtime;

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

fn compile(source: &str) -> Build {
    build("t.st", source, &Dialect::generic()).expect("not too large")
}

/// Builds one source file, insisting that it compiled cleanly.
fn built(source: &str) -> Build {
    let build = compile(source);
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
            .unwrap_or_else(|| panic!("no slot called {name}; there are {}", self.slots()));
        self.runtime
            .memory()
            .read_slot(slot)
            .cloned()
            .unwrap_or_else(|| panic!("slot {name} is outside memory"))
    }

    fn int(&self, name: &str) -> i64 {
        let value = self.get(name);
        value
            .as_i64()
            .unwrap_or_else(|| panic!("{name} is not an integer: {value:?}"))
    }

    fn boolean(&self, name: &str) -> bool {
        match self.get(name) {
            Value::Bool(flag) => flag,
            other => panic!("{name} is not a BOOL: {other:?}"),
        }
    }

    fn slots(&self) -> String {
        self.runtime.program().slot_names.join(", ")
    }
}

/// Builds a source file into a runtime, without running it.
fn loaded(source: &str) -> Runtime {
    let compiled = built(source).compiled.expect("a compiled program");
    Runtime::new(
        compiled.program,
        compiled.memory,
        Clock::virtual_default(),
        compiled.tasks,
    )
}

/// Compiles a source file and runs it for `scans` scans, faulting on nothing.
fn run_scans(source: &str, scans: u64) -> Ran {
    let mut runtime = loaded(source);
    runtime.run_scans(scans);
    assert!(
        !runtime.has_faulted(),
        "the program faulted: {}",
        runtime
            .faults()
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<String>>()
            .join("; ")
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

/// Runs one program body of declarations and statements for one scan.
fn one(declarations: &str, body: &str) -> Ran {
    run(&program(declarations, body))
}

/// The diagnostic codes a source file produces.
fn codes(source: &str) -> Vec<&'static str> {
    compile(source)
        .diagnostics
        .items()
        .iter()
        .map(|d| d.code.0)
        .collect()
}

/// Runs a program that is expected to fault, and returns the first fault.
fn fault_of(source: &str) -> String {
    let mut runtime = loaded(source);
    runtime.run_scans(1);
    let Some(fault) = runtime.faults().first() else {
        panic!("expected a fault");
    };
    fault.to_string()
}

/// A bit address, for driving the process image from outside the program.
fn bit(location: AddressLocation, byte: u32, offset: u32) -> DirectAddress {
    DirectAddress {
        location,
        size: AddressSize::Bit,
        size_letter_written: true,
        path: Some(vec![byte, offset]),
    }
}

// ---------------------------------------------------------------------------
// Arithmetic
// ---------------------------------------------------------------------------

#[test]
fn an_operation_between_two_widths_is_done_at_the_wider_one() {
    // INT * DINT is a DINT multiplication. Doing it at INT would wrap at
    // 32767 and produce a number that still looks like a reading.
    let ran = one(
        "  Small : INT;\n  Large : DINT;\n  Answer : DINT;",
        "  Small := 300;\n  Large := 1000000;\n  Answer := Small * Large;",
    );
    assert_eq!(ran.int("Main.Answer"), 300_000_000);
}

#[test]
fn unary_plus_is_the_identity_and_unary_minus_negates() {
    // Bug: `+X` compiled to the negation instruction, so every unary plus
    // returned the wrong sign. Nothing in the source says `+` and means `-`.
    let ran = one(
        "  Source : DINT;\n  Plus : DINT;\n  Minus : DINT;",
        "  Source := 7;\n  Plus := +Source;\n  Minus := -Source;",
    );
    assert_eq!(ran.int("Main.Plus"), 7, "`+X` must not change X");
    assert_eq!(ran.int("Main.Minus"), -7);
}

#[test]
fn unary_plus_on_a_literal_is_the_literal() {
    let ran = one("  N : DINT;", "  N := +5;");
    assert_eq!(ran.int("Main.N"), 5);
}

#[test]
fn an_integer_operand_widens_to_the_real_it_is_used_with() {
    let ran = one(
        "  Ratio : REAL;\n  Answer : LREAL;",
        "  Ratio := 1.5;\n  Answer := Ratio + 1;",
    );
    assert_eq!(ran.get("Main.Answer"), Value::lreal(2.5));
}

#[test]
fn integer_division_truncates_toward_zero() {
    let ran = one(
        "  Down : DINT;\n  Rest : DINT;",
        "  Down := -7 / 2;\n  Rest := -7 MOD 2;",
    );
    assert_eq!(ran.int("Main.Down"), -3);
    assert_eq!(ran.int("Main.Rest"), -1);
}

#[test]
fn a_literal_takes_the_type_of_the_variable_it_is_assigned_to() {
    // SINT holds -128 and does not hold 128, so the sign has to be part of the
    // value before the range is checked.
    let ran = one(
        "  Edge : SINT;\n  Top : SINT;",
        "  Edge := -128;\n  Top := 127;",
    );
    assert_eq!(ran.get("Main.Edge"), Value::Sint(-128));
    assert_eq!(ran.get("Main.Top"), Value::Sint(127));
}

#[test]
fn unsigned_arithmetic_stays_unsigned() {
    let ran = one(
        "  Count : UINT;\n  Answer : DINT;",
        "  Count := 40000;\n  Count := Count + 1;\n  Answer := Count;",
    );
    assert_eq!(ran.get("Main.Count"), Value::Uint(40_001));
    assert_eq!(ran.int("Main.Answer"), 40_001);
}

#[test]
fn integer_overflow_wraps_at_the_declared_width() {
    // salman policy, chosen to match what a controller does. What matters here
    // is that the wrap happens at the width the variable was declared with,
    // not at whatever width the interpreter used on the way.
    let ran = one("  Edge : INT;", "  Edge := 32767;\n  Edge := Edge + INT#1;");
    assert_eq!(ran.get("Main.Edge"), Value::Int(i16::MIN));
}

#[test]
fn a_duration_scales_by_a_number_and_compares_with_a_duration() {
    let ran = one(
        "  Base : TIME;\n  Scaled : TIME;\n  Longer : BOOL;",
        "  Base := T#1s;\n  Scaled := Base * 3;\n  Longer := Scaled > T#2s;",
    );
    assert_eq!(
        ran.get("Main.Scaled"),
        Value::Time(Duration::from_nanos(3_000_000_000))
    );
    assert!(ran.boolean("Main.Longer"));
}

#[test]
fn a_bit_operation_keeps_the_width_of_its_operands() {
    let ran = one(
        "  Mask : WORD;\n  Answer : WORD;",
        "  Mask := 16#00FF;\n  Answer := Mask AND 16#0F0F;",
    );
    assert_eq!(ran.get("Main.Answer"), Value::Word(0x000F));
}

#[test]
fn not_inverts_every_bit_of_the_width_it_is_written_on() {
    let ran = one(
        "  Bits : BYTE;\n  Flag : BOOL;",
        "  Bits := NOT BYTE#16#0F;\n  Flag := NOT FALSE;",
    );
    assert_eq!(ran.get("Main.Bits"), Value::Byte(0xF0));
    assert!(ran.boolean("Main.Flag"));
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

#[test]
fn an_if_chain_runs_exactly_one_branch() {
    for (input, expected) in [(0, 3), (1, 1), (5, 2)] {
        let ran = one(
            "  Choice : DINT;\n  Answer : DINT;",
            &format!(
                "  Choice := {input};
  IF Choice = 1 THEN Answer := 1;
  ELSIF Choice > 1 THEN Answer := 2;
  ELSE Answer := 3; END_IF;"
            ),
        );
        assert_eq!(ran.int("Main.Answer"), expected, "for a choice of {input}");
    }
}

#[test]
fn a_case_selector_is_evaluated_once_and_not_again_for_each_arm() {
    // The first arm changes the selector variable. If the selector were read
    // again for the second arm, both arms would run.
    let ran = one(
        "  Stage : DINT;\n  Answer : DINT;",
        "  Stage := 1;
  CASE Stage OF
    1: Stage := 2; Answer := 10;
    2: Answer := 20;
  ELSE
    Answer := 99;
  END_CASE;",
    );
    assert_eq!(ran.int("Main.Stage"), 2);
    assert_eq!(ran.int("Main.Answer"), 10, "the second arm also ran");
}

#[test]
fn a_case_inside_a_case_does_not_disturb_the_selector_around_it() {
    let ran = one(
        "  Outer : DINT;\n  Inner : DINT;\n  Answer : DINT;",
        "  Outer := 2;
  Inner := 7;
  CASE Outer OF
    1: Answer := 1;
    2:
      CASE Inner OF
        7: Answer := 70;
      ELSE
        Answer := 71;
      END_CASE;
    3: Answer := 3;
  ELSE
    Answer := 99;
  END_CASE;",
    );
    assert_eq!(ran.int("Main.Answer"), 70);
}

#[test]
fn a_case_range_label_matches_every_value_in_it_and_none_outside_it() {
    for (input, expected) in [(2, 0), (3, 1), (5, 1), (9, 1), (10, 0)] {
        let ran = one(
            "  Reading : DINT;\n  InBand : DINT;",
            &format!(
                "  Reading := {input};
  InBand := 0;
  CASE Reading OF
    3..9: InBand := 1;
  END_CASE;"
            ),
        );
        assert_eq!(ran.int("Main.InBand"), expected, "for a reading of {input}");
    }
}

#[test]
fn a_for_loop_counts_down_when_its_step_is_negative() {
    let ran = one(
        "  I : DINT;\n  Total : DINT;",
        "  Total := 0;\n  FOR I := 5 TO 1 BY -1 DO Total := Total + I; END_FOR;",
    );
    assert_eq!(ran.int("Main.Total"), 15);
}

#[test]
fn a_for_loop_whose_step_overshoots_stops_at_the_limit() {
    let ran = one(
        "  I : DINT;\n  Trips : DINT;",
        "  Trips := 0;\n  FOR I := 1 TO 10 BY 3 DO Trips := Trips + 1; END_FOR;",
    );
    assert_eq!(ran.int("Main.Trips"), 4, "1, 4, 7 and 10");
}

#[test]
fn a_for_loop_whose_range_is_empty_never_runs_its_body() {
    let ran = one(
        "  I : DINT;\n  Trips : DINT;",
        "  Trips := 0;\n  FOR I := 5 TO 1 DO Trips := Trips + 1; END_FOR;",
    );
    assert_eq!(ran.int("Main.Trips"), 0);
}

#[test]
fn the_bounds_of_a_for_loop_are_evaluated_once_at_entry() {
    // salman policy, recorded in docs/CONFORMANCE.md: a body that changes what
    // TO was computed from does not change the trip count.
    let source = "\
VAR_GLOBAL Limit : DINT := 3; END_VAR

PROGRAM Main
VAR I : DINT; Trips : DINT; END_VAR
  Trips := 0;
  Limit := 3;
  FOR I := 1 TO Limit DO
    Trips := Trips + 1;
    Limit := 10;
  END_FOR;
END_PROGRAM
";
    let ran = run(source);
    assert_eq!(ran.int("Main.Trips"), 3);
    assert_eq!(ran.int("Limit"), 10, "the body did change the variable");
}

#[test]
fn exit_leaves_the_innermost_loop_and_continue_starts_its_next_pass() {
    let ran = one(
        "  I : DINT;\n  J : DINT;\n  Counted : DINT;",
        "  Counted := 0;
  FOR I := 1 TO 3 DO
    FOR J := 1 TO 3 DO
      IF J = 2 THEN CONTINUE; END_IF;
      IF J = 3 THEN EXIT; END_IF;
      Counted := Counted + 1;
    END_FOR;
  END_FOR;",
    );
    assert_eq!(
        ran.int("Main.Counted"),
        3,
        "the outer loop must still run three times"
    );
}

#[test]
fn a_while_loop_that_is_false_at_entry_never_runs() {
    let ran = one(
        "  Trips : DINT;",
        "  Trips := 0;\n  WHILE FALSE DO Trips := Trips + 1; END_WHILE;",
    );
    assert_eq!(ran.int("Main.Trips"), 0);
}

#[test]
fn a_repeat_loop_runs_its_body_before_it_tests_and_continue_goes_to_the_test() {
    let ran = one(
        "  Trips : DINT;",
        "  Trips := 0;
  REPEAT
    Trips := Trips + 1;
    IF Trips < 3 THEN CONTINUE; END_IF;
  UNTIL Trips >= 5 END_REPEAT;",
    );
    assert_eq!(ran.int("Main.Trips"), 5);
}

// ---------------------------------------------------------------------------
// Arrays
// ---------------------------------------------------------------------------

#[test]
fn a_two_dimensional_array_is_linearised_row_by_row() {
    let source = "\
PROGRAM Main
VAR
  Grid : ARRAY [1..2, 1..3] OF DINT;
  I : DINT;
  J : DINT;
  Corner : DINT;
END_VAR
  FOR I := 1 TO 2 DO
    FOR J := 1 TO 3 DO
      Grid[I, J] := I * 10 + J;
    END_FOR;
  END_FOR;
  Corner := Grid[2, 3];
END_PROGRAM
";
    let ran = run(source);
    // Every element holds its own coordinates, so no two subscripts can have
    // landed on one slot without this failing.
    for (row, column) in [(1, 1), (1, 2), (1, 3), (2, 1), (2, 2), (2, 3)] {
        assert_eq!(
            ran.int(&format!("Main.Grid[{row},{column}]")),
            i64::from(row * 10 + column),
            "at [{row},{column}]"
        );
    }
    assert_eq!(ran.int("Main.Corner"), 23);
}

#[test]
fn an_array_is_indexed_from_its_declared_lower_bound() {
    let source = "\
PROGRAM Main
VAR
  Readings : ARRAY [-2..2] OF DINT;
  I : DINT;
  First : DINT;
  Last : DINT;
END_VAR
  FOR I := -2 TO 2 DO
    Readings[I] := I;
  END_FOR;
  First := Readings[-2];
  Last := Readings[2];
END_PROGRAM
";
    let ran = run(source);
    assert_eq!(ran.int("Main.First"), -2);
    assert_eq!(ran.int("Main.Last"), 2);
    assert_eq!(ran.int("Main.Readings[0]"), 0);
}

#[test]
fn a_subscript_outside_the_bounds_stops_the_scan_and_names_them() {
    let fault = fault_of(
        "PROGRAM Main
VAR Readings : ARRAY [1..3] OF DINT; I : DINT; END_VAR
  I := 5;
  Readings[I] := 1;
END_PROGRAM
",
    );
    assert!(fault.contains("array index 5"), "{fault}");
    assert!(fault.contains("1..3"), "{fault}");
}

#[test]
fn each_dimension_is_checked_against_its_own_bounds() {
    // A subscript inside the flattened array but outside its own dimension
    // would otherwise alias quietly into the next row.
    let fault = fault_of(
        "PROGRAM Main
VAR Grid : ARRAY [1..2, 1..3] OF DINT; J : DINT; END_VAR
  J := 5;
  Grid[1, J] := 1;
END_PROGRAM
",
    );
    assert!(fault.contains("array index 5"), "{fault}");
    assert!(
        fault.contains("1..3"),
        "the second dimension's bounds, not the whole array's: {fault}"
    );
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

#[test]
fn a_function_called_twice_in_one_expression_gets_both_answers_right() {
    // A FUNCTION has one static frame, so the second call overwrites the
    // first's arguments. The first result has to be off the frame by then.
    let source = "\
PROGRAM Main
VAR Answer : DINT; END_VAR
  Answer := Doubled(3) + Doubled(4);
END_PROGRAM

FUNCTION Doubled : DINT
VAR_INPUT X : DINT; END_VAR
  Doubled := X * 2;
END_FUNCTION
";
    assert_eq!(run(source).int("Main.Answer"), 14);
}

#[test]
fn a_function_may_call_another_function() {
    let source = "\
PROGRAM Main
VAR Answer : DINT; END_VAR
  Answer := Product(4);
END_PROGRAM

FUNCTION Increment : DINT
VAR_INPUT X : DINT; END_VAR
  Increment := X + 1;
END_FUNCTION

FUNCTION Product : DINT
VAR_INPUT X : DINT; END_VAR
  Product := Increment(X) * Increment(X + 1);
END_FUNCTION
";
    assert_eq!(run(source).int("Main.Answer"), 30, "5 times 6");
}

#[test]
fn a_function_keeps_no_state_between_calls() {
    // Bug: a function's static frame was never cleared, so a VAR carried its
    // last value into the next call and the same call answered differently the
    // second time. salman's own diagnostic for an unbound parameter tells the
    // engineer that a function keeps no state; this is what makes that true.
    let source = "\
PROGRAM Main
VAR First : DINT; Second : DINT; END_VAR
  First := Tally();
  Second := Tally();
END_PROGRAM

FUNCTION Tally : DINT
VAR Seen : DINT; END_VAR
  Seen := Seen + 1;
  Tally := Seen;
END_FUNCTION
";
    let ran = run(source);
    assert_eq!(ran.int("Main.First"), 1);
    assert_eq!(ran.int("Main.Second"), 1, "the function remembered a call");
}

#[test]
fn a_function_local_starts_from_its_declared_initial_value_on_every_call() {
    let source = "\
PROGRAM Main
VAR First : DINT; Second : DINT; END_VAR
  First := Offset();
  Second := Offset();
END_PROGRAM

FUNCTION Offset : DINT
VAR Base : DINT := 100; END_VAR
  Base := Base + 1;
  Offset := Base;
END_FUNCTION
";
    let ran = run(source);
    assert_eq!(ran.int("Main.First"), 101);
    assert_eq!(ran.int("Main.Second"), 101);
}

#[test]
fn an_argument_written_for_a_var_in_out_parameter_reaches_it_and_comes_back() {
    // Bug: the compiler bound positional arguments to VAR_INPUT parameters
    // only, while the checker counted VAR_IN_OUT among them. The second
    // argument was accepted, silently dropped, and the parameter kept whatever
    // the previous call had left in it.
    let source = "\
PROGRAM Main
VAR Counter : DINT; Answer : DINT; END_VAR
  Counter := 5;
  Answer := Bump(2, Counter);
END_PROGRAM

FUNCTION Bump : DINT
VAR_INPUT Amount : DINT; END_VAR
VAR_IN_OUT Total : DINT; END_VAR
  Total := Total + Amount;
  Bump := Total;
END_FUNCTION
";
    let ran = run(source);
    assert_eq!(ran.int("Main.Answer"), 7);
    assert_eq!(ran.int("Main.Counter"), 7, "the value was not copied back");
}

#[test]
fn a_function_converts_an_argument_to_the_type_its_parameter_declares() {
    let source = "\
PROGRAM Main
VAR Small : INT; Answer : DINT; END_VAR
  Small := 300;
  Answer := Scaled(Small);
END_PROGRAM

FUNCTION Scaled : DINT
VAR_INPUT X : DINT; END_VAR
  Scaled := X * 1000;
END_FUNCTION
";
    assert_eq!(run(source).int("Main.Answer"), 300_000);
}

// ---------------------------------------------------------------------------
// Function blocks
// ---------------------------------------------------------------------------

#[test]
fn two_instances_of_one_function_block_keep_separate_state() {
    let source = "\
PROGRAM Main
VAR
  Left : Tally; Right : Tally;
  A : DINT; B : DINT;
END_VAR
  Left(); Left(); Right();
  A := Left.Total;
  B := Right.Total;
END_PROGRAM

FUNCTION_BLOCK Tally
VAR_OUTPUT Total : DINT; END_VAR
  Total := Total + 1;
END_FUNCTION_BLOCK
";
    let ran = run(source);
    assert_eq!(ran.int("Main.A"), 2);
    assert_eq!(ran.int("Main.B"), 1);
}

#[test]
fn a_function_block_keeps_its_state_from_one_scan_to_the_next() {
    let source = "\
PROGRAM Main
VAR Counter : Tally; Seen : DINT; END_VAR
  Counter();
  Seen := Counter.Total;
END_PROGRAM

FUNCTION_BLOCK Tally
VAR_OUTPUT Total : DINT; END_VAR
  Total := Total + 1;
END_FUNCTION_BLOCK
";
    assert_eq!(run_scans(source, 4).int("Main.Seen"), 4);
}

#[test]
fn an_instance_nested_three_blocks_deep_gets_storage_of_its_own() {
    // Bug: the layout pass ran a fixed three times, which settles a block
    // nested two deep and not one nested three deep — and only when the
    // containing POU is written above the blocks it uses, which is how people
    // write. `Spare` and the innermost instance then shared a slot, so
    // assigning to one changed the other and nothing was said.
    let source = "\
PROGRAM Main
VAR
  Chain : Outer;
  Spare : DINT;
END_VAR
  Spare := 5;
  Chain(V := 9);
END_PROGRAM

FUNCTION_BLOCK Outer
VAR_INPUT V : DINT; END_VAR
VAR Middle : Centre; END_VAR
  Middle(V := V);
END_FUNCTION_BLOCK

FUNCTION_BLOCK Centre
VAR_INPUT V : DINT; END_VAR
VAR Core : Inner; END_VAR
  Core(V := V);
END_FUNCTION_BLOCK

FUNCTION_BLOCK Inner
VAR_INPUT V : DINT; END_VAR
VAR Seen : DINT; END_VAR
  Seen := V;
END_FUNCTION_BLOCK
";
    let ran = run(source);
    assert_eq!(ran.int("Main.Spare"), 5, "a nested instance took this slot");
    assert_eq!(ran.int("Main.Chain.Middle.Core.Seen"), 9);
}

#[test]
fn the_order_the_blocks_are_written_in_does_not_change_what_a_program_computes() {
    // The same program with its declarations reversed. Before the layout pass
    // iterated to a fixpoint these two gave different answers, which is the
    // worst kind of bug: it moves when the file is rearranged.
    let blocks = "\
FUNCTION_BLOCK Inner
VAR_INPUT V : DINT; END_VAR
VAR Seen : DINT; END_VAR
  Seen := V;
END_FUNCTION_BLOCK

FUNCTION_BLOCK Centre
VAR_INPUT V : DINT; END_VAR
VAR Core : Inner; END_VAR
  Core(V := V);
END_FUNCTION_BLOCK

FUNCTION_BLOCK Outer
VAR_INPUT V : DINT; END_VAR
VAR Middle : Centre; END_VAR
  Middle(V := V);
END_FUNCTION_BLOCK
";
    let main = "\
PROGRAM Main
VAR Chain : Outer; Spare : DINT; END_VAR
  Spare := 5;
  Chain(V := 9);
END_PROGRAM
";
    let blocks_first = run(&format!("{blocks}\n{main}"));
    let main_first = run(&format!("{main}\n{blocks}"));
    for name in ["Main.Spare", "Main.Chain.Middle.Core.Seen"] {
        assert_eq!(
            blocks_first.int(name),
            main_first.int(name),
            "{name} depends on the order the file was written in"
        );
    }
    assert_eq!(main_first.int("Main.Spare"), 5);
}

#[test]
fn a_declared_initial_value_inside_a_function_block_reaches_every_instance() {
    // Bug: initial values were recorded only for a top-level instance's own
    // variables, so `Setpoint : REAL := 20.0` inside a block started at zero in
    // every instance of it.
    let source = "\
PROGRAM Main
VAR Left : Holder; Right : Holder; A : DINT; B : DINT; END_VAR
  Left();
  Right();
  A := Left.Out;
  B := Right.Out;
END_PROGRAM

FUNCTION_BLOCK Holder
VAR Seed : DINT := 42; END_VAR
VAR_OUTPUT Out : DINT; END_VAR
  Out := Seed;
END_FUNCTION_BLOCK
";
    let ran = run(source);
    assert_eq!(ran.int("Main.A"), 42);
    assert_eq!(ran.int("Main.B"), 42);
}

#[test]
fn a_retained_variable_inside_a_function_block_survives_a_warm_restart() {
    // Bug: a nested instance took its container's persistence and nothing
    // else, so a RETAIN inside a block was volatile and a retained counter
    // silently restarted from zero.
    let source = "\
PROGRAM Main
VAR Keep : Keeper; END_VAR
  Keep();
END_PROGRAM

FUNCTION_BLOCK Keeper
VAR RETAIN Kept : DINT; END_VAR
  Kept := Kept + 1;
END_FUNCTION_BLOCK
";
    let mut runtime = loaded(source);
    runtime.run_scans(3);
    let slot = runtime
        .program()
        .slot_index("Main.Keep.Kept")
        .expect("the retained variable has a slot");
    assert_eq!(runtime.memory().read_slot(slot), Some(&Value::Dint(3)));
    runtime.memory_mut().restart(Restart::Warm);
    assert_eq!(
        runtime.memory().read_slot(slot),
        Some(&Value::Dint(3)),
        "a warm restart cleared a RETAIN"
    );
}

#[test]
fn a_function_block_that_holds_an_instance_of_itself_is_refused() {
    // It has no finite size. Before this it compiled, its layout never
    // settled, and every variable declared after it was misplaced.
    let source = "\
PROGRAM Main
VAR Repeated : Looper; Spare : DINT; END_VAR
  Spare := 5;
END_PROGRAM

FUNCTION_BLOCK Looper
VAR Inner : Looper; V : DINT; END_VAR
  V := 1;
END_FUNCTION_BLOCK
";
    let build = compile(source);
    assert!(
        build.diagnostics.has_errors(),
        "{}",
        build.render_diagnostics()
    );
    assert!(build.compiled.is_none(), "a bad layout must not be run");
    assert!(codes(source).contains(&"E0501"), "{:?}", codes(source));
}

#[test]
fn two_function_blocks_that_hold_each_other_are_refused() {
    let source = "\
PROGRAM Main
VAR First : Ping; END_VAR
  First();
END_PROGRAM

FUNCTION_BLOCK Ping
VAR Other : Pong; V : DINT; END_VAR
  V := 1;
END_FUNCTION_BLOCK

FUNCTION_BLOCK Pong
VAR Back : Ping; V : DINT; END_VAR
  V := 2;
END_FUNCTION_BLOCK
";
    assert!(compile(source).compiled.is_none());
}

#[test]
fn a_standard_timer_runs_from_a_program_and_binds_its_outputs() {
    let source = "\
PROGRAM Main
VAR
  Delay : TON;
  Running : BOOL;
  Elapsed : TIME;
  Go : BOOL;
END_VAR
  Go := TRUE;
  Delay(IN := Go, PT := T#3ms, Q => Running, ET => Elapsed);
END_PROGRAM
";
    let ran = run_scans(source, 6);
    assert!(ran.boolean("Main.Running"), "the timer never expired");
    assert_eq!(
        ran.get("Main.Elapsed"),
        Value::Time(Duration::from_nanos(3_000_000)),
        "ET is clamped at PT"
    );
}

#[test]
fn a_function_block_instance_inside_a_structure_is_reached_through_the_field() {
    let source = "\
TYPE Station : STRUCT Delay : TON; Count : DINT; END_STRUCT; END_TYPE

PROGRAM Main
VAR Bay : Station; Go : BOOL; END_VAR
  Go := TRUE;
  Bay.Delay(IN := Go, PT := T#3ms);
  Bay.Count := 5;
END_PROGRAM
";
    let ran = run_scans(source, 6);
    assert_eq!(ran.int("Main.Bay.Count"), 5);
    assert_eq!(
        ran.get("Main.Bay.Delay.ET"),
        Value::Time(Duration::from_nanos(3_000_000))
    );
}

// ---------------------------------------------------------------------------
// Structures, arrays and enumerations as whole values
// ---------------------------------------------------------------------------

#[test]
fn assigning_one_structure_to_another_copies_every_field() {
    // Bug: an aggregate assignment compiled to one load and one store, so it
    // copied the first field and left the rest of the target as it was.
    let source = "\
TYPE Point : STRUCT X : DINT; Y : DINT; END_STRUCT; END_TYPE

PROGRAM Main
VAR Here : Point; There : Point; END_VAR
  There.X := 1;
  There.Y := 2;
  Here := There;
END_PROGRAM
";
    let ran = run(source);
    assert_eq!(ran.int("Main.Here.X"), 1);
    assert_eq!(ran.int("Main.Here.Y"), 2, "only the first field was copied");
}

#[test]
fn assigning_one_array_to_another_copies_every_element() {
    let source = "\
PROGRAM Main
VAR
  Left : ARRAY [1..3] OF DINT;
  Right : ARRAY [1..3] OF DINT;
END_VAR
  Right[1] := 7; Right[2] := 8; Right[3] := 9;
  Left := Right;
END_PROGRAM
";
    let ran = run(source);
    for (index, expected) in [(1, 7), (2, 8), (3, 9)] {
        assert_eq!(ran.int(&format!("Main.Left[{index}]")), expected);
    }
}

#[test]
fn an_unqualified_enumeration_value_compiles_and_selects_its_arm() {
    // The checker resolves `Green` from the type the context wants; the
    // compiler had no address to give for it and failed the build.
    let source = "\
TYPE Colour : (Red, Green, Blue); END_TYPE

PROGRAM Main
VAR Shade : Colour; Answer : DINT; END_VAR
  Shade := Green;
  CASE Shade OF
    Red: Answer := 1;
    Green: Answer := 2;
    Blue: Answer := 3;
  END_CASE;
END_PROGRAM
";
    assert_eq!(run(source).int("Main.Answer"), 2);
}

#[test]
fn a_qualified_enumeration_value_means_the_same_as_an_unqualified_one() {
    let source = "\
TYPE Colour : (Red, Green, Blue); END_TYPE

PROGRAM Main
VAR Shade : Colour; Same : BOOL; END_VAR
  Shade := Colour#Green;
  Same := Shade = Green;
END_PROGRAM
";
    assert!(run(source).boolean("Main.Same"));
}

#[test]
fn a_structure_field_and_a_global_are_reached_from_a_program_body() {
    let source = "\
TYPE Point : STRUCT X : DINT; Y : DINT; END_STRUCT; END_TYPE

VAR_GLOBAL
  Offset : DINT := 3;
  Origin : Point;
END_VAR

PROGRAM Main
VAR Here : Point; Answer : DINT; END_VAR
  Here.X := 4;
  Here.Y := Here.X + 1;
  Origin.Y := 9;
  Answer := Offset + Origin.Y + Here.Y;
END_PROGRAM
";
    let ran = run(source);
    assert_eq!(ran.int("Main.Answer"), 17);
    assert_eq!(ran.int("Origin.Y"), 9);
}

// ---------------------------------------------------------------------------
// The process image and the scan
// ---------------------------------------------------------------------------

#[test]
fn an_input_read_twice_in_one_scan_reads_the_same_value_both_times() {
    // The scan model: inputs are latched once, at the start. A program that
    // saw an input change part way through a scan is being lied to.
    let source = "\
PROGRAM Main
VAR First : BOOL; Second : BOOL; END_VAR
  First := %IX0.0;
  Second := %IX0.0;
END_PROGRAM
";
    let mut runtime = loaded(source);
    let sensor = bit(AddressLocation::Input, 0, 0);
    let position = runtime
        .memory()
        .input_image()
        .resolve(&sensor)
        .expect("the address resolves");
    runtime
        .memory_mut()
        .physical_inputs_mut()
        .write(position, &Value::Bool(true));
    runtime.run_scans(1);
    let ran = Ran { runtime };
    assert!(ran.boolean("Main.First"));
    assert!(ran.boolean("Main.Second"));
}

#[test]
fn an_input_that_changes_between_scans_is_seen_on_the_next_one() {
    let source = "\
PROGRAM Main
VAR Seen : BOOL; Scans : DINT; END_VAR
  Seen := %IX0.0;
  Scans := Scans + 1;
END_PROGRAM
";
    let mut runtime = loaded(source);
    let sensor = bit(AddressLocation::Input, 0, 0);
    let position = runtime
        .memory()
        .input_image()
        .resolve(&sensor)
        .expect("the address resolves");
    runtime.run_scans(1);
    let slot = runtime.program().slot_index("Main.Seen").expect("a slot");
    assert_eq!(runtime.memory().read_slot(slot), Some(&Value::Bool(false)));
    runtime
        .memory_mut()
        .physical_inputs_mut()
        .write(position, &Value::Bool(true));
    runtime.run_scans(1);
    assert_eq!(runtime.memory().read_slot(slot), Some(&Value::Bool(true)));
}

#[test]
fn an_output_is_readable_within_the_scan_that_wrote_it_and_published_at_its_end() {
    // Seal-in logic depends on the first half; the outside world on the second.
    let source = "\
PROGRAM Main
VAR Echo : BOOL; END_VAR
  %QX0.1 := TRUE;
  Echo := %QX0.1;
END_PROGRAM
";
    let mut runtime = loaded(source);
    runtime.run_scans(1);
    let coil = bit(AddressLocation::Output, 0, 1);
    let position = runtime
        .memory()
        .output_image()
        .resolve(&coil)
        .expect("the address resolves");
    assert_eq!(
        runtime.memory().physical_outputs().read(position),
        Some(Value::Bool(true)),
        "the coil never reached the world"
    );
    let ran = Ran { runtime };
    assert!(ran.boolean("Main.Echo"));
}

#[test]
fn marker_memory_is_written_through_within_a_scan() {
    let source = "\
PROGRAM Main
VAR Answer : DINT; END_VAR
  %MB1 := 16#7F;
  IF %MX1.0 THEN Answer := 1; ELSE Answer := 2; END_IF;
END_PROGRAM
";
    assert_eq!(run(source).int("Main.Answer"), 1);
}

#[test]
fn two_instances_of_one_program_keep_separate_state() {
    let source = "\
PROGRAM Counter
VAR Seen : DINT; END_VAR
  Seen := Seen + 1;
END_PROGRAM

CONFIGURATION Plant
  RESOURCE R1 ON CPU
    TASK Fast (INTERVAL := T#10ms, PRIORITY := 1);
    PROGRAM P1 WITH Fast : Counter;
    PROGRAM P2 WITH Fast : Counter;
  END_RESOURCE
END_CONFIGURATION
";
    // One scan of the task runs both instances, so three scans is three each.
    let ran = run_scans(source, 3);
    assert_eq!(ran.int("P1.Seen"), 3);
    assert_eq!(ran.int("P2.Seen"), 3);
}

#[test]
fn a_global_is_shared_between_two_programs_that_name_it() {
    let source = "\
VAR_GLOBAL Shared : DINT; END_VAR

PROGRAM Raise
  Shared := Shared + 1;
END_PROGRAM

PROGRAM Watch
VAR Seen : DINT; END_VAR
  Seen := Shared;
END_PROGRAM

CONFIGURATION Plant
  RESOURCE R1 ON CPU
    TASK Fast (INTERVAL := T#10ms, PRIORITY := 1);
    PROGRAM P1 WITH Fast : Raise;
    PROGRAM P2 WITH Fast : Watch;
  END_RESOURCE
END_CONFIGURATION
";
    let ran = run_scans(source, 2);
    assert_eq!(ran.int("Shared"), 2);
    assert_eq!(ran.int("P2.Seen"), 2);
}

// ---------------------------------------------------------------------------
// What salman refuses rather than getting quietly wrong
// ---------------------------------------------------------------------------

#[test]
fn a_var_external_declaration_is_refused_rather_than_given_private_storage() {
    // It used to compile to a variable of its own, so a POU that wrote it wrote
    // a private copy no other POU could see, and read back only what it had
    // written itself. A VAR_GLOBAL is visible by name without the VAR_EXTERNAL
    // block, which is what such code means.
    let source = "\
VAR_GLOBAL Shared : DINT := 1; END_VAR

PROGRAM Main
VAR_EXTERNAL Shared : DINT; END_VAR
VAR Answer : DINT; END_VAR
  Shared := Shared + 10;
  Answer := Shared;
END_PROGRAM
";
    assert!(codes(source).contains(&"U0501"), "{:?}", codes(source));
    assert!(compile(source).compiled.is_none());
}

#[test]
fn assigning_a_whole_structure_through_a_subscript_is_refused() {
    // An element of an array of structures is a documented gap. What matters
    // is that it is refused rather than half-done.
    let source = "\
TYPE Point : STRUCT X : DINT; Y : DINT; END_STRUCT; END_TYPE

PROGRAM Main
VAR Route : ARRAY [1..2] OF Point; Here : Point; END_VAR
  Route[1] := Here;
END_PROGRAM
";
    let build = compile(source);
    assert!(
        build.diagnostics.has_errors(),
        "{}",
        build.render_diagnostics()
    );
    assert!(build.compiled.is_none());
}
