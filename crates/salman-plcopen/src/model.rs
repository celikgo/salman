// SPDX-License-Identifier: Apache-2.0
//! What a PLCopen XML document says, once it has been read.
//!
//! Deliberately a **model of the document**, not of salman's own AST. An
//! importer that parsed straight into salman's types would have nowhere to put
//! what the document says and salman does not model, and the whole point of
//! ADR-0003 is that anything lost in a translation is named rather than
//! quietly dropped.

use core::fmt;

/// What kind of program organisation unit.
///
/// v2.01 has exactly three, restricted by the schema. There is no class, no
/// interface and no method; those arrived with IEC 61131-10, which is a
/// different format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PouKind {
    /// `function`.
    Function,
    /// `functionBlock`.
    FunctionBlock,
    /// `program`.
    Program,
}

impl PouKind {
    /// Reads the attribute value the schema permits.
    #[must_use]
    pub fn from_attribute(value: &str) -> Option<Self> {
        Some(match value {
            "function" => Self::Function,
            "functionBlock" => Self::FunctionBlock,
            "program" => Self::Program,
            _ => return None,
        })
    }

    /// The Structured Text keywords that open and close it.
    #[must_use]
    pub const fn keywords(self) -> (&'static str, &'static str) {
        match self {
            Self::Function => ("FUNCTION", "END_FUNCTION"),
            Self::FunctionBlock => ("FUNCTION_BLOCK", "END_FUNCTION_BLOCK"),
            Self::Program => ("PROGRAM", "END_PROGRAM"),
        }
    }
}

impl fmt::Display for PouKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Function => "function",
            Self::FunctionBlock => "functionBlock",
            Self::Program => "program",
        })
    }
}

/// Which `VAR` section a variable is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VarSection {
    /// `localVars`.
    Local,
    /// `tempVars`.
    Temp,
    /// `inputVars`.
    Input,
    /// `outputVars`.
    Output,
    /// `inOutVars`.
    InOut,
    /// `externalVars`.
    External,
    /// `globalVars`.
    Global,
    /// `accessVars`.
    Access,
}

impl VarSection {
    /// Reads the element name the schema permits.
    #[must_use]
    pub fn from_element(name: &str) -> Option<Self> {
        Some(match name {
            "localVars" => Self::Local,
            "tempVars" => Self::Temp,
            "inputVars" => Self::Input,
            "outputVars" => Self::Output,
            "inOutVars" => Self::InOut,
            "externalVars" => Self::External,
            "globalVars" => Self::Global,
            "accessVars" => Self::Access,
            _ => return None,
        })
    }

    /// The Structured Text keyword that opens it.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Local => "VAR",
            Self::Temp => "VAR_TEMP",
            Self::Input => "VAR_INPUT",
            Self::Output => "VAR_OUTPUT",
            Self::InOut => "VAR_IN_OUT",
            Self::External => "VAR_EXTERNAL",
            Self::Global => "VAR_GLOBAL",
            Self::Access => "VAR_ACCESS",
        }
    }
}

/// One declared variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    /// Its name, as written.
    pub name: String,
    /// Its type, as the document names it.
    ///
    /// Kept as the document's own spelling rather than resolved, because the
    /// document may name a type salman does not implement and saying which one
    /// beats guessing at the nearest.
    pub type_name: String,
    /// The `%`-located address, if it has one.
    pub address: Option<String>,
    /// The initial value, as written.
    pub initial_value: Option<String>,
}

/// A POU's declarations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Interface {
    /// The return type, for a function.
    pub return_type: Option<String>,
    /// The variable sections, in the order the document gave them.
    ///
    /// The schema makes this an unbounded choice, so sections may appear in
    /// any order and may repeat. Preserving the order is what lets an export
    /// round-trip.
    pub sections: Vec<(VarSection, Vec<Variable>)>,
}

/// A POU's body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// Structured Text, with the XHTML wrapper already removed.
    StructuredText {
        /// The code.
        text: String,
        /// The XHTML element it was wrapped in, so an export can write it back
        /// the way the file it came from did.
        ///
        /// The two vendor families disagree about this and both are valid;
        /// see the crate documentation.
        wrapper: String,
    },
    /// A language salman does not read, named rather than discarded.
    Other {
        /// The element name, such as `LD` or `FBD`.
        language: String,
    },
}

/// One program organisation unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pou {
    /// Its name.
    pub name: String,
    /// What kind.
    pub kind: PouKind,
    /// Its declarations.
    pub interface: Interface,
    /// Its bodies. The schema permits several — one per worksheet.
    pub bodies: Vec<Body>,
}

/// Which version of the format a document declared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Version {
    /// v2.01, the current release and the one salman modelled.
    #[default]
    V2_01,
    /// v2.0, which salman reads without having read its schema.
    V2_0,
}

impl Version {
    /// Whether this is the version salman actually modelled.
    ///
    /// `false` does not mean the document is wrong or that salman read it
    /// wrongly. It means salman has not checked what the two versions differ
    /// about, and a caller should say so rather than let a user assume.
    #[must_use]
    pub const fn is_the_version_salman_modelled(self) -> bool {
        matches!(self, Self::V2_01)
    }

    /// How it is written.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::V2_01 => "v2.01",
            Self::V2_0 => "v2.0",
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A whole document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Project {
    /// Which version of the format it declared.
    pub version: Version,
    /// What wrote it, from `fileHeader/@companyName`.
    pub company: String,
    /// From `fileHeader/@productName`.
    pub product: String,
    /// From `fileHeader/@productVersion`.
    pub product_version: String,
    /// From `contentHeader/@name`.
    pub name: String,
    /// The POUs, in document order.
    pub pous: Vec<Pou>,
}

impl Project {
    /// Renders the POUs as Structured Text salman can compile.
    ///
    /// A POU whose body is in a language salman does not read is emitted as a
    /// comment saying so, rather than skipped: a file that silently lost half
    /// its program would compile and be wrong.
    ///
    /// **Every string the document controls is checked before it reaches the
    /// output.** Structured Text has no escaping, so a variable named
    /// `X : INT; END_VAR Motor := TRUE; END_PROGRAM PROGRAM Injected VAR Y`
    /// turned one `PROGRAM` into two before this check existed — declarations
    /// and statements that were in no document. Anything that is not a single
    /// identifier, address or literal becomes a comment naming it, and
    /// [`Project::rejected`] lists them.
    #[must_use]
    pub fn to_structured_text(&self) -> String {
        self.render().0
    }

    /// Everything a document contained that salman would not put into source.
    ///
    /// Empty for any document salman wrote and for any ordinary export. A
    /// non-empty list means the file contained a name, address or value that
    /// is not one, and a caller should say so rather than let a user wonder
    /// where a variable went.
    #[must_use]
    pub fn rejected(&self) -> Vec<crate::render::Rejected> {
        self.render().1
    }

    /// The rendering and what it refused, in one pass.
    fn render(&self) -> (String, Vec<crate::render::Rejected>) {
        use crate::render::{Rejected, Shape, is_one, rejection_comment};
        use core::fmt::Write;

        let mut out = String::new();
        let mut rejected = Vec::new();

        // Checks one document-controlled string, recording it if it is refused.
        let mut check = |pou: &str, what: &'static str, value: &str, shape: Shape| {
            if is_one(value, shape) {
                true
            } else {
                rejected.push(Rejected {
                    pou: pou.to_string(),
                    what,
                    value: value.to_string(),
                });
                false
            }
        };

        for pou in &self.pous {
            let (open, close) = pou.kind.keywords();
            if !check(&pou.name, "name", &pou.name, Shape::Identifier) {
                let _ = writeln!(
                    out,
                    "{}",
                    rejection_comment("POU", Shape::Identifier, &pou.name)
                );
                continue;
            }
            let _ = write!(out, "{open} {}", pou.name);
            if let Some(returns) = &pou.interface.return_type {
                if check(&pou.name, "return type", returns, Shape::Identifier) {
                    let _ = write!(out, " : {returns}");
                } else {
                    // A function with no return type does not parse, so the
                    // whole POU is refused rather than emitted broken.
                    let _ = writeln!(
                        out,
                        "\n{}",
                        rejection_comment("return type", Shape::Identifier, returns)
                    );
                    continue;
                }
            }
            let _ = writeln!(out);

            for (section, variables) in &pou.interface.sections {
                if variables.is_empty() {
                    continue;
                }
                let _ = writeln!(out, "{}", section.keyword());
                for variable in variables {
                    if !check(&pou.name, "name", &variable.name, Shape::Identifier)
                        || !check(&pou.name, "type", &variable.type_name, Shape::Identifier)
                    {
                        let _ = writeln!(
                            out,
                            "  {}",
                            rejection_comment("variable", Shape::Identifier, &variable.name)
                        );
                        continue;
                    }
                    let _ = write!(out, "  {}", variable.name);
                    if let Some(address) = &variable.address {
                        if check(&pou.name, "address", address, Shape::Address) {
                            let _ = write!(out, " AT {address}");
                        } else {
                            // Dropping the address quietly would leave a
                            // variable that looks located and is not, which is
                            // the failure the located-variable work exists to
                            // stop. The declaration goes, not just the address.
                            let _ = writeln!(
                                out,
                                "\n  {}",
                                rejection_comment("address", Shape::Address, address)
                            );
                            continue;
                        }
                    }
                    let _ = write!(out, " : {}", variable.type_name);
                    if let Some(initial) = &variable.initial_value {
                        if check(&pou.name, "initial value", initial, Shape::Literal) {
                            let _ = write!(out, " := {initial}");
                        } else {
                            let _ =
                                write!(out, " (* initial value {initial:?} was not imported *)");
                        }
                    }
                    let _ = writeln!(out, ";");
                }
                let _ = writeln!(out, "END_VAR");
            }

            for body in &pou.bodies {
                match body {
                    Body::StructuredText { text, .. } => {
                        // The body is code by definition, so it is not checked
                        // for shape — but it is what a compiler will read, and
                        // it is the one part of a document that is *meant* to
                        // be Structured Text.
                        let _ = writeln!(out, "{}", text.trim_end());
                    }
                    Body::Other { language } => {
                        let _ = writeln!(
                            out,
                            "(* salman does not read {}; this body was not imported *)",
                            language
                                .chars()
                                .filter(char::is_ascii_alphanumeric)
                                .collect::<String>()
                        );
                    }
                }
            }
            let _ = writeln!(out, "{close}\n");
        }
        (out, rejected)
    }

    /// Every body in a language salman does not read, with the POU it is in.
    ///
    /// The list a caller needs to tell a user what did not survive the import.
    pub fn unread_bodies(&self) -> impl Iterator<Item = (&str, &str)> {
        self.pous.iter().flat_map(|pou| {
            pou.bodies.iter().filter_map(move |body| match body {
                Body::Other { language } => Some((pou.name.as_str(), language.as_str())),
                Body::StructuredText { .. } => None,
            })
        })
    }
}
