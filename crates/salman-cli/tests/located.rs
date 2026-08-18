// SPDX-License-Identifier: Apache-2.0
//! Variables located in the process image, and driving them from a test.
//!
//! `AT %IX0.0` was refused for the whole of v0.1, because a located variable
//! given storage of its own is a variable that looks located and never changes
//! when the input does. It binds to the image now: it **is** the location, with
//! no slot and no copy.
//!
//! These tests also cover the half that makes the first half usable — a test
//! playing the part of a field device, driving `%I` and reading `%Q`, which is
//! what every protocol added after this will do through the same door.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_core::value::Value;
use salman_lang::dialect::Dialect;
use salman_vm::clock::Clock;
use salman_vm::project::build;
use salman_vm::task::Runtime;

/// A conveyor whose inputs and outputs are all in the process image.
const CONVEYOR: &str = "PROGRAM Conveyor\n\
     VAR\n\
       Start AT %IX0.0 : BOOL;\n\
       Stop  AT %IX0.1 : BOOL;\n\
       Motor AT %QX0.0 : BOOL;\n\
       Count AT %MW2   : UINT;\n\
       Latch : RS;\n\
     END_VAR\n\
       Latch(S := Start, R1 := Stop);\n\
       Motor := Latch.Q1;\n\
       IF Motor THEN Count := Count + 1; END_IF;\n\
     END_PROGRAM\n";

fn compiled(source: &str) -> salman_vm::compile::Compiled {
    let built = build("t.st", source, &Dialect::generic()).expect("not too large");
    let rendered = built.render_diagnostics();
    let Some(compiled) = built.compiled else {
        panic!("expected this to compile:\n{rendered}")
    };
    compiled
}

fn refused(source: &str) -> Vec<&'static str> {
    let built = build("t.st", source, &Dialect::generic()).expect("not too large");
    assert!(
        built.diagnostics.has_errors(),
        "expected a refusal:\n{source}"
    );
    built.diagnostics.items().iter().map(|d| d.code.0).collect()
}

// -- the binding itself --------------------------------------------------

#[test]
fn a_located_variable_gets_no_storage_of_its_own() {
    // The whole reason `AT` was refused. A slot would be a copy, and the watch
    // list would show a name whose value never moved.
    let program = compiled(CONVEYOR).program;
    for absent in ["Conveyor.Start", "Conveyor.Motor", "Conveyor.Count"] {
        assert!(
            program.slot_index(absent).is_none(),
            "{absent} has a slot; it should be its location instead. Slots: {:?}",
            program.slot_names
        );
    }
    // The function block instance inside the same POU still has slots.
    assert!(program.slot_index("Conveyor.Latch.Q1").is_some());
}

#[test]
fn a_located_output_follows_a_located_input_through_the_image() {
    let compiled = compiled(CONVEYOR);
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    let start = salman_test::runner::parse_address_public("%IX0.0").expect("an address");
    let motor = salman_test::runner::parse_address_public("%QX0.0").expect("an address");

    runtime.run_scans(1);
    assert_eq!(
        runtime.memory().read_address(&motor).unwrap(),
        Some(Value::Bool(false))
    );

    // The world raises the start input, as a device would.
    runtime
        .memory_mut()
        .drive_input(&start, &Value::Bool(true))
        .unwrap();
    runtime.run_scans(1);
    assert_eq!(
        runtime.memory().read_address(&motor).unwrap(),
        Some(Value::Bool(true))
    );
}

#[test]
fn an_input_driven_mid_scan_is_not_seen_until_the_next_latch() {
    // Driving the physical input is what a device does; the scan's snapshot is
    // what the program reads. Writing the image directly would let a test do
    // something no device can.
    let compiled = compiled(CONVEYOR);
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    let start = salman_test::runner::parse_address_public("%IX0.0").expect("an address");
    runtime
        .memory_mut()
        .drive_input(&start, &Value::Bool(true))
        .unwrap();

    // Before any scan the program has seen nothing.
    assert_eq!(
        runtime.memory().read_address(&start).unwrap(),
        Some(Value::Bool(false))
    );
    runtime.run_scans(1);
    assert_eq!(
        runtime.memory().read_address(&start).unwrap(),
        Some(Value::Bool(true))
    );
}

#[test]
fn bit_and_word_locations_overlay_each_other_as_they_do_on_a_controller() {
    let source = "PROGRAM P\n\
         VAR Whole AT %QW0 : UINT; Bit AT %QX1.0 : BOOL; END_VAR\n\
           Whole := 256;\n\
         END_PROGRAM\n";
    let compiled = compiled(source);
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    runtime.run_scans(1);
    let bit = salman_test::runner::parse_address_public("%QX1.0").expect("an address");
    // 256 is bit 0 of the second byte, and %QX1.0 names exactly that bit.
    assert_eq!(
        runtime.memory().read_address(&bit).unwrap(),
        Some(Value::Bool(true))
    );
}

// -- what is refused ------------------------------------------------------

#[test]
fn a_declaration_narrower_than_its_location_is_refused() {
    assert!(
        refused("PROGRAM P\nVAR B AT %IW4 : BOOL; END_VAR\n  ;\nEND_PROGRAM\n").contains(&"E0503")
    );
}

#[test]
fn a_declaration_wider_than_its_location_is_refused() {
    assert!(
        refused("PROGRAM P\nVAR W AT %IX0.0 : UINT; END_VAR\n  ;\nEND_PROGRAM\n")
            .contains(&"E0503")
    );
}

#[test]
fn every_size_letter_has_a_type_that_fits_it() {
    for (address, ty) in [
        ("%IX0.0", "BOOL"),
        ("%IB1", "BYTE"),
        ("%IW2", "WORD"),
        ("%ID3", "DWORD"),
        ("%IL4", "LWORD"),
    ] {
        let source = format!("PROGRAM P\nVAR V AT {address} : {ty}; END_VAR\n  ;\nEND_PROGRAM\n");
        let built = build("t.st", &source, &Dialect::generic()).expect("not too large");
        assert!(
            !built.diagnostics.has_errors(),
            "{address} : {ty} should fit:\n{}",
            built.render_diagnostics()
        );
    }
}

#[test]
fn a_program_may_not_write_its_own_inputs() {
    assert!(
        refused("PROGRAM P\nVAR S AT %IX0.0 : BOOL; END_VAR\n  S := TRUE;\nEND_PROGRAM\n")
            .contains(&"E0504")
    );
}

#[test]
fn a_location_past_the_end_of_the_image_is_refused() {
    assert!(
        refused("PROGRAM P\nVAR F AT %IW60000 : UINT; END_VAR\n  ;\nEND_PROGRAM\n")
            .contains(&"E0503")
    );
}

#[test]
fn a_partly_specified_location_is_refused() {
    // `%IW*` says the configuration will supply the location, and salman has no
    // configuration that does.
    assert!(
        refused("PROGRAM P\nVAR S AT %IW* : UINT; END_VAR\n  ;\nEND_PROGRAM\n").contains(&"E0503")
    );
}

// -- driving one from a declarative test ----------------------------------

#[test]
fn a_declarative_test_can_drive_and_read_a_location() {
    let compiled = compiled(CONVEYOR);
    let cases = salman_test::spec::parse(
        "- test: \"the motor follows the start input\"\n\
         \x20 pou: Conveyor\n\
         \x20 steps:\n\
         \x20   - { scans: 1, expect: { \"%QX0.0\": false } }\n\
         \x20   - { set: { \"%IX0.0\": true }, scans: 1, expect: { \"%QX0.0\": true } }\n\
         \x20   - { set: { \"%IX0.1\": true }, scans: 1, expect: { \"%QX0.0\": false } }\n",
    )
    .expect("the test file parses");
    let outcomes = salman_test::run_all(&compiled, &cases);
    let summary = salman_test::Summary::of(&outcomes);
    assert!(summary.is_ok(), "{}", salman_test::render_text(&outcomes));
}

#[test]
fn naming_a_variable_that_has_no_slot_says_to_write_the_location_instead() {
    let compiled = compiled(CONVEYOR);
    let cases = salman_test::spec::parse(
        "- test: t\n  pou: Conveyor\n  steps:\n    - { expect: { Motor: true } }\n",
    )
    .expect("the test file parses");
    let outcomes = salman_test::run_all(&compiled, &cases);
    let report = salman_test::render_text(&outcomes);
    assert!(report.contains("write the location"), "{report}");
}

#[test]
fn a_recorded_trace_can_name_a_location() {
    let compiled = compiled(CONVEYOR);
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    let motor = salman_test::runner::parse_address_public("%QX0.0").expect("an address");
    runtime.record(vec![salman_vm::trace::Signal::address(motor, "%QX0.0")]);
    runtime.run_scans(2);
    let trace = runtime.trace().expect("recording");
    assert_eq!(trace.len(), 2);
    assert!(trace.render().contains("%QX0.0"), "{}", trace.render());
}

// -- initial values ------------------------------------------------------

#[test]
fn an_initial_value_on_a_located_output_reaches_the_image() {
    // A located variable has no slot, so the ordinary initial-value path — set
    // the slot before the first scan — cannot carry it. Dropping it silently is
    // exactly the class of quiet loss the `AT` binding exists to stop.
    let compiled = compiled(
        "PROGRAM P\n\
         VAR Motor AT %QX0.0 : BOOL := TRUE; Level AT %MW2 : UINT := 7; END_VAR\n\
           ;\n\
         END_PROGRAM\n",
    );
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    runtime.run_scans(1);
    let motor = salman_test::runner::parse_address_public("%QX0.0").unwrap();
    let level = salman_test::runner::parse_address_public("%MW2").unwrap();
    assert_eq!(
        runtime.memory().read_address(&motor).unwrap(),
        Some(Value::Bool(true))
    );
    assert_eq!(
        runtime.memory().read_address(&level).unwrap(),
        Some(Value::Word(7))
    );
}

#[test]
fn a_cold_restart_puts_a_located_initial_value_back() {
    // A cold restart clears the image. Everything else that carries an initial
    // value is restored there, and a located variable that came back as zero
    // would be the same loss one restart later.
    let compiled = compiled(
        "PROGRAM P\n\
         VAR Level AT %MW2 : UINT := 7; END_VAR\n\
           Level := 99;\n\
         END_PROGRAM\n",
    );
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    let level = salman_test::runner::parse_address_public("%MW2").unwrap();
    runtime.run_scans(1);
    assert_eq!(
        runtime.memory().read_address(&level).unwrap(),
        Some(Value::Word(99))
    );
    runtime
        .memory_mut()
        .restart(salman_vm::memory::Restart::Cold);
    assert_eq!(
        runtime.memory().read_address(&level).unwrap(),
        Some(Value::Word(7)),
        "a cold restart restores the declared initial value"
    );
}

#[test]
fn an_initial_value_on_a_located_input_is_refused() {
    // Every scan begins by latching the physical inputs over the input image,
    // so this value would be gone before the first statement ran. Accepting it
    // would look like an assignment and behave like nothing.
    assert_eq!(
        refused(
            "PROGRAM P\n\
             VAR Sensor AT %IX0.0 : BOOL := TRUE; END_VAR\n\
               ;\n\
             END_PROGRAM\n"
        ),
        ["E0504"]
    );
}
