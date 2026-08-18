//! IEC 61131-3 front end: lexer, parser, AST and type checker.
//!
//! # Edition
//!
//! salman targets **IEC 61131-3 Edition 3.0 (2013)**. Where this crate cites a
//! clause or table number it means that edition, and the citation registry in
//! `salman_core::clause` records how far each number could be verified against
//! a public source.
//!
//! # What is implemented
//!
//! Structured Text, for the dialects in [`dialect::DialectId`]. The graphical
//! languages (LD, FBD, SFC), Instruction List, and the Edition 3 object
//! oriented extensions are **not** implemented; their keywords are reserved so
//! that meeting one produces a clear message rather than a baffling syntax
//! error.
//!
//! # Untrusted input
//!
//! Everything in this crate treats its input as hostile. Nesting is bounded,
//! source size is bounded, identifier length is bounded, and no path may panic
//! on malformed input — the workspace denies `unwrap`, `expect` and `panic` in
//! library code, and the parsers are fuzzed in CI.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
// Indexing is denied in library code specifically: this crate slices buffers
// whose bounds come from untrusted source text, and `[i]` there is a panic
// waiting for a malformed file. Test code indexes freely — a panicking test is
// a failing test, which is what tests are for.
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

pub mod address;
pub mod ast;
pub mod codes;
pub mod dialect;
pub mod lexer;
pub mod parser;
pub mod token;
pub mod types;

pub use address::DirectAddress;
pub use ast::{CompilationUnit, Expr, ExprKind, Item, Pou, Stmt, StmtKind};
pub use dialect::{Dialect, DialectId};
pub use lexer::{TokenStream, lex};
pub use parser::{parse, parse_source};
pub use token::{Keyword, LiteralValue, Punct, Token, TokenKind};
