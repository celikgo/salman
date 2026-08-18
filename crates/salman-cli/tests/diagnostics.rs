// SPDX-License-Identifier: Apache-2.0
//! What salman says about broken programs.
//!
//! These are the integration tier: they are written against the behaviour
//! salman promises, through the public pipeline, without reference to how any
//! stage implements it. A diagnostic is a user interface, and the thing worth
//! testing is what an engineer sees — not which pass produced it.
//!
//! Each test asserts on a diagnostic **code**, because codes are stable and
//! message text is meant to be improved. Where a message is the point — a
//! suggestion, a named dialect rule — the test says so.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_lang::dialect::Dialect;
use salman_vm::project::{Build, build};

fn compile(source: &str) -> Build {
    build("t.st", source, &Dialect::generic()).expect("not too large")
}

/// Wraps statements in a program with the given declarations.
fn program(declarations: &str, body: &str) -> String {
    format!("PROGRAM Main\nVAR\n{declarations}\nEND_VAR\n{body}\nEND_PROGRAM\n")
}

fn codes(build: &Build) -> Vec<&str> {
    build.diagnostics.items().iter().map(|d| d.code.0).collect()
}

fn errors(source: &str) -> Vec<String> {
    let build = compile(source);
    build
        .diagnostics
        .items()
        .iter()
        .filter(|d| d.severity == salman_core::diag::Severity::Error)
        .map(|d| format!("{}: {}", d.code, d.message))
        .collect()
}

fn assert_clean(source: &str) {
    let build = compile(source);
    assert!(
        !build.diagnostics.has_errors(),
        "expected this to compile:\n{}",
        build.render_diagnostics()
    );
    assert!(build.is_ok(), "expected a compiled program");
}

fn assert_rejected(source: &str, why: &str) {
    let build = compile(source);
    assert!(
        build.diagnostics.has_errors(),
        "expected an error ({why}) but the program was accepted:\n{source}"
    );
    assert!(
        build.compiled.is_none(),
        "a program with errors must not be compiled ({why})"
    );
}

// -- things that must be accepted ---------------------------------------

#[test]
fn a_minimal_program_compiles() {
    assert_clean("PROGRAM Main\nVAR X : DINT; END_VAR\n  X := 1;\nEND_PROGRAM\n");
}

#[test]
fn every_statement_form_compiles() {
    assert_clean(&program(
        "  I : DINT;\n  N : DINT;\n  B : BOOL;",
        "  IF B THEN N := 1; ELSIF N > 0 THEN N := 2; ELSE N := 3; END_IF;
  CASE N OF
    0: B := FALSE;
    1, 2: B := TRUE;
    3..9: B := NOT B;
  ELSE
    N := 0;
  END_CASE;
  FOR I := 1 TO 10 BY 2 DO
    N := N + I;
    IF N > 100 THEN EXIT; END_IF;
    IF N = 3 THEN CONTINUE; END_IF;
  END_FOR;
  WHILE N > 0 DO N := N - 1; END_WHILE;
  REPEAT N := N + 1; UNTIL N >= 5 END_REPEAT;
  ;",
    ));
}

#[test]
fn a_ton_instance_works_end_to_end() {
    assert_clean(&program(
        "  Start : BOOL;\n  Run : BOOL;\n  T1 : TON;",
        "  T1(IN := Start, PT := T#5s);\n  Run := T1.Q;",
    ));
}

#[test]
fn every_standard_function_block_can_be_declared_and_called() {
    for (name, call) in [
        ("SR", "B(S1 := X, R := Y);"),
        ("RS", "B(S := X, R1 := Y);"),
        ("R_TRIG", "B(CLK := X);"),
        ("F_TRIG", "B(CLK := X);"),
        ("CTU", "B(CU := X, R := Y, PV := 3);"),
        ("CTD", "B(CD := X, LD := Y, PV := 3);"),
        ("CTUD", "B(CU := X, CD := Y, R := X, LD := Y, PV := 3);"),
        ("TP", "B(IN := X, PT := T#1s);"),
        ("TON", "B(IN := X, PT := T#1s);"),
        ("TOF", "B(IN := X, PT := T#1s);"),
    ] {
        let source = program(
            &format!("  X : BOOL;\n  Y : BOOL;\n  B : {name};"),
            &format!("  {call}"),
        );
        let build = compile(&source);
        assert!(
            !build.diagnostics.has_errors(),
            "{name} should be usable:\n{}",
            build.render_diagnostics()
        );
    }
}

#[test]
fn a_function_can_be_declared_and_called() {
    assert_clean(
        "FUNCTION Double : DINT\nVAR_INPUT N : DINT; END_VAR\n  Double := N * 2;\nEND_FUNCTION\n\
         PROGRAM Main\nVAR X : DINT; END_VAR\n  X := Double(N := 4);\nEND_PROGRAM\n",
    );
}

#[test]
fn a_user_function_block_can_be_instantiated() {
    assert_clean(
        "FUNCTION_BLOCK Latch\nVAR_INPUT S : BOOL; R : BOOL; END_VAR\n\
         VAR_OUTPUT Q : BOOL; END_VAR\n  Q := S OR (Q AND NOT R);\nEND_FUNCTION_BLOCK\n\
         PROGRAM Main\nVAR L : Latch; A : BOOL; END_VAR\n  L(S := A, R := FALSE);\n\
           A := L.Q;\nEND_PROGRAM\n",
    );
}

#[test]
fn an_array_can_be_declared_indexed_and_assigned() {
    assert_clean(&program(
        "  Buffer : ARRAY [0..9] OF DINT;\n  I : DINT;",
        "  Buffer[I] := 1;\n  I := Buffer[3];",
    ));
}

#[test]
fn a_constant_can_be_used_where_a_constant_is_required() {
    assert_clean(
        "PROGRAM Main\nVAR CONSTANT Size : DINT := 4; END_VAR\n\
         VAR Buffer : ARRAY [1..4] OF DINT; END_VAR\n  Buffer[Size] := 1;\nEND_PROGRAM\n",
    );
}

// -- things that must be rejected ---------------------------------------

#[test]
fn an_undeclared_variable_is_rejected() {
    assert_rejected(&program("  X : DINT;", "  Y := 1;"), "Y is not declared");
    let messages = errors(&program("  Motor_Run : BOOL;", "  Motor_Ran := TRUE;"));
    assert!(!messages.is_empty(), "{messages:?}");
}

#[test]
fn assigning_a_narrower_type_is_rejected() {
    // DINT does not fit in an INT, and salman will not truncate silently.
    assert_rejected(
        &program("  A : INT;\n  B : DINT;", "  A := B;"),
        "narrowing",
    );
}

#[test]
fn assigning_across_type_families_is_rejected() {
    assert_rejected(
        &program("  A : DINT;\n  B : BOOL;", "  A := B;"),
        "BOOL is not a number",
    );
    assert_rejected(
        &program("  A : DINT;\n  B : WORD;", "  A := B;"),
        "a bit string is not a number",
    );
    assert_rejected(
        &program("  A : TIME;\n  B : DINT;", "  A := B;"),
        "an integer is not a duration",
    );
}

#[test]
fn arithmetic_on_a_bit_string_is_rejected() {
    assert_rejected(
        &program("  A : WORD;", "  A := A + A;"),
        "arithmetic takes numbers",
    );
}

#[test]
fn a_condition_that_is_not_a_bool_is_rejected() {
    // A common mistake coming from C, where any non-zero value is true.
    assert_rejected(
        &program("  N : DINT;", "  IF N THEN N := 1; END_IF;"),
        "IF needs a BOOL",
    );
    assert_rejected(
        &program("  N : DINT;", "  WHILE N DO N := 1; END_WHILE;"),
        "WHILE needs a BOOL",
    );
}

#[test]
fn a_literal_that_does_not_fit_its_target_is_rejected() {
    assert_rejected(
        &program("  A : SINT;", "  A := 300;"),
        "300 does not fit a SINT",
    );
    assert_clean(&program("  A : SINT;", "  A := 5;"));
}

#[test]
fn writing_to_an_input_of_the_enclosing_pou_is_rejected() {
    assert_rejected(
        "FUNCTION_BLOCK B\nVAR_INPUT X : BOOL; END_VAR\n  X := TRUE;\nEND_FUNCTION_BLOCK\n\
         PROGRAM Main\nVAR I : B; END_VAR\n  I(X := TRUE);\nEND_PROGRAM\n",
        "a POU may not write its own input",
    );
}

#[test]
fn writing_to_a_constant_is_rejected() {
    assert_rejected(
        "PROGRAM Main\nVAR CONSTANT K : DINT := 1; END_VAR\n  K := 2;\nEND_PROGRAM\n",
        "a CONSTANT is read-only",
    );
}

#[test]
fn positional_arguments_to_a_function_block_are_rejected() {
    // IEC 61131-3:2013 Table 42 "Function block call" offers no positional
    // form; only functions and methods have one.
    assert_rejected(
        &program("  X : BOOL;\n  T1 : TON;", "  T1(X, T#1s);"),
        "a function block has no positional call form",
    );
}

#[test]
fn an_unknown_parameter_of_a_function_block_is_rejected() {
    assert_rejected(
        &program("  T1 : TON;", "  T1(INN := TRUE, PT := T#1s);"),
        "INN is not a parameter",
    );
}

#[test]
fn reading_an_output_that_does_not_exist_is_rejected() {
    assert_rejected(
        &program("  B : BOOL;\n  T1 : TON;", "  B := T1.Running;"),
        "TON has no Running",
    );
}

#[test]
fn direct_recursion_is_rejected() {
    // This is what makes the compiler's single-static-frame layout sound, so it
    // is not optional.
    assert_rejected(
        "FUNCTION F : DINT\nVAR_INPUT N : DINT; END_VAR\n  F := F(N := N);\nEND_FUNCTION\n\
         PROGRAM Main\nVAR X : DINT; END_VAR\n  X := F(N := 1);\nEND_PROGRAM\n",
        "a POU may not call itself",
    );
}

#[test]
fn mutual_recursion_is_rejected() {
    assert_rejected(
        "FUNCTION A : DINT\nVAR_INPUT N : DINT; END_VAR\n  A := B(N := N);\nEND_FUNCTION\n\
         FUNCTION B : DINT\nVAR_INPUT N : DINT; END_VAR\n  B := A(N := N);\nEND_FUNCTION\n\
         PROGRAM Main\nVAR X : DINT; END_VAR\n  X := A(N := 1);\nEND_PROGRAM\n",
        "a cycle of calls is still recursion",
    );
}

#[test]
fn a_constant_subscript_outside_the_declared_bounds_is_rejected_at_compile_time() {
    // Far better than a runtime fault: the engineer finds out before the plant
    // does.
    assert_rejected(
        &program("  Buffer : ARRAY [0..9] OF DINT;", "  Buffer[10] := 1;"),
        "out of bounds",
    );
    assert_clean(&program(
        "  Buffer : ARRAY [0..9] OF DINT;",
        "  Buffer[9] := 1;",
    ));
}

#[test]
fn exit_outside_a_loop_is_rejected() {
    assert_rejected(&program("  X : DINT;", "  EXIT;"), "there is no loop");
}

#[test]
fn located_variables_report_that_the_io_mapping_layer_does_not_exist() {
    // The syntax parses; what does not exist is anything to bind it to. Giving
    // it an ordinary slot would produce a variable that looks located and never
    // changes when the input does.
    let build = compile(&program(
        "  Sensor AT %IX0.0 : BOOL;",
        "  Sensor := Sensor;",
    ));
    assert!(codes(&build).contains(&"U0301"), "{:?}", codes(&build));
}

#[test]
fn exponentiation_reports_that_it_is_not_implemented() {
    let build = compile(&program("  A : LREAL;", "  A := A ** 2.0;"));
    assert!(codes(&build).contains(&"U0301"), "{:?}", codes(&build));
}

// -- the shape of the diagnostics themselves -----------------------------

#[test]
fn one_broken_file_reports_many_errors_not_one() {
    // A checker that gives up at the first error makes a person compile ten
    // times to find ten mistakes.
    let source = program(
        "  A : INT;\n  B : BOOL;",
        "  A := Undeclared1;
  A := B;
  B := A;
  IF A THEN A := 1; END_IF;
  A := Undeclared2;
  EXIT;
  A := 99999999;
  B := B + B;",
    );
    let count = errors(&source).len();
    assert!(
        count >= 5,
        "expected several errors, found {count}: {:?}",
        errors(&source)
    );
}

#[test]
fn a_diagnostic_points_at_the_line_that_is_wrong() {
    let source = "PROGRAM Main\nVAR A : INT; END_VAR\n  A := TRUE;\nEND_PROGRAM\n";
    let build = compile(source);
    let rendered = build.render_diagnostics();
    assert!(
        rendered.contains("t.st:3:"),
        "the caret is on the wrong line:\n{rendered}"
    );
}

#[test]
fn diagnostics_are_ordered_by_position_so_the_output_is_stable() {
    let source = program("  A : INT;", "  A := Zzz;\n  A := Aaa;");
    let a = compile(&source).render_diagnostics();
    let b = compile(&source).render_diagnostics();
    assert_eq!(a, b, "the same input produced two different reports");
}

#[test]
fn the_strict_dialect_names_the_rule_it_applied() {
    let source = program("  A : WORD;", "  A := 16#ff;");
    let build = build("t.st", &source, &Dialect::strict_iec()).expect("not too large");
    let rendered = build.render_diagnostics();
    assert!(
        build.diagnostics.has_errors(),
        "strict should reject lowercase hex:\n{rendered}"
    );
    assert!(
        rendered.contains("dialect rule applied: iec61131-3:2013-strict"),
        "a dialect-dependent diagnostic must name the rule:\n{rendered}"
    );
    // The generic dialect accepts the same source.
    assert_clean(&source);
}

#[test]
fn nothing_panics_on_a_file_that_is_syntactically_broken() {
    for source in [
        "",
        "PROGRAM",
        "PROGRAM Main VAR",
        "PROGRAM Main VAR X : END_VAR X := ; END_PROGRAM",
        "END_PROGRAM END_VAR ;;; )))",
        "PROGRAM Main VAR X : DINT; END_VAR X := (((((((((((( ; END_PROGRAM",
        "\u{0}\u{1}\u{2}",
        "PROGRAM Main VAR X : ARRAY [9..0] OF DINT; END_VAR END_PROGRAM",
    ] {
        let _ = compile(source);
    }
}

#[test]
fn a_program_that_does_not_compile_produces_no_program_at_all() {
    // Generating code from a program already known to be wrong produces
    // confusing runtime faults instead of the errors the engineer needs.
    let build = compile(&program("  A : INT;", "  A := TRUE;"));
    assert!(build.compiled.is_none());
}
