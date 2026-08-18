// SPDX-License-Identifier: Apache-2.0
//! Reading a project file, and refusing one that says something impossible.
//!
//! Most of these tests are about refusal, and deliberately so. A mapping is
//! read once and then acts every scan against real equipment; a mistake in it
//! is not found by reading the file back, it is found by a motor running when
//! it should not. Every check here is one that would otherwise have to be
//! found that way.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_modbus::device::Table;
use salman_project::map::{Direction, Flow, Mapping, MappingError};
use salman_project::spec::{Project, ProjectError, Protocol};

/// The image size the runtime uses.
const IMAGE_BYTES: usize = 1024;

fn parse(text: &str) -> Result<Project, ProjectError> {
    Project::parse(text, IMAGE_BYTES)
}

fn problems(text: &str) -> Vec<String> {
    parse(text).unwrap_err().problems()
}

/// A one-device project whose `map:` holds the given entries.
///
/// Written as a helper rather than as a string in each test because YAML is
/// indentation-sensitive and a Rust line continuation eats leading spaces —
/// which produced a file missing every key after the first and a set of tests
/// that all failed for the same irrelevant reason.
fn with_map(entries: &[&str]) -> String {
    let mut text = String::from(
        "sources: [a.st]\ndevices:\n  - name: d\n    protocol: modbus-tcp\n    address: \"127.0.0.1:502\"\n    map:\n",
    );
    for entry in entries {
        text.push_str("      - ");
        text.push_str(entry);
        text.push('\n');
    }
    text
}

const WORKING: &str = "\
dialect: generic
sources:
  - conveyor.st
devices:
  - name: press
    protocol: modbus-tcp
    address: \"10.4.2.7:502\"
    unit: 1
    map:
      - { table: input-registers, from: 0, count: 4, to: \"%IW0\" }
      - { table: coils, from: 0, count: 8, to: \"%QX0.0\" }
";

// -- reading -------------------------------------------------------------

#[test]
fn a_working_project_reads() {
    let project = parse(WORKING).expect("this project is fine");
    assert_eq!(project.dialect, "generic");
    assert_eq!(project.sources, ["conveyor.st"]);
    assert_eq!(project.devices.len(), 1);

    let device = &project.devices[0];
    assert_eq!(device.name, "press");
    assert_eq!(device.protocol, Protocol::ModbusTcp);
    assert_eq!(device.address, "10.4.2.7:502");
    assert_eq!(device.unit, 1);
    assert_eq!(device.mappings.len(), 2);

    let registers = &device.mappings[0];
    assert_eq!(registers.table, Table::InputRegisters);
    assert_eq!(registers.device_start, 0);
    assert_eq!(registers.count, 4);
    assert_eq!(registers.direction().unwrap(), Direction::Input);

    let coils = &device.mappings[1];
    assert_eq!(coils.direction().unwrap(), Direction::Output);
}

#[test]
fn the_unit_defaults_to_the_one_the_guide_specifies_for_an_end_device() {
    // MG §4.4.1.2: 0xFF for a server that is the device itself. A project
    // going through a gateway carries a serial address and has to say so.
    let project = parse(
        "sources: [a.st]\ndevices:\n  - { name: d, protocol: modbus-tcp, address: \"127.0.0.1:502\" }\n",
    )
    .unwrap();
    assert_eq!(project.devices[0].unit, 0xFF);
}

#[test]
fn the_dialect_defaults_to_generic() {
    let project = parse("sources: [a.st]\n").unwrap();
    assert_eq!(project.dialect, "generic");
    assert!(project.devices.is_empty());
}

#[test]
fn a_project_may_have_no_devices_at_all() {
    // Everything salman did before v0.2 is this project: sources and nothing
    // else. It must stay expressible.
    let project = parse("sources: [a.st, b.st]\n").unwrap();
    assert_eq!(project.sources.len(), 2);
}

#[test]
fn mappings_can_be_walked_with_the_device_they_belong_to() {
    let project = parse(WORKING).unwrap();
    let found: Vec<_> = project
        .mappings()
        .map(|(device, mapping)| (device.name.as_str(), mapping.image.to_string()))
        .collect();
    assert_eq!(
        found,
        [("press", "%IW0".into()), ("press", "%QX0.0".into())]
    );
}

// -- refusals ------------------------------------------------------------

#[test]
fn a_misspelt_key_is_refused_rather_than_ignored() {
    // The whole reason for `deny_unknown_fields`. An ignored `mapp:` would
    // leave the device configured and unmapped, and the program would read
    // zeros from an input it believed was live.
    let text = with_map(&["{ table: coils, from: 0, count: 1, to: \"%QX0.0\" }"])
        .replace("    map:", "    mapp:");
    assert!(matches!(parse(&text), Err(ProjectError::Syntax(_))));
}

#[test]
fn a_project_with_no_sources_is_refused() {
    let found = problems("sources: []\n");
    assert!(
        found.iter().any(|p| p.contains("at least one source")),
        "{found:?}"
    );
}

#[test]
fn a_bit_table_mapped_to_a_word_address_is_refused() {
    // Eight coils are eight bits. Presenting them as words would need salman
    // to invent a packing, and any packing it invented would be wrong on some
    // device.
    let found = problems(&with_map(&[
        "{ table: coils, from: 0, count: 8, to: \"%QW0\" }",
    ]));
    assert!(
        found
            .iter()
            .any(|p| p.contains("coils") && p.contains("a bit at a time")),
        "{found:?}"
    );
}

#[test]
fn a_word_table_mapped_to_a_bit_address_is_refused() {
    let found = problems(&with_map(&[
        "{ table: holding-registers, from: 0, count: 2, to: \"%QX0.0\" }",
    ]));
    assert!(
        found.iter().any(|p| p.contains("holding registers")),
        "{found:?}"
    );
}

#[test]
fn writing_to_a_table_modbus_cannot_write_is_refused_when_the_file_is_read() {
    // There is no function code that writes a discrete input. A mapping that
    // asked for one would fail on the first scan against a live plant, which
    // is the worst possible moment to find out.
    let found = problems(&with_map(&[
        "{ table: discrete-inputs, from: 0, count: 4, to: \"%QX0.0\" }",
    ]));
    assert!(
        found
            .iter()
            .any(|p| p.contains("discrete inputs") && p.contains("cannot be written")),
        "{found:?}"
    );
}

#[test]
fn reading_a_read_only_table_into_the_input_image_is_fine() {
    // The direction that does work, asserted next to the one that does not.
    parse(&with_map(&[
        "{ table: discrete-inputs, from: 0, count: 4, to: \"%IX0.0\" }",
    ]))
    .expect("reading discrete inputs into %I is exactly what they are for");
}

#[test]
fn a_mapping_onto_the_marker_area_is_refused() {
    // %M is the program's own memory. A device mapped there would be read or
    // written at no defined point in the scan, and picking one would be an
    // invention rather than a decision.
    let found = problems(&with_map(&[
        "{ table: holding-registers, from: 0, count: 1, to: \"%MW0\" }",
    ]));
    assert!(
        found.iter().any(|p| p.contains("no direction")),
        "{found:?}"
    );
}

#[test]
fn two_mappings_that_claim_the_same_image_bits_are_refused() {
    // Whichever ran second would win, and which one that is would depend on
    // the order they happen to be written in the file.
    let found = problems(&with_map(&[
        "{ table: input-registers, from: 0, count: 4, to: \"%IW0\" }",
        "{ table: input-registers, from: 10, count: 2, to: \"%IW3\" }",
    ]));
    assert!(
        found.iter().any(|p| p.contains("claim the same")),
        "{found:?}"
    );
}

#[test]
fn two_devices_that_claim_the_same_image_bits_are_refused() {
    // Harder to spot than the single-device case, because each device's
    // section reads correctly on its own.
    let text = concat!(
        "sources: [a.st]\n",
        "devices:\n",
        "  - name: first\n",
        "    protocol: modbus-tcp\n",
        "    address: \"127.0.0.1:502\"\n",
        "    map: [{ table: input-registers, from: 0, count: 2, to: \"%IW0\" }]\n",
        "  - name: second\n",
        "    protocol: modbus-tcp\n",
        "    address: \"127.0.0.1:503\"\n",
        "    map: [{ table: input-registers, from: 0, count: 2, to: \"%IW1\" }]\n",
    );
    let found = problems(text);
    assert!(
        found
            .iter()
            .any(|p| p.contains("first") && p.contains("second")),
        "{found:?}"
    );
}

#[test]
fn mappings_that_sit_next_to_each_other_are_not_an_overlap() {
    // The boundary of the overlap check, in the direction that must succeed.
    parse(&with_map(&[
        "{ table: input-registers, from: 0, count: 4, to: \"%IW0\" }",
        "{ table: input-registers, from: 10, count: 2, to: \"%IW4\" }",
    ]))
    .expect("four words from word 0 end at word 3, so word 4 is free");
}

#[test]
fn two_devices_are_refused_if_they_share_a_name() {
    let found = problems(
        "sources: [a.st]\ndevices:\n  - { name: d, protocol: modbus-tcp, address: \"127.0.0.1:502\" }\n  - { name: d, protocol: modbus-tcp, address: \"127.0.0.1:503\" }\n",
    );
    assert!(found.iter().any(|p| p.contains("two devices")), "{found:?}");
}

#[test]
fn a_mapping_of_nothing_is_refused() {
    let found = problems(&with_map(&[
        "{ table: coils, from: 0, count: 0, to: \"%QX0.0\" }",
    ]));
    assert!(!found.is_empty(), "a mapping of zero items was accepted");
}

#[test]
fn a_mapping_that_runs_off_the_end_of_the_image_is_refused() {
    let found = problems(&with_map(&[
        "{ table: input-registers, from: 0, count: 125, to: \"%IW1000\" }",
    ]));
    assert!(
        found.iter().any(|p| p.contains("process image")),
        "{found:?}"
    );
}

#[test]
fn a_mapping_that_would_pass_the_top_of_the_device_address_space_is_refused() {
    let mapping = Mapping {
        table: Table::HoldingRegisters,
        device_start: 0xFFFE,
        count: 4,
        image: address("%IW0"),
    };
    assert_eq!(
        mapping.check(IMAGE_BYTES).unwrap_err(),
        MappingError::PastTheAddressSpace {
            start: 0xFFFE,
            count: 4,
        }
    );
}

#[test]
fn the_last_address_of_the_device_space_is_usable() {
    // The boundary in the direction that must work: one register at 0xFFFF.
    let mapping = Mapping {
        table: Table::HoldingRegisters,
        device_start: 0xFFFF,
        count: 1,
        image: address("%IW0"),
    };
    mapping
        .check(IMAGE_BYTES)
        .expect("the last register exists");
}

#[test]
fn an_address_that_is_not_an_address_is_refused_with_what_was_written() {
    let found = problems(&with_map(&[
        "{ table: coils, from: 0, count: 1, to: \"Motor\" }",
    ]));
    assert!(
        found.iter().any(|p| p.contains("Motor")),
        "the message should quote what was written: {found:?}"
    );
}

#[test]
fn a_double_word_address_is_refused_because_word_order_is_undefined() {
    // %ID spans two registers, and the specification defines byte order
    // *within* a register while saying nothing about the order of registers
    // within a wider value. Refusing to guess is the correct behaviour here,
    // not a gap: see docs/adr/ADR-0012-modbus-addressing.md.
    let found = problems(&with_map(&[
        "{ table: holding-registers, from: 0, count: 2, to: \"%ID0\" }",
    ]));
    assert!(found.iter().any(|p| p.contains("%IX or %IW")), "{found:?}");
}

#[test]
fn every_problem_in_a_file_is_reported_at_once() {
    // Three faults, one run. A reader that stopped at the first would make a
    // bad file take three edits to fix.
    let mut text = with_map(&[
        "{ table: coils, from: 0, count: 8, to: \"%QW0\" }",
        "{ table: discrete-inputs, from: 0, count: 4, to: \"%QX0.0\" }",
    ]);
    text = text.replace("sources: [a.st]", "sources: []");
    let found = problems(&text);
    assert!(found.len() >= 3, "expected several problems, got {found:?}");
}

// -- the model itself ----------------------------------------------------

fn address(written: &str) -> salman_lang::address::DirectAddress {
    use salman_core::span::SourceMap;
    use salman_lang::dialect::Dialect;
    use salman_lang::token::TokenKind;
    let mut sources = SourceMap::new();
    let file = sources.add("t", written).unwrap();
    let (stream, _) = salman_lang::lexer::lex(file, written, &Dialect::generic());
    match stream.tokens().first().map(|t| t.kind) {
        Some(TokenKind::DirectAddress(index)) => stream.address(index).unwrap().clone(),
        other => panic!("{written} lexed as {other:?}"),
    }
}

#[test]
fn a_word_mapping_occupies_sixteen_bits_per_register() {
    let mapping = Mapping {
        table: Table::InputRegisters,
        device_start: 0,
        count: 4,
        image: address("%IW2"),
    };
    // Word 2 begins at bit 32, and four registers are 64 bits.
    assert_eq!(mapping.image_bit_range().unwrap(), (32, 64));
}

#[test]
fn a_bit_mapping_occupies_one_bit_per_coil() {
    let mapping = Mapping {
        table: Table::Coils,
        device_start: 0,
        count: 8,
        image: address("%QX1.3"),
    };
    // Byte 1 bit 3 is bit 11, and eight coils are eight bits.
    assert_eq!(mapping.image_bit_range().unwrap(), (11, 8));
}

#[test]
fn a_bit_mapping_and_a_word_mapping_can_be_compared_for_overlap() {
    // The reason the range is in bits rather than bytes: %QX1.3 and %QW0 are
    // measured in different units and can still collide.
    let word = Mapping {
        table: Table::HoldingRegisters,
        device_start: 0,
        count: 1,
        image: address("%QW0"),
    };
    let bit = Mapping {
        table: Table::Coils,
        device_start: 0,
        count: 1,
        image: address("%QX1.3"),
    };
    assert!(
        word.overlaps(&bit).unwrap(),
        "word 0 covers bits 0 to 15, and %QX1.3 is bit 11"
    );

    let clear = Mapping {
        table: Table::Coils,
        device_start: 0,
        count: 1,
        image: address("%QX2.0"),
    };
    assert!(!word.overlaps(&clear).unwrap(), "bit 16 is past word 0");
}

#[test]
fn mappings_in_different_areas_never_overlap() {
    let input = Mapping {
        table: Table::InputRegisters,
        device_start: 0,
        count: 1,
        image: address("%IW0"),
    };
    let output = Mapping {
        table: Table::HoldingRegisters,
        device_start: 0,
        count: 1,
        image: address("%QW0"),
    };
    assert!(!input.overlaps(&output).unwrap());
}

#[test]
fn a_table_knows_how_it_is_addressed() {
    assert_eq!(Flow::of(Table::Coils), Flow::Bit);
    assert_eq!(Flow::of(Table::DiscreteInputs), Flow::Bit);
    assert_eq!(Flow::of(Table::HoldingRegisters), Flow::Word);
    assert_eq!(Flow::of(Table::InputRegisters), Flow::Word);
}
