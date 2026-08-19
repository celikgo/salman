// SPDX-License-Identifier: Apache-2.0
//! Reading a PLCopen XML v2.01 document.
//!
//! # What this is strict about, and what it is not
//!
//! **Strict about the namespace, and about depth.** An element in a namespace
//! this reader does not know is skipped with its whole subtree, and every
//! element is matched as a **direct child** of the one it belongs to rather
//! than wherever its name appears. Two formats with the same element names and
//! different meanings is exactly the situation PLCopen XML and IEC 61131-10
//! are in, and a reader that matched on `project` alone would read one as the
//! other.
//!
//! Both halves of that were once only half true, and review found it. Matching
//! by name at any depth meant an `<addData>` blob holding an `<interface>`
//! replaced the POU's own, a section-named element nested anywhere closed the
//! section and lost every variable after it, and a `</variable>` at any depth
//! ended the variable. Every one of those is a document quietly producing a
//! different program from the one it describes, which is the failure this
//! whole layer exists to prevent. [`Reader::children`] is the discipline that
//! replaced it.
//!
//! **Not strict about the ST wrapper.** Any single element in the XHTML
//! namespace is accepted, because the specification constrains the namespace
//! and not the name, and the two vendor families disagree. See the crate
//! documentation.
//!
//! **Not a validator.** salman does not ship the schema and cannot validate
//! against one it does not have. What this reports is what it could not read,
//! which is a different and more useful thing than a list of schema
//! violations.

use core::fmt;
use std::io::Read;

use xml::attribute::OwnedAttribute;
use xml::name::OwnedName;
use xml::reader::{EventReader, ParserConfig, XmlEvent};

use crate::model::{Body, Interface, Pou, PouKind, Project, VarSection, Variable, Version};
use crate::{NAMESPACE, NAMESPACE_V2_0, XHTML_NAMESPACE};

/// The most an entity may expand to.
///
/// Set explicitly rather than left at the parser's default, because a default
/// that changes in a patch release is a limit salman did not choose. An
/// entity-expansion bomb is the attack every XML reader has to answer for.
const MAX_ENTITY_EXPANSION_LENGTH: usize = 1_000_000;

/// How deeply entities may nest.
const MAX_ENTITY_EXPANSION_DEPTH: u8 = 10;

/// Why a document could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The XML itself is malformed.
    Xml {
        /// What the parser said.
        message: String,
    },
    /// The root element is not a PLCopen `project`.
    NotAPlcopenProject {
        /// What the root element was, with its namespace.
        found: String,
    },
    /// The document is in a namespace salman does not read.
    ///
    /// Distinguished from "not a project" because it is almost always
    /// IEC 61131-10, which is a different format rather than a broken file.
    WrongNamespace {
        /// The namespace found.
        found: String,
        /// The one salman reads.
        expected: &'static str,
    },
    /// A `<ST>` element did not contain exactly one XHTML element.
    ///
    /// The schema requires exactly one, and both vendor families satisfy it
    /// in different ways; a document satisfying neither is one salman would be
    /// guessing about.
    StructuredTextNotWrapped {
        /// Which POU.
        pou: String,
        /// What was found instead.
        found: String,
    },
    /// A `pouType` attribute salman does not know.
    UnknownPouKind {
        /// Which POU.
        pou: String,
        /// The value found.
        found: String,
    },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xml { message } => write!(f, "this is not well-formed XML: {message}"),
            Self::NotAPlcopenProject { found } => write!(
                f,
                "the root element is {found} and a PLCopen document's is `project`"
            ),
            Self::WrongNamespace { found, expected } => write!(
                f,
                "this document is in the namespace {found} and salman reads {expected}. \
                 If it is IEC 61131-10, that is a different format rather than a later \
                 version of this one, and salman does not read it"
            ),
            Self::StructuredTextNotWrapped { pou, found } => write!(
                f,
                "the Structured Text body of {pou} is not inside an XHTML element: {found}. \
                 PLCopen XML stores ST as formatted text, so the code sits inside exactly \
                 one element from the XHTML namespace — which element is not specified, and \
                 salman accepts any of them"
            ),
            Self::UnknownPouKind { pou, found } => write!(
                f,
                "{pou} has pouType {found:?}, and v2.01 permits function, functionBlock and \
                 program"
            ),
        }
    }
}

impl core::error::Error for ReadError {}

/// Reads a PLCopen XML document.
///
/// # Errors
///
/// Returns [`ReadError`]. A construct salman does not model is not an error —
/// it is recorded on the [`Project`] so a caller can say what did not survive.
pub fn read(source: impl Read) -> Result<Project, ReadError> {
    let reader = ParserConfig::new()
        .trim_whitespace(false)
        .cdata_to_characters(true)
        .max_entity_expansion_length(MAX_ENTITY_EXPANSION_LENGTH)
        .max_entity_expansion_depth(MAX_ENTITY_EXPANSION_DEPTH)
        .create_reader(source);

    Reader {
        events: reader,
        project: Project::default(),
        namespace: NAMESPACE,
    }
    .run()
}

/// Walks the document.
struct Reader<R: Read> {
    events: EventReader<R>,
    project: Project,
    /// The namespace this document actually used, decided by its root.
    namespace: &'static str,
}

impl<R: Read> Reader<R> {
    fn run(mut self) -> Result<Project, ReadError> {
        // Find the root and check it is what it claims to be.
        loop {
            match self.next()? {
                XmlEvent::StartElement {
                    name, attributes, ..
                } => {
                    self.project.version = check_root(&name)?;
                    self.namespace = match self.project.version {
                        Version::V2_01 => NAMESPACE,
                        Version::V2_0 => NAMESPACE_V2_0,
                    };
                    let _ = attributes;
                    break;
                }
                XmlEvent::EndDocument => {
                    return Err(ReadError::NotAPlcopenProject {
                        found: "an empty document".to_string(),
                    });
                }
                _ => {}
            }
        }

        loop {
            match self.next()? {
                XmlEvent::StartElement {
                    name, attributes, ..
                } => match name.local_name.as_str() {
                    "fileHeader" if self.in_plcopen(&name) => {
                        self.project.company = attribute(&attributes, "companyName");
                        self.project.product = attribute(&attributes, "productName");
                        self.project.product_version = attribute(&attributes, "productVersion");
                    }
                    "contentHeader" if self.in_plcopen(&name) => {
                        self.project.name = attribute(&attributes, "name");
                    }
                    "pou" if self.in_plcopen(&name) => {
                        let pou = self.read_pou(&attributes)?;
                        self.project.pous.push(pou);
                    }
                    _ => {}
                },
                XmlEvent::EndDocument => break,
                _ => {}
            }
        }
        Ok(self.project)
    }

    /// Calls `on_child` for each **direct** child of the element just entered,
    /// and skips the subtree of any child the closure does not take.
    ///
    /// This is the discipline the whole reader is built on, and it exists
    /// because the earlier version did not have it. Matching an element by
    /// name wherever it appeared meant an `<addData>` blob containing an
    /// `<interface>` replaced the POU's own, a section-named element nested
    /// anywhere closed the section early and lost every variable after it, and
    /// a `</variable>` at any depth ended the variable. Every one of those is a
    /// document quietly producing a different program from the one it
    /// describes.
    ///
    /// `on_child` returns `true` if it consumed the child **including its end
    /// tag**, and `false` to have it skipped whole.
    fn children<F>(&mut self, mut on_child: F) -> Result<(), ReadError>
    where
        F: FnMut(&mut Self, &OwnedName, &[OwnedAttribute]) -> Result<bool, ReadError>,
    {
        loop {
            match self.next()? {
                XmlEvent::StartElement {
                    name, attributes, ..
                } => {
                    if !on_child(self, &name, &attributes)? {
                        self.skip_subtree()?;
                    }
                }
                // The end tag of the element whose children these are.
                XmlEvent::EndElement { .. } | XmlEvent::EndDocument => return Ok(()),
                _ => {}
            }
        }
    }

    /// Reads one `<pou>`, from its start tag to its end tag.
    fn read_pou(&mut self, attributes: &[OwnedAttribute]) -> Result<Pou, ReadError> {
        let name = attribute(attributes, "name");
        let kind_text = attribute(attributes, "pouType");
        let kind =
            PouKind::from_attribute(&kind_text).ok_or_else(|| ReadError::UnknownPouKind {
                pou: name.clone(),
                found: kind_text,
            })?;

        let mut pou = Pou {
            name: name.clone(),
            kind,
            interface: Interface::default(),
            bodies: Vec::new(),
        };

        self.children(|reader, element, _| {
            if !reader.in_plcopen(element) {
                return Ok(false);
            }
            match element.local_name.as_str() {
                "interface" => {
                    pou.interface = reader.read_interface()?;
                    Ok(true)
                }
                "body" => {
                    reader.read_body(&name, &mut pou.bodies)?;
                    Ok(true)
                }
                // `actions`, `transitions`, `addData`, `documentation`: all
                // skipped whole, so nothing inside them can be mistaken for a
                // child of the POU.
                _ => Ok(false),
            }
        })?;
        Ok(pou)
    }

    /// Reads an `<interface>`.
    fn read_interface(&mut self) -> Result<Interface, ReadError> {
        let mut interface = Interface::default();
        self.children(|reader, element, _| {
            if !reader.in_plcopen(element) {
                return Ok(false);
            }
            if element.local_name == "returnType" {
                interface.return_type = Some(reader.read_type_name()?);
                return Ok(true);
            }
            let Some(section) = VarSection::from_element(&element.local_name) else {
                return Ok(false);
            };
            let variables = reader.read_var_section()?;
            interface.sections.push((section, variables));
            Ok(true)
        })?;
        Ok(interface)
    }

    /// Reads the variables of one `<...Vars>` section.
    ///
    /// A `<variable>` is only a variable when it is a direct child of a
    /// section. One found anywhere else used to be parsed in full and then
    /// dropped on the floor, with nothing recording that it had been.
    fn read_var_section(&mut self) -> Result<Vec<Variable>, ReadError> {
        let mut variables = Vec::new();
        self.children(|reader, element, attributes| {
            if reader.in_plcopen(element) && element.local_name == "variable" {
                variables.push(reader.read_variable(attributes)?);
                Ok(true)
            } else {
                Ok(false)
            }
        })?;
        Ok(variables)
    }

    /// Reads one `<variable>`.
    fn read_variable(&mut self, attributes: &[OwnedAttribute]) -> Result<Variable, ReadError> {
        let mut variable = Variable {
            name: attribute(attributes, "name"),
            type_name: String::new(),
            address: non_empty(attribute(attributes, "address")),
            initial_value: None,
        };

        self.children(|reader, element, _| {
            if !reader.in_plcopen(element) {
                return Ok(false);
            }
            match element.local_name.as_str() {
                "type" => {
                    variable.type_name = reader.read_type_name()?;
                    Ok(true)
                }
                "initialValue" => {
                    variable.initial_value = reader.read_initial_value()?;
                    Ok(true)
                }
                _ => Ok(false),
            }
        })?;
        Ok(variable)
    }

    /// Reads a `<type>` or `<returnType>`, whose one child names the type.
    ///
    /// `<BOOL/>` for an elementary type, `<derived name="Foo"/>` for a named
    /// one. Anything else keeps its element name, so a type salman does not
    /// model is reported by its own name rather than as the nearest guess.
    /// Only the **first** child counts: a composite type's inner elements are
    /// not names of the type.
    fn read_type_name(&mut self) -> Result<String, ReadError> {
        let mut found: Option<String> = None;
        self.children(|reader, element, attributes| {
            if found.is_some() || !reader.in_plcopen(element) {
                return Ok(false);
            }
            found = Some(match element.local_name.as_str() {
                "derived" => attribute(attributes, "name"),
                "string" | "wstring" => {
                    let upper = element.local_name.to_uppercase();
                    match non_empty(attribute(attributes, "length")) {
                        Some(length) => format!("{upper}[{length}]"),
                        None => upper,
                    }
                }
                other => other.to_string(),
            });
            Ok(false)
        })?;
        Ok(found.unwrap_or_default())
    }

    /// Reads an `<initialValue>`, whose `simpleValue` child carries it.
    ///
    /// The **first** `simpleValue` that is a direct child, and no other. An
    /// earlier version took the last one found anywhere in the subtree, so a
    /// structure or array initialiser — which holds one `simpleValue` per
    /// field — came back as its final field's value, silently, for the whole
    /// variable.
    fn read_initial_value(&mut self) -> Result<Option<String>, ReadError> {
        let mut value: Option<String> = None;
        let mut composite = false;
        self.children(|reader, element, attributes| {
            if !reader.in_plcopen(element) {
                return Ok(false);
            }
            match element.local_name.as_str() {
                "simpleValue" if value.is_none() => {
                    value = non_empty(attribute(attributes, "value"));
                }
                // A structure or array initialiser is not one value, and
                // salman does not model it. Taking any single field's value
                // would be worse than taking none.
                "arrayValue" | "structValue" => composite = true,
                _ => {}
            }
            Ok(false)
        })?;
        Ok(if composite { None } else { value })
    }

    /// Reads a `<body>`, which holds one language element.
    fn read_body(&mut self, pou: &str, bodies: &mut Vec<Body>) -> Result<(), ReadError> {
        let mut taken = false;
        self.children(|reader, element, _| {
            if !reader.in_plcopen(element) || taken {
                return Ok(false);
            }
            if element.local_name == "ST" {
                let body = reader.read_structured_text(pou)?;
                bodies.push(body);
                taken = true;
                return Ok(true);
            }
            // A language salman does not read, named rather than discarded.
            // `addData` and `documentation` are also permitted here and are
            // not languages, so they are skipped rather than named.
            if matches!(element.local_name.as_str(), "IL" | "LD" | "FBD" | "SFC") {
                bodies.push(Body::Other {
                    language: element.local_name.clone(),
                });
                taken = true;
            }
            Ok(false)
        })?;
        Ok(())
    }

    /// Reads an `<ST>` and unwraps the one XHTML element the code is in.
    ///
    /// **Exactly one**, which the schema requires and which an earlier version
    /// did not check: two elements were accepted and their text concatenated
    /// with nothing between them, so the last token of one and the first of the
    /// next fused into a single identifier.
    fn read_structured_text(&mut self, pou: &str) -> Result<Body, ReadError> {
        let mut wrapper: Option<String> = None;
        let mut text = String::new();
        let mut stray = String::new();
        let mut extra = 0_usize;
        let mut depth = 0_usize;

        loop {
            match self.next()? {
                XmlEvent::StartElement { name, .. } => {
                    if depth == 0 {
                        if name.namespace.as_deref() == Some(XHTML_NAMESPACE) {
                            if wrapper.is_some() {
                                extra += 1;
                            } else {
                                wrapper = Some(name.local_name.clone());
                            }
                        } else {
                            return Err(ReadError::StructuredTextNotWrapped {
                                pou: pou.to_string(),
                                found: format!("an element <{name}> in another namespace"),
                            });
                        }
                    }
                    depth += 1;
                }
                XmlEvent::Characters(chunk) | XmlEvent::CData(chunk) => {
                    if depth > 0 {
                        text.push_str(&chunk);
                    } else if !chunk.trim().is_empty() {
                        stray.push_str(&chunk);
                    }
                }
                XmlEvent::EndElement { name } => {
                    if depth == 0 {
                        let _ = name;
                        break;
                    }
                    depth -= 1;
                }
                XmlEvent::EndDocument => break,
                _ => {}
            }
        }

        if extra > 0 {
            return Err(ReadError::StructuredTextNotWrapped {
                pou: pou.to_string(),
                found: format!(
                    "{} elements in the XHTML namespace; the schema permits exactly one, and \
                     joining their text would fuse the last token of one to the first of the \
                     next",
                    extra + 1
                ),
            });
        }

        match wrapper {
            Some(wrapper) => Ok(Body::StructuredText { text, wrapper }),
            None => Err(ReadError::StructuredTextNotWrapped {
                pou: pou.to_string(),
                found: if stray.trim().is_empty() {
                    "nothing at all".to_string()
                } else {
                    format!("bare text: {:?}", stray.trim())
                },
            }),
        }
    }

    /// Steps past an element and everything inside it.
    fn skip_subtree(&mut self) -> Result<(), ReadError> {
        let mut depth = 0_usize;
        loop {
            match self.next()? {
                XmlEvent::StartElement { .. } => depth += 1,
                XmlEvent::EndElement { .. } => {
                    if depth == 0 {
                        return Ok(());
                    }
                    depth -= 1;
                }
                XmlEvent::EndDocument => return Ok(()),
                _ => {}
            }
        }
    }

    /// Whether an element is in the namespace this document declared.
    fn in_plcopen(&self, name: &OwnedName) -> bool {
        name.namespace.as_deref() == Some(self.namespace)
    }

    fn next(&mut self) -> Result<XmlEvent, ReadError> {
        self.events.next().map_err(|error| ReadError::Xml {
            message: error.to_string(),
        })
    }
}

/// Whether the root element is a PLCopen project in the namespace salman reads.
fn check_root(name: &OwnedName) -> Result<Version, ReadError> {
    if name.local_name != "project" {
        return Err(ReadError::NotAPlcopenProject {
            found: format!("<{name}>"),
        });
    }
    match name.namespace.as_deref() {
        Some(NAMESPACE) => Ok(Version::V2_01),
        Some(NAMESPACE_V2_0) => Ok(Version::V2_0),
        Some(other) => Err(ReadError::WrongNamespace {
            found: other.to_string(),
            expected: NAMESPACE,
        }),
        None => Err(ReadError::WrongNamespace {
            found: "none".to_string(),
            expected: NAMESPACE,
        }),
    }
}

/// An attribute's value, or empty.
fn attribute(attributes: &[OwnedAttribute], wanted: &str) -> String {
    attributes
        .iter()
        .find(|a| a.name.local_name == wanted)
        .map(|a| a.value.clone())
        .unwrap_or_default()
}

/// `None` for an empty string, so an absent attribute and a blank one read the
/// same way.
fn non_empty(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}
