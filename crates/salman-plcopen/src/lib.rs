// SPDX-License-Identifier: Apache-2.0
//! The PLCopen XML exchange format.
//!
//! salman targets **v2.01**: "XML Formats for IEC 61131-3", Official Release,
//! 2009-05-08, namespace `http://www.plcopen.org/xml/tc6_0201`. See
//! `docs/adr/ADR-0003-plcopen-xml-canonical.md` for why that version and not
//! another, including the fact that there is no v3.0 and that IEC 61131-10 is
//! a different, incompatible format rather than a later one.
//!
//! # The thing that surprises everyone
//!
//! **Structured Text is not stored as text.** `<ST>` has the schema type
//! `formattedText`, whose entire definition is a sequence of exactly one
//! element from the XHTML namespace, with `processContents="lax"`. So this
//! does not validate:
//!
//! ```xml
//! <ST>a := TRUE;</ST>
//! ```
//!
//! and neither does a bare `CDATA` section. The code has to sit inside an
//! XHTML element — and the specification never says **which** element, has no
//! worked ST example anywhere in its eighty numbered pages, and imports no
//! XHTML schema, so the namespace is constrained and the element name is not.
//!
//! Real tools have split into two families as a result, and both validate:
//!
//! | Family | What it writes |
//! |---|---|
//! | CODESYS, TwinCAT, ctrlX, WAGO, Schneider | `<ST><xhtml xmlns="…/xhtml">code</xhtml></ST>` — an element named `xhtml`, which does not exist in XHTML 1.1 |
//! | Beremiz, OpenPLC Editor | `<ST><xhtml:p><![CDATA[code]]></xhtml:p></ST>` |
//!
//! **A reader that keys on the element name fails on half the ecosystem**, so
//! this one accepts any single element in the XHTML namespace and takes its
//! text. That is under-specification in the standard rather than vendors
//! misbehaving, and there is no correct answer to look up.
//!
//! # What salman does not do
//!
//! It does not ship a copy of the schema, and it does not validate against
//! one. No redistribution grant for the v2.01 schema could be found anywhere,
//! and not finding permission is not permission. It also makes no conformance
//! claim: PLCopen runs no conformance programme for this format.

pub mod compat;
pub mod model;
pub mod read;
pub mod render;
pub mod write;

pub use model::{Body, Interface, Pou, PouKind, Project, VarSection, Variable};
pub use read::{ReadError, read};
pub use render::{Rejected, Shape};
pub use write::{WriteError, write};

/// The namespace of the version salman modelled: v2.01.
pub const NAMESPACE: &str = "http://www.plcopen.org/xml/tc6_0201";

/// The namespace of v2.0, which salman also reads.
///
/// v2.0 is from December 2008 and v2.01 from May 2009, and real exports use
/// both — the one genuine vendor export PLCopen itself publishes, a CODESYS
/// V3.5 SP16 file, is v2.0. Refusing it would mean refusing files a major
/// vendor actually produces in order to be tidy about a version number.
///
/// salman reads it and **says so**: [`Project::version`] records which
/// namespace the document used, because salman modelled v2.01 and has not read
/// the v2.0 schema. Anything the two differ about is something salman has not
/// checked, and a caller can say that to a user rather than discovering it.
///
/// [`Project::version`]: crate::model::Project::version
pub const NAMESPACE_V2_0: &str = "http://www.plcopen.org/xml/tc6_0200";

/// The namespace Structured Text is wrapped in.
pub const XHTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
