// SPDX-License-Identifier: Apache-2.0
//! Reading PLCopen XML v2.01.
//!
//! The two tests that justify the module are
//! `both_families_of_structured_text_wrapper_are_read` and
//! `a_document_in_another_namespace_is_refused_by_name`. The first is because
//! the format under-specifies how Structured Text is stored and the ecosystem
//! has split into two mutually-unreadable camps as a result; a reader that
//! keys on the element name works against half of it. The second is because
//! IEC 61131-10 uses many of the same element names for a different format,
//! so matching on `project` alone would read one as the other.
//!
//! # Checked against a file salman did not write
//!
//! These tests build their own documents. The reader was additionally run over
//! `OopMotionControlLibrary.xml`, a genuine CODESYS V3.5 SP16 Patch 1 export
//! that PLCopen itself publishes — the only real vendor export it distributes.
//! salman read it correctly: it detected the v2.0 namespace and reported that
//! this is not the version salman modelled, read the CODESYS product string,
//! found all three programs, and found every body wrapped in `<xhtml>`, which
//! is exactly the CODESYS family the format's under-specification produced.
//!
//! The Structured Text it yields does not compile, and the reason is the right
//! one: all 54 diagnostics are unknown names, and every unknown type — `Axis`,
//! `CamTable`, `MC_CAM_REF`, `itfAxis`, `itfCommand`,
//! `itfSynchronizedAxisCommand` — belongs to the motion library that an
//! *interface library template* references and does not contain. The file is
//! not committed here: it is PLCopen's, and salman has established no right to
//! redistribute it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_plcopen::model::{Body, PouKind, VarSection, Version};
use salman_plcopen::read::{ReadError, read};

/// A document with one program, wrapped the way `wrapper` says.
fn document(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<project xmlns="http://www.plcopen.org/xml/tc6_0201">
  <fileHeader companyName="Some Vendor" productName="Their IDE"
              productVersion="1.2.3" creationDateTime="2026-08-19T10:00:00"/>
  <contentHeader name="Conveyor">
    <coordinateInfo>
      <fbd><scaling x="1" y="1"/></fbd>
      <ld><scaling x="1" y="1"/></ld>
      <sfc><scaling x="1" y="1"/></sfc>
    </coordinateInfo>
  </contentHeader>
  <types>
    <dataTypes/>
    <pous>
      <pou name="Main" pouType="program">
        <interface>
          <inputVars>
            <variable name="Start"><type><BOOL/></type></variable>
          </inputVars>
          <localVars>
            <variable name="Count">
              <type><INT/></type>
              <initialValue><simpleValue value="7"/></initialValue>
            </variable>
            <variable name="Motor" address="%QX0.0"><type><BOOL/></type></variable>
          </localVars>
        </interface>
        <body>
          {body}
        </body>
      </pou>
    </pous>
  </types>
  <instances><configurations/></instances>
</project>
"#
    )
}

/// What the CODESYS family writes.
const CODESYS_ST: &str = r#"<ST><xhtml xmlns="http://www.w3.org/1999/xhtml">IF Start THEN Count := Count + 1; END_IF;</xhtml></ST>"#;

/// What the Beremiz family writes.
const BEREMIZ_ST: &str = r#"<ST><xhtml:p xmlns:xhtml="http://www.w3.org/1999/xhtml"><![CDATA[IF Start THEN Count := Count + 1; END_IF;]]></xhtml:p></ST>"#;

// -- the split ecosystem -------------------------------------------------

#[test]
fn both_families_of_structured_text_wrapper_are_read() {
    // The specification constrains the namespace and not the element name, has
    // no worked ST example anywhere in its eighty numbered pages, and imports
    // no XHTML schema. Both of these validate against it, and a reader that
    // keyed on the name would fail against half the tools in use.
    for (family, body, expected_wrapper) in [
        ("CODESYS lineage", CODESYS_ST, "xhtml"),
        ("Beremiz lineage", BEREMIZ_ST, "p"),
    ] {
        let project = read(document(body).as_bytes()).unwrap_or_else(|e| panic!("{family}: {e}"));
        assert_eq!(project.pous.len(), 1, "{family}");
        let Body::StructuredText { text, wrapper } = &project.pous[0].bodies[0] else {
            panic!("{family}: not read as Structured Text")
        };
        assert_eq!(
            text.trim(),
            "IF Start THEN Count := Count + 1; END_IF;",
            "{family}"
        );
        assert_eq!(
            wrapper, expected_wrapper,
            "{family}: the wrapper is remembered so an export can write it back"
        );
    }
}

#[test]
fn structured_text_that_is_not_wrapped_at_all_is_refused() {
    // `<ST>a := TRUE;</ST>` does not validate against the schema, and reading
    // it anyway would mean accepting a document no conforming tool produces
    // while quietly disagreeing with every one that does.
    let error = read(document("<ST>Count := 1;</ST>").as_bytes()).unwrap_err();
    let ReadError::StructuredTextNotWrapped { pou, found } = &error else {
        panic!("{error}")
    };
    assert_eq!(pou, "Main");
    assert!(found.contains("Count := 1;"), "{found}");
    // And the message explains the rule, because nobody has read that clause.
    assert!(error.to_string().contains("formatted text"), "{error}");
}

// -- namespaces ----------------------------------------------------------

#[test]
fn a_document_in_another_namespace_is_refused_by_name() {
    // IEC 61131-10 uses many of the same element names for a different format.
    // A reader matching on `project` alone would read one as the other and
    // produce something plausible from a document it does not understand.
    let iec = r#"<?xml version="1.0"?>
<Project xmlns="www.iec.ch/public/TC65SC65BWG7TF10" schemaVersion="1.0"/>"#;
    let error = read(iec.as_bytes()).unwrap_err();
    // The root is `Project`, not `project`, so it fails on the name first —
    // which is itself one of the differences between the two formats.
    assert!(
        matches!(
            error,
            ReadError::NotAPlcopenProject { .. } | ReadError::WrongNamespace { .. }
        ),
        "{error}"
    );

    // And a document with the right root in a namespace salman does not read
    // names the namespace and says what that probably means. v1.01 is a real
    // earlier release; salman reads v2.0 and v2.01 and nothing older.
    let wrong = r#"<?xml version="1.0"?>
<project xmlns="http://www.plcopen.org/xml/tc6_0101"/>"#;
    let error = read(wrong.as_bytes()).unwrap_err();
    let ReadError::WrongNamespace { found, .. } = &error else {
        panic!("{error}")
    };
    assert_eq!(found, "http://www.plcopen.org/xml/tc6_0101");
    assert!(error.to_string().contains("different format"), "{error}");
}

#[test]
fn something_that_is_not_xml_says_so() {
    assert!(matches!(
        read("this is not xml".as_bytes()),
        Err(ReadError::Xml { .. })
    ));
}

#[test]
fn a_document_with_no_root_at_all_is_refused() {
    assert!(read(r#"<?xml version="1.0"?>"#.as_bytes()).is_err());
}

// -- what a document says ------------------------------------------------

#[test]
fn the_file_header_says_what_wrote_it() {
    // Worth keeping: which tool produced an export is the first thing anybody
    // asks when it does not import cleanly.
    let project = read(document(CODESYS_ST).as_bytes()).unwrap();
    assert_eq!(project.company, "Some Vendor");
    assert_eq!(project.product, "Their IDE");
    assert_eq!(project.product_version, "1.2.3");
    assert_eq!(project.name, "Conveyor");
}

#[test]
fn declarations_keep_their_sections_their_order_and_their_details() {
    let project = read(document(CODESYS_ST).as_bytes()).unwrap();
    let interface = &project.pous[0].interface;
    assert_eq!(interface.sections.len(), 2);

    let (section, variables) = &interface.sections[0];
    assert_eq!(*section, VarSection::Input);
    assert_eq!(variables[0].name, "Start");
    assert_eq!(variables[0].type_name, "BOOL");

    let (section, variables) = &interface.sections[1];
    assert_eq!(*section, VarSection::Local);
    assert_eq!(variables[0].name, "Count");
    assert_eq!(variables[0].type_name, "INT");
    assert_eq!(variables[0].initial_value.as_deref(), Some("7"));
    assert_eq!(variables[1].name, "Motor");
    assert_eq!(variables[1].address.as_deref(), Some("%QX0.0"));
}

#[test]
fn a_pou_kind_the_version_does_not_have_is_refused_with_what_it_permits() {
    // v2.01 has exactly three. `class`, `interface` and `method` arrived with
    // IEC 61131-10, which is a different format.
    let text = document(CODESYS_ST).replace(r#"pouType="program""#, r#"pouType="class""#);
    let error = read(text.as_bytes()).unwrap_err();
    assert!(error.to_string().contains("functionBlock"), "{error}");
}

// -- what salman cannot read --------------------------------------------

#[test]
fn a_body_in_a_language_salman_does_not_read_is_named_rather_than_dropped() {
    // A file that quietly lost half its program would compile and be wrong,
    // which is the failure this whole layer exists to prevent.
    let ladder = r#"<LD><leftPowerRail localId="1" height="20" width="10">
        <position x="0" y="0"/><connectionPointOut/></leftPowerRail></LD>"#;
    let project = read(document(ladder).as_bytes()).unwrap();
    assert_eq!(
        project.pous[0].bodies,
        vec![Body::Other {
            language: "LD".to_string()
        }]
    );

    let unread: Vec<(&str, &str)> = project.unread_bodies().collect();
    assert_eq!(unread, [("Main", "LD")]);

    // And it appears in the Structured Text as a comment saying so, rather
    // than as nothing.
    let st = project.to_structured_text();
    assert!(st.contains("salman does not read LD"), "{st}");
}

// -- turning it into something salman compiles ---------------------------

#[test]
fn a_document_becomes_structured_text_salman_can_compile() {
    let project = read(document(CODESYS_ST).as_bytes()).unwrap();
    let st = project.to_structured_text();

    for expected in [
        "PROGRAM Main",
        "VAR_INPUT",
        "  Start : BOOL;",
        "VAR",
        "  Count : INT := 7;",
        "  Motor AT %QX0.0 : BOOL;",
        "END_VAR",
        "IF Start THEN Count := Count + 1; END_IF;",
        "END_PROGRAM",
    ] {
        assert!(st.contains(expected), "{expected:?} missing from:\n{st}");
    }
}

#[test]
fn the_structured_text_a_document_becomes_actually_compiles() {
    // The claim worth making. "Imports PLCopen XML" means nothing unless what
    // comes out is something salman can then run.
    let project = read(document(CODESYS_ST).as_bytes()).unwrap();
    let st = project.to_structured_text();
    let built = salman_vm::project::build(
        "imported.st",
        &st,
        &salman_lang::dialect::Dialect::generic(),
    )
    .expect("not too large");
    assert!(
        built.compiled.is_some(),
        "what the importer produced does not compile:\n{st}\n{}",
        built.render_diagnostics()
    );
}

#[test]
fn a_function_keeps_its_return_type() {
    let text = document(CODESYS_ST)
        .replace(r#"pouType="program""#, r#"pouType="function""#)
        .replace("<interface>", "<interface><returnType><INT/></returnType>");
    let project = read(text.as_bytes()).unwrap();
    assert_eq!(project.pous[0].kind, PouKind::Function);
    assert_eq!(
        project.pous[0].interface.return_type.as_deref(),
        Some("INT")
    );
    assert!(project.to_structured_text().contains("FUNCTION Main : INT"));
}

#[test]
fn a_named_type_keeps_the_name_the_document_gave_it() {
    // `<derived name="Motor"/>` is how a document names a user type. Reading
    // it as "derived" would lose the only useful part.
    let text = document(CODESYS_ST).replace(
        "<type><INT/></type>",
        r#"<type><derived name="Speed"/></type>"#,
    );
    let project = read(text.as_bytes()).unwrap();
    let (_, variables) = &project.pous[0].interface.sections[1];
    assert_eq!(variables[0].type_name, "Speed");
}

#[test]
fn a_string_keeps_its_declared_length() {
    let text = document(CODESYS_ST).replace(
        "<type><INT/></type>",
        r#"<type><string length="32"/></type>"#,
    );
    let project = read(text.as_bytes()).unwrap();
    let (_, variables) = &project.pous[0].interface.sections[1];
    assert_eq!(variables[0].type_name, "STRING[32]");
}

// -- robustness ----------------------------------------------------------

#[test]
fn no_document_makes_the_reader_panic() {
    // The reader takes a file salman did not write. Every mutation of a valid
    // document has to come back as an answer or an error, never a panic.
    let valid = document(CODESYS_ST);
    let bytes = valid.as_bytes();
    let mut seed = 0xC0FF_EE00_1234_5678_u64;
    let mut next = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for round in 0..2_000 {
        let mut corrupted = bytes.to_vec();
        if round % 3 == 0 {
            // Truncate somewhere.
            corrupted.truncate((next() as usize) % bytes.len().max(1));
        } else {
            for _ in 0..=(next() % 4) {
                let at = (next() as usize) % corrupted.len().max(1);
                if let Some(byte) = corrupted.get_mut(at) {
                    *byte = (next() >> 33) as u8;
                }
            }
        }
        let _ = read(&corrupted[..]);
    }
}

#[test]
fn reading_the_same_document_twice_gives_the_same_project() {
    let text = document(CODESYS_ST);
    assert_eq!(
        read(text.as_bytes()).unwrap(),
        read(text.as_bytes()).unwrap()
    );
}

// -- versions ------------------------------------------------------------

#[test]
fn a_v2_0_document_is_read_and_says_it_is_not_the_version_salman_modelled() {
    // v2.0 is from December 2008 and v2.01 from May 2009, and real exports use
    // both — the one genuine vendor export PLCopen itself publishes is v2.0.
    // Refusing it would mean refusing files a major vendor produces in order
    // to be tidy about a version number. Reading it silently would be claiming
    // salman had checked a schema it has not read.
    let text = document(CODESYS_ST).replace("tc6_0201", "tc6_0200");
    let project = read(text.as_bytes()).expect("v2.0 is read");
    assert_eq!(project.version, Version::V2_0);
    assert!(!project.version.is_the_version_salman_modelled());
    assert_eq!(project.version.to_string(), "v2.0");

    // And everything else is read the same way.
    assert_eq!(project.pous.len(), 1);
    assert_eq!(project.company, "Some Vendor");
    assert!(project.to_structured_text().contains("PROGRAM Main"));
}

#[test]
fn a_v2_01_document_says_it_is_the_version_salman_modelled() {
    let project = read(document(CODESYS_ST).as_bytes()).unwrap();
    assert_eq!(project.version, Version::V2_01);
    assert!(project.version.is_the_version_salman_modelled());
}

#[test]
fn the_two_versions_are_not_read_as_one_document() {
    // An element in the other version's namespace inside a document of this
    // one is not a child salman recognises. Matching on local name alone
    // would read across the boundary.
    let text = document(CODESYS_ST).replace(
        r#"<pou name="Main" pouType="program">"#,
        r#"<pou xmlns="http://www.plcopen.org/xml/tc6_0200" name="Other" pouType="program"/>
      <pou name="Main" pouType="program">"#,
    );
    let project = read(text.as_bytes()).unwrap();
    assert_eq!(
        project.pous.len(),
        1,
        "only the POU in this document's own namespace is read"
    );
    assert_eq!(project.pous[0].name, "Main");
}

// -- what review found: matching by name at any depth ---------------------

#[test]
fn an_interface_inside_an_add_data_blob_does_not_replace_the_pous_own() {
    // Found by review. `<addData>` is where vendors put anything they like,
    // and the schema hangs it off almost every element. Matching `<interface>`
    // by name wherever it appeared meant a vendor blob replaced the POU's
    // declarations with its own — a document producing a different program
    // from the one it describes.
    let text = document(CODESYS_ST).replace(
        "<interface>",
        r#"<addData>
             <data name="vendor" handleUnknown="implementation">
               <interface><localVars><variable name="Vendor"><type><BOOL/></type></variable></localVars></interface>
             </data>
           </addData>
           <interface>"#,
    );
    let project = read(text.as_bytes()).unwrap();
    let interface = &project.pous[0].interface;
    let names: Vec<&str> = interface
        .sections
        .iter()
        .flat_map(|(_, vars)| vars.iter().map(|v| v.name.as_str()))
        .collect();
    assert_eq!(
        names,
        ["Start", "Count", "Motor"],
        "a vendor blob's interface replaced the POU's own"
    );
}

#[test]
fn a_section_named_element_nested_inside_a_section_does_not_close_it() {
    // The same fault from the other side: a `</localVars>` at any depth ended
    // the section, and every variable after it vanished with nothing recording
    // that it had.
    let text = document(CODESYS_ST).replace(
        r#"<variable name="Count">"#,
        r#"<variable name="Before">
             <type><BOOL/></type>
             <addData><data name="v" handleUnknown="discard"><localVars/></data></addData>
           </variable>
           <variable name="Count">"#,
    );
    let project = read(text.as_bytes()).unwrap();
    let (_, locals) = &project.pous[0].interface.sections[1];
    let names: Vec<&str> = locals.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        ["Before", "Count", "Motor"],
        "the variables after the nested element were lost"
    );
}

#[test]
fn a_type_element_nested_below_a_variable_does_not_become_the_variables_type() {
    // `read_variable` matched `<type>` at any depth and stopped at the first
    // `</variable>` at any depth, so a nested one took over.
    let text = document(CODESYS_ST).replace(
        r#"<variable name="Count">
              <type><INT/></type>"#,
        r#"<variable name="Count">
              <type><INT/></type>
              <addData><data name="v" handleUnknown="discard">
                <variable name="Inner"><type><LREAL/></type></variable>
              </data></addData>"#,
    );
    let project = read(text.as_bytes()).unwrap();
    let (_, locals) = &project.pous[0].interface.sections[1];
    assert_eq!(locals[0].name, "Count");
    assert_eq!(
        locals[0].type_name, "INT",
        "a nested variable's type replaced this one's"
    );
    assert_eq!(
        locals.len(),
        2,
        "the nested variable must not become a declaration of its own"
    );
}

#[test]
fn two_xhtml_elements_inside_st_are_refused_rather_than_joined() {
    // Found by review. The schema requires exactly one, and joining two with
    // nothing between them fuses the last token of the first to the first
    // token of the second: `END_IF` and `Motor` become `END_IFMotor`, which is
    // a different program that may well compile.
    let two = r#"<ST><xhtml:p xmlns:xhtml="http://www.w3.org/1999/xhtml">Count := 1;</xhtml:p><xhtml:p xmlns:xhtml="http://www.w3.org/1999/xhtml">Count := 2;</xhtml:p></ST>"#;
    let error = read(document(two).as_bytes()).unwrap_err();
    let ReadError::StructuredTextNotWrapped { found, .. } = &error else {
        panic!("{error}")
    };
    assert!(found.contains("exactly one"), "{found}");
    assert!(found.contains("fuse"), "the message must say why: {found}");
}

#[test]
fn a_composite_initial_value_is_not_reduced_to_one_of_its_fields() {
    // Found by review. An array or structure initialiser holds one
    // `simpleValue` per field, and taking the last one found anywhere made a
    // whole variable's initial value into its final field's — silently.
    // salman does not model composite initialisers, and no value is better
    // than a wrong one.
    let text = document(CODESYS_ST).replace(
        "<initialValue><simpleValue value=\"7\"/></initialValue>",
        r#"<initialValue><arrayValue>
             <value><simpleValue value="1"/></value>
             <value><simpleValue value="2"/></value>
             <value><simpleValue value="99"/></value>
           </arrayValue></initialValue>"#,
    );
    let project = read(text.as_bytes()).unwrap();
    let (_, locals) = &project.pous[0].interface.sections[1];
    assert_eq!(
        locals[0].initial_value, None,
        "a composite initialiser was reduced to one of its fields"
    );
}

#[test]
fn an_addata_element_inside_a_body_is_not_mistaken_for_a_language() {
    // `<addData>` and `<documentation>` are permitted inside `<body>` and are
    // not programming languages. Reporting one as a language salman cannot
    // read would put a false loss on the report.
    let body = format!(
        "{CODESYS_ST}<addData><data name=\"v\" handleUnknown=\"discard\"><x/></data></addData>"
    );
    let project = read(document(&body).as_bytes()).unwrap();
    assert_eq!(project.pous[0].bodies.len(), 1);
    assert_eq!(project.unread_bodies().count(), 0);
}
