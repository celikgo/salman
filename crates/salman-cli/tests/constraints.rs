// SPDX-License-Identifier: Apache-2.0
//! Constraints a declaration makes, and salman keeps.
//!
//! Each of these covers something salman previously accepted and did not mean:
//! a subrange whose bounds were decoration, a `STRING[n]` whose length was
//! decoration, and an `EN` that named the standard's execution control and did
//! nothing. `docs/CONFORMANCE.md` named all three under "Where salman accepts
//! something and does not mean it" before they were implemented.
//!
//! Written through the public pipeline, so they read as behaviour rather than
//! as internals.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_core::time::Duration;
use salman_core::value::Value;
use salman_lang::dialect::Dialect;
use salman_vm::clock::Clock;
use salman_vm::exec::{ExecLimits, execute};
use salman_vm::memory::{Restart, SlotId};
use salman_vm::project::build;
use salman_vm::task::Runtime;

/// A compiled program, run for a number of scans.
struct Ran {
    runtime: Runtime,
    diagnostics: String,
}

impl Ran {
    fn get(&self, name: &str) -> Value {
        let found = self
            .runtime
            .program()
            .slot_names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name) || n.ends_with(&format!(".{name}")))
            .and_then(|i| u32::try_from(i).ok())
            .map(SlotId);
        let Some(slot) = found else {
            panic!(
                "no slot called {name}; program has {:?}",
                self.runtime.program().slot_names
            )
        };
        self.runtime
            .memory()
            .read_slot(slot)
            .cloned()
            .expect("slot exists")
    }

    fn faults(&self) -> String {
        self.runtime
            .faults()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn run(source: &str, scans: u64) -> Ran {
    let built = build("t.st", source, &Dialect::generic()).expect("not too large");
    let diagnostics = built.render_diagnostics();
    let compiled = built
        .compiled
        .unwrap_or_else(|| panic!("expected this to compile:\n{diagnostics}"));
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    runtime.run_scans(scans);
    Ran {
        runtime,
        diagnostics,
    }
}

fn refused(source: &str) -> String {
    let built = build("t.st", source, &Dialect::generic()).expect("not too large");
    assert!(
        built.diagnostics.has_errors(),
        "expected this to be refused:\n{source}"
    );
    built.render_diagnostics()
}

// ---------------------------------------------------------------------------
// A subrange means what it says
// ---------------------------------------------------------------------------

#[test]
fn a_subrange_bound_is_enforced_when_the_value_is_not_a_constant() {
    // The checker already refused `Level := 200;` because it could see the
    // constant. Assigning the same 200 through a variable used to succeed, and
    // the subrange held 200 — the declaration was decoration.
    let ran = run(
        "PROGRAM P\nVAR Level : INT (0..100); N : INT; END_VAR\n\
           N := 200;\n  Level := N;\nEND_PROGRAM\n",
        1,
    );
    let faults = ran.faults();
    assert!(
        faults.contains("Level was given 200, which its declared range 0..100 excludes"),
        "{faults}"
    );
}

#[test]
fn a_value_inside_the_declared_range_is_stored_without_complaint() {
    let ran = run(
        "PROGRAM P\nVAR Level : INT (0..100); N : INT; END_VAR\n\
           N := 42;\n  Level := N;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "", "{}", ran.diagnostics);
    assert_eq!(ran.get("Level"), Value::Int(42));
}

#[test]
fn both_ends_of_a_subrange_are_inclusive() {
    for value in [0, 100] {
        let ran = run(
            &format!(
                "PROGRAM P\nVAR Level : INT (0..100); N : INT; END_VAR\n\
                   N := {value};\n  Level := N;\nEND_PROGRAM\n"
            ),
            1,
        );
        assert_eq!(
            ran.faults(),
            "",
            "{value} is inside the range and must be accepted"
        );
    }
    for value in [-1, 101] {
        let ran = run(
            &format!(
                "PROGRAM P\nVAR Level : INT (0..100); N : INT; END_VAR\n\
                   N := {value};\n  Level := N;\nEND_PROGRAM\n"
            ),
            1,
        );
        assert!(
            !ran.faults().is_empty(),
            "{value} is outside the range and must be refused"
        );
    }
}

#[test]
fn a_subrange_is_enforced_when_it_is_a_function_parameter() {
    // The constraint belongs to the declaration, so it applies wherever a
    // value is stored into one — not only at an assignment statement.
    let ran = run(
        "FUNCTION Clamp : INT\nVAR_INPUT Level : INT (0..100); END_VAR\n\
           Clamp := Level;\nEND_FUNCTION\n\
         PROGRAM P\nVAR N : INT; R : INT; END_VAR\n  N := 500;\n\
           R := Clamp(Level := N);\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("declared range 0..100"),
        "{}",
        ran.faults()
    );
}

#[test]
fn a_subrange_is_enforced_on_a_function_block_input() {
    let ran = run(
        "FUNCTION_BLOCK Gauge\nVAR_INPUT Level : INT (0..100); END_VAR\n\
         VAR_OUTPUT Seen : INT; END_VAR\n  Seen := Level;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR G : Gauge; N : INT; END_VAR\n  N := 500;\n\
           G(Level := N);\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("declared range 0..100"),
        "{}",
        ran.faults()
    );
}

#[test]
fn a_named_subrange_type_carries_its_bounds_to_every_variable_of_that_type() {
    let ran = run(
        "TYPE Percent : INT (0..100); END_TYPE\n\
         PROGRAM P\nVAR A : Percent; N : INT; END_VAR\n  N := 200;\n  A := N;\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("declared range 0..100"),
        "{}",
        ran.faults()
    );
}

#[test]
fn a_for_loop_whose_constant_limit_leaves_the_control_variables_range_warns() {
    // It is very likely a mistake and salman says so, but it is not *certainly*
    // one — the loop below proves it — so this warns rather than refusing.
    let ran = run(
        "PROGRAM P\nVAR I : INT (0..3); Total : INT; END_VAR\n\
           FOR I := 0 TO 10 DO Total := Total + 1; IF I >= 3 THEN EXIT; END_IF; END_FOR;\n\
         END_PROGRAM\n",
        1,
    );
    assert!(ran.diagnostics.contains("W0302"), "{}", ran.diagnostics);
    assert!(
        ran.diagnostics.contains("`I` cannot hold (0..3)"),
        "the warning should name the variable and its range: {}",
        ran.diagnostics
    );
}

#[test]
fn a_for_loop_that_exits_before_leaving_its_range_is_correct_and_runs() {
    // This is the programme the old compile-time refusal rejected. Every value
    // the control variable takes is inside its declared range; the limit is
    // only ever compared against, never stored.
    let ran = run(
        "PROGRAM P\nVAR I : INT (0..3); Total : INT; END_VAR\n\
           FOR I := 0 TO 10 DO Total := Total + 1; IF I >= 3 THEN EXIT; END_IF; END_FOR;\n\
         END_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "", "a correct loop must not fault");
    assert_eq!(ran.get("Total"), Value::Int(4));
}

#[test]
fn the_same_loop_without_an_exit_faults_where_the_warning_said_it_would() {
    let ran = run(
        "PROGRAM P\nVAR I : INT (0..3); Total : INT; END_VAR\n\
           FOR I := 0 TO 10 DO Total := Total + 1; END_FOR;\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("declared range 0..3"),
        "{}",
        ran.faults()
    );
}

#[test]
fn a_descending_loop_over_a_non_negative_subrange_is_accepted() {
    // `BY -1` is a step, not a value of the control variable. Checking it
    // against the declared type made a descending loop over a subrange
    // unwritable.
    let ran = run(
        "PROGRAM P\nVAR I : INT (0..3); Total : INT; END_VAR\n\
           FOR I := 3 TO 0 BY -1 DO Total := Total + 1; END_FOR;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "");
    assert_eq!(ran.get("Total"), Value::Int(4));
}

#[test]
fn a_loop_counting_exactly_over_its_control_variables_range_runs() {
    // The value that ends a FOR is one past its end by construction, so a
    // naive range check on the increment made every ordinary loop over a
    // subrange impossible.
    let ran = run(
        "PROGRAM P\nVAR I : INT (0..3); Total : INT; END_VAR\n\
           FOR I := 0 TO 3 DO Total := Total + 1; END_FOR;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "");
    assert_eq!(ran.get("Total"), Value::Int(4));
}

#[test]
fn the_initial_value_of_a_for_loop_is_still_checked_against_the_declared_range() {
    // `FOR ... :=` is stored into the control variable immediately, so unlike
    // the limit and the step it keeps the declared type.
    let rendered = refused(
        "PROGRAM P\nVAR I : INT (0..3); Total : INT; END_VAR\n\
           FOR I := 9 TO 3 BY -1 DO Total := Total + 1; END_FOR;\nEND_PROGRAM\n",
    );
    assert!(rendered.contains("E0404"), "{rendered}");
}

// ---------------------------------------------------------------------------
// EN and ENO are reserved for the calling convention, and only there
// ---------------------------------------------------------------------------

#[test]
fn a_structure_field_may_be_called_en_or_eno() {
    // Deliberate asymmetry, not an oversight. A POU variable named EN is
    // refused because it would collide with the execution control at every
    // call to that POU. A structure is never callable, so `F.EN` is a member
    // access that can collide with nothing — and refusing it would invent a
    // restriction IEC 61131-3 does not have.
    let ran = run(
        "TYPE Flags : STRUCT EN : BOOL; ENO : BOOL; END_STRUCT; END_TYPE\n\
         PROGRAM P\nVAR F : Flags; Seen : BOOL; END_VAR\n\
           F.EN := TRUE;\n  Seen := F.EN;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "", "{}", ran.diagnostics);
    assert_eq!(ran.get("Seen"), Value::Bool(true));
}

#[test]
fn a_global_may_not_be_called_en_or_eno_either() {
    // A global is in scope inside every POU, so it would collide at a call
    // site exactly as a local would.
    for name in ["EN", "ENO"] {
        let rendered = refused(&format!(
            "VAR_GLOBAL {name} : BOOL; END_VAR\n\
             PROGRAM P\nVAR X : BOOL; END_VAR\n  X := FALSE;\nEND_PROGRAM\n"
        ));
        assert!(rendered.contains("E0324"), "{name}: {rendered}");
    }
}

#[test]
fn a_function_block_input_may_not_be_called_en_or_eno() {
    // This is the case the rule exists for: `FB(EN := x)` would otherwise mean
    // both "bind the input" and "decide whether to call".
    let rendered = refused(
        "FUNCTION_BLOCK FB\nVAR_INPUT EN : BOOL; END_VAR\n\
         VAR_OUTPUT Q : BOOL; END_VAR\n  Q := EN;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR B : FB; END_VAR\n  B(EN := TRUE);\nEND_PROGRAM\n",
    );
    assert!(rendered.contains("E0324"), "{rendered}");
}

#[test]
fn a_for_loop_that_steps_out_of_its_control_variables_range_at_run_time_is_caught() {
    // With the limit in a variable the checker cannot see it, so the bound has
    // to hold at run time. The increment writes the control variable, so the
    // declared range applies there too — this is the case a bound exists for.
    let ran = run(
        "PROGRAM P\nVAR I : INT (0..3); Limit : INT; Total : INT; END_VAR\n\
           Limit := 10;\n  FOR I := 0 TO Limit DO Total := Total + 1; END_FOR;\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("declared range 0..3"),
        "{}",
        ran.faults()
    );
}

#[test]
fn a_fault_names_the_variable_so_a_reader_knows_which_one() {
    let ran = run(
        "PROGRAM P\nVAR Pressure : INT (0..10); N : INT; END_VAR\n\
           N := 99;\n  Pressure := N;\nEND_PROGRAM\n",
        1,
    );
    assert!(ran.faults().contains("Pressure"), "{}", ran.faults());
}

// ---------------------------------------------------------------------------
// A string length means what it says
// ---------------------------------------------------------------------------

#[test]
fn assigning_a_longer_string_keeps_the_characters_that_fit() {
    // IEC 61131-3 gives the target the leading characters that fit rather than
    // refusing the assignment. Before this, a STRING[4] held ten characters and
    // reported them.
    let ran = run(
        "PROGRAM P\nVAR Short : STRING[4]; Long : STRING[20] := 'abcdefghij'; END_VAR\n\
           Short := Long;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("Short"), Value::string(b"abcd"));
}

#[test]
fn a_string_that_already_fits_is_copied_whole() {
    let ran = run(
        "PROGRAM P\nVAR Short : STRING[8]; Long : STRING[20] := 'abc'; END_VAR\n\
           Short := Long;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("Short"), Value::string(b"abc"));
}

#[test]
fn a_string_length_is_enforced_on_a_function_block_input() {
    let ran = run(
        "FUNCTION_BLOCK Label\nVAR_INPUT Text : STRING[3]; END_VAR\n\
         VAR_OUTPUT Seen : STRING[3]; END_VAR\n  Seen := Text;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR L : Label; Long : STRING[20] := 'abcdefghij'; END_VAR\n\
           L(Text := Long);\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("P.L.Seen"), Value::string(b"abc"));
}

// ---------------------------------------------------------------------------
// EN and ENO
// ---------------------------------------------------------------------------

/// A function block that accumulates, so that "did the call happen" is visible
/// in its state rather than only in a flag salman set.
const ACCUMULATOR: &str = "FUNCTION_BLOCK Acc\n\
     VAR_INPUT Add : INT; END_VAR\nVAR_OUTPUT Total : INT; END_VAR\n\
       Total := Total + Add;\nEND_FUNCTION_BLOCK\n";

#[test]
fn a_call_with_enable_false_does_not_happen_at_all() {
    let ran = run(
        &format!(
            "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; END_VAR\n\
               A(EN := FALSE, Add := 1);\nEND_PROGRAM\n"
        ),
        5,
    );
    assert_eq!(
        ran.get("Total"),
        Value::Int(0),
        "the block ran although EN was false"
    );
}

#[test]
fn a_call_with_enable_true_happens_normally() {
    let ran = run(
        &format!(
            "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; END_VAR\n\
               A(EN := TRUE, Add := 1);\nEND_PROGRAM\n"
        ),
        5,
    );
    assert_eq!(ran.get("Total"), Value::Int(5));
}

#[test]
fn enable_can_be_a_computed_condition_and_the_call_follows_it() {
    let ran = run(
        &format!(
            "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; Count : INT; END_VAR\n\
               A(EN := Count < 3, Add := 1);\n  Count := Count + 1;\nEND_PROGRAM\n"
        ),
        6,
    );
    assert_eq!(ran.get("Count"), Value::Int(6));
    assert_eq!(
        ran.get("Total"),
        Value::Int(3),
        "the block ran after its condition went false"
    );
}

#[test]
fn a_call_that_does_not_happen_does_not_write_its_inputs_either() {
    // The input binding is part of the call. If EN false still wrote `Add`,
    // the next enabled scan would use a value from a call that never ran.
    let ran = run(
        &format!(
            "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; END_VAR\n\
               A(EN := FALSE, Add := 7);\nEND_PROGRAM\n"
        ),
        1,
    );
    assert_eq!(ran.get("P.A.Add"), Value::Int(0));
}

#[test]
fn enable_out_reports_whether_the_call_happened() {
    let ran = run(
        &format!(
            "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; Ran : BOOL; Count : INT; END_VAR\n\
               A(EN := Count < 2, Add := 1, ENO => Ran);\n  Count := Count + 1;\nEND_PROGRAM\n"
        ),
        1,
    );
    assert_eq!(ran.get("Ran"), Value::Bool(true));

    let later = run(
        &format!(
            "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; Ran : BOOL; Count : INT; END_VAR\n\
               A(EN := Count < 2, Add := 1, ENO => Ran);\n  Count := Count + 1;\nEND_PROGRAM\n"
        ),
        4,
    );
    assert_eq!(later.get("Ran"), Value::Bool(false));
}

#[test]
fn enable_out_without_enable_is_always_true_because_the_call_always_happens() {
    let ran = run(
        &format!(
            "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; Ran : BOOL; END_VAR\n\
               A(Add := 1, ENO => Ran);\nEND_PROGRAM\n"
        ),
        1,
    );
    assert_eq!(ran.get("Ran"), Value::Bool(true));
    assert_eq!(ran.get("Total"), Value::Int(1));
}

#[test]
fn enable_works_on_a_standard_function_block_too() {
    let ran = run(
        "PROGRAM P\nVAR T : TON; Go : BOOL; END_VAR\n\
           T(EN := Go, IN := TRUE, PT := T#1ms);\nEND_PROGRAM\n",
        10,
    );
    assert_eq!(
        ran.get("P.T.IN"),
        Value::Bool(false),
        "a disabled TON must not be started"
    );
}

#[test]
fn a_variable_may_not_be_called_en_or_eno() {
    for name in ["EN", "ENO", "eno"] {
        let rendered = refused(&format!(
            "PROGRAM P\nVAR {name} : BOOL; END_VAR\n  ;\nEND_PROGRAM\n"
        ));
        assert!(rendered.contains("E0324"), "{name}: {rendered}");
        assert!(
            rendered.contains("calling convention"),
            "the message should say why: {rendered}"
        );
    }
}

#[test]
fn enable_written_as_an_output_is_reported_by_name() {
    // `F(ENO := ok)` looks plausible and does the opposite of what it says.
    let rendered = refused(&format!(
        "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; Ok : BOOL; END_VAR\n\
           A(ENO := Ok, Add := 1);\nEND_PROGRAM\n"
    ));
    assert!(
        rendered.contains("`ENO` is an output, not an input"),
        "{rendered}"
    );

    let other = refused(&format!(
        "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; Ok : BOOL; END_VAR\n\
           A(EN => Ok, Add := 1);\nEND_PROGRAM\n"
    ));
    assert!(other.contains("`EN` is an input, not an output"), "{other}");
}

#[test]
fn enable_must_be_a_boolean() {
    let rendered = refused(&format!(
        "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; N : INT; END_VAR\n\
           A(EN := N, Add := 1);\nEND_PROGRAM\n"
    ));
    assert!(
        rendered.contains("E04"),
        "a non-BOOL EN should be a type error: {rendered}"
    );
}

#[test]
fn enable_on_a_call_whose_result_is_used_is_refused_rather_than_invented() {
    // With EN false there is no call and therefore no result. salman will not
    // make one up.
    let rendered = refused(
        "FUNCTION F : INT\nVAR_INPUT N : INT; END_VAR\n  F := N;\nEND_FUNCTION\n\
         PROGRAM P\nVAR R : INT; END_VAR\n  R := F(EN := FALSE, N := 1);\nEND_PROGRAM\n",
    );
    assert!(rendered.contains("U0501"), "{rendered}");
    assert!(
        rendered.contains("no value when the call does not happen"),
        "{rendered}"
    );
}

// ---------------------------------------------------------------------------
// What a constrained variable starts at
//
// A declared initial value never reaches `Body::coerce`: it is written into the
// slot before the first scan. That makes the initial value the one place a
// constraint could be enforced everywhere else and still not hold, which is
// exactly the shape of gap these tests exist to close.
// ---------------------------------------------------------------------------

#[test]
fn a_subrange_variable_never_starts_at_a_value_its_own_declaration_excludes() {
    // The elementary default of INT is 0, and `INT (10..20)` excludes it. A
    // variable that starts outside its own declared range would fault on being
    // assigned the value it already holds, which is indefensible.
    let ran = run(
        "PROGRAM P\nVAR Level : INT (10..20); END_VAR\n  ;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("Level"), Value::Int(10));
}

#[test]
fn a_subrange_wholly_below_zero_starts_at_the_bound_nearest_zero() {
    let ran = run(
        "PROGRAM P\nVAR Offset : INT (-20..-10); END_VAR\n  ;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("Offset"), Value::Int(-10));
}

#[test]
fn a_subrange_that_holds_zero_still_starts_at_the_elementary_default() {
    // The policy changes as little as possible: where the declaration permits
    // the standard's initial value, that is what the variable gets.
    let ran = run(
        "PROGRAM P\nVAR Trim : INT (-5..5); END_VAR\n  ;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("Trim"), Value::Int(0));
}

#[test]
fn reading_a_subranges_initial_value_and_writing_it_straight_back_does_not_fault() {
    // This is the test that makes the initial value's legality mean something
    // rather than being a number in a table: the run-time check is asked about
    // the value the declaration produced.
    let ran = run(
        "PROGRAM P\nVAR Level : INT (10..20); N : INT; END_VAR\n\
           N := Level;\n  Level := N;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "");
    assert_eq!(ran.get("Level"), Value::Int(10));
}

#[test]
fn every_element_of_an_array_of_subranges_starts_inside_the_range() {
    let ran = run(
        "PROGRAM P\nVAR Levels : ARRAY [0..2] OF INT (50..100); END_VAR\n  ;\nEND_PROGRAM\n",
        1,
    );
    for index in 0..3 {
        assert_eq!(ran.get(&format!("Levels[{index}]")), Value::Int(50));
    }
}

#[test]
fn a_structure_field_of_subrange_type_starts_inside_its_range() {
    let ran = run(
        "TYPE Tank : STRUCT Level : INT (50..100); END_STRUCT; END_TYPE\n\
         PROGRAM P\nVAR T : Tank; END_VAR\n  ;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("P.T.level"), Value::Int(50));
}

#[test]
fn a_functions_local_subrange_starts_inside_its_range_on_every_call() {
    // A FUNCTION's frame is re-initialised on every call rather than read from
    // the load-time table, so it needs the policy applied a second time. It had
    // its own path, and its own way of missing it.
    let ran = run(
        "FUNCTION Read : INT\nVAR_INPUT Ignored : INT; END_VAR\n\
         VAR Level : INT (50..100); END_VAR\n  Read := Level;\nEND_FUNCTION\n\
         PROGRAM P\nVAR R : INT; END_VAR\n  R := Read(Ignored := 1);\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("R"), Value::Int(50));
}

#[test]
fn a_function_blocks_subrange_variable_starts_inside_its_range_in_every_instance() {
    let ran = run(
        "FUNCTION_BLOCK Gauge\nVAR Level : INT (50..100); END_VAR\n\
         VAR_OUTPUT Seen : INT; END_VAR\n  Seen := Level;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR A : Gauge; B : Gauge; END_VAR\n  A();\n  B();\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("P.A.Seen"), Value::Int(50));
    assert_eq!(ran.get("P.B.Seen"), Value::Int(50));
}

#[test]
fn a_typed_literal_initialiser_outside_the_declared_range_is_refused() {
    // `:= 50` was refused because the untyped literal is checked against the
    // type its context wants. `:= INT#50` said what it was, skipped that path
    // entirely, and started the variable at 50 in a range of 0..10.
    let rendered = refused("PROGRAM P\nVAR V : INT (0..10) := INT#50; END_VAR\n  ;\nEND_PROGRAM\n");
    assert!(rendered.contains("E0404"), "{rendered}");
    assert!(rendered.contains("0..10"), "{rendered}");
}

#[test]
fn a_folded_initialiser_is_judged_on_the_value_it_folds_to() {
    // `50 - 40` is 10, which the range holds. Judging the literals rather than
    // the value would refuse it for the 50.
    let ran = run(
        "PROGRAM P\nVAR V : INT (0..10) := 50 - 40; END_VAR\n  ;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("V"), Value::Int(10));
}

#[test]
fn an_initialiser_naming_a_constant_too_long_for_the_declared_string_is_refused() {
    // An assignment to a shorter string keeps what fits. A declaration is not
    // an assignment: it says how long the variable is, and an initial value
    // that contradicts it is a mistake rather than something to cut quietly.
    let rendered = refused(
        "PROGRAM P\nVAR CONSTANT Long : STRING[20] := 'abcdefghij'; END_VAR\n\
         VAR S : STRING[3] := Long; END_VAR\n  ;\nEND_PROGRAM\n",
    );
    assert!(rendered.contains("E0404"), "{rendered}");
    assert!(rendered.contains("holds 3"), "{rendered}");
}

// ---------------------------------------------------------------------------
// What shape of subrange works
// ---------------------------------------------------------------------------

#[test]
fn an_unsigned_subrange_is_enforced_like_a_signed_one() {
    let ran = run(
        "PROGRAM P\nVAR Digit : USINT (1..9); N : USINT; END_VAR\n\
           N := 200;\n  Digit := N;\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("declared range 1..9"),
        "{}",
        ran.faults()
    );
    assert_eq!(ran.get("Digit"), Value::Usint(1), "and it started in range");
}

#[test]
fn a_wholly_negative_subrange_is_enforced() {
    let ran = run(
        "PROGRAM P\nVAR Below : INT (-10..-1); N : INT; END_VAR\n\
           N := 5;\n  Below := N;\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("declared range -10..-1"),
        "{}",
        ran.faults()
    );
}

#[test]
fn a_subrange_of_one_value_accepts_that_value_and_refuses_its_neighbours() {
    let ran = run(
        "PROGRAM P\nVAR Fixed : INT (5..5); N : INT; END_VAR\n\
           N := 5;\n  Fixed := N;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "");
    assert_eq!(ran.get("Fixed"), Value::Int(5));

    for neighbour in [4, 6] {
        let ran = run(
            &format!(
                "PROGRAM P\nVAR Fixed : INT (5..5); N : INT; END_VAR\n\
                   N := {neighbour};\n  Fixed := N;\nEND_PROGRAM\n"
            ),
            1,
        );
        assert!(!ran.faults().is_empty(), "{neighbour} must be refused");
    }
}

#[test]
fn a_value_too_large_for_a_signed_sixty_four_bit_integer_is_still_a_range_violation() {
    // Reporting it as "this is not an integer salman can read" would send a
    // reader looking for a fault that is not there. Every declarable subrange
    // excludes it, which is what the message should say.
    let ran = run(
        "PROGRAM P\nVAR Small : ULINT (0..100); N : ULINT; END_VAR\n\
           N := ULINT#18446744073709551615;\n  Small := N;\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("declared range 0..100"),
        "{}",
        ran.faults()
    );
}

#[test]
fn a_narrowing_assignment_into_a_subrange_is_refused_rather_than_range_checked_after_truncation() {
    // `Level : SINT (0..100)` given a DINT of 300: 300 truncates to SINT as 44,
    // which the range holds. `coerce` converts before it checks, so if this
    // assignment were legal salman would store 44 and say nothing. It is not
    // legal — every implicit conversion salman performs widens — and that is
    // what makes checking after the conversion sound rather than lucky.
    let rendered = refused(
        "PROGRAM P\nVAR Level : SINT (0..100); N : DINT; END_VAR\n\
           N := 300;\n  Level := N;\nEND_PROGRAM\n",
    );
    assert!(rendered.contains("E0401"), "{rendered}");
    assert!(rendered.contains("narrowing"), "{rendered}");
}

#[test]
fn a_widening_conversion_before_the_range_check_cannot_change_its_verdict() {
    // The other half of the same argument: an INT of 300 widened to DINT is
    // still 300, so the check sees the value that is actually stored.
    let ran = run(
        "PROGRAM P\nVAR Level : DINT (0..100); N : INT; END_VAR\n\
           N := 300;\n  Level := N;\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("Level was given 300"),
        "{}",
        ran.faults()
    );
}

// ---------------------------------------------------------------------------
// An enumeration is a subrange in all but name
// ---------------------------------------------------------------------------

#[test]
fn an_enumeration_refuses_a_value_that_is_not_one_of_its_own() {
    let ran = run(
        "TYPE Colour : (Red, Green, Blue); END_TYPE\n\
         PROGRAM P\nVAR Shade : Colour; N : INT; END_VAR\n\
           N := 77;\n  Shade := N;\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults()
            .contains("Shade was given 77, which is not one of its declared values (0, 1, 2)"),
        "{}",
        ran.faults()
    );
}

#[test]
fn an_enumeration_is_a_set_and_not_a_range_so_a_gap_between_its_values_is_refused() {
    // A range check over 0..2 would accept 1. The values need not be
    // contiguous, and the check has to be membership rather than bounds.
    let ran = run(
        "TYPE Sparse : (Low := 0, High := 2); END_TYPE\n\
         PROGRAM P\nVAR S : Sparse; N : INT; END_VAR\n  N := 1;\n  S := N;\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults()
            .contains("not one of its declared values (0, 2)"),
        "{}",
        ran.faults()
    );
}

#[test]
fn an_enumeration_starts_at_its_first_declared_value() {
    let ran = run(
        "TYPE Mode : (Standby := 3, Running := 4); END_TYPE\n\
         PROGRAM P\nVAR M : Mode; END_VAR\n  ;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("M"), Value::Int(3));
}

#[test]
fn an_out_of_set_enumeration_constant_is_refused_where_the_checker_can_see_it() {
    let rendered = refused(
        "TYPE Colour : (Red, Green, Blue); END_TYPE\n\
         PROGRAM P\nVAR Shade : Colour; END_VAR\n  Shade := 77;\nEND_PROGRAM\n",
    );
    assert!(rendered.contains("E0404"), "{rendered}");
    assert!(rendered.contains("(0, 1, 2)"), "{rendered}");
}

#[test]
fn naming_an_enumeration_value_is_always_accepted_because_it_is_always_a_member() {
    let ran = run(
        "TYPE Colour : (Red, Green, Blue); END_TYPE\n\
         PROGRAM P\nVAR Shade : Colour; END_VAR\n  Shade := Colour#Blue;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "");
    assert_eq!(ran.get("Shade"), Value::Int(2));
}

// ---------------------------------------------------------------------------
// The one place that stores into a declared destination without `coerce`
// ---------------------------------------------------------------------------

#[test]
fn an_aggregate_copy_cannot_carry_a_value_its_element_type_excludes() {
    // `copy_wide` moves a structure slot by slot and never calls `coerce`. That
    // is sound only because both sides have the same declared type and every
    // other way a slot acquires a value is checked — the initial value by
    // `Compiler::declared_default`, every scalar store by `coerce`. This is the
    // test that holds the induction up: the copy produces a legal value, and
    // the value it produced survives a read-back through the checked path.
    let ran = run(
        "TYPE Tank : STRUCT Level : INT (50..100); END_STRUCT; END_TYPE\n\
         PROGRAM P\nVAR A : Tank; B : Tank; Seen : INT (50..100); END_VAR\n\
           A := B;\n  Seen := A.Level;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "");
    assert_eq!(ran.get("Seen"), Value::Int(50));
}

#[test]
fn an_output_bound_out_of_a_function_block_arrives_as_the_receiving_variables_type() {
    // The copy-back path took the source type from the destination, so `coerce`
    // compared a type with itself, emitted no conversion, and left an INT value
    // sitting in a DINT slot — a wrong type nobody sees until something reads
    // it.
    let ran = run(
        "FUNCTION_BLOCK Count\nVAR_INPUT N : INT; END_VAR\nVAR_OUTPUT Out : INT; END_VAR\n\
           Out := N;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR C : Count; Wide : DINT; END_VAR\n\
           C(N := 99, Out => Wide);\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("Wide"), Value::Dint(99));
}

// ---------------------------------------------------------------------------
// A FOR loop over exactly the range its control variable declares
// ---------------------------------------------------------------------------

#[test]
fn a_for_loop_over_exactly_its_control_variables_declared_range_runs_without_faulting() {
    // The value that ends a FOR loop is one past its end by construction. When
    // the loop's end is the control variable's own upper bound, checking the
    // incremented value after storing it made an ordinary loop impossible to
    // write.
    let ran = run(
        "PROGRAM P\nVAR I : INT (0..3); Total : INT; END_VAR\n\
           FOR I := 0 TO 3 DO Total := Total + 1; END_FOR;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "");
    assert_eq!(ran.get("Total"), Value::Int(4));
}

#[test]
fn a_for_loop_leaves_its_control_variable_at_the_last_value_the_body_saw() {
    // salman policy, and the direct consequence of testing a candidate before
    // it is allowed to reach the control variable.
    let ran = run(
        "PROGRAM P\nVAR I : INT; Total : INT; END_VAR\n\
           FOR I := 0 TO 3 DO Total := Total + 1; END_FOR;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("I"), Value::Int(3));
}

#[test]
fn a_descending_for_loop_over_a_subrange_that_excludes_its_step_still_compiles() {
    // `BY -1` is a step, not a value the control variable holds. Checking it
    // against the declared type refused every countdown over a non-negative
    // subrange.
    let ran = run(
        "PROGRAM P\nVAR I : INT (0..3); Total : INT; END_VAR\n\
           FOR I := 3 TO 0 BY -1 DO Total := Total + 1; END_FOR;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "");
    assert_eq!(ran.get("Total"), Value::Int(4));
}

// ---------------------------------------------------------------------------
// Every site that stores a value into a declared destination
//
// `Body::coerce` in `crates/salman-vm/src/compile.rs` calls itself "the single
// place a value becomes a value of a declared type". These enumerate the sites
// that store one and hold it to that, one site at a time: a subrange violation
// there faults, an over-long string there truncates, or the site moves a value
// between two destinations of one declared type and needs neither.
// ---------------------------------------------------------------------------

// -- an assignment through a subscript, and into a global -------------------

#[test]
fn a_subrange_bound_is_enforced_on_an_array_element() {
    let ran = run(
        "PROGRAM P\nVAR A : ARRAY [0..3] OF INT (0..10); N : INT; END_VAR\n\
           N := 99;\n  A[1] := N;\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("declared range 0..10"),
        "{}",
        ran.faults()
    );
}

#[test]
fn a_string_length_is_enforced_on_an_array_element() {
    let ran = run(
        "PROGRAM P\nVAR A : ARRAY [0..1] OF STRING[4]; L : STRING[20] := 'abcdefghij'; END_VAR\n\
           A[1] := L;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("P.A[1]"), Value::string(b"abcd"));
}

#[test]
fn a_subrange_bound_is_enforced_on_a_global() {
    // A global is written through a different instruction from a local, and
    // the declaration is the same promise either way.
    let ran = run(
        "VAR_GLOBAL G : INT (0..10); END_VAR\n\
         PROGRAM P\nVAR N : INT; END_VAR\n  N := 99;\n  G := N;\nEND_PROGRAM\n",
        1,
    );
    assert!(ran.faults().contains("G was given 99"), "{}", ran.faults());
}

// -- the wide-copy path ------------------------------------------------------

#[test]
fn a_whole_structure_is_copied_slot_by_slot_between_two_variables_of_one_type() {
    // The same type on both sides, so every slot of the source already
    // satisfies the constraint the destination declares. No coercion is needed
    // here, and none is emitted.
    let ran = run(
        "TYPE S : STRUCT Level : INT (0..10); END_STRUCT; END_TYPE\n\
         PROGRAM P\nVAR X : S; Y : S; N : INT; END_VAR\n\
           N := 7;\n  Y.Level := N;\n  X := Y;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "", "{}", ran.diagnostics);
    assert_eq!(ran.get("P.X.level"), Value::Int(7));
}

#[test]
fn a_whole_structure_passed_to_a_function_block_is_one_type_on_both_sides_too() {
    let ran = run(
        "TYPE S : STRUCT Level : INT (0..10); END_STRUCT; END_TYPE\n\
         FUNCTION_BLOCK Gauge\nVAR_INPUT V : S; END_VAR\nVAR_OUTPUT Seen : INT; END_VAR\n\
           Seen := V.Level;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR G : Gauge; X : S; N : INT; END_VAR\n\
           N := 3;\n  X.Level := N;\n  G(V := X);\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "", "{}", ran.diagnostics);
    assert_eq!(ran.get("P.G.Seen"), Value::Int(3));
}

// -- a function block output, bound out with `=>` ---------------------------

#[test]
fn a_function_block_checks_a_subrange_on_its_own_output_where_it_writes_it() {
    // Inside the block, writing VAR_OUTPUT is an ordinary assignment to a
    // declared variable, so the block's own declaration is what applies.
    let ran = run(
        "FUNCTION_BLOCK Gauge\nVAR_INPUT N : INT; END_VAR\nVAR_OUTPUT Level : INT (0..10); END_VAR\n\
           Level := N;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR G : Gauge; N : INT; END_VAR\n  N := 99;\n  G(N := N);\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("Level was given 99"),
        "{}",
        ran.faults()
    );
}

#[test]
fn an_output_binding_is_checked_against_the_callers_declaration_not_the_blocks() {
    // The value is going into a variable the caller declared, so it is the
    // caller's constraint that decides. The block's output is a plain INT and
    // has nothing to say about what the caller does with it.
    let ran = run(
        "FUNCTION_BLOCK Gauge\nVAR_INPUT N : INT; END_VAR\nVAR_OUTPUT Level : INT; END_VAR\n\
           Level := N;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR G : Gauge; N : INT; Tight : INT (0..10); END_VAR\n\
           N := 99;\n  G(N := N, Level => Tight);\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("Tight was given 99"),
        "{}",
        ran.faults()
    );
}

#[test]
fn an_output_bound_into_a_shorter_string_keeps_the_characters_that_fit() {
    let ran = run(
        "FUNCTION_BLOCK Label\nVAR_INPUT S : STRING[20]; END_VAR\n\
         VAR_OUTPUT Text : STRING[20]; END_VAR\n  Text := S;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR L : Label; Long : STRING[20] := 'abcdefghij'; Short : STRING[4]; END_VAR\n\
           L(S := Long, Text => Short);\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("Short"), Value::string(b"abcd"));
}

// -- VAR_IN_OUT, copied back ------------------------------------------------

#[test]
fn a_subrange_is_enforced_where_a_function_block_copies_a_var_in_out_back() {
    let ran = run(
        "FUNCTION_BLOCK Setter\nVAR_IN_OUT X : INT; END_VAR\n  X := 99;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR S : Setter; Tight : INT (0..10); END_VAR\n\
           S(X := Tight);\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("Tight was given 99"),
        "{}",
        ran.faults()
    );
}

#[test]
fn a_subrange_is_enforced_where_a_function_copies_a_var_in_out_back() {
    let ran = run(
        "FUNCTION Set : INT\nVAR_IN_OUT X : INT; END_VAR\n  X := 99;\n  Set := 0;\nEND_FUNCTION\n\
         PROGRAM P\nVAR Tight : INT (0..10); R : INT; END_VAR\n\
           R := Set(X := Tight);\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("Tight was given 99"),
        "{}",
        ran.faults()
    );
}

// -- a function argument, positional as well as named -----------------------

#[test]
fn a_subrange_is_enforced_on_a_positional_function_argument() {
    // The positional form fills the same parameters as the named one, so it
    // has to reach the same check.
    let ran = run(
        "FUNCTION Clamp : INT\nVAR_INPUT Level : INT (0..100); END_VAR\n\
           Clamp := Level;\nEND_FUNCTION\n\
         PROGRAM P\nVAR N : INT; R : INT; END_VAR\n  N := 500;\n  R := Clamp(N);\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("Level was given 500"),
        "{}",
        ran.faults()
    );
}

#[test]
fn a_string_length_is_enforced_on_a_function_parameter() {
    let ran = run(
        "FUNCTION Head : INT\nVAR_INPUT Text : STRING[3]; END_VAR\n  Head := 0;\nEND_FUNCTION\n\
         PROGRAM P\nVAR R : INT; Long : STRING[20] := 'abcdefghij'; END_VAR\n\
           R := Head(Long);\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("Head.Text"), Value::string(b"abc"));
}

// -- a function result, assigned through the function's own name ------------

#[test]
fn a_subrange_on_a_return_type_is_enforced_where_the_result_is_assigned() {
    let ran = run(
        "TYPE Percent : INT (0..100); END_TYPE\n\
         FUNCTION Scale : Percent\nVAR_INPUT N : INT; END_VAR\n  Scale := N;\nEND_FUNCTION\n\
         PROGRAM P\nVAR N : INT; R : INT; END_VAR\n  N := 500;\n  R := Scale(N);\nEND_PROGRAM\n",
        1,
    );
    assert!(
        ran.faults().contains("Scale was given 500"),
        "{}",
        ran.faults()
    );
}

#[test]
fn a_function_returning_a_short_string_returns_the_characters_that_fit() {
    let ran = run(
        "FUNCTION Head : STRING[3]\nVAR_INPUT Text : STRING[20]; END_VAR\n\
           Head := Text;\nEND_FUNCTION\n\
         PROGRAM P\nVAR R : STRING[20]; Long : STRING[20] := 'abcdefghij'; END_VAR\n\
           R := Head(Long);\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("R"), Value::string(b"abc"));
}

// -- sites that carry no declaration, and need no coercion ------------------

#[test]
fn a_case_selector_is_salmans_own_temporary_and_carries_no_constraint() {
    // The selector is evaluated once into a temporary that no declaration
    // names. Its value may leave the range of the variable it was computed
    // from without that meaning anything, because nothing declared it.
    let ran = run(
        "PROGRAM P\nVAR Level : INT (0..10); Offset : INT; Seen : INT; END_VAR\n\
           Offset := 100;\n\
           CASE Level + Offset OF 100: Seen := 1; ELSE Seen := 2; END_CASE;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.faults(), "", "{}", ran.diagnostics);
    assert_eq!(ran.get("Seen"), Value::Int(1));
}

#[test]
fn enable_out_is_a_bool_and_a_bool_carries_neither_a_range_nor_a_length() {
    // `ENO` is written straight into its target without a coercion, which is
    // sound only because the checker admits nothing but a BOOL there — and a
    // BOOL has no subrange to violate and no length to exceed.
    let rendered = refused(&format!(
        "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; N : INT; END_VAR\n\
           A(Add := 1, ENO => N);\nEND_PROGRAM\n"
    ));
    assert!(rendered.contains("E0401"), "{rendered}");
    assert!(rendered.contains("the output `ENO`"), "{rendered}");

    let ran = run(
        &format!(
            "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; Flags : ARRAY [0..1] OF BOOL; END_VAR\n\
               A(Add := 1, ENO => Flags[1]);\nEND_PROGRAM\n"
        ),
        1,
    );
    assert_eq!(ran.get("P.Flags[1]"), Value::Bool(true));
}

// -- a declared initial value ------------------------------------------------

#[test]
fn a_declared_initial_value_inside_a_function_block_is_refused_as_well() {
    // An initial value inside a function block belongs to every instance of
    // it, and is written by the layout rather than by any instruction. It
    // reaches memory by a different road and has to meet the same test.
    let rendered = refused(
        "FUNCTION_BLOCK Gauge\nVAR Level : INT (0..100) := INT#200; END_VAR\n\
           ;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR G : Gauge; END_VAR\n  G();\nEND_PROGRAM\n",
    );
    assert!(rendered.contains("E0404"), "{rendered}");
}

#[test]
fn a_declared_initial_value_of_a_global_is_refused_on_the_same_terms() {
    let rendered = refused(
        "VAR_GLOBAL Level : INT (0..100) := INT#200; END_VAR\n\
         PROGRAM P\n  ;\nEND_PROGRAM\n",
    );
    assert!(rendered.contains("E0404"), "{rendered}");
}

#[test]
fn one_initial_value_outside_its_range_is_reported_once() {
    // The literal reports itself, with a span on the literal. The check that
    // catches everything else must not say the same thing again.
    let built = build(
        "t.st",
        "PROGRAM P\nVAR Level : INT (0..100) := 200; END_VAR\n  ;\nEND_PROGRAM\n",
        &Dialect::generic(),
    )
    .expect("not too large");
    assert_eq!(built.diagnostics.error_count(), 1, "{}", {
        built.render_diagnostics()
    });
}

// ---------------------------------------------------------------------------
// What a declared string length counts
// ---------------------------------------------------------------------------

#[test]
fn a_wide_string_is_truncated_by_code_units_not_by_bytes() {
    let ran = run(
        "PROGRAM P\nVAR Short : WSTRING[3]; Long : WSTRING[20] := \"abcdefghij\"; END_VAR\n\
           Short := Long;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("Short"), Value::wstring([0x61u16, 0x62, 0x63]));
}

#[test]
fn truncating_a_wide_string_cuts_at_the_declared_count_even_through_a_surrogate_pair() {
    // salman policy. A `WSTRING` is held as 16-bit code units because salman
    // does not interpret its contents, and this is the one place a truncation
    // could start: dropping the orphaned lead surrogate would make `WSTRING[2]`
    // hold two code units for some values and one for others, which is a length
    // that depends on the data.
    let ran = run(
        "PROGRAM P\nVAR Short : WSTRING[2]; Long : WSTRING[20] := \"a$D83D$DE00\"; END_VAR\n\
           Short := Long;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(
        ran.get("Short"),
        Value::wstring([0x61u16, 0xD83D]),
        "two code units, the second of them half of a pair"
    );
}

#[test]
fn a_string_with_no_declared_length_is_truncated_at_the_dialect_default() {
    // `STRING` with no `[n]` takes the dialect's default length, and that
    // default is a real constraint rather than a note in a table.
    let long = "x".repeat(200);
    let ran = run(
        &format!(
            "PROGRAM P\nVAR Plain : STRING; Long : STRING[200] := '{long}'; END_VAR\n\
               Plain := Long;\nEND_PROGRAM\n"
        ),
        1,
    );
    let Value::String(bytes) = ran.get("Plain") else {
        panic!("Plain should be a STRING")
    };
    assert_eq!(bytes.len(), 80);
}

#[test]
fn a_string_of_exactly_the_declared_length_is_not_truncated() {
    // The off-by-one either way: four characters into a STRING[4] must arrive
    // whole, and five must lose exactly one.
    let ran = run(
        "PROGRAM P\nVAR Four : STRING[4]; Source : STRING[20] := 'abcd'; END_VAR\n\
           Four := Source;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("Four"), Value::string(b"abcd"));

    let over = run(
        "PROGRAM P\nVAR Four : STRING[4]; Source : STRING[20] := 'abcde'; END_VAR\n\
           Four := Source;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(over.get("Four"), Value::string(b"abcd"));
}

#[test]
fn a_declared_string_length_counts_the_bytes_salman_stores() {
    // salman policy: `STRING[n]` is n bytes. IEC `STRING` is a sequence of
    // single-byte characters whose encoding the system sets, and salman holds
    // one as bytes because real projects contain values that are not valid
    // UTF-8. A source character outside ASCII therefore occupies more than one
    // position, and it occupies the same number of positions in the checker as
    // it does at run time — which is the part that matters.
    let refusal = refused("PROGRAM P\nVAR Two : STRING[2] := 'aé'; END_VAR\n  ;\nEND_PROGRAM\n");
    assert!(
        refusal.contains("3 characters long"),
        "the two-character literal 'aé' is three bytes: {refusal}"
    );

    let ran = run(
        "PROGRAM P\nVAR Two : STRING[2]; Source : STRING[20] := 'aéb'; END_VAR\n\
           Two := Source;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(
        ran.get("Two"),
        Value::string([0x61u8, 0xC3]),
        "the cut is at two bytes, and salman does not re-encode what it stores"
    );
}

// ---------------------------------------------------------------------------
// EN and ENO: how the call is compiled, not only what it answers
// ---------------------------------------------------------------------------

/// A function that counts how often it has been called, through a `VAR_IN_OUT`.
const COUNTER: &str = "FUNCTION Bump : BOOL\n\
     VAR_IN_OUT Calls : INT; END_VAR\n  Calls := Calls + 1;\n  Bump := TRUE;\n\
     END_FUNCTION\n";

#[test]
fn enable_is_evaluated_exactly_once() {
    // An `EN` expression with a side effect must not run twice: the value is
    // computed, branched on, and never recomputed for the `ENO` arm.
    let ran = run(
        &format!(
            "{COUNTER}{ACCUMULATOR}PROGRAM P\nVAR A : Acc; Calls : INT; Ok : BOOL; END_VAR\n\
               A(EN := Bump(Calls := Calls), Add := 1, ENO => Ok);\nEND_PROGRAM\n"
        ),
        1,
    );
    assert_eq!(ran.get("P.Calls"), Value::Int(1));
    assert_eq!(ran.get("Ok"), Value::Bool(true));
}

#[test]
fn enable_works_inside_a_for_body() {
    // Jump patching is where a feature like this breaks. The EN branch's two
    // jumps and the loop's own must not be patched to each other's targets.
    let ran = run(
        &format!(
            "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; I : INT; Ran : BOOL; END_VAR\n\
               FOR I := 1 TO 4 DO A(EN := I < 3, Add := 1, ENO => Ran); END_FOR;\nEND_PROGRAM\n"
        ),
        1,
    );
    assert_eq!(ran.get("Total"), Value::Int(2), "enabled for I = 1 and 2");
    assert_eq!(ran.get("Ran"), Value::Bool(false), "and not for I = 4");
}

#[test]
fn enable_works_inside_a_case_arm() {
    let ran = run(
        &format!(
            "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; K : INT; Ran : BOOL; Reached : INT; END_VAR\n\
               K := 1;\n\
               CASE K OF\n\
                 1: A(EN := FALSE, Add := 1, ENO => Ran); Reached := 1;\n\
                 2: Reached := 2;\n\
               ELSE Reached := 9;\n\
               END_CASE;\nEND_PROGRAM\n"
        ),
        1,
    );
    assert_eq!(ran.get("Total"), Value::Int(0), "the call did not happen");
    assert_eq!(ran.get("Ran"), Value::Bool(false));
    assert_eq!(
        ran.get("Reached"),
        Value::Int(1),
        "and the arm carried on past the skipped call rather than falling out"
    );
}

#[test]
fn enable_works_when_its_own_expression_is_a_call() {
    let ran = run(
        &format!(
            "FUNCTION Positive : BOOL\nVAR_INPUT N : INT; END_VAR\n\
               Positive := N > 0;\nEND_FUNCTION\n\
             {ACCUMULATOR}PROGRAM P\nVAR A : Acc; K : INT; END_VAR\n\
               K := 1;\n  A(EN := Positive(N := K), Add := 5);\nEND_PROGRAM\n"
        ),
        1,
    );
    assert_eq!(ran.get("Total"), Value::Int(5));
}

#[test]
fn a_call_with_enable_false_does_not_copy_back_a_var_in_out() {
    // The copy-back is part of the call, so a call that does not happen leaves
    // the caller's variable exactly as it was.
    let ran = run(
        "FUNCTION_BLOCK Mutate\nVAR_IN_OUT X : INT; END_VAR\n  X := 99;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR M : Mutate; Held : INT; END_VAR\n\
           Held := 7;\n  M(EN := FALSE, X := Held);\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("Held"), Value::Int(7));
}

#[test]
fn a_call_with_enable_false_does_not_write_an_output_bound_with_an_arrow() {
    let ran = run(
        &format!(
            "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; Seen : INT; END_VAR\n\
               Seen := 42;\n  A(EN := FALSE, Add := 1, Total => Seen);\nEND_PROGRAM\n"
        ),
        1,
    );
    assert_eq!(ran.get("Seen"), Value::Int(42));
}

#[test]
fn a_disabled_timer_does_not_advance() {
    // A TON keeps its own state, so "the call did not happen" has to mean the
    // block's own step did not run either — not merely that its inputs were
    // left alone.
    let ran = run(
        "PROGRAM P\nVAR Delay : TON; Go : BOOL; Elapsed : TIME; END_VAR\n\
           Delay(EN := Go, IN := TRUE, PT := T#100ms, ET => Elapsed);\nEND_PROGRAM\n",
        20,
    );
    assert_eq!(ran.get("Elapsed"), Value::Time(Duration::ZERO));
    assert_eq!(ran.get("P.Delay.ET"), Value::Time(Duration::ZERO));
    assert_eq!(ran.get("P.Delay.Q"), Value::Bool(false));
}

#[test]
fn enable_written_as_a_literal_true_behaves_exactly_as_no_enable_does() {
    // salman does not fold it away. The point is that it does not need to: the
    // branch is taken every scan and the observable result is identical.
    let source = "PROGRAM P\nVAR A : Acc; END_VAR\n  A({en}Add := 1);\nEND_PROGRAM\n";
    let with = run(
        &format!("{ACCUMULATOR}{}", source.replace("{en}", "EN := TRUE, ")),
        5,
    );
    let without = run(&format!("{ACCUMULATOR}{}", source.replace("{en}", "")), 5);
    assert_eq!(with.get("Total"), Value::Int(5));
    assert_eq!(with.get("Total"), without.get("Total"));
}

#[test]
fn the_operand_stack_bound_of_a_call_with_enable_and_enable_out_is_exact() {
    // `Body::emit` tracks depth as it goes to compute `max_stack`. A branch
    // where one arm pushes and the other does not would leave that bound wrong
    // in one direction or the other, and nothing else would notice. This runs
    // the routine and compares the compiler's static bound against the depth
    // the interpreter actually reached.
    //
    // The callee is a block with an empty body, whose own routine uses no
    // operand stack at all: nested calls share one stack, so `max_stack` is a
    // bound on a routine's own frame rather than on the whole call, and a
    // callee that pushed anything would be measuring the callee.
    let source = "FUNCTION_BLOCK Nop\nVAR_INPUT Add : INT; END_VAR\n  ;\nEND_FUNCTION_BLOCK\n\
         PROGRAM P\nVAR A : Nop; Go : BOOL; Ran : BOOL; END_VAR\n\
           Go := TRUE;\n  A(EN := Go, Add := 1, ENO => Ran);\nEND_PROGRAM\n"
        .to_string();
    for enabled in [true, false] {
        let built = build("t.st", &source, &Dialect::generic()).expect("not too large");
        let compiled = built.compiled.expect("this compiles");
        let mut memory = compiled.memory.clone();
        let routine = compiled
            .program
            .routine_index("P")
            .expect("the program is a routine");
        let declared = compiled
            .program
            .routine(routine)
            .expect("it exists")
            .max_stack;
        if !enabled {
            let go = compiled.program.slot_index("P.Go").expect("Go has a slot");
            memory.set_initial(go, Value::Bool(false));
            memory.restart(Restart::Cold);
        }
        let executed = execute(
            &compiled.program,
            &mut memory,
            &Clock::virtual_default(),
            routine,
            0,
            ExecLimits::default(),
        )
        .expect("no fault");
        assert_eq!(
            u32::try_from(executed.peak_stack).expect("small"),
            declared,
            "with EN {enabled}: the compiler said {declared} and the interpreter reached {}",
            executed.peak_stack
        );
    }
}

#[test]
fn naming_enable_twice_in_one_call_is_refused() {
    // Binding it twice has no meaning, and the reading salman would otherwise
    // fall into — the last one wins — makes the first argument invisible. Here
    // that would skip a call a reader can plainly see enabled.
    let rendered = refused(&format!(
        "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; END_VAR\n\
           A(EN := TRUE, EN := FALSE, Add := 1);\nEND_PROGRAM\n"
    ));
    assert!(rendered.contains("E0325"), "{rendered}");
    assert!(
        rendered.contains("`EN` is given an argument twice"),
        "{rendered}"
    );
}

#[test]
fn naming_enable_out_twice_in_one_call_is_refused() {
    // Worse than EN: the first target was never written at all, so a variable
    // the engineer bound kept whatever it held before and nothing said so.
    let rendered = refused(&format!(
        "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; First : BOOL; Second : BOOL; END_VAR\n\
           A(Add := 1, ENO => First, ENO => Second);\nEND_PROGRAM\n"
    ));
    assert!(rendered.contains("E0325"), "{rendered}");
    assert!(
        rendered.contains("`ENO` is given an argument twice"),
        "{rendered}"
    );
}

#[test]
fn naming_any_parameter_twice_in_one_call_is_refused_on_the_same_terms() {
    // EN and ENO are not special here. They inherited a gap that every named
    // parameter had, and closing it for them alone would have been arbitrary.
    let block = refused(&format!(
        "{ACCUMULATOR}PROGRAM P\nVAR A : Acc; END_VAR\n\
           A(Add := 1, Add := 2);\nEND_PROGRAM\n"
    ));
    assert!(block.contains("E0325"), "{block}");

    let function = refused(
        "FUNCTION Twice : INT\nVAR_INPUT N : INT; END_VAR\n  Twice := N;\nEND_FUNCTION\n\
         PROGRAM P\nVAR R : INT; END_VAR\n  R := Twice(N := 1, N := 2);\nEND_PROGRAM\n",
    );
    assert!(function.contains("E0325"), "{function}");
}

#[test]
fn a_structure_field_may_still_be_called_en_because_a_structure_is_not_called() {
    // The reservation is on what a POU declares, because that is what could
    // shadow the execution control at a call site. A STRUCT has no call site.
    let ran = run(
        "TYPE Panel : STRUCT EN : BOOL; END_STRUCT; END_TYPE\n\
         PROGRAM P\nVAR Board : Panel; END_VAR\n  Board.EN := TRUE;\nEND_PROGRAM\n",
        1,
    );
    assert_eq!(ran.get("P.Board.en"), Value::Bool(true));
}

// ---------------------------------------------------------------------------
// Retained values across a restart
// ---------------------------------------------------------------------------

#[test]
fn a_retained_subrange_keeps_a_value_across_a_warm_restart_and_that_value_was_checked() {
    // A RETAIN variable is not re-checked when it is restored, and does not
    // need to be: the only way the program can put a value into it is through
    // `coerce`, and the only value it can start at is one `declared_default`
    // chose. A warm restart moves nothing across that boundary.
    let source = "PROGRAM P\nVAR RETAIN Level : INT (10..20); END_VAR\nVAR N : INT; END_VAR\n\
                    N := 15;\n  Level := N;\nEND_PROGRAM\n";
    let built = build("t.st", source, &Dialect::generic()).expect("not too large");
    let compiled = built.compiled.expect("this compiles");
    let slot = compiled
        .program
        .slot_index("P.Level")
        .expect("Level has a slot");

    let mut memory = compiled.memory.clone();
    assert_eq!(
        memory.read_slot(slot),
        Some(&Value::Int(10)),
        "it starts inside its range"
    );
    execute(
        &compiled.program,
        &mut memory,
        &Clock::virtual_default(),
        compiled.program.routine_index("P").expect("a routine"),
        0,
        ExecLimits::default(),
    )
    .expect("no fault");
    assert_eq!(memory.read_slot(slot), Some(&Value::Int(15)));

    memory.restart(Restart::Warm);
    assert_eq!(
        memory.read_slot(slot),
        Some(&Value::Int(15)),
        "a warm restart keeps it"
    );

    memory.restart(Restart::Cold);
    assert_eq!(
        memory.read_slot(slot),
        Some(&Value::Int(10)),
        "and a cold restart puts it back to a value its declaration permits, \
         not to the elementary default its range excludes"
    );
}
