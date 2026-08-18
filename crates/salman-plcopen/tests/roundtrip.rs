// SPDX-License-Identifier: Apache-2.0
//! Writing PLCopen XML, and what survives a round trip.
//!
//! A round trip is the only honest basis for a compatibility claim. "salman
//! imports PLCopen XML" means nothing without a statement of what comes back
//! out unchanged, and every tool in this space makes the claim while very few
//! publish the losses.
//!
//! These tests are that statement, in the only form that cannot go stale: they
//! run. What they establish is narrow and real — a document salman reads,
//! writes and reads again yields the same model — and it is deliberately not
//! dressed up as more. A **compatibility matrix** needs a corpus of real
//! vendor exports with clean provenance, which does not exist; see
//! `docs/adr/ADR-0003-plcopen-xml-canonical.md`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_plcopen::model::{
    Body, Interface, Pou, PouKind, Project, VarSection, Variable, Version,
};
use salman_plcopen::read::read;
use salman_plcopen::write::write;

/// A fixed timestamp, because an export that embedded "now" could not be
/// compared with anything.
const CREATED: &str = "2026-08-19T10:00:00";

fn project() -> Project {
    Project {
        version: Version::V2_01,
        company: "Some Vendor".to_string(),
        product: "Their IDE".to_string(),
        product_version: "1.2.3".to_string(),
        name: "Conveyor".to_string(),
        pous: vec![Pou {
            name: "Main".to_string(),
            kind: PouKind::Program,
            interface: Interface {
                return_type: None,
                sections: vec![
                    (
                        VarSection::Input,
                        vec![Variable {
                            name: "Start".to_string(),
                            type_name: "BOOL".to_string(),
                            address: None,
                            initial_value: None,
                        }],
                    ),
                    (
                        VarSection::Local,
                        vec![
                            Variable {
                                name: "Count".to_string(),
                                type_name: "INT".to_string(),
                                address: None,
                                initial_value: Some("7".to_string()),
                            },
                            Variable {
                                name: "Motor".to_string(),
                                type_name: "BOOL".to_string(),
                                address: Some("%QX0.0".to_string()),
                                initial_value: None,
                            },
                            Variable {
                                name: "Label".to_string(),
                                type_name: "STRING[32]".to_string(),
                                address: None,
                                initial_value: None,
                            },
                            Variable {
                                name: "Drive".to_string(),
                                type_name: "Axis".to_string(),
                                address: None,
                                initial_value: None,
                            },
                        ],
                    ),
                ],
            },
            bodies: vec![Body::StructuredText {
                text: "IF Start THEN Count := Count + 1; END_IF;".to_string(),
                wrapper: "xhtml".to_string(),
            }],
        }],
    }
}

fn to_xml(project: &Project) -> String {
    let mut out = Vec::new();
    write(project, CREATED, &mut out).expect("writing to a Vec cannot fail");
    String::from_utf8(out).expect("the writer emits UTF-8")
}

// -- the round trip ------------------------------------------------------

#[test]
fn a_project_written_and_read_back_is_the_same_project() {
    let original = project();
    let xml = to_xml(&original);
    let returned = read(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    assert_eq!(returned, original, "the round trip changed it:\n{xml}");
}

#[test]
fn a_document_read_written_and_read_again_is_the_same_document() {
    // The other direction: start from XML rather than from a model. This is
    // the shape a real import/export takes.
    let first = to_xml(&project());
    let model = read(first.as_bytes()).unwrap();
    let second = to_xml(&model);
    assert_eq!(first, second, "writing is not a fixed point");
    assert_eq!(read(second.as_bytes()).unwrap(), model);
}

#[test]
fn writing_is_deterministic() {
    // Two exports of one project must be byte-identical, or a round-trip test
    // compares noise and a compatibility matrix records the weather. It is why
    // the timestamp is an argument rather than a clock reading.
    assert_eq!(to_xml(&project()), to_xml(&project()));
}

// -- what the schema insists on ------------------------------------------

#[test]
fn the_four_required_children_are_written_in_the_order_the_schema_fixes() {
    let xml = to_xml(&project());
    let order = ["<fileHeader", "<contentHeader", "<types", "<instances"];
    let mut last = 0;
    for element in order {
        let at = xml
            .find(element)
            .unwrap_or_else(|| panic!("{element} is missing from:\n{xml}"));
        assert!(at > last, "{element} is out of order in:\n{xml}");
        last = at;
    }
    // And the two inside `types`, in their order.
    let types = xml.find("<types").unwrap();
    let data_types = xml.find("<dataTypes").unwrap();
    let pous = xml.find("<pous").unwrap();
    assert!(types < data_types && data_types < pous, "{xml}");
    assert!(xml.contains("<configurations"), "{xml}");
}

#[test]
fn the_coordinate_info_the_schema_requires_is_written_even_for_structured_text() {
    // The single commonest reason a hand-written PLCopen file fails
    // validation. It is required, it must carry all three graphical languages
    // with a scaling each, and it means nothing at all for a project that is
    // only Structured Text. salman writes it because the schema says so.
    let xml = to_xml(&project());
    assert!(xml.contains("<coordinateInfo"), "{xml}");
    for language in ["<fbd", "<ld", "<sfc"] {
        assert!(xml.contains(language), "{language} missing from:\n{xml}");
    }
    assert_eq!(xml.matches("<scaling").count(), 3, "{xml}");
}

#[test]
fn structured_text_is_written_inside_an_xhtml_element() {
    // `<ST>code</ST>` does not validate. The code has to sit inside exactly
    // one element from the XHTML namespace.
    let xml = to_xml(&project());
    assert!(
        xml.contains("xmlns:xhtml=\"http://www.w3.org/1999/xhtml\""),
        "{xml}"
    );
    assert!(xml.contains("<xhtml:xhtml>"), "{xml}");
    assert!(
        !xml.contains("<ST>IF Start"),
        "the code was written bare, which no conforming reader accepts:\n{xml}"
    );
}

#[test]
fn the_wrapper_a_document_came_with_is_written_back() {
    // A file from the Beremiz family goes back looking like the Beremiz
    // family. An exporter that normalised everything to its own preference
    // would make every round trip through salman a diff.
    let mut beremiz = project();
    beremiz.pous[0].bodies = vec![Body::StructuredText {
        text: "X := 1;".to_string(),
        wrapper: "p".to_string(),
    }];
    let xml = to_xml(&beremiz);
    assert!(xml.contains("<xhtml:p>"), "{xml}");
    assert_eq!(read(xml.as_bytes()).unwrap(), beremiz);
}

// -- types ---------------------------------------------------------------

#[test]
fn an_elementary_type_is_an_element_and_a_user_type_is_a_reference() {
    let xml = to_xml(&project());
    assert!(xml.contains("<BOOL"), "{xml}");
    assert!(xml.contains("<INT"), "{xml}");
    assert!(xml.contains(r#"<derived name="Axis""#), "{xml}");
}

#[test]
fn a_string_keeps_its_length_and_its_lower_case_element_name() {
    // The schema spells `string` and `wstring` in lower case, and an uppercase
    // `STRING` element fails validation while the lowercase one passes. It is
    // a trap worth having a test for.
    let xml = to_xml(&project());
    assert!(xml.contains(r#"<string length="32""#), "{xml}");
    assert!(!xml.contains("<STRING"), "{xml}");
}

#[test]
fn a_type_the_version_does_not_have_is_written_as_a_reference_rather_than_invented() {
    // v2.01's type set is frozen at IEC 61131-3 2nd edition, so LTIME is not
    // an element it has. Writing `<LTIME/>` would produce a document no
    // conforming reader accepts; writing it as a named type at least says what
    // was meant.
    let mut with_ltime = project();
    with_ltime.pous[0].interface.sections[1].1[0].type_name = "LTIME".to_string();
    let xml = to_xml(&with_ltime);
    assert!(xml.contains(r#"<derived name="LTIME""#), "{xml}");
    assert!(!xml.contains("<LTIME/>"), "{xml}");
}

// -- what does not survive, said plainly ---------------------------------

#[test]
fn a_body_salman_could_not_read_comes_back_empty_and_the_loss_is_reportable() {
    // salman did not read what was inside a ladder body, so it cannot write it
    // back. The element is kept so the shape of the document survives, and the
    // loss is on `unread_bodies` so a caller can tell a user — rather than an
    // export that looks complete and is not.
    let mut with_ladder = project();
    with_ladder.pous[0].bodies = vec![Body::Other {
        language: "LD".to_string(),
    }];
    let xml = to_xml(&with_ladder);
    assert!(xml.contains("<LD"), "{xml}");

    let returned = read(xml.as_bytes()).unwrap();
    assert_eq!(returned, with_ladder, "the shape survived");
    assert_eq!(
        returned.unread_bodies().collect::<Vec<_>>(),
        [("Main", "LD")],
        "and the loss is reportable"
    );
}

#[test]
fn a_v2_0_document_is_written_as_v2_01() {
    // salman modelled v2.01 and writes it. A file read as v2.0 comes back as
    // v2.01, and the version field says so rather than the export silently
    // claiming to be what it came from.
    let mut older = project();
    older.version = Version::V2_0;
    let xml = to_xml(&older);
    assert!(xml.contains("tc6_0201"), "{xml}");
    assert_eq!(read(xml.as_bytes()).unwrap().version, Version::V2_01);
}

// -- and it is still a document salman reads -----------------------------

#[test]
fn what_salman_writes_becomes_structured_text_that_compiles() {
    // The end-to-end claim: model to XML to model to Structured Text to a
    // compiled program. Anything less leaves "supports PLCopen XML" meaning
    // whatever the reader hopes.
    let xml = to_xml(&project());
    let returned = read(xml.as_bytes()).unwrap();
    let st = returned.to_structured_text();
    // `Axis` is a type this fixture references and does not declare, exactly
    // as a real library export does, so it is replaced here with one salman
    // knows — the point of this test is the path, not the fixture.
    let st = st.replace("Drive : Axis;", "Drive : BOOL;");
    let built = salman_vm::project::build(
        "roundtrip.st",
        &st,
        &salman_lang::dialect::Dialect::generic(),
    )
    .expect("not too large");
    assert!(
        built.compiled.is_some(),
        "the round trip produced something that does not compile:\n{st}\n{}",
        built.render_diagnostics()
    );
}
