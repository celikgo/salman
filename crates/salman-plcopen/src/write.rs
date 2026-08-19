// SPDX-License-Identifier: Apache-2.0
//! Writing a PLCopen XML v2.01 document.
//!
//! # What the schema insists on, whether or not it makes sense here
//!
//! `<project>` is a strict sequence of four required children —
//! `fileHeader`, `contentHeader`, `types`, `instances` — in that order, and
//! `<types>` requires `dataTypes` before `pous`, and `<instances>` requires
//! `configurations`. Leaving out an empty one is not permitted.
//!
//! The awkward one is `contentHeader/coordinateInfo`, which is **required** and
//! must carry all three of `fbd`, `ld` and `sfc`, each with a `scaling`, even
//! for a project that contains nothing but Structured Text. It is the single
//! commonest reason a hand-written PLCopen file fails validation — 29 of the
//! 45 files in one published benchmark suite fail on this alone. salman writes
//! it because the schema says so, not because it means anything here.
//!
//! # Determinism
//!
//! The writer takes the creation timestamp as an argument rather than reading
//! a clock. Two exports of the same project have to be byte-identical, or a
//! round-trip test compares noise and a compatibility matrix records the
//! weather.

use core::fmt;
use std::io::Write;

use xml::EmitterConfig;
use xml::writer::{EventWriter, XmlEvent};

use crate::model::{Body, Project};
use crate::{NAMESPACE, XHTML_NAMESPACE};

/// The XHTML element salman wraps Structured Text in when a document did not
/// come from somewhere with an opinion.
///
/// `<xhtml>` is what the CODESYS family writes, which makes it the form most
/// tools have certainly seen. It is not an element XHTML 1.1 defines; nothing
/// in the namespace is checked, because the schema imports no XHTML schema and
/// processes that content leniently.
pub const DEFAULT_ST_WRAPPER: &str = "xhtml";

/// Why a document could not be written.
#[derive(Debug)]
pub enum WriteError {
    /// The underlying writer failed.
    Io(std::io::Error),
    /// The XML emitter refused.
    Xml(String),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Xml(message) => write!(f, "{message}"),
        }
    }
}

impl core::error::Error for WriteError {}

impl From<xml::writer::Error> for WriteError {
    fn from(error: xml::writer::Error) -> Self {
        match error {
            xml::writer::Error::Io(io) => Self::Io(io),
            other => Self::Xml(other.to_string()),
        }
    }
}

/// Writes `project` as PLCopen XML v2.01.
///
/// `created` is the `creationDateTime` to record, in the XML Schema `dateTime`
/// form — for instance `2026-08-19T10:00:00`. It is an argument rather than a
/// clock reading because two exports of one project must be byte-identical.
///
/// # Errors
///
/// Returns [`WriteError`] if the underlying writer fails.
pub fn write(project: &Project, created: &str, out: impl Write) -> Result<(), WriteError> {
    let mut writer = EmitterConfig::new()
        .perform_indent(true)
        .write_document_declaration(true)
        .create_writer(out);

    writer.write(
        XmlEvent::start_element("project")
            .default_ns(NAMESPACE)
            .ns("xhtml", XHTML_NAMESPACE),
    )?;

    // fileHeader. Every attribute here is required by the schema.
    writer.write(
        XmlEvent::start_element("fileHeader")
            .attr("companyName", &project.company)
            .attr("productName", &project.product)
            .attr("productVersion", &project.product_version)
            .attr("creationDateTime", created),
    )?;
    writer.write(XmlEvent::end_element())?;

    // contentHeader, with the coordinateInfo the schema requires even here.
    writer.write(XmlEvent::start_element("contentHeader").attr("name", &project.name))?;
    write_coordinate_info(&mut writer)?;
    writer.write(XmlEvent::end_element())?;

    // types: dataTypes then pous, both required, in that order.
    writer.write(XmlEvent::start_element("types"))?;
    writer.write(XmlEvent::start_element("dataTypes"))?;
    writer.write(XmlEvent::end_element())?;
    writer.write(XmlEvent::start_element("pous"))?;
    for pou in &project.pous {
        write_pou(&mut writer, pou)?;
    }
    writer.write(XmlEvent::end_element())?;
    writer.write(XmlEvent::end_element())?;

    // instances: configurations, required.
    writer.write(XmlEvent::start_element("instances"))?;
    writer.write(XmlEvent::start_element("configurations"))?;
    writer.write(XmlEvent::end_element())?;
    writer.write(XmlEvent::end_element())?;

    writer.write(XmlEvent::end_element())?;
    Ok(())
}

/// The three scalings the schema requires, meaningful or not.
fn write_coordinate_info<W: Write>(writer: &mut EventWriter<W>) -> Result<(), WriteError> {
    writer.write(XmlEvent::start_element("coordinateInfo"))?;
    for language in ["fbd", "ld", "sfc"] {
        writer.write(XmlEvent::start_element(language))?;
        writer.write(
            XmlEvent::start_element("scaling")
                .attr("x", "1")
                .attr("y", "1"),
        )?;
        writer.write(XmlEvent::end_element())?;
        writer.write(XmlEvent::end_element())?;
    }
    writer.write(XmlEvent::end_element())?;
    Ok(())
}

/// Writes one POU.
fn write_pou<W: Write>(
    writer: &mut EventWriter<W>,
    pou: &crate::model::Pou,
) -> Result<(), WriteError> {
    let kind = pou.kind.to_string();
    writer.write(
        XmlEvent::start_element("pou")
            .attr("name", &pou.name)
            .attr("pouType", &kind),
    )?;

    writer.write(XmlEvent::start_element("interface"))?;
    if let Some(returns) = &pou.interface.return_type {
        writer.write(XmlEvent::start_element("returnType"))?;
        write_type(writer, returns)?;
        writer.write(XmlEvent::end_element())?;
    }
    for (section, variables) in &pou.interface.sections {
        writer.write(XmlEvent::start_element(element_for(*section)))?;
        for variable in variables {
            let mut element = XmlEvent::start_element("variable").attr("name", &variable.name);
            if let Some(address) = &variable.address {
                element = element.attr("address", address);
            }
            writer.write(element)?;

            writer.write(XmlEvent::start_element("type"))?;
            write_type(writer, &variable.type_name)?;
            writer.write(XmlEvent::end_element())?;

            if let Some(initial) = &variable.initial_value {
                writer.write(XmlEvent::start_element("initialValue"))?;
                writer.write(XmlEvent::start_element("simpleValue").attr("value", initial))?;
                writer.write(XmlEvent::end_element())?;
                writer.write(XmlEvent::end_element())?;
            }
            writer.write(XmlEvent::end_element())?;
        }
        writer.write(XmlEvent::end_element())?;
    }
    writer.write(XmlEvent::end_element())?;

    for body in &pou.bodies {
        writer.write(XmlEvent::start_element("body"))?;
        match body {
            Body::StructuredText { text, wrapper } => {
                writer.write(XmlEvent::start_element("ST"))?;
                // Whatever wrapper the document came with, so a file that came
                // from one family goes back looking like that family. A blank
                // one — a project salman built rather than read — gets the
                // form most tools have certainly seen.
                let wrapper = if wrapper.is_empty() {
                    DEFAULT_ST_WRAPPER
                } else {
                    wrapper.as_str()
                };
                writer.write(XmlEvent::start_element(("xhtml", wrapper)))?;
                writer.write(XmlEvent::characters(text))?;
                writer.write(XmlEvent::end_element())?;
                writer.write(XmlEvent::end_element())?;
            }
            Body::Other { language } => {
                // Written back as an empty element of the right name. salman
                // did not read what was inside it, so it cannot write it back,
                // and an export that silently dropped the body would look
                // complete and be wrong. See `Project::unread_bodies`, which
                // is how a caller tells a user this happened.
                writer.write(XmlEvent::start_element(language.as_str()))?;
                writer.write(XmlEvent::end_element())?;
            }
        }
        writer.write(XmlEvent::end_element())?;
    }

    writer.write(XmlEvent::end_element())?;
    Ok(())
}

/// Writes a type, as an elementary element or a `derived` reference.
fn write_type<W: Write>(writer: &mut EventWriter<W>, name: &str) -> Result<(), WriteError> {
    // A `STRING[32]` came from `<string length="32"/>` and goes back as one.
    if let Some((base, rest)) = name.split_once('[')
        && let Some(length) = rest.strip_suffix(']')
        && matches!(base.to_lowercase().as_str(), "string" | "wstring")
    {
        let lower = base.to_lowercase();
        writer.write(XmlEvent::start_element(lower.as_str()).attr("length", length))?;
        writer.write(XmlEvent::end_element())?;
        return Ok(());
    }

    // `STRING` and `WSTRING` without a declared length. The schema spells them
    // in lower case and gives the length attribute as optional, so an
    // unbounded one is `<string/>`. Writing `<derived name="STRING"/>` instead
    // — which is what happened before — keeps the name and changes what the
    // document means: another reader sees a user-declared type where an
    // elementary one was meant, and salman's own model round-trips regardless,
    // so nothing noticed.
    if matches!(name, "STRING" | "WSTRING") {
        writer.write(XmlEvent::start_element(name.to_lowercase().as_str()))?;
        writer.write(XmlEvent::end_element())?;
        return Ok(());
    }

    if is_elementary(name) {
        writer.write(XmlEvent::start_element(name))?;
        writer.write(XmlEvent::end_element())?;
    } else {
        writer.write(XmlEvent::start_element("derived").attr("name", name))?;
        writer.write(XmlEvent::end_element())?;
    }
    Ok(())
}

/// Whether the schema has an element of this name for an elementary type.
///
/// v2.01's type set is frozen at IEC 61131-3 2nd edition: `LTIME`, `LDATE`,
/// `LTOD`, `LDT`, `CHAR` and `WCHAR` are **not** in it. `STRING` and `WSTRING`
/// are, and the schema spells them in lower case — they are handled above
/// rather than here. Anything not on this list is written as a `derived`
/// reference, which is what a user-declared type is.
fn is_elementary(name: &str) -> bool {
    matches!(
        name,
        "BOOL"
            | "BYTE"
            | "WORD"
            | "DWORD"
            | "LWORD"
            | "SINT"
            | "INT"
            | "DINT"
            | "LINT"
            | "USINT"
            | "UINT"
            | "UDINT"
            | "ULINT"
            | "REAL"
            | "LREAL"
            | "TIME"
            | "DATE"
            | "DT"
            | "TOD"
    )
}

/// The element name for a variable section.
const fn element_for(section: crate::model::VarSection) -> &'static str {
    use crate::model::VarSection as S;
    match section {
        S::Local => "localVars",
        S::Temp => "tempVars",
        S::Input => "inputVars",
        S::Output => "outputVars",
        S::InOut => "inOutVars",
        S::External => "externalVars",
        S::Global => "globalVars",
        S::Access => "accessVars",
    }
}
