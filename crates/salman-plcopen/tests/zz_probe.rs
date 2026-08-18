// SPDX-License-Identifier: Apache-2.0
//! temporary probe
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_plcopen::model::{Body, Interface, Pou, PouKind, Project, VarSection, Variable, Version};
use salman_plcopen::read::read;
use salman_plcopen::write::write;

fn doc(body: &str, interface: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<project xmlns="http://www.plcopen.org/xml/tc6_0201">
  <fileHeader companyName="V" productName="P" productVersion="1" creationDateTime="2026-01-01T00:00:00"/>
  <contentHeader name="C"><coordinateInfo><fbd><scaling x="1" y="1"/></fbd><ld><scaling x="1" y="1"/></ld><sfc><scaling x="1" y="1"/></sfc></coordinateInfo></contentHeader>
  <types><dataTypes/><pous>
      <pou name="Main" pouType="program">
        {interface}
        {body}
      </pou>
  </pous></types>
  <instances><configurations/></instances>
</project>
"#
    )
}

#[test]
fn probe_multiple_xhtml_elements_in_st() {
    let body = r#"<body><ST xmlns:x="http://www.w3.org/1999/xhtml"><x:p>IF a THEN</x:p><x:p>b := 1;</x:p></ST></body>"#;
    let p = read(doc(body, "<interface/>").as_bytes());
    println!("MULTI-XHTML => {p:#?}");
}

#[test]
fn probe_addData_nested_interface_and_body() {
    let interface = r#"<interface><localVars><variable name="Real"><type><BOOL/></type></variable></localVars></interface>"#;
    let body = r#"<body><ST xmlns:x="http://www.w3.org/1999/xhtml"><x:p>real := 1;</x:p></ST></body>
        <addData>
          <data name="http://www.3s-software.com/plcopenxml/method" handleUnknown="implementation">
            <Method name="M">
              <interface><localVars><variable name="Hidden"><type><INT/></type></variable></localVars></interface>
              <body><ST xmlns:x="http://www.w3.org/1999/xhtml"><x:p>hidden := 2;</x:p></ST></body>
            </Method>
          </data>
        </addData>"#;
    let p = read(doc(body, interface).as_bytes()).unwrap();
    println!("ADDDATA => {p:#?}");
    println!("ST =>\n{}", p.to_structured_text());
}

#[test]
fn probe_struct_initial_value() {
    let interface = r#"<interface><localVars>
        <variable name="S"><type><derived name="Rec"/></type>
          <initialValue><structValue>
            <value member="a"><simpleValue value="1"/></value>
            <value member="b"><simpleValue value="2"/></value>
          </structValue></initialValue>
        </variable>
        <variable name="A"><type><array><dimension lower="0" upper="9"/><baseType><INT/></baseType></array></type>
          <initialValue><arrayValue><value repetitionValue="10"><simpleValue value="0"/></value></arrayValue></initialValue>
        </variable>
      </localVars></interface>"#;
    let body = r#"<body><ST xmlns:x="http://www.w3.org/1999/xhtml"><x:p>;</x:p></ST></body>"#;
    let p = read(doc(body, interface).as_bytes()).unwrap();
    println!("INITVAL => {p:#?}");
    println!("ST =>\n{}", p.to_structured_text());
}

#[test]
fn probe_namespace_leak_in_type_and_initial_value() {
    let interface = r#"<interface><localVars>
        <variable name="V"><type xmlns:o="urn:other"><o:derived name="Injected"/></type>
          <initialValue xmlns:o="urn:other"><o:simpleValue value="99"/></initialValue>
        </variable>
      </localVars></interface>"#;
    let body = r#"<body><ST xmlns:x="http://www.w3.org/1999/xhtml"><x:p>;</x:p></ST></body>"#;
    let p = read(doc(body, interface).as_bytes()).unwrap();
    println!("NS-LEAK => {p:#?}");
}

#[test]
fn probe_body_documentation_and_adddata() {
    let body = r#"<body><ST xmlns:x="http://www.w3.org/1999/xhtml"><x:p>;</x:p></ST><documentation><x:p xmlns:x="http://www.w3.org/1999/xhtml">note</x:p></documentation><addData><data name="n" handleUnknown="discard"><q/></data></addData></body>"#;
    let p = read(doc(body, "<interface/>").as_bytes()).unwrap();
    println!("BODY-EXTRA => {:#?}", p.pous[0].bodies);
    println!("unread = {:?}", p.unread_bodies().collect::<Vec<_>>());
    println!("ST =>\n{}", p.to_structured_text());
}

#[test]
fn probe_multiline_st_roundtrip() {
    let original = Project {
        version: Version::V2_01,
        company: "c".into(),
        product: "p".into(),
        product_version: "1".into(),
        name: "n".into(),
        pous: vec![Pou {
            name: "Main".into(),
            kind: PouKind::Program,
            interface: Interface::default(),
            bodies: vec![Body::StructuredText {
                text: "\nIF a THEN\n  b := 1;\nEND_IF;\n".into(),
                wrapper: "xhtml".into(),
            }],
        }],
    };
    let mut bytes = Vec::new();
    write(&original, "2026-01-01T00:00:00", &mut bytes).unwrap();
    println!("XML =>\n{}", String::from_utf8_lossy(&bytes));
    let back = read(&bytes[..]).unwrap();
    println!("EQ = {}", back == original);
    println!("BACK => {:?}", back.pous[0].bodies);
}

#[test]
fn probe_to_structured_text_injection() {
    let p = Project {
        version: Version::V2_01,
        company: "c".into(),
        product: "p".into(),
        product_version: "1".into(),
        name: "n".into(),
        pous: vec![
            Pou {
                name: "Main".into(),
                kind: PouKind::Program,
                interface: Interface {
                    return_type: None,
                    sections: vec![(
                        VarSection::Local,
                        vec![Variable {
                            name: "END_VAR\nx".into(),
                            type_name: "BOOL".into(),
                            address: None,
                            initial_value: None,
                        }],
                    )],
                },
                bodies: vec![Body::StructuredText {
                    text: "END_PROGRAM\nPROGRAM Evil\nVAR END_VAR\n".into(),
                    wrapper: "xhtml".into(),
                }],
            },
            Pou {
                name: "Second".into(),
                kind: PouKind::Program,
                interface: Interface::default(),
                bodies: vec![Body::Other {
                    language: "LD".into(),
                }],
            },
        ],
    };
    println!("ST =>\n{}", p.to_structured_text());
}

#[test]
fn probe_other_language_comment_close() {
    let p = Project {
        version: Version::V2_01,
        company: "c".into(),
        product: "p".into(),
        product_version: "1".into(),
        name: "n".into(),
        pous: vec![Pou {
            name: "Main".into(),
            kind: PouKind::Program,
            interface: Interface::default(),
            bodies: vec![Body::Other {
                language: "LD".into(),
            }],
        }],
    };
    let st = p.to_structured_text();
    println!("ST =>\n{st}");
    let built =
        salman_vm::project::build("x.st", &st, &salman_lang::dialect::Dialect::generic()).unwrap();
    println!("compiled = {}", built.compiled.is_some());
    println!("{}", built.render_diagnostics());
}

#[test]
fn probe_var_access_and_matrix() {
    let rows = salman_plcopen::compat::matrix();
    for r in &rows {
        println!("{:?} {}", r.outcome, r.construct);
    }
}

#[test]
fn probe_empty_returntype_and_nested_variable() {
    let interface = r#"<interface><localVars>
        <variable name="Outer"><type><derived name="T"/></type>
          <addData><data name="x" handleUnknown="discard"><variable name="Inner"><type><INT/></type></variable></data></addData>
        </variable>
      </localVars></interface>"#;
    let body = r#"<body><ST xmlns:x="http://www.w3.org/1999/xhtml"><x:p>;</x:p></ST></body>"#;
    let p = read(doc(body, interface).as_bytes()).unwrap();
    println!("NESTED-VAR => {:#?}", p.pous[0].interface);
}

#[test]
fn probe_st_text_with_cdata_and_entities() {
    let p = Project {
        version: Version::V2_01,
        company: "c".into(),
        product: "p".into(),
        product_version: "1".into(),
        name: "n".into(),
        pous: vec![Pou {
            name: "Main".into(),
            kind: PouKind::Program,
            interface: Interface::default(),
            bodies: vec![Body::StructuredText {
                text: "IF a < b AND c > d THEN e := 'x'; END_IF;".into(),
                wrapper: "xhtml".into(),
            }],
        }],
    };
    let mut bytes = Vec::new();
    write(&p, "2026-01-01T00:00:00", &mut bytes).unwrap();
    println!("XML =>\n{}", String::from_utf8_lossy(&bytes));
    println!("EQ = {}", read(&bytes[..]).unwrap() == p);
}
