// SPDX-License-Identifier: Apache-2.0
//! A Structured Text program, a process image, and a Modbus device, joined up.
//!
//! This is the thing v0.2 was for. A program declares `Level AT %IW0`, a
//! project file says that word comes from a device's input register 0, and the
//! program reads what the device holds — over a real socket, with no hardware
//! and no vendor tool anywhere.
//!
//! The other half of the file is about what salman refuses to do: drive the
//! outputs of real equipment. That refusal is the reason to read this file.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use salman_core::posture::PostureState;
use salman_lang::dialect::Dialect;
use salman_link::link::{Link, LinkError, Peer, nth};
use salman_modbus::device::{BitTable, Device, WordTable};
use salman_modbus_net::client::Client;
use salman_modbus_net::server::{Server, ServerHandle};
use salman_project::spec::Project;
use salman_vm::clock::Clock;
use salman_vm::compile::IMAGE_BYTES as RUNTIME_IMAGE_BYTES;
use salman_vm::project::build_all;
use salman_vm::task::Runtime;

const IMAGE_BYTES: usize = 1024;

/// A program that reads a level from `%IW0` and drives a pump at `%QX0.0`.
const PROGRAM: &str = "\
PROGRAM Pumping
VAR
  Level AT %IW0  : WORD;
  Pump  AT %QX0.0 : BOOL;
  Alarm AT %QX0.1 : BOOL;
END_VAR
  Pump  := Level < 100;
  Alarm := Level > 900;
END_PROGRAM
";

const PROJECT: &str = "\
sources: [pumping.st]
devices:
  - name: tank
    protocol: modbus-tcp
    address: \"127.0.0.1:0\"
    unit: 1
    map:
      - { table: input-registers, from: 0, count: 1, to: \"%IW0\" }
      - { table: coils, from: 0, count: 2, to: \"%QX0.0\" }
";

fn simulating() -> PostureState {
    let mut posture = PostureState::new();
    posture.simulate();
    posture
}

fn tank() -> Device {
    Device::empty()
        .with_registers(WordTable::InputRegisters, 0, 10)
        .with_bits(BitTable::Coils, 0, 10)
}

fn simulator(device: Device) -> ServerHandle {
    Server::bind("127.0.0.1:0", device, &simulating(), 0)
        .unwrap()
        .spawn()
        .unwrap()
}

/// Builds the whole thing: a simulator, a link to it, and a runtime.
fn joined_up(handle: &ServerHandle, peer: Peer) -> (Link, Runtime) {
    let project = Project::parse(PROJECT, IMAGE_BYTES).expect("the project is valid");
    let device = &project.devices[0];
    let client = Client::connect(handle.address(), Duration::from_millis(500)).unwrap();
    let link = Link::new(
        &device.name,
        client,
        device.unit,
        peer,
        device.mappings.clone(),
        &simulating(),
        0,
    )
    .expect("a simulated peer may be written");

    let built = build_all(&[("pumping.st", PROGRAM)], &Dialect::generic()).unwrap();
    let compiled = built.compiled.expect("the program compiles");
    let runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    (link, runtime)
}

// -- the whole path ------------------------------------------------------

#[test]
fn a_program_reads_a_register_from_a_device_through_the_process_image() {
    let handle = simulator(tank());
    handle
        .with_device(|device| device.set_register(WordTable::InputRegisters, 0, 50))
        .unwrap();

    let (mut link, mut runtime) = joined_up(&handle, Peer::Simulated);

    // The scan: poll, latch, run, publish, write back.
    link.poll_inputs(runtime.memory_mut()).unwrap();
    runtime.run_scans(1);
    link.publish_outputs(runtime.memory()).unwrap();

    // A level of 50 is below 100, so the pump runs and the alarm does not.
    handle
        .with_device(|device| {
            assert_eq!(device.bit(BitTable::Coils, 0), Some(true), "the pump");
            assert_eq!(device.bit(BitTable::Coils, 1), Some(false), "the alarm");
        })
        .unwrap();
}

#[test]
fn a_change_at_the_device_reaches_the_program_on_the_next_scan() {
    let handle = simulator(tank());
    let (mut link, mut runtime) = joined_up(&handle, Peer::Simulated);

    for (level, pump, alarm) in [
        (50_u16, true, false),
        (500, false, false),
        (950, false, true),
    ] {
        handle
            .with_device(|device| device.set_register(WordTable::InputRegisters, 0, level))
            .unwrap();
        link.poll_inputs(runtime.memory_mut()).unwrap();
        runtime.run_scans(1);
        link.publish_outputs(runtime.memory()).unwrap();
        handle
            .with_device(|device| {
                assert_eq!(
                    device.bit(BitTable::Coils, 0),
                    Some(pump),
                    "pump at {level}"
                );
                assert_eq!(
                    device.bit(BitTable::Coils, 1),
                    Some(alarm),
                    "alarm at {level}"
                );
            })
            .unwrap();
    }
}

#[test]
fn an_input_that_changes_during_a_scan_is_not_seen_until_the_next_poll() {
    // The process image's whole reason for existing, now with a real device on
    // the other end. A program that saw an input change part way through its
    // own logic could take two decisions from two different pictures of the
    // world, and no controller works that way.
    let handle = simulator(tank());
    handle
        .with_device(|device| device.set_register(WordTable::InputRegisters, 0, 50))
        .unwrap();
    let (mut link, mut runtime) = joined_up(&handle, Peer::Simulated);

    link.poll_inputs(runtime.memory_mut()).unwrap();
    // The device changes after the poll and before the scan.
    handle
        .with_device(|device| device.set_register(WordTable::InputRegisters, 0, 950))
        .unwrap();
    runtime.run_scans(1);
    link.publish_outputs(runtime.memory()).unwrap();

    handle
        .with_device(|device| {
            assert_eq!(
                device.bit(BitTable::Coils, 0),
                Some(true),
                "the scan ran on the level it latched, which was 50"
            );
            assert_eq!(device.bit(BitTable::Coils, 1), Some(false));
        })
        .unwrap();

    // And on the next scan it catches up.
    link.poll_inputs(runtime.memory_mut()).unwrap();
    runtime.run_scans(1);
    link.publish_outputs(runtime.memory()).unwrap();
    handle
        .with_device(|device| {
            assert_eq!(device.bit(BitTable::Coils, 0), Some(false));
            assert_eq!(device.bit(BitTable::Coils, 1), Some(true));
        })
        .unwrap();
}

#[test]
fn several_registers_land_in_consecutive_image_words() {
    let handle = simulator(tank());
    handle
        .with_device(|device| {
            for address in 0..4_u16 {
                device.set_register(WordTable::InputRegisters, address, 0x1000 + address);
            }
        })
        .unwrap();

    let project = Project::parse(
        "sources: [a.st]\ndevices:\n  - name: tank\n    protocol: modbus-tcp\n    address: \"x\"\n    unit: 1\n    map:\n      - { table: input-registers, from: 0, count: 4, to: \"%IW0\" }\n",
        IMAGE_BYTES,
    )
    .unwrap();
    let client = Client::connect(handle.address(), Duration::from_millis(500)).unwrap();
    let mut link = Link::new(
        "tank",
        client,
        1,
        Peer::Simulated,
        project.devices[0].mappings.clone(),
        &simulating(),
        0,
    )
    .unwrap();

    let source = "PROGRAM P\nVAR R0 AT %IW0 : WORD; R1 AT %IW1 : WORD; R2 AT %IW2 : WORD; R3 AT %IW3 : WORD; END_VAR\n  ;\nEND_PROGRAM\n";
    let built = build_all(&[("t.st", source)], &Dialect::generic()).unwrap();
    let compiled = built.compiled.unwrap();
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    link.poll_inputs(runtime.memory_mut()).unwrap();
    runtime.run_scans(1);

    for word in 0..4_u16 {
        let address = salman_test::runner::parse_address_public(&format!("%IW{word}")).unwrap();
        assert_eq!(
            runtime.memory().read_address(&address).unwrap(),
            Some(salman_core::value::Value::Word(0x1000 + word)),
            "word {word}"
        );
    }
}

// -- what salman will not do ---------------------------------------------

#[test]
fn a_link_to_a_live_device_refuses_output_mappings_when_it_is_built() {
    // The refusal this crate exists to make. An engineering write is one
    // value, once, because a person decided to; a control loop writes every
    // scan for ever. A tool that asked once and then wrote ten thousand times
    // would have turned a per-call confirmation into a licence to drive a
    // plant, so salman does not do it at all.
    let handle = simulator(tank());
    let project = Project::parse(PROJECT, IMAGE_BYTES).unwrap();
    let client = Client::connect(handle.address(), Duration::from_millis(500)).unwrap();
    let error = Link::new(
        "tank",
        client,
        1,
        Peer::Live,
        project.devices[0].mappings.clone(),
        &simulating(),
        0,
    )
    .unwrap_err();

    assert!(
        matches!(error, LinkError::WouldDriveALiveDevice { .. }),
        "{error}"
    );
    let said = error.to_string();
    assert!(said.contains("no watchdog"), "{said}");
    assert!(
        said.contains("no setting that enables this"),
        "the message must say the refusal is categorical: {said}"
    );
}

#[test]
fn a_link_to_a_live_device_may_read_it() {
    // Reading real equipment is the entire point of a diagnostic tool. The
    // refusal is about driving outputs and nothing else.
    let handle = simulator(tank());
    handle
        .with_device(|device| device.set_register(WordTable::InputRegisters, 0, 77))
        .unwrap();
    let project = Project::parse(
        "sources: [a.st]\ndevices:\n  - name: tank\n    protocol: modbus-tcp\n    address: \"x\"\n    unit: 1\n    map:\n      - { table: input-registers, from: 0, count: 1, to: \"%IW0\" }\n",
        IMAGE_BYTES,
    )
    .unwrap();
    let client = Client::connect(handle.address(), Duration::from_millis(500)).unwrap();
    let mut link = Link::new(
        "tank",
        client,
        1,
        Peer::Live,
        project.devices[0].mappings.clone(),
        &simulating(),
        0,
    )
    .expect("reading a live device is allowed");
    assert_eq!(link.peer(), Peer::Live);

    let source = "PROGRAM P\nVAR Level AT %IW0 : WORD; END_VAR\n  ;\nEND_PROGRAM\n";
    let built = build_all(&[("t.st", source)], &Dialect::generic()).unwrap();
    let compiled = built.compiled.unwrap();
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );
    link.poll_inputs(runtime.memory_mut()).unwrap();
    runtime.run_scans(1);
    let address = salman_test::runner::parse_address_public("%IW0").unwrap();
    assert_eq!(
        runtime.memory().read_address(&address).unwrap(),
        Some(salman_core::value::Value::Word(77))
    );
}

#[test]
fn a_link_will_not_run_at_the_observing_posture() {
    let handle = simulator(tank());
    let project = Project::parse(PROJECT, IMAGE_BYTES).unwrap();
    let client = Client::connect(handle.address(), Duration::from_millis(500)).unwrap();
    let error = Link::new(
        "tank",
        client,
        1,
        Peer::Simulated,
        project.devices[0].mappings.clone(),
        &PostureState::new(),
        0,
    )
    .unwrap_err();
    assert!(matches!(error, LinkError::Refused { .. }), "{error}");
}

// -- address arithmetic --------------------------------------------------

#[test]
fn a_bit_address_advances_into_the_next_byte() {
    let first = salman_test::runner::parse_address_public("%QX0.6").unwrap();
    assert_eq!(nth(&first, 0).unwrap().to_string(), "%QX0.6");
    assert_eq!(nth(&first, 1).unwrap().to_string(), "%QX0.7");
    assert_eq!(nth(&first, 2).unwrap().to_string(), "%QX1.0");
    assert_eq!(nth(&first, 10).unwrap().to_string(), "%QX2.0");
}

#[test]
fn a_word_address_advances_a_word_at_a_time() {
    let first = salman_test::runner::parse_address_public("%IW3").unwrap();
    assert_eq!(nth(&first, 0).unwrap().to_string(), "%IW3");
    assert_eq!(nth(&first, 4).unwrap().to_string(), "%IW7");
}

#[test]
fn a_width_no_modbus_table_fills_has_no_run() {
    // %ID spans two registers and the word order is undefined, so there is no
    // honest answer to "what is the next one".
    let first = salman_test::runner::parse_address_public("%ID0").unwrap();
    assert!(nth(&first, 1).is_none());
}

#[test]
fn a_flat_bit_address_advances_the_way_the_process_image_reads_it() {
    // `%IX13` is bit 13, not byte 13, so the next item is bit 14 — which is
    // written `%IX1.6`. Advancing it as though it were byte 13 walked the run
    // off to bit 104 and away from wherever the program was reading.
    let first = salman_test::runner::parse_address_public("%IX13").unwrap();
    assert_eq!(nth(&first, 0).unwrap().to_string(), "%IX1.5");
    assert_eq!(nth(&first, 1).unwrap().to_string(), "%IX1.6");
    assert_eq!(nth(&first, 3).unwrap().to_string(), "%IX2.0");
}

#[test]
fn every_step_of_a_bit_run_lands_where_the_process_image_puts_it() {
    // The run and the image have to agree at every item, not only the first.
    use salman_vm::memory::{ImageLayout, ProcessImage};
    let image = ProcessImage::new(1024, ImageLayout::default());
    for start in ["%QX0.0", "%QX13", "%QX1.5", "%QX7"] {
        let first = salman_test::runner::parse_address_public(start).unwrap();
        let base = image.resolve(&first).unwrap();
        let base_bit = u64::from(base.byte) * 8 + u64::from(base.bit);
        for step in 0..20_u16 {
            let moved = nth(&first, step).unwrap();
            let at = image.resolve(&moved).unwrap();
            assert_eq!(
                u64::from(at.byte) * 8 + u64::from(at.bit),
                base_bit + u64::from(step),
                "{start} + {step} landed in the wrong place"
            );
        }
    }
}

#[test]
fn a_value_the_process_image_will_not_store_is_reported_rather_than_dropped() {
    // Found by review. `drive_input` says both whether the address resolved
    // and whether the value landed, and the second answer was discarded — so
    // a device could report a value, salman could fail to store it, and the
    // program would read whatever was there before with nothing anywhere
    // saying so.
    //
    // The case that provokes it: a mapping whose run walks past the end of the
    // process image. The project file checks that against the size it was
    // given; this checks it against the image the runtime actually has.
    let handle = simulator(
        Device::empty()
            .with_registers(WordTable::InputRegisters, 0, 200)
            .with_bits(BitTable::Coils, 0, 10),
    );
    let image_words = RUNTIME_IMAGE_BYTES / 2;
    let project = Project::parse(
        &format!(
            "sources: [a.st]\ndevices:\n  - name: tank\n    protocol: modbus-tcp\n    address: \"x\"\n    unit: 1\n    map:\n      - {{ table: input-registers, from: 0, count: 4, to: \"%IW{}\" }}\n",
            image_words - 2
        ),
        // Declared as twice the size it really is, which is how a mapping that
        // does not fit gets past the file check at all.
        RUNTIME_IMAGE_BYTES * 2,
    )
    .expect("the file check passes against the size it was told");

    let client = Client::connect(handle.address(), Duration::from_millis(500)).unwrap();
    let mut link = Link::new(
        "tank",
        client,
        1,
        Peer::Simulated,
        project.devices[0].mappings.clone(),
        &simulating(),
        0,
    )
    .unwrap();

    let built = build_all(
        &[(
            "t.st",
            "PROGRAM P\nVAR X : INT; END_VAR\n  X := 1;\nEND_PROGRAM\n",
        )],
        &Dialect::generic(),
    )
    .unwrap();
    let compiled = built.compiled.unwrap();
    let mut runtime = Runtime::new(
        compiled.program.clone(),
        compiled.memory.clone(),
        Clock::virtual_default(),
        compiled.tasks.clone(),
    );

    let error = link
        .poll_inputs(runtime.memory_mut())
        .expect_err("this mapping does not fit the image the runtime has");
    let said = error.to_string();
    assert!(
        said.contains("tank"),
        "the message must name the device: {said}"
    );
    assert!(
        matches!(
            error,
            LinkError::AddressRun { .. } | LinkError::StoreRefused { .. }
        ),
        "{error}"
    );
}
