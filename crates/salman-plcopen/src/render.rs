// SPDX-License-Identifier: Apache-2.0
//! Turning a document into Structured Text, safely.
//!
//! # A document is untrusted input
//!
//! This is the part of the importer that is easy to get wrong and was.
//! Rendering builds source text by interpolating strings that came out of a
//! file salman did not write, and Structured Text has no escaping: a variable
//! named
//!
//! ```text
//! X : INT; END_VAR
//!   Motor := TRUE;
//! END_PROGRAM
//! PROGRAM Injected
//! VAR Y
//! ```
//!
//! turns one `PROGRAM` into two, with a body that was in no document. Found by
//! review, and demonstrated before it was fixed: the importer produced
//! declarations and statements that nothing in the file asked for.
//!
//! So every string a document controls is checked before it reaches the
//! output, against **salman's own lexer** rather than against a second idea of
//! what an identifier is. A name that is not one identifier, an address that is
//! not one address, an initial value that is not one literal: none of them are
//! rendered, each is replaced by a comment naming the problem, and
//! [`Project::rejected`] lists them so a caller can tell a user what did not
//! survive rather than leaving it to be noticed.

use salman_core::span::SourceMap;
use salman_lang::dialect::Dialect;
use salman_lang::token::TokenKind;

/// Why a string from a document was not put into the output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    /// Which POU it was in.
    pub pou: String,
    /// What it was: `name`, `type`, `address` or `initial value`.
    pub what: &'static str,
    /// The string itself, as the document had it.
    pub value: String,
}

/// What a document-controlled string is allowed to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// One identifier: a name or a type name.
    Identifier,
    /// One `%`-address.
    Address,
    /// One literal value.
    Literal,
}

impl Shape {
    /// What to call it in a message.
    const fn describe(self) -> &'static str {
        match self {
            Self::Identifier => "a single identifier",
            Self::Address => "a single %-address",
            Self::Literal => "a single literal value",
        }
    }
}

/// Whether `value` is exactly one token of the given shape.
///
/// Lexed with salman's own lexer, so the definition of an identifier here and
/// in a source file cannot drift apart. A keyword is refused as well as a
/// malformed name: `END_VAR` lexes as a keyword and not an identifier, which is
/// what stops the interesting half of an injection.
#[must_use]
pub fn is_one(value: &str, shape: Shape) -> bool {
    if value.is_empty() || value.len() > 512 {
        return false;
    }
    let mut sources = SourceMap::new();
    let Ok(file) = sources.add("plcopen", value) else {
        return false;
    };
    let (stream, diagnostics) = salman_lang::lexer::lex(file, value, &Dialect::generic());
    if diagnostics.has_errors() {
        return false;
    }
    match stream.tokens() {
        [token, end] if matches!(end.kind, TokenKind::Eof) => matches!(
            (shape, token.kind),
            // A type name may be an elementary type — `INT` lexes as one
            // rather than as an identifier — or a user-declared name.
            (Shape::Identifier, TokenKind::Ident | TokenKind::TypeName(_))
                | (Shape::Address, TokenKind::DirectAddress(_))
                | (Shape::Literal, TokenKind::Literal(_))
        ),
        _ => false,
    }
}

/// The comment written where a rejected string would have gone.
#[must_use]
pub fn rejection_comment(what: &str, shape: Shape, value: &str) -> String {
    // The value is rendered with `{:?}`, which escapes it, so a comment cannot
    // itself carry a `*)` and end early.
    format!(
        "(* salman did not import this {what}: {value:?} is not {} *)",
        shape.describe()
    )
}
