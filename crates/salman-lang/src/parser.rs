// SPDX-License-Identifier: Apache-2.0
//! The Structured Text parser.
//!
//! Citation policy: salman cites IEC 61131-3:2013 (Edition 3.0) clause, table
//! and figure numbers as references only. No IEC text is reproduced. Clause
//! numbers are edition-specific; see `docs/IEC_CITATIONS.md`.
//!
//! # Design
//!
//! Recursive descent, one token of lookahead almost everywhere, with two
//! deliberate exceptions that are documented where they happen: telling a
//! `CASE` label list apart from a statement, and telling an enumeration from a
//! subrange in a `TYPE` declaration.
//!
//! Two properties matter more than anything else here.
//!
//! **It never stops at the first error.** A file with ten mistakes must report
//! ten mistakes; a parser that gives up after one turns a morning's work into a
//! week's. Every construct that fails produces an [`ExprKind::Error`],
//! [`StmtKind::Error`] or [`TypeRef::Error`] node and resynchronises at the
//! next `;` or the next `END_` keyword.
//!
//! **It never panics and never recurses without a bound.** This code parses
//! files salman did not write. Nesting is counted against
//! [`Dialect::max_nesting_depth`], and exceeding it produces
//! [`codes::E_PARSE_NESTING_TOO_DEEP`] rather than a stack overflow. The bound
//! covers left-associative operator chains as well as bracket nesting, because
//! `a+a+a+…` builds a tree as deep as the chain is long and *dropping* that
//! tree costs stack in proportion — see the depth note in [`crate::ast`].
//!
//! Every loop in this file either consumes a token or breaks. Where that is not
//! obvious from the loop condition, the loop records `self.pos` before the body
//! and forces progress afterwards.
//!
//! # The expression grammar, and the trap in it
//!
//! The precedence chain below is the Edition 3 one. Annex A of Edition 3 is
//! **normative** (it was the informative Annex B in Edition 2), and its
//! productions agree with the row order of
//! IEC 61131-3:2013 Table 71 "Operators of the ST language".
//!
//! Every level is left-associative. From loosest to tightest:
//!
//! | salman function        | Annex A production name | operators              |
//! |------------------------|-------------------------|------------------------|
//! | `parse_or`             | `or_expression`         | `OR`                   |
//! | `parse_xor`            | `xor_expression`        | `XOR`                  |
//! | `parse_and`            | `and_expression`        | `AND`, `&`             |
//! | `parse_equality`       | **`comparison`**        | `=`, `<>`              |
//! | `parse_relational`     | **`equ_expression`**    | `<`, `>`, `<=`, `>=`   |
//! | `parse_additive`       | `add_expression`        | `+`, `-`               |
//! | `parse_multiplicative` | `term`                  | `*`, `/`, `MOD`        |
//! | `parse_power`          | `power_expression`      | `**`                   |
//! | `parse_unary`          | `unary_expression`      | `-`, `+`, `NOT`        |
//! | `parse_postfix`        | (postfix of primary)    | `.`, `[]`, `^`, `()`   |
//! | `parse_primary`        | `primary_expression`    | literals, names, `( )` |
//!
//! The two bold rows are the trap. The standard's production called
//! `comparison` is the `=`/`<>` level and its `equ_expression` is the
//! `<`/`>`/`<=`/`>=` level — the opposite of what the names suggest. salman
//! names its functions after what they parse; the table above is there so that
//! a reader checking this file against Annex A does not conclude salman has
//! them backwards.
//!
//! # `-2 ** 2`
//!
//! Unary binds *tighter* than `**` in Edition 3, so `-2 ** 2` is `(-2) ** 2`,
//! which is `4`. CODESYS and Beckhoff publish the Edition 2 ordering and give
//! `-4`. salman implements the Edition 3 order (see
//! [`crate::dialect::UnaryPowerBinding`]) and warns —
//! [`codes::W_POWER_OPERAND_BINDING`] — on any unparenthesised unary operand of
//! `**`, so nobody is silently bitten when code moves between tools. A
//! parenthesised operand does not warn: that is what [`ExprKind::Paren`] is
//! kept in the tree for.

use salman_core::clause;
use salman_core::diag::{Diagnostic, Diagnostics, Edit};
use salman_core::ident::Ident;
use salman_core::span::{FileId, Span};
use salman_core::value::ElementaryType;

use crate::ast::{
    Arg, ArrayDim, BinaryOp, CaseArm, CaseLabel, CompilationUnit, ConfigurationDecl, EnumValue,
    Expr, ExprKind, IfBranch, Item, Name, NodeId, Pou, PouKind, ProgramInstance, ResourceDecl,
    Retention, Stmt, StmtKind, TaskDecl, TypeDecl, TypeDeclKind, TypeRef, UnaryOp, VarBlock,
    VarDecl, VarQualifiers, VarSection,
};
use crate::codes;
use crate::dialect::{Dialect, UnaryPowerBinding};
use crate::lexer::{TokenStream, lex};
use crate::token::{Keyword, LiteralValue, Punct, Token, TokenKind};

/// How many tokens the `CASE` arm scanner reads before deciding that what it is
/// looking at cannot be a label list.
///
/// A bound rather than a preference: without one, a file consisting of
/// `1,1,1,1,…` with no colon would make the scanner walk to the end of the file
/// once per statement.
const MAX_CASE_LABEL_LOOKAHEAD: usize = 4096;

/// Parses a token stream into a compilation unit.
///
/// Never fails and never panics. A file of pure noise yields a unit with no
/// items and one diagnostic per thing that could not be understood.
///
/// `source` is the text the stream was lexed from. It is needed because a
/// [`TokenKind::Ident`] token carries only a span: the spelling an engineer
/// wrote lives in the source text and nowhere else, and salman's identifiers
/// are case-*preserving*, so the spelling cannot be reconstructed from the
/// token. Passing text that is not what `stream` was lexed from produces
/// nonsense names, not a panic.
#[must_use]
pub fn parse(
    file: FileId,
    source: &str,
    stream: &TokenStream,
    dialect: &Dialect,
) -> (CompilationUnit, Diagnostics) {
    let tokens = stream.tokens();
    let eof = tokens.last().copied().unwrap_or(Token {
        kind: TokenKind::Eof,
        span: Span::at(file, 0),
    });
    let mut parser = Parser {
        source,
        stream,
        tokens,
        eof,
        pos: 0,
        dialect,
        diags: Diagnostics::new(),
        next_id: 0,
        depth: 0,
        fused: false,
    };
    let items = parser.parse_items();
    (
        CompilationUnit {
            file,
            items,
            node_count: parser.next_id,
        },
        parser.diags,
    )
}

/// Lexes and parses one source file, merging both sets of diagnostics.
///
/// This is what almost every caller wants: the lexer's diagnostics and the
/// parser's describe the same file and belong in the same list. Lexical
/// diagnostics come first, then syntactic ones; call
/// [`Diagnostics::sort`](salman_core::diag::Diagnostics::sort) if you want them
/// in source order instead.
#[must_use]
pub fn parse_source(
    file: FileId,
    source: &str,
    dialect: &Dialect,
) -> (CompilationUnit, Diagnostics) {
    let (stream, mut diags) = lex(file, source, dialect);
    let (unit, parse_diags) = parse(file, source, &stream, dialect);
    diags.extend(parse_diags);
    (unit, diags)
}

// ---------------------------------------------------------------------------
// Token classification
//
// Free functions rather than methods, so that each is a statement about the
// grammar that can be read without the parser's state in mind.
// ---------------------------------------------------------------------------

/// Whether a keyword closes a block: every `END_` word the language has.
fn is_end_keyword(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Keyword(
            Keyword::EndAction
                | Keyword::EndCase
                | Keyword::EndClass
                | Keyword::EndConfiguration
                | Keyword::EndFor
                | Keyword::EndFunction
                | Keyword::EndFunctionBlock
                | Keyword::EndIf
                | Keyword::EndInterface
                | Keyword::EndMethod
                | Keyword::EndProgram
                | Keyword::EndRepeat
                | Keyword::EndResource
                | Keyword::EndStep
                | Keyword::EndStruct
                | Keyword::EndTransition
                | Keyword::EndType
                | Keyword::EndVar
                | Keyword::EndWhile
        )
    )
}

/// The variable section a keyword opens, if it opens one.
fn var_section_of(kind: TokenKind) -> Option<VarSection> {
    let TokenKind::Keyword(keyword) = kind else {
        return None;
    };
    Some(match keyword {
        Keyword::Var => VarSection::Local,
        Keyword::VarInput => VarSection::Input,
        Keyword::VarOutput => VarSection::Output,
        Keyword::VarInOut => VarSection::InOut,
        Keyword::VarTemp => VarSection::Temp,
        Keyword::VarGlobal => VarSection::Global,
        Keyword::VarExternal => VarSection::External,
        Keyword::VarAccess => VarSection::Access,
        Keyword::VarConfig => VarSection::Config,
        _ => return None,
    })
}

/// Whether a keyword can begin a top-level item.
fn starts_top_level_item(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Keyword(
            Keyword::Program
                | Keyword::Function
                | Keyword::FunctionBlock
                | Keyword::Type
                | Keyword::VarGlobal
                | Keyword::Configuration
        )
    )
}

/// Whether a keyword begins a program organization unit or a configuration.
///
/// Distinct from [`starts_top_level_item`] because `VAR_GLOBAL` is legal
/// *inside* a `CONFIGURATION`, so it cannot be treated there as the start of
/// something new.
fn starts_pou_or_configuration(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Keyword(
            Keyword::Program
                | Keyword::Function
                | Keyword::FunctionBlock
                | Keyword::Type
                | Keyword::Configuration
        )
    )
}

/// Whether a token ends the statement list it is met in.
///
/// Deliberately generous: `IF x THEN y := 1; END_WHILE` should report a
/// mismatched closing keyword, not swallow the rest of the file looking for
/// `END_IF`. Everything that could plausibly close *some* enclosing block
/// stops the list, and the caller says what it actually expected.
fn ends_statement_list(kind: TokenKind) -> bool {
    is_end_keyword(kind)
        || var_section_of(kind).is_some()
        || starts_top_level_item(kind)
        || matches!(
            kind,
            TokenKind::Keyword(
                Keyword::Else | Keyword::Elsif | Keyword::Until | Keyword::Resource | Keyword::Task
            ) | TokenKind::Eof
        )
}

// ---------------------------------------------------------------------------
// Operator tables
//
// One function per precedence level, so that `parse_binary_level` can be
// driven by a function pointer and every level is written once.
// ---------------------------------------------------------------------------

/// `OR`. Annex A `or_expression`; Table 71's loosest binary row.
fn match_or(kind: TokenKind) -> Option<BinaryOp> {
    matches!(kind, TokenKind::Keyword(Keyword::Or)).then_some(BinaryOp::Or)
}

/// `XOR`. Annex A `xor_expression`.
fn match_xor(kind: TokenKind) -> Option<BinaryOp> {
    matches!(kind, TokenKind::Keyword(Keyword::Xor)).then_some(BinaryOp::Xor)
}

/// `AND` and its other spelling `&`. Annex A `and_expression`.
fn match_and(kind: TokenKind) -> Option<BinaryOp> {
    matches!(
        kind,
        TokenKind::Keyword(Keyword::And) | TokenKind::Punct(Punct::Ampersand)
    )
    .then_some(BinaryOp::And)
}

/// `=` and `<>`. Annex A calls this level `comparison`; see the module docs.
fn match_equality(kind: TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::Punct(Punct::Equal) => Some(BinaryOp::Eq),
        TokenKind::Punct(Punct::NotEqual) => Some(BinaryOp::Ne),
        _ => None,
    }
}

/// `<`, `>`, `<=`, `>=`. Annex A calls this level `equ_expression`.
fn match_relational(kind: TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::Punct(Punct::Less) => Some(BinaryOp::Lt),
        TokenKind::Punct(Punct::Greater) => Some(BinaryOp::Gt),
        TokenKind::Punct(Punct::LessEqual) => Some(BinaryOp::Le),
        TokenKind::Punct(Punct::GreaterEqual) => Some(BinaryOp::Ge),
        _ => None,
    }
}

/// `+` and `-`. Annex A `add_expression`.
fn match_additive(kind: TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::Punct(Punct::Plus) => Some(BinaryOp::Add),
        TokenKind::Punct(Punct::Minus) => Some(BinaryOp::Sub),
        _ => None,
    }
}

/// `*`, `/` and `MOD`. Annex A `term`.
fn match_multiplicative(kind: TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::Punct(Punct::Star) => Some(BinaryOp::Mul),
        TokenKind::Punct(Punct::Slash) => Some(BinaryOp::Div),
        TokenKind::Keyword(Keyword::Mod) => Some(BinaryOp::Mod),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The parser
// ---------------------------------------------------------------------------

struct Parser<'a> {
    source: &'a str,
    stream: &'a TokenStream,
    tokens: &'a [Token],
    /// The stream's last token, returned by `peek` past the end so that no
    /// lookahead has to deal with an absent token.
    eof: Token,
    pos: usize,
    dialect: &'a Dialect,
    diags: Diagnostics,
    next_id: u32,
    /// Current nesting depth, in tree levels rather than in tokens.
    depth: u32,
    /// Set once the depth bound has been exceeded.
    ///
    /// A file nested deeper than the bound is not a file with a typo in it, so
    /// once the bound is hit salman stops parsing rather than unwinding through
    /// a hundred and twenty-eight frames emitting a cascade of consequences.
    /// Every loop checks this, and every diagnostic after it is suppressed.
    fused: bool,
}

impl<'a> Parser<'a> {
    // -- source, tokens, ids -------------------------------------------------

    /// The source text a span covers, or `""` if the span is out of range.
    fn text(&self, span: Span) -> &'a str {
        self.source
            .get(span.start as usize..span.end as usize)
            .unwrap_or("")
    }

    fn peek(&self) -> Token {
        self.tokens.get(self.pos).copied().unwrap_or(self.eof)
    }

    fn peek_at(&self, offset: usize) -> Token {
        self.tokens
            .get(self.pos.saturating_add(offset))
            .copied()
            .unwrap_or(self.eof)
    }

    /// Consumes and returns the next token. At end of input the position does
    /// not move, so no loop can run off the end by bumping.
    fn bump(&mut self) -> Token {
        let token = self.peek();
        if !matches!(token.kind, TokenKind::Eof) {
            self.pos = self.pos.saturating_add(1);
        }
        token
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn at_punct(&self, punct: Punct) -> bool {
        matches!(self.peek().kind, TokenKind::Punct(p) if p == punct)
    }

    fn at_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.peek().kind, TokenKind::Keyword(k) if k == keyword)
    }

    fn eat_punct(&mut self, punct: Punct) -> bool {
        if self.at_punct(punct) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_keyword(&mut self, keyword: Keyword) -> bool {
        if self.at_keyword(keyword) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn next_node_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn expr(&mut self, kind: ExprKind, span: Span) -> Expr {
        Expr {
            id: self.next_node_id(),
            kind,
            span,
        }
    }

    fn error_expr(&mut self, span: Span) -> Expr {
        self.expr(ExprKind::Error, span)
    }

    fn stmt(&mut self, kind: StmtKind, span: Span) -> Stmt {
        Stmt {
            id: self.next_node_id(),
            kind,
            span,
        }
    }

    // -- diagnostics ---------------------------------------------------------

    /// Records a diagnostic, unless the parser has been fused by a depth
    /// failure — in which case everything after it is a consequence, not a
    /// cause.
    fn push(&mut self, diagnostic: Diagnostic) {
        if self.fused {
            return;
        }
        self.diags.push(diagnostic);
    }

    fn error_at(
        &mut self,
        code: salman_core::diag::DiagCode,
        span: Span,
        message: impl Into<String>,
        label: impl Into<String>,
    ) {
        self.push(Diagnostic::error(code, message).with_primary(span, label));
    }

    // -- depth bounding ------------------------------------------------------

    /// Whether a chain `extra` levels deep still fits inside the dialect's
    /// nesting bound.
    fn has_depth_for(&self, extra: u32) -> bool {
        self.depth.saturating_add(extra) <= self.dialect.max_nesting_depth
    }

    /// Enters one level of nesting. Returns `false` — having reported and fused
    /// — when the bound is reached; the caller must then return without calling
    /// [`Parser::leave`].
    fn enter(&mut self) -> bool {
        if self.fused {
            return false;
        }
        if !self.has_depth_for(1) {
            let span = self.peek().span;
            self.report_too_deep(span);
            return false;
        }
        self.depth = self.depth.saturating_add(1);
        true
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Reports the depth bound and fuses the parser.
    ///
    /// Pushed straight onto the list rather than through [`Parser::push`],
    /// because this is the one diagnostic that must survive fusing.
    fn report_too_deep(&mut self, span: Span) {
        if self.fused {
            return;
        }
        let limit = self.dialect.max_nesting_depth;
        self.diags.push(
            Diagnostic::error(
                codes::E_PARSE_NESTING_TOO_DEEP,
                format!("this file nests more than {limit} levels deep"),
            )
            .with_primary(span, "salman stopped parsing here")
            .with_note(
                "The bound is not a style rule. This tree is built and dropped with recursion, \
                 so unbounded nesting in a file salman did not write is a stack overflow waiting \
                 to happen. A chain of operators counts too: `a + a + a + …` builds a tree as \
                 deep as the chain is long.",
            )
            .with_dialect_rule(self.dialect.rule(
                "maximum nesting depth",
                &format!("{limit} levels of expressions, statements and operator chains"),
            )),
        );
        self.fused = true;
    }

    // -- expecting things ----------------------------------------------------

    fn expect_punct(&mut self, punct: Punct) -> Option<Token> {
        if self.at_punct(punct) {
            return Some(self.bump());
        }
        let found = self.peek();
        self.error_at(
            codes::E_EXPECTED_TOKEN,
            found.span,
            format!("expected `{}`, found {}", punct.text(), found.describe()),
            format!("expected `{}` here", punct.text()),
        );
        None
    }

    fn expect_keyword(&mut self, keyword: Keyword) -> Option<Token> {
        if self.at_keyword(keyword) {
            return Some(self.bump());
        }
        let found = self.peek();
        self.error_at(
            codes::E_EXPECTED_TOKEN,
            found.span,
            format!("expected `{}`, found {}", keyword.text(), found.describe()),
            format!("expected `{}` here", keyword.text()),
        );
        None
    }

    /// Consumes the keyword that closes a block, reporting the opener's span as
    /// context when it is missing. Returns the span to end the construct at.
    fn expect_end(&mut self, keyword: Keyword, opener: Span, opener_text: &str) -> Span {
        if self.at_keyword(keyword) {
            return self.bump().span;
        }
        let found = self.peek();
        self.push(
            Diagnostic::error(
                codes::E_UNCLOSED_BLOCK,
                format!("`{opener_text}` is never closed"),
            )
            .with_primary(
                found.span,
                format!(
                    "expected `{}` here, found {}",
                    keyword.text(),
                    found.describe()
                ),
            )
            .with_secondary(opener, format!("`{opener_text}` starts here")),
        );
        found.span
    }

    /// Consumes an identifier and turns it into a [`Name`].
    ///
    /// Returns `None`, having said why, when the next token is not an
    /// identifier or when the identifier rules refuse it. The only refusal that
    /// can arise from lexer output is an over-long identifier, which the lexer
    /// has already reported as [`codes::E_IDENT_TOO_LONG`]; salman reports it
    /// again here so that `parse` is self-contained — an error node in the tree
    /// always has a diagnostic of its own beside it.
    fn expect_name(&mut self) -> Option<Name> {
        let token = self.peek();
        if !matches!(token.kind, TokenKind::Ident) {
            self.error_at(
                codes::E_EXPECTED_TOKEN,
                token.span,
                format!("expected a name, found {}", token.describe()),
                "expected a name here",
            );
            return None;
        }
        self.bump();
        match Ident::new(self.text(token.span)) {
            Ok(ident) => Some(Name {
                ident,
                span: token.span,
            }),
            Err(err) => {
                self.error_at(
                    codes::E_BAD_NAME,
                    token.span,
                    format!("this cannot be used as a name: {err}"),
                    "not a usable identifier",
                );
                None
            }
        }
    }

    // -- recovery ------------------------------------------------------------

    /// Skips to just past the next `;`, or up to the next keyword that closes
    /// or opens a block. Consumes at least one token unless already stopped.
    fn recover_statement(&mut self) {
        while !self.fused && !self.at_eof() {
            if self.at_punct(Punct::Semicolon) {
                self.bump();
                return;
            }
            if ends_statement_list(self.peek().kind) {
                return;
            }
            self.bump();
        }
    }

    /// Skips to the start of the next top-level item.
    fn recover_top_level(&mut self) {
        while !self.fused && !self.at_eof() {
            if starts_top_level_item(self.peek().kind) {
                return;
            }
            self.bump();
        }
    }

    // -- top level -----------------------------------------------------------

    fn parse_items(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while !self.fused && !self.at_eof() {
            let before = self.pos;
            match self.peek().kind {
                TokenKind::Keyword(Keyword::Program) => {
                    if let Some(pou) = self.parse_pou(PouKind::Program) {
                        items.push(Item::Pou(pou));
                    }
                }
                TokenKind::Keyword(Keyword::Function) => {
                    if let Some(pou) = self.parse_pou(PouKind::Function) {
                        items.push(Item::Pou(pou));
                    }
                }
                TokenKind::Keyword(Keyword::FunctionBlock) => {
                    if let Some(pou) = self.parse_pou(PouKind::FunctionBlock) {
                        items.push(Item::Pou(pou));
                    }
                }
                TokenKind::Keyword(Keyword::Type) => items.push(Item::Types(self.parse_types())),
                TokenKind::Keyword(Keyword::VarGlobal) => {
                    items.push(Item::Globals(self.parse_var_block()));
                }
                TokenKind::Keyword(Keyword::Configuration) => {
                    if let Some(config) = self.parse_configuration() {
                        items.push(Item::Configuration(config));
                    }
                }
                TokenKind::Keyword(k) if k.is_unimplemented_construct() => {
                    self.report_unimplemented(k);
                    self.bump();
                    self.recover_top_level();
                }
                _ => {
                    let found = self.peek();
                    self.push(
                        Diagnostic::error(
                            codes::E_EXPECTED_DECLARATION,
                            format!("expected a declaration, found {}", found.describe()),
                        )
                        .with_primary(
                            found.span,
                            "a file holds PROGRAM, FUNCTION, FUNCTION_BLOCK, TYPE, VAR_GLOBAL \
                             and CONFIGURATION declarations",
                        )
                        .with_clause(clause::PROGRAM_ORGANIZATION_UNITS),
                    );
                    self.bump();
                    self.recover_top_level();
                }
            }
            if self.pos == before {
                self.bump();
            }
        }
        items
    }

    fn report_unimplemented(&mut self, keyword: Keyword) {
        let span = self.peek().span;
        self.push(
            Diagnostic::error(
                codes::U_UNIMPLEMENTED_CONSTRUCT,
                format!("salman does not implement `{}`", keyword.text()),
            )
            .with_primary(span, "this construct is reserved but not implemented")
            .with_note(
                "salman implements Structured Text. The graphical languages, Instruction List \
                 and the Edition 3 object-oriented extensions are not implemented; their \
                 keywords are reserved so that meeting one says so instead of producing a \
                 baffling syntax error.",
            ),
        );
    }

    // -- program organization units -----------------------------------------

    fn parse_pou(&mut self, kind: PouKind) -> Option<Pou> {
        let opener = self.bump().span;
        let Some(name) = self.expect_name() else {
            self.recover_top_level();
            return None;
        };

        // A FUNCTION declares the type of the value it returns; the other two
        // kinds do not return a value at all. IEC 61131-3:2013 §6.6.2
        // "Functions".
        let return_type = if kind == PouKind::Function {
            if self.expect_punct(Punct::Colon).is_some() {
                Some(self.parse_type_ref())
            } else {
                Some(TypeRef::Error(name.span))
            }
        } else {
            None
        };

        let mut var_blocks = Vec::new();
        let mut body = Vec::new();
        let end_keyword = match kind {
            PouKind::Program => Keyword::EndProgram,
            PouKind::Function => Keyword::EndFunction,
            PouKind::FunctionBlock => Keyword::EndFunctionBlock,
        };

        while !self.fused && !self.at_eof() && !self.at_keyword(end_keyword) {
            let before = self.pos;
            if var_section_of(self.peek().kind).is_some() {
                // salman policy: variable blocks are accepted anywhere in a POU,
                // not only before the first statement. Whether the standard's
                // grammar permits that is a question for the checker, which can
                // say so with the whole POU in view; refusing it here would turn
                // an ordering complaint into a cascade of syntax errors.
                var_blocks.push(self.parse_var_block());
            } else if starts_top_level_item(self.peek().kind) {
                break;
            } else {
                body.append(&mut self.parse_statements());
                if self.pos == before {
                    let found = self.peek();
                    self.error_at(
                        codes::E_EXPECTED_STATEMENT,
                        found.span,
                        format!("expected a statement, found {}", found.describe()),
                        format!("expected a statement or `{}` here", end_keyword.text()),
                    );
                    self.bump();
                }
            }
            if self.pos == before {
                self.bump();
            }
        }

        let end = self.expect_end(end_keyword, opener, kind.keyword());
        self.eat_punct(Punct::Semicolon);
        Some(Pou {
            kind,
            name,
            return_type,
            var_blocks,
            body,
            span: opener.join(end),
        })
    }

    // -- variable blocks -----------------------------------------------------

    fn parse_var_block(&mut self) -> VarBlock {
        let opener_token = self.bump();
        let section = var_section_of(opener_token.kind).unwrap_or(VarSection::Local);

        let mut qualifiers = VarQualifiers::default();
        loop {
            if self.eat_keyword(Keyword::Retain) {
                qualifiers.retention = Retention::Retain;
            } else if self.eat_keyword(Keyword::NonRetain) {
                qualifiers.retention = Retention::NonRetain;
            } else if self.eat_keyword(Keyword::Constant) {
                qualifiers.constant = true;
            } else if self.eat_keyword(Keyword::Persistent) {
                qualifiers.persistent = true;
            } else {
                break;
            }
        }

        let mut decls = Vec::new();
        while !self.fused
            && !self.at_eof()
            && !is_end_keyword(self.peek().kind)
            && var_section_of(self.peek().kind).is_none()
            && !starts_top_level_item(self.peek().kind)
        {
            let before = self.pos;
            if let Some(decl) = self.parse_var_decl() {
                decls.push(decl);
            }
            if self.pos == before {
                self.bump();
            }
        }

        let end = self.expect_end(Keyword::EndVar, opener_token.span, section.keyword());
        self.eat_punct(Punct::Semicolon);
        VarBlock {
            section,
            qualifiers,
            decls,
            span: opener_token.span.join(end),
        }
    }

    /// One declaration: `A, B : DINT := 0;`, `S AT %IX0.0 : BOOL;`.
    fn parse_var_decl(&mut self) -> Option<VarDecl> {
        let start = self.peek().span;
        let mut names = Vec::new();
        loop {
            let Some(name) = self.expect_name() else {
                self.recover_statement();
                return None;
            };
            names.push(name);
            if self.at_punct(Punct::Dot) {
                // `Cell.Motor.Speed AT %QW1 : INT;` — the VAR_CONFIG and
                // VAR_ACCESS instance path. The AST has nowhere to put a path,
                // so salman says so rather than mangling it into a name.
                let span = self.peek().span;
                self.push(
                    Diagnostic::error(
                        codes::U_UNIMPLEMENTED_CONSTRUCT,
                        "salman does not implement instance paths in a declaration",
                    )
                    .with_primary(span, "a declared name may not be qualified")
                    .with_note(
                        "VAR_CONFIG and VAR_ACCESS name variables by their path through the \
                         configuration. salman parses neither form yet.",
                    )
                    .with_clause(clause::VARIABLE_SECTIONS),
                );
                self.recover_statement();
                return None;
            }
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }

        let mut located_at = None;
        let mut located_at_span = None;
        if self.at_keyword(Keyword::At) {
            let at_span = self.bump().span;
            let token = self.peek();
            if let TokenKind::DirectAddress(index) = token.kind {
                self.bump();
                located_at = self.stream.address(index).cloned();
                located_at_span = Some(at_span.join(token.span));
            } else {
                self.push(
                    Diagnostic::error(
                        codes::E_EXPECTED_TOKEN,
                        format!("expected a direct address, found {}", token.describe()),
                    )
                    .with_primary(
                        token.span,
                        "`AT` must be followed by an address like `%IX0.0`",
                    )
                    .with_clause(clause::DIRECTLY_REPRESENTED_VARIABLES),
                );
            }
        }

        if self.expect_punct(Punct::Colon).is_none() {
            self.recover_statement();
            return None;
        }
        let ty = self.parse_type_ref();
        let init = if self.eat_punct(Punct::Assign) {
            Some(self.parse_expr())
        } else {
            None
        };
        let end = self.peek().span;
        if self.expect_punct(Punct::Semicolon).is_none() {
            self.recover_statement();
        }
        Some(VarDecl {
            names,
            located_at,
            located_at_span,
            ty,
            init,
            span: start.join(end),
        })
    }

    // -- types ---------------------------------------------------------------

    /// A type as written in a declaration, after the `:`.
    fn parse_type_ref(&mut self) -> TypeRef {
        let token = self.peek();
        match token.kind {
            TokenKind::Keyword(Keyword::Array) => self.parse_array_type(),
            TokenKind::TypeName(ty @ (ElementaryType::String | ElementaryType::WString)) => {
                self.bump();
                self.parse_string_type(ty, token.span)
            }
            TokenKind::TypeName(ty) => {
                self.bump();
                let base = TypeRef::Elementary {
                    ty,
                    span: token.span,
                };
                self.maybe_subrange(base, token.span)
            }
            TokenKind::Ident => {
                let Some(name) = self.expect_name() else {
                    return TypeRef::Error(token.span);
                };
                let span = name.span;
                self.maybe_subrange(TypeRef::Named(name), span)
            }
            TokenKind::Keyword(Keyword::Struct) | TokenKind::Punct(Punct::LParen) => {
                let what = if matches!(token.kind, TokenKind::Punct(Punct::LParen)) {
                    "an enumeration"
                } else {
                    "a structure"
                };
                self.push(
                    Diagnostic::error(
                        codes::U_UNIMPLEMENTED_CONSTRUCT,
                        format!("salman does not implement {what} declared inline"),
                    )
                    .with_primary(
                        token.span,
                        "declare this type in a `TYPE ... END_TYPE` block",
                    )
                    .with_note(
                        "salman's tree has one place for a structure or an enumeration: a named \
                         type declaration. Naming the type is also what lets a diagnostic \
                         elsewhere say which type it means.",
                    )
                    .with_clause(clause::USER_DEFINED_DATA_TYPES),
                );
                // Skip the whole inline type rather than resynchronising on the
                // next `;`, which inside a STRUCT would stop after its first
                // field and turn one refusal into a run of consequences.
                self.skip_inline_type();
                TypeRef::Error(token.span)
            }
            _ => {
                self.error_at(
                    codes::E_EXPECTED_TYPE,
                    token.span,
                    format!("expected a type, found {}", token.describe()),
                    "expected a type here",
                );
                TypeRef::Error(token.span)
            }
        }
    }

    /// Consumes an inline `STRUCT ... END_STRUCT` or `( ... )` salman refuses,
    /// leaving the position where the declaration's `;` should be.
    fn skip_inline_type(&mut self) {
        if self.at_keyword(Keyword::Struct) {
            while !self.fused && !self.at_eof() && !self.at_keyword(Keyword::EndStruct) {
                self.bump();
            }
            self.eat_keyword(Keyword::EndStruct);
            return;
        }
        let mut depth = 0u32;
        while !self.fused && !self.at_eof() {
            if self.at_punct(Punct::LParen) {
                depth = depth.saturating_add(1);
            } else if self.at_punct(Punct::RParen) {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    self.bump();
                    return;
                }
            } else if self.at_punct(Punct::Semicolon) {
                return;
            }
            self.bump();
        }
    }

    /// `STRING`, `STRING[80]` or `STRING(80)`.
    ///
    /// Both bracket forms are accepted. The bracket spelling is the one Annex A
    /// gives; the parenthesised one is what a good deal of vendor code is
    /// written in, and refusing it buys nothing.
    fn parse_string_type(&mut self, ty: ElementaryType, start: Span) -> TypeRef {
        let close = if self.at_punct(Punct::LBracket) {
            Punct::RBracket
        } else if self.at_punct(Punct::LParen) {
            Punct::RParen
        } else {
            return TypeRef::String {
                ty,
                max_len: None,
                span: start,
            };
        };
        self.bump();
        let len = self.parse_expr();
        let end = self.expect_punct(close).map_or(len.span, |t| t.span);
        TypeRef::String {
            ty,
            max_len: Some(Box::new(len)),
            span: start.join(end),
        }
    }

    /// `ARRAY [0..9, 1..4] OF REAL`.
    fn parse_array_type(&mut self) -> TypeRef {
        let start = self.bump().span;
        let mut dims = Vec::new();
        if self.expect_punct(Punct::LBracket).is_some() {
            loop {
                if self.fused || self.at_eof() {
                    break;
                }
                if self.at_punct(Punct::Star) {
                    let span = self.bump().span;
                    self.push(
                        Diagnostic::error(
                            codes::U_UNIMPLEMENTED_CONSTRUCT,
                            "salman does not implement variable-length arrays",
                        )
                        .with_primary(span, "`ARRAY [*]` needs bounds salman can see")
                        .with_clause(clause::USER_DEFINED_DATA_TYPES),
                    );
                } else {
                    let before = self.pos;
                    let low = self.parse_expr();
                    self.expect_punct(Punct::DotDot);
                    let high = self.parse_expr();
                    let span = low.span.join(high.span);
                    dims.push(ArrayDim { low, high, span });
                    if self.pos == before {
                        break;
                    }
                }
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
            self.expect_punct(Punct::RBracket);
        }
        self.expect_keyword(Keyword::Of);
        let element = self.parse_type_ref();
        let span = start.join(element.span());
        TypeRef::Array {
            dims,
            element: Box::new(element),
            span,
        }
    }

    /// Wraps `base` in a subrange when `( low .. high )` follows it.
    fn maybe_subrange(&mut self, base: TypeRef, start: Span) -> TypeRef {
        if !self.at_punct(Punct::LParen) {
            return base;
        }
        self.bump();
        let low = self.parse_expr();
        self.expect_punct(Punct::DotDot);
        let high = self.parse_expr();
        let end = self
            .expect_punct(Punct::RParen)
            .map_or(high.span, |t| t.span);
        TypeRef::Subrange {
            base: Box::new(base),
            low: Box::new(low),
            high: Box::new(high),
            span: start.join(end),
        }
    }

    /// A `TYPE ... END_TYPE` block.
    fn parse_types(&mut self) -> Vec<TypeDecl> {
        let opener = self.bump().span;
        let mut decls = Vec::new();
        while !self.fused
            && !self.at_eof()
            && !is_end_keyword(self.peek().kind)
            && !starts_top_level_item(self.peek().kind)
        {
            let before = self.pos;
            if let Some(decl) = self.parse_type_decl() {
                decls.push(decl);
            }
            if self.pos == before {
                self.bump();
            }
        }
        self.expect_end(Keyword::EndType, opener, "TYPE");
        self.eat_punct(Punct::Semicolon);
        decls
    }

    fn parse_type_decl(&mut self) -> Option<TypeDecl> {
        let start = self.peek().span;
        let Some(name) = self.expect_name() else {
            self.recover_statement();
            return None;
        };
        if self.expect_punct(Punct::Colon).is_none() {
            self.recover_statement();
            return None;
        }

        let kind = self.parse_type_decl_kind();
        let init = if self.eat_punct(Punct::Assign) {
            Some(self.parse_expr())
        } else {
            None
        };
        let end = self.peek().span;
        if self.expect_punct(Punct::Semicolon).is_none() {
            self.recover_statement();
        }
        Some(TypeDecl {
            name,
            kind,
            init,
            span: start.join(end),
        })
    }

    fn parse_type_decl_kind(&mut self) -> TypeDeclKind {
        let token = self.peek();
        match token.kind {
            TokenKind::Keyword(Keyword::Struct) => self.parse_struct_body(),
            TokenKind::Punct(Punct::LParen) => self.parse_enum_body(None),
            TokenKind::Keyword(Keyword::Array) => match self.parse_array_type() {
                TypeRef::Array { dims, element, .. } => TypeDeclKind::Array {
                    dims,
                    element: *element,
                },
                other => TypeDeclKind::Alias(other),
            },
            // `Speed : INT (0..100);` and `Colour : INT (Red, Green);` both
            // start `INT (`. Two tokens of lookahead separate them: an
            // enumeration lists names, so the token after `(` is an identifier
            // and the one after that closes the list, separates it or assigns a
            // value. Anything else is a subrange.
            TokenKind::TypeName(ty) if self.enum_base_follows() => {
                self.bump();
                self.parse_enum_body(Some(ty))
            }
            _ => {
                let ty = self.parse_type_ref();
                match ty {
                    TypeRef::Subrange {
                        base, low, high, ..
                    } => TypeDeclKind::Subrange {
                        base: *base,
                        low: *low,
                        high: *high,
                    },
                    TypeRef::Array { dims, element, .. } => TypeDeclKind::Array {
                        dims,
                        element: *element,
                    },
                    other => TypeDeclKind::Alias(other),
                }
            }
        }
    }

    /// Whether the current `TYPE_NAME (` begins an enumeration rather than a
    /// subrange. See the call site for why this needs three tokens.
    fn enum_base_follows(&self) -> bool {
        matches!(self.peek_at(1).kind, TokenKind::Punct(Punct::LParen))
            && matches!(self.peek_at(2).kind, TokenKind::Ident)
            && matches!(
                self.peek_at(3).kind,
                TokenKind::Punct(Punct::Comma | Punct::RParen | Punct::Assign)
            )
    }

    fn parse_struct_body(&mut self) -> TypeDeclKind {
        let opener = self.bump().span;
        let mut fields = Vec::new();
        while !self.fused
            && !self.at_eof()
            && !is_end_keyword(self.peek().kind)
            && !starts_top_level_item(self.peek().kind)
        {
            let before = self.pos;
            if let Some(field) = self.parse_var_decl() {
                fields.push(field);
            }
            if self.pos == before {
                self.bump();
            }
        }
        self.expect_end(Keyword::EndStruct, opener, "STRUCT");
        TypeDeclKind::Struct(fields)
    }

    fn parse_enum_body(&mut self, base: Option<ElementaryType>) -> TypeDeclKind {
        self.expect_punct(Punct::LParen);
        let mut values = Vec::new();
        loop {
            if self.fused || self.at_eof() || self.at_punct(Punct::RParen) {
                break;
            }
            let before = self.pos;
            match self.expect_name() {
                Some(name) => {
                    let value = if self.eat_punct(Punct::Assign) {
                        Some(self.parse_expr())
                    } else {
                        None
                    };
                    values.push(EnumValue { name, value });
                }
                None => break,
            }
            if self.pos == before {
                break;
            }
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::RParen);
        TypeDeclKind::Enum { base, values }
    }

    // -- configurations ------------------------------------------------------

    fn parse_configuration(&mut self) -> Option<ConfigurationDecl> {
        let opener = self.bump().span;
        let Some(name) = self.expect_name() else {
            self.recover_top_level();
            return None;
        };
        let mut var_blocks = Vec::new();
        let mut resources = Vec::new();
        while !self.fused
            && !self.at_eof()
            && !self.at_keyword(Keyword::EndConfiguration)
            && !starts_pou_or_configuration(self.peek().kind)
        {
            let before = self.pos;
            if var_section_of(self.peek().kind).is_some() {
                var_blocks.push(self.parse_var_block());
            } else if self.at_keyword(Keyword::Resource) {
                if let Some(resource) = self.parse_resource() {
                    resources.push(resource);
                }
            } else {
                let found = self.peek();
                self.push(
                    Diagnostic::error(
                        codes::E_EXPECTED_DECLARATION,
                        format!(
                            "expected `RESOURCE` or `VAR_GLOBAL`, found {}",
                            found.describe()
                        ),
                    )
                    .with_primary(
                        found.span,
                        "a configuration holds global variables and resources",
                    )
                    .with_note(
                        "salman requires tasks and program instances to sit inside a RESOURCE. \
                         The single-resource shorthand, which writes them directly in the \
                         configuration, is not implemented.",
                    )
                    .with_clause(clause::CONFIGURATION_ELEMENTS),
                );
                self.bump();
            }
            if self.pos == before {
                self.bump();
            }
        }
        let end = self.expect_end(Keyword::EndConfiguration, opener, "CONFIGURATION");
        self.eat_punct(Punct::Semicolon);
        Some(ConfigurationDecl {
            name,
            var_blocks,
            resources,
            span: opener.join(end),
        })
    }

    fn parse_resource(&mut self) -> Option<ResourceDecl> {
        let opener = self.bump().span;
        let name = self.expect_name()?;
        let on_type = if self.eat_keyword(Keyword::On) {
            self.expect_name()
        } else {
            None
        };
        let mut var_blocks = Vec::new();
        let mut tasks = Vec::new();
        let mut programs = Vec::new();
        while !self.fused
            && !self.at_eof()
            && !self.at_keyword(Keyword::EndResource)
            && !matches!(
                self.peek().kind,
                TokenKind::Keyword(
                    Keyword::Function
                        | Keyword::FunctionBlock
                        | Keyword::Type
                        | Keyword::Configuration
                )
            )
        {
            let before = self.pos;
            if var_section_of(self.peek().kind).is_some() {
                var_blocks.push(self.parse_var_block());
            } else if self.at_keyword(Keyword::Task) {
                if let Some(task) = self.parse_task() {
                    tasks.push(task);
                }
            } else if self.at_keyword(Keyword::Program) {
                if let Some(instance) = self.parse_program_instance() {
                    programs.push(instance);
                }
            } else {
                let found = self.peek();
                self.error_at(
                    codes::E_EXPECTED_DECLARATION,
                    found.span,
                    format!("expected `TASK` or `PROGRAM`, found {}", found.describe()),
                    "a resource holds global variables, tasks and program instances",
                );
                self.bump();
            }
            if self.pos == before {
                self.bump();
            }
        }
        let end = self.expect_end(Keyword::EndResource, opener, "RESOURCE");
        self.eat_punct(Punct::Semicolon);
        Some(ResourceDecl {
            name,
            on_type,
            var_blocks,
            tasks,
            programs,
            span: opener.join(end),
        })
    }

    /// `TASK Fast (INTERVAL := T#10ms, PRIORITY := 1);`
    fn parse_task(&mut self) -> Option<TaskDecl> {
        let opener = self.bump().span;
        let Some(name) = self.expect_name() else {
            self.recover_statement();
            return None;
        };
        let mut single = None;
        let mut interval = None;
        let mut priority = None;
        if self.eat_punct(Punct::LParen) {
            loop {
                if self.fused || self.at_eof() || self.at_punct(Punct::RParen) {
                    break;
                }
                let before = self.pos;
                let token = self.peek();
                match token.kind {
                    TokenKind::Keyword(Keyword::Single) => {
                        self.bump();
                        self.expect_punct(Punct::Assign);
                        single = Some(self.parse_expr());
                    }
                    TokenKind::Keyword(Keyword::Interval) => {
                        self.bump();
                        self.expect_punct(Punct::Assign);
                        interval = Some(self.parse_expr());
                    }
                    TokenKind::Keyword(Keyword::Priority) => {
                        self.bump();
                        self.expect_punct(Punct::Assign);
                        priority = Some(self.parse_expr());
                    }
                    _ => {
                        self.error_at(
                            codes::E_EXPECTED_TOKEN,
                            token.span,
                            format!(
                                "expected `SINGLE`, `INTERVAL` or `PRIORITY`, found {}",
                                token.describe()
                            ),
                            "not a task parameter",
                        );
                        self.bump();
                    }
                }
                if self.pos == before {
                    break;
                }
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
            self.expect_punct(Punct::RParen);
        }
        let end = self.peek().span;
        if self.expect_punct(Punct::Semicolon).is_none() {
            self.recover_statement();
        }
        Some(TaskDecl {
            name,
            single,
            interval,
            priority,
            span: opener.join(end),
        })
    }

    /// `PROGRAM Main WITH Fast : Conveyor;`
    fn parse_program_instance(&mut self) -> Option<ProgramInstance> {
        let opener = self.bump().span;
        let Some(name) = self.expect_name() else {
            self.recover_statement();
            return None;
        };
        let task = if self.eat_keyword(Keyword::With) {
            self.expect_name()
        } else {
            None
        };
        if self.expect_punct(Punct::Colon).is_none() {
            self.recover_statement();
            return None;
        }
        let Some(program_type) = self.expect_name() else {
            self.recover_statement();
            return None;
        };
        let end = self.peek().span;
        if self.expect_punct(Punct::Semicolon).is_none() {
            self.recover_statement();
        }
        Some(ProgramInstance {
            name,
            task,
            program_type,
            span: opener.join(end),
        })
    }

    // -- statements ----------------------------------------------------------

    /// A list of statements, ending at whatever closes the enclosing block.
    fn parse_statements(&mut self) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        while !self.fused && !self.at_eof() && !ends_statement_list(self.peek().kind) {
            let before = self.pos;
            let stmt = self.parse_stmt();
            stmts.push(stmt);
            if self.pos == before {
                self.bump();
            }
        }
        stmts
    }

    fn parse_stmt(&mut self) -> Stmt {
        if !self.enter() {
            let span = self.peek().span;
            return self.stmt(StmtKind::Error, span);
        }
        let stmt = self.parse_stmt_inner();
        self.leave();
        stmt
    }

    /// One statement. IEC 61131-3:2013 Table 72 "ST language statements".
    fn parse_stmt_inner(&mut self) -> Stmt {
        let token = self.peek();
        match token.kind {
            // Row 11: the empty statement.
            TokenKind::Punct(Punct::Semicolon) => {
                self.bump();
                self.stmt(StmtKind::Empty, token.span)
            }
            TokenKind::Keyword(Keyword::If) => self.parse_if(),
            TokenKind::Keyword(Keyword::Case) => self.parse_case(),
            TokenKind::Keyword(Keyword::For) => self.parse_for(),
            TokenKind::Keyword(Keyword::While) => self.parse_while(),
            TokenKind::Keyword(Keyword::Repeat) => self.parse_repeat(),
            // Row 3.
            TokenKind::Keyword(Keyword::Return) => self.parse_bare_statement(StmtKind::Return),
            // Row 10.
            TokenKind::Keyword(Keyword::Exit) => self.parse_bare_statement(StmtKind::Exit),
            // Row 9, new in Edition 3. Beckhoff's documentation still describes
            // CONTINUE as a non-standard extension, which is Edition-2-era
            // wording that predates the row; salman implements it as standard.
            // Only one source salman could read names row 9 as CONTINUE, and a
            // second corroborates the row position structurally rather than by
            // name, so the row number is the weaker half of this citation.
            TokenKind::Keyword(Keyword::Continue) => self.parse_bare_statement(StmtKind::Continue),
            TokenKind::Keyword(k) if k.is_unimplemented_construct() => {
                self.report_unimplemented(k);
                self.bump();
                self.recover_statement();
                self.stmt(StmtKind::Error, token.span)
            }
            _ => self.parse_expression_statement(),
        }
    }

    /// `RETURN;`, `EXIT;`, `CONTINUE;`.
    fn parse_bare_statement(&mut self, kind: StmtKind) -> Stmt {
        let start = self.bump().span;
        let end = self.peek().span;
        if self.expect_punct(Punct::Semicolon).is_none() {
            self.recover_statement();
        }
        self.stmt(kind, start.join(end))
    }

    /// Row 1 (`:=` and `?=`) and row 2 (a call used as a statement).
    fn parse_expression_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        let target = self.parse_expr();
        if matches!(target.kind, ExprKind::Error) {
            // Whatever went wrong has already been reported; saying "expected
            // `:=`" on top of it would only add noise.
            self.recover_statement();
            return self.stmt(StmtKind::Error, start);
        }

        if self.at_punct(Punct::Assign) || self.at_punct(Punct::AssignAttempt) {
            let attempt = self.at_punct(Punct::AssignAttempt);
            self.bump();
            let value = self.parse_expr();
            let end = self.peek().span;
            if self.expect_punct(Punct::Semicolon).is_none() {
                self.recover_statement();
            }
            let span = start.join(end);
            let kind = if attempt {
                // Parsed rather than refused, so that the message about
                // references naming what salman does not implement comes from
                // the checker and reads like an answer.
                // IEC 61131-3:2013 Table 52 "Assignment attempt".
                StmtKind::AssignAttempt { target, value }
            } else {
                StmtKind::Assign { target, value }
            };
            return self.stmt(kind, span);
        }

        if matches!(target.kind, ExprKind::Call { .. }) {
            let end = self.peek().span;
            if self.expect_punct(Punct::Semicolon).is_none() {
                self.recover_statement();
            }
            let span = start.join(end);
            return self.stmt(StmtKind::Call(target), span);
        }

        let found = self.peek();
        self.push(
            Diagnostic::error(
                codes::E_EXPECTED_STATEMENT,
                format!("expected `:=` or a call, found {}", found.describe()),
            )
            .with_primary(
                found.span,
                "a statement is an assignment, a call, or one of the control constructs",
            )
            .with_secondary(target.span, "this expression is not a statement on its own")
            .with_clause(clause::ST_STATEMENTS),
        );
        self.recover_statement();
        self.stmt(StmtKind::Error, start.join(found.span))
    }

    /// Row 4: `IF ... THEN ... ELSIF ... ELSE ... END_IF`.
    fn parse_if(&mut self) -> Stmt {
        let opener = self.bump().span;
        let mut branches = Vec::new();
        let mut branch_start = opener;
        loop {
            let condition = self.parse_expr();
            self.expect_keyword(Keyword::Then);
            let body = self.parse_statements();
            let end = self.peek().span;
            branches.push(IfBranch {
                condition,
                body,
                span: branch_start.join(end),
            });
            if self.fused || !self.at_keyword(Keyword::Elsif) {
                break;
            }
            branch_start = self.bump().span;
        }
        let else_body = if self.eat_keyword(Keyword::Else) {
            Some(self.parse_statements())
        } else {
            None
        };
        let end = self.expect_end(Keyword::EndIf, opener, "IF");
        self.eat_punct(Punct::Semicolon);
        self.stmt(
            StmtKind::If {
                branches,
                else_body,
            },
            opener.join(end),
        )
    }

    /// Row 7: `WHILE ... DO ... END_WHILE`.
    fn parse_while(&mut self) -> Stmt {
        let opener = self.bump().span;
        let condition = self.parse_expr();
        self.expect_keyword(Keyword::Do);
        let body = self.parse_statements();
        let end = self.expect_end(Keyword::EndWhile, opener, "WHILE");
        self.eat_punct(Punct::Semicolon);
        self.stmt(StmtKind::While { condition, body }, opener.join(end))
    }

    /// Row 8: `REPEAT ... UNTIL ... END_REPEAT`.
    fn parse_repeat(&mut self) -> Stmt {
        let opener = self.bump().span;
        let body = self.parse_statements();
        self.expect_keyword(Keyword::Until);
        let until = self.parse_expr();
        let end = self.expect_end(Keyword::EndRepeat, opener, "REPEAT");
        self.eat_punct(Punct::Semicolon);
        self.stmt(StmtKind::Repeat { body, until }, opener.join(end))
    }

    /// Row 6: `FOR v := a TO b BY c DO ... END_FOR`.
    ///
    /// Three things about `FOR` that no source salman could check settles, so
    /// each is salman's decision and each is marked in the code below:
    ///
    /// * **`TO` and `BY` are evaluated once, at loop entry.** Evaluating them
    ///   every pass would let a side effect in the bound change the trip count
    ///   mid-loop, which is not something a reader of the source can see.
    /// * **Assigning to the control variable inside the body is an error.**
    ///   The parser flags the case it can see — a statement in the body whose
    ///   whole left-hand side is the control variable, at any nesting. It
    ///   cannot see assignment through a `VAR_IN_OUT`, through an alias or by a
    ///   callee, and it does not pretend to: the general check belongs in the
    ///   checker, which knows what the names mean.
    /// * **The control variable's value after normal termination is
    ///   unspecified** in salman's documentation and deterministic in its
    ///   implementation. Code that reads it after the loop is relying on
    ///   something salman may change.
    fn parse_for(&mut self) -> Stmt {
        let opener = self.bump().span;
        let Some(variable) = self.expect_name() else {
            self.recover_statement();
            return self.stmt(StmtKind::Error, opener);
        };
        self.expect_punct(Punct::Assign);
        let from = self.parse_expr();
        self.expect_keyword(Keyword::To);
        // salman policy: `TO` is evaluated exactly once, at loop entry.
        let to = self.parse_expr();
        // salman policy: `BY` is evaluated exactly once, at loop entry, and is
        // 1 when it is absent.
        let by = if self.eat_keyword(Keyword::By) {
            Some(self.parse_expr())
        } else {
            None
        };
        self.expect_keyword(Keyword::Do);
        let body = self.parse_statements();
        let end = self.expect_end(Keyword::EndFor, opener, "FOR");
        self.eat_punct(Punct::Semicolon);

        // salman policy: the body may not assign to the control variable.
        let mut assignments = Vec::new();
        collect_assignments_to(&body, &variable, &mut assignments);
        for span in assignments {
            self.push(
                Diagnostic::error(
                    codes::E_FOR_CONTROL_VARIABLE_ASSIGNED,
                    format!("`{variable}` is the control variable of this `FOR` loop"),
                )
                .with_primary(span, "assigning to it here is a salman rule violation")
                .with_secondary(variable.span, "the loop controls this variable")
                .with_note(
                    "This is a salman rule, not a rule salman could verify in the standard: no \
                     public source available to salman states whether IEC 61131-3:2013 Table 72 \
                     \"ST language statements\" forbids it. salman refuses it because a loop \
                     whose counter the body moves has a trip count no reader can work out from \
                     the source. Use a WHILE loop when the counter really is yours to move.",
                )
                .with_clause(clause::ST_STATEMENTS),
            );
        }

        // salman policy: after the loop ends normally the control variable's
        // value is deterministic but undocumented. Nothing here records it,
        // because recording it in the tree would make it a promise.
        self.stmt(
            StmtKind::For {
                variable,
                from,
                to,
                by,
                body,
            },
            opener.join(end),
        )
    }

    /// Row 5: `CASE ... OF ... ELSE ... END_CASE`.
    fn parse_case(&mut self) -> Stmt {
        let opener = self.bump().span;
        let selector = self.parse_expr();
        self.expect_keyword(Keyword::Of);
        let mut arms = Vec::new();
        while !self.fused
            && !self.at_eof()
            && !self.at_keyword(Keyword::Else)
            && !self.at_keyword(Keyword::EndCase)
            && !is_end_keyword(self.peek().kind)
            && !starts_top_level_item(self.peek().kind)
        {
            let before = self.pos;
            if self.looks_like_case_label() {
                arms.push(self.parse_case_arm());
            } else {
                let found = self.peek();
                self.push(
                    Diagnostic::error(
                        codes::E_EXPECTED_STATEMENT,
                        format!("expected a `CASE` label, found {}", found.describe()),
                    )
                    .with_primary(found.span, "every arm of a CASE begins with `label:`")
                    .with_clause(clause::ST_STATEMENTS),
                );
                self.recover_statement();
            }
            if self.pos == before {
                self.bump();
            }
        }
        let else_body = if self.eat_keyword(Keyword::Else) {
            Some(self.parse_statements())
        } else {
            None
        };
        let end = self.expect_end(Keyword::EndCase, opener, "CASE");
        self.eat_punct(Punct::Semicolon);
        self.check_case_labels(&arms);
        self.stmt(
            StmtKind::Case {
                selector,
                arms,
                else_body,
            },
            opener.join(end),
        )
    }

    /// Whether what follows is a `CASE` label list rather than a statement.
    ///
    /// The one place the parser needs unbounded lookahead. A label list is
    /// made only of constants, `,`, `..` and a sign, and ends at a `:` that is
    /// not part of a `:=`; a statement reaches `:=`, `(`, `.` or `;` first. The
    /// scan is capped at [`MAX_CASE_LABEL_LOOKAHEAD`] tokens so that a file of
    /// commas cannot make it quadratic.
    fn looks_like_case_label(&self) -> bool {
        let mut index = self.pos;
        let mut saw_constant = false;
        let limit = self.pos.saturating_add(MAX_CASE_LABEL_LOOKAHEAD);
        while index < limit {
            let Some(token) = self.tokens.get(index) else {
                return false;
            };
            match token.kind {
                TokenKind::Literal(_) | TokenKind::Ident | TokenKind::TypeName(_) => {
                    saw_constant = true;
                }
                TokenKind::Punct(
                    Punct::Hash | Punct::Comma | Punct::DotDot | Punct::Minus | Punct::Plus,
                ) => {}
                TokenKind::Punct(Punct::Colon) => return saw_constant,
                _ => return false,
            }
            index = index.saturating_add(1);
        }
        false
    }

    fn parse_case_arm(&mut self) -> CaseArm {
        let start = self.peek().span;
        let mut labels = Vec::new();
        loop {
            if self.fused || self.at_eof() {
                break;
            }
            let before = self.pos;
            labels.push(self.parse_case_label());
            if self.pos == before {
                break;
            }
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::Colon);
        let mut body = Vec::new();
        while !self.fused
            && !self.at_eof()
            && !ends_statement_list(self.peek().kind)
            && !self.looks_like_case_label()
        {
            let before = self.pos;
            let stmt = self.parse_stmt();
            body.push(stmt);
            if self.pos == before {
                self.bump();
            }
        }
        let end = self.peek().span;
        CaseArm {
            labels,
            body,
            span: start.join(end),
        }
    }

    fn parse_case_label(&mut self) -> CaseLabel {
        let low = self.parse_expr();
        if self.eat_punct(Punct::DotDot) {
            let high = self.parse_expr();
            CaseLabel::Range { low, high }
        } else {
            CaseLabel::Single(low)
        }
    }

    /// Refuses duplicate and overlapping `CASE` labels.
    ///
    /// **This is a salman rule.** No source available to salman states whether
    /// IEC 61131-3:2013 Table 72 "ST language statements" forbids two arms that
    /// can both match; salman refuses them because which arm wins would then
    /// depend on the order the arms happen to be written in, and a reader
    /// cannot see that from one arm.
    ///
    /// Only labels whose value the parser can work out are checked: integer and
    /// boolean literals, with a sign or parentheses around them. A label that
    /// names a constant — `Limits.Max`, or an enumeration value — has no value
    /// here, so those are compared by spelling only and the real check belongs
    /// in the checker, which knows what the names mean. `Colour#Red` and a bare
    /// `Red` are two different spellings and are not compared with each other.
    fn check_case_labels(&mut self, arms: &[CaseArm]) {
        let mut ranges: Vec<(i128, i128, Span)> = Vec::new();
        let mut named: Vec<(String, Span)> = Vec::new();
        for arm in arms {
            for label in &arm.labels {
                match label {
                    CaseLabel::Single(expr) => {
                        if let Some(value) = const_int(expr) {
                            ranges.push((value, value, expr.span));
                        } else if let Some(key) = spelling_key(expr) {
                            named.push((key, expr.span));
                        }
                    }
                    CaseLabel::Range { low, high } => {
                        if let (Some(lo), Some(hi)) = (const_int(low), const_int(high)) {
                            ranges.push((lo, hi, low.span.join(high.span)));
                        }
                    }
                }
            }
        }

        ranges.sort_by_key(|(low, high, span)| (*low, *high, span.start));
        let mut covered: Option<(i128, i128, Span)> = None;
        for (low, high, span) in ranges {
            match covered {
                Some((clow, chigh, cspan)) if low <= chigh => {
                    let duplicate = low == clow && high == chigh;
                    self.report_case_label_clash(span, cspan, duplicate);
                    covered = Some((clow, chigh.max(high), cspan));
                }
                _ => covered = Some((low, high, span)),
            }
        }

        named.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.start.cmp(&b.1.start)));
        for pair in named.windows(2) {
            if let [(first_key, first_span), (second_key, second_span)] = pair
                && first_key == second_key
            {
                self.report_case_label_clash(*second_span, *first_span, true);
            }
        }
    }

    fn report_case_label_clash(&mut self, span: Span, earlier: Span, duplicate: bool) {
        let (code, message, label) = if duplicate {
            (
                codes::E_DUPLICATE_CASE_LABEL,
                "this `CASE` label is already covered by an earlier one",
                "duplicate label",
            )
        } else {
            (
                codes::E_OVERLAPPING_CASE_LABELS,
                "this `CASE` label overlaps an earlier one",
                "overlapping label",
            )
        };
        self.push(
            Diagnostic::error(code, message)
                .with_primary(span, label)
                .with_secondary(earlier, "already covered here")
                .with_note(
                    "This is a salman rule, not a rule salman could verify in the standard: no \
                     public source available to salman states whether IEC 61131-3:2013 Table 72 \
                     \"ST language statements\" forbids duplicate or overlapping CASE labels. \
                     salman refuses them because which arm runs would otherwise depend on the \
                     order the arms happen to be written in.",
                )
                .with_clause(clause::ST_STATEMENTS),
        );
    }

    // -- expressions ---------------------------------------------------------

    /// An expression, at the loosest precedence.
    ///
    /// This is the only entry point that counts against the nesting budget, so
    /// every recursion back into it — a parenthesised subexpression, an
    /// argument, an array index — costs one level.
    fn parse_expr(&mut self) -> Expr {
        if !self.enter() {
            let span = self.peek().span;
            return self.error_expr(span);
        }
        let expr = self.parse_or();
        self.leave();
        expr
    }

    /// One left-associative precedence level.
    ///
    /// `chain` counts how deep the tree this loop is building has become, and
    /// is charged against the nesting budget: the tree `a+a+a+…` is as deep as
    /// the chain is long, and it is dropped by recursion.
    fn parse_binary_level(
        &mut self,
        next: fn(&mut Self) -> Expr,
        matches_op: fn(TokenKind) -> Option<BinaryOp>,
    ) -> Expr {
        let mut lhs = next(self);
        let mut chain = 0u32;
        while !self.fused {
            let Some(op) = matches_op(self.peek().kind) else {
                break;
            };
            chain = chain.saturating_add(1);
            if !self.has_depth_for(chain) {
                let span = self.peek().span;
                self.report_too_deep(span);
                break;
            }
            self.bump();
            let rhs = next(self);
            let span = lhs.span.join(rhs.span);
            lhs = self.expr(
                ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            );
        }
        lhs
    }

    fn parse_or(&mut self) -> Expr {
        self.parse_binary_level(Self::parse_xor, match_or)
    }

    fn parse_xor(&mut self) -> Expr {
        self.parse_binary_level(Self::parse_and, match_xor)
    }

    fn parse_and(&mut self) -> Expr {
        self.parse_binary_level(Self::parse_equality, match_and)
    }

    /// `=` and `<>`. Annex A names this level `comparison`; see the module docs.
    fn parse_equality(&mut self) -> Expr {
        self.parse_binary_level(Self::parse_relational, match_equality)
    }

    /// `<`, `>`, `<=`, `>=`. Annex A names this level `equ_expression`.
    fn parse_relational(&mut self) -> Expr {
        self.parse_binary_level(Self::parse_additive, match_relational)
    }

    fn parse_additive(&mut self) -> Expr {
        self.parse_binary_level(Self::parse_multiplicative, match_additive)
    }

    fn parse_multiplicative(&mut self) -> Expr {
        self.parse_binary_level(Self::parse_power, match_multiplicative)
    }

    /// `**`.
    ///
    /// Left-associative, so `2 ** 3 ** 2` is `(2 ** 3) ** 2` = 64 rather than
    /// `2 ** (3 ** 2)` = 512.
    //
    // UNVERIFIED: no source salman could read states the associativity of `**`
    // itself. Table 71 fixes its precedence and says nothing about grouping,
    // and the Annex A production is a repetition, which is conventionally read
    // as left-associative. Three open implementations group it to the left, so
    // salman does too — but this is the weakest thing in the expression
    // grammar, and code that depends on it should be written with parentheses.
    fn parse_power(&mut self) -> Expr {
        let mut lhs = self.parse_unary();
        let mut chain = 0u32;
        while !self.fused && self.at_punct(Punct::Power) {
            chain = chain.saturating_add(1);
            if !self.has_depth_for(chain) {
                let span = self.peek().span;
                self.report_too_deep(span);
                break;
            }
            self.bump();
            let rhs = self.parse_unary();
            self.warn_on_bare_unary_operand(&lhs);
            self.warn_on_bare_unary_operand(&rhs);
            lhs = self.build_power(lhs, rhs);
        }
        lhs
    }

    /// Builds one `**` node, honouring the dialect's binding rule.
    ///
    /// Under [`UnaryPowerBinding::UnaryTighter`] — Edition 3, and both dialects
    /// salman ships — the tree is just `Pow(lhs, rhs)`, so `-2 ** 2` is
    /// `Pow(Neg(2), 2)` = 4. Under [`UnaryPowerBinding::PowerTighter`] — the
    /// Edition 2 ordering CODESYS and Beckhoff publish — a unary left operand is
    /// lifted back out over the `**`, giving `Neg(Pow(2, 2))` = -4. Doing it as
    /// a rotation here rather than as a second precedence chain keeps one
    /// grammar in the file, and composes correctly for stacked unaries and for
    /// chains of `**`.
    fn build_power(&mut self, lhs: Expr, rhs: Expr) -> Expr {
        let span = lhs.span.join(rhs.span);
        if self.dialect.unary_power_binding == UnaryPowerBinding::PowerTighter
            && let ExprKind::Unary { op, operand } = lhs.kind
        {
            let inner_span = operand.span.join(rhs.span);
            let inner = self.expr(
                ExprKind::Binary {
                    op: BinaryOp::Pow,
                    lhs: operand,
                    rhs: Box::new(rhs),
                },
                inner_span,
            );
            return Expr {
                id: lhs.id,
                kind: ExprKind::Unary {
                    op,
                    operand: Box::new(inner),
                },
                span,
            };
        }
        self.expr(
            ExprKind::Binary {
                op: BinaryOp::Pow,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            span,
        )
    }

    /// Warns when an operand of `**` is an unparenthesised unary expression.
    ///
    /// A parenthesised operand does not warn, because parentheses are exactly
    /// the fix being suggested — which is why [`ExprKind::Paren`] survives into
    /// the tree instead of being folded away.
    fn warn_on_bare_unary_operand(&mut self, operand: &Expr) {
        if !matches!(operand.kind, ExprKind::Unary { .. }) {
            return;
        }
        let span = operand.span;
        let replacement = format!("({})", self.text(span));
        self.push(
            Diagnostic::warning(
                codes::W_POWER_OPERAND_BINDING,
                "dialects disagree about how tightly this binds to `**`",
            )
            .with_primary(span, "this operand is a unary expression")
            .with_note(
                "IEC 61131-3:2013 Table 71 \"Operators of the ST language\" lists negation, unary \
                 plus and NOT above exponentiation, so salman reads `-2 ** 2` as `(-2) ** 2`, \
                 which is 4. CODESYS and Beckhoff publish the Edition 2 ordering and read it as \
                 `-(2 ** 2)`, which is -4. Parentheses say which you meant and mean the same \
                 thing everywhere.",
            )
            .with_dialect_rule(self.dialect.rule(
                "unary operators against `**`",
                match self.dialect.unary_power_binding {
                    UnaryPowerBinding::UnaryTighter => "the unary operator binds tighter",
                    UnaryPowerBinding::PowerTighter => "`**` binds tighter",
                },
            ))
            .with_suggestion("parenthesise the operand", vec![Edit { span, replacement }])
            .with_clause(clause::TABLE_ST_OPERATORS),
        );
    }

    /// `-x`, `+x`, `NOT x`.
    ///
    /// Recursive rather than iterative so that a chain of a hundred thousand
    /// minus signs is caught by the depth bound instead of building a tree that
    /// overflows the stack when it is dropped.
    fn parse_unary(&mut self) -> Expr {
        let token = self.peek();
        let op = match token.kind {
            TokenKind::Punct(Punct::Minus) => UnaryOp::Neg,
            TokenKind::Punct(Punct::Plus) => UnaryOp::Plus,
            TokenKind::Keyword(Keyword::Not) => UnaryOp::Not,
            _ => return self.parse_postfix(),
        };
        if !self.enter() {
            return self.error_expr(token.span);
        }
        self.bump();
        let operand = self.parse_unary();
        self.leave();
        let span = token.span.join(operand.span);
        self.expr(
            ExprKind::Unary {
                op,
                operand: Box::new(operand),
            },
            span,
        )
    }

    /// A primary expression followed by any number of `.field`, `[i]`, `^` and
    /// `(args)`.
    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_primary();
        let mut chain = 0u32;
        while !self.fused {
            let token = self.peek();
            if !matches!(
                token.kind,
                TokenKind::Punct(Punct::Dot | Punct::LBracket | Punct::Caret | Punct::LParen)
            ) {
                break;
            }
            chain = chain.saturating_add(1);
            if !self.has_depth_for(chain) {
                self.report_too_deep(token.span);
                break;
            }
            match token.kind {
                TokenKind::Punct(Punct::Dot) => {
                    self.bump();
                    let Some(field) = self.expect_name() else {
                        break;
                    };
                    let span = expr.span.join(field.span);
                    expr = self.expr(
                        ExprKind::Member {
                            base: Box::new(expr),
                            field,
                        },
                        span,
                    );
                }
                TokenKind::Punct(Punct::LBracket) => {
                    self.bump();
                    let mut indices = Vec::new();
                    loop {
                        if self.fused || self.at_eof() || self.at_punct(Punct::RBracket) {
                            break;
                        }
                        let before = self.pos;
                        let index = self.parse_expr();
                        indices.push(index);
                        if self.pos == before {
                            break;
                        }
                        if !self.eat_punct(Punct::Comma) {
                            break;
                        }
                    }
                    let end = self
                        .expect_punct(Punct::RBracket)
                        .map_or(token.span, |t| t.span);
                    let span = expr.span.join(end);
                    expr = self.expr(
                        ExprKind::Index {
                            base: Box::new(expr),
                            indices,
                        },
                        span,
                    );
                }
                TokenKind::Punct(Punct::Caret) => {
                    self.bump();
                    let span = expr.span.join(token.span);
                    expr = self.expr(ExprKind::Deref(Box::new(expr)), span);
                }
                _ => {
                    let (args, end) = self.parse_args();
                    let span = expr.span.join(end);
                    expr = self.expr(
                        ExprKind::Call {
                            callee: Box::new(expr),
                            args,
                        },
                        span,
                    );
                }
            }
        }
        expr
    }

    /// An argument list, from `(` to `)`. Returns the closing span.
    fn parse_args(&mut self) -> (Vec<Arg>, Span) {
        let open = self.bump().span;
        let mut args = Vec::new();
        if self.at_punct(Punct::RParen) {
            return (args, self.bump().span);
        }
        loop {
            if self.fused || self.at_eof() {
                break;
            }
            let before = self.pos;
            let arg = self.parse_arg();
            args.push(arg);
            if self.pos == before {
                break;
            }
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        let close = self.expect_punct(Punct::RParen).map_or(open, |t| t.span);
        (args, close)
    }

    /// One argument: positional, `IN := x`, or `Q => y`.
    ///
    /// IEC 61131-3:2013 Table 20 "Function call" and Table 42 "Function block
    /// call". Whether a *particular* callee accepts a positional argument is
    /// decided where the callee is known, not here.
    fn parse_arg(&mut self) -> Arg {
        if matches!(self.peek().kind, TokenKind::Ident) {
            match self.peek_at(1).kind {
                TokenKind::Punct(Punct::Assign) => {
                    if let Some(name) = self.expect_name() {
                        self.bump();
                        let value = self.parse_expr();
                        return Arg::Input { name, value };
                    }
                }
                TokenKind::Punct(Punct::RightArrow) => {
                    if let Some(name) = self.expect_name() {
                        self.bump();
                        // `Q => ` with nothing after it is legal and discards
                        // the output.
                        let target = if self.at_punct(Punct::RParen) || self.at_punct(Punct::Comma)
                        {
                            None
                        } else {
                            Some(self.parse_expr())
                        };
                        return Arg::Output { name, target };
                    }
                }
                _ => {}
            }
        }
        Arg::Positional(self.parse_expr())
    }

    fn parse_primary(&mut self) -> Expr {
        let token = self.peek();
        match token.kind {
            TokenKind::Literal(index) => {
                self.bump();
                match self.stream.literal(index) {
                    Some(value) => {
                        let value = value.clone();
                        self.expr(ExprKind::Literal(value), token.span)
                    }
                    None => self.error_expr(token.span),
                }
            }
            TokenKind::DirectAddress(index) => {
                self.bump();
                match self.stream.address(index) {
                    Some(address) => {
                        let address = address.clone();
                        self.expr(ExprKind::Direct(address), token.span)
                    }
                    None => self.error_expr(token.span),
                }
            }
            TokenKind::Ident => {
                if matches!(self.peek_at(1).kind, TokenKind::Punct(Punct::Hash)) {
                    return self.parse_enum_value();
                }
                let Some(name) = self.expect_name() else {
                    return self.error_expr(token.span);
                };
                let span = name.span;
                self.expr(ExprKind::Var(name), span)
            }
            TokenKind::Punct(Punct::LParen) => {
                self.bump();
                let inner = self.parse_expr();
                let end = self
                    .expect_punct(Punct::RParen)
                    .map_or(inner.span, |t| t.span);
                self.expr(ExprKind::Paren(Box::new(inner)), token.span.join(end))
            }
            TokenKind::Keyword(
                k @ (Keyword::Null | Keyword::Ref | Keyword::This | Keyword::Super),
            ) => {
                self.bump();
                self.push(
                    Diagnostic::error(
                        codes::U_UNIMPLEMENTED_CONSTRUCT,
                        format!("salman does not implement `{}`", k.text()),
                    )
                    .with_primary(token.span, "salman has no reference types")
                    .with_note(
                        "References, interfaces and the object-oriented extensions of Edition 3 \
                         are reserved so that meeting one says so, and are not implemented.",
                    ),
                );
                self.error_expr(token.span)
            }
            _ => {
                // Deliberately does not consume: the caller's recovery decides
                // what to do with this token, and eating it here would swallow
                // the `;` in `X := ;`.
                self.push(
                    Diagnostic::error(
                        codes::E_EXPECTED_EXPRESSION,
                        format!("expected an expression, found {}", token.describe()),
                    )
                    .with_primary(token.span, "expected an expression here")
                    .with_clause(clause::ST_EXPRESSIONS),
                );
                self.error_expr(token.span)
            }
        }
    }

    /// `Colour#Red`. The lexer hands this over as three tokens because it has
    /// no way to know that `Colour` is not a literal prefix.
    fn parse_enum_value(&mut self) -> Expr {
        let Some(ty) = self.expect_name() else {
            let span = self.peek().span;
            return self.error_expr(span);
        };
        self.bump();
        let Some(value) = self.expect_name() else {
            return self.error_expr(ty.span);
        };
        let span = ty.span.join(value.span);
        self.expr(ExprKind::EnumValue { ty, value }, span)
    }
}

// ---------------------------------------------------------------------------
// Small pure helpers
// ---------------------------------------------------------------------------

/// The integer value of a label expression, when the parser can work it out.
///
/// Only literals with an optional sign and parentheses. Anything that names
/// something has no value until the checker has resolved the name.
fn const_int(expr: &Expr) -> Option<i128> {
    match &expr.kind {
        ExprKind::Literal(LiteralValue::Int {
            magnitude,
            negative,
            ..
        }) => {
            let value = i128::try_from(*magnitude).ok()?;
            if *negative {
                value.checked_neg()
            } else {
                Some(value)
            }
        }
        ExprKind::Literal(LiteralValue::Bool(value)) => Some(i128::from(*value)),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => const_int(operand)?.checked_neg(),
        ExprKind::Unary {
            op: UnaryOp::Plus,
            operand,
        } => const_int(operand),
        ExprKind::Paren(inner) => const_int(inner),
        _ => None,
    }
}

/// A comparison key for a label that names something rather than valuing it.
///
/// Lower-cased because IEC identifiers are case-insensitive. Deliberately does
/// not try to make `Colour#Red` and a bare `Red` compare equal: deciding that
/// they are the same value needs the enumeration's declaration, which is the
/// checker's business.
fn spelling_key(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Var(name) => Some(name.ident.to_key().as_str().to_string()),
        ExprKind::EnumValue { ty, value } => Some(format!(
            "{}#{}",
            ty.ident.to_key().as_str(),
            value.ident.to_key().as_str()
        )),
        _ => None,
    }
}

/// Collects the spans of assignments whose whole left-hand side is `variable`.
///
/// Walks into nested statements, because a `FOR` body that moves its counter
/// inside an `IF` is moving it just the same. Sees only direct assignment: an
/// assignment through a `VAR_IN_OUT` parameter, through a reference or by a
/// callee is invisible here and is the checker's to find.
fn collect_assignments_to(body: &[Stmt], variable: &Name, out: &mut Vec<Span>) {
    for stmt in body {
        match &stmt.kind {
            StmtKind::Assign { target, .. } | StmtKind::AssignAttempt { target, .. } => {
                if let ExprKind::Var(name) = &target.kind
                    && name.ident == variable.ident
                {
                    out.push(target.span);
                }
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    collect_assignments_to(&branch.body, variable, out);
                }
                if let Some(body) = else_body {
                    collect_assignments_to(body, variable, out);
                }
            }
            StmtKind::Case {
                arms, else_body, ..
            } => {
                for arm in arms {
                    collect_assignments_to(&arm.body, variable, out);
                }
                if let Some(body) = else_body {
                    collect_assignments_to(body, variable, out);
                }
            }
            StmtKind::For { body, .. }
            | StmtKind::While { body, .. }
            | StmtKind::Repeat { body, .. } => collect_assignments_to(body, variable, out),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use salman_core::diag::Severity;
    use salman_core::span::SourceMap;

    use crate::ast::{Item, TypeDeclKind, VarSection};

    // -- helpers -------------------------------------------------------------

    fn parse_with(dialect: &Dialect, src: &str) -> (CompilationUnit, Diagnostics, SourceMap) {
        let mut map = SourceMap::new();
        let file = map.add("t.st", src).unwrap();
        let (unit, diags) = parse_source(file, src, dialect);
        (unit, diags, map)
    }

    fn parse_text(src: &str) -> (CompilationUnit, Diagnostics, SourceMap) {
        parse_with(&Dialect::generic(), src)
    }

    fn parse_ok(src: &str) -> CompilationUnit {
        let (unit, diags, map) = parse_text(src);
        assert!(
            !diags.has_errors(),
            "unexpected errors parsing:\n{src}\n{}",
            diags.render(&map)
        );
        unit
    }

    fn codes_of(diags: &Diagnostics, severity: Severity) -> Vec<&'static str> {
        diags
            .items()
            .iter()
            .filter(|d| d.severity == severity)
            .map(|d| d.code.0)
            .collect()
    }

    fn errors(src: &str) -> Vec<&'static str> {
        let (_, diags, _) = parse_text(src);
        codes_of(&diags, Severity::Error)
    }

    fn warnings(src: &str) -> Vec<&'static str> {
        let (_, diags, _) = parse_text(src);
        codes_of(&diags, Severity::Warning)
    }

    fn in_program(body: &str) -> String {
        format!("PROGRAM Main\n{body}\nEND_PROGRAM\n")
    }

    fn body_of(body: &str) -> Vec<Stmt> {
        let src = in_program(body);
        let unit = parse_ok(&src);
        unit.pou("Main").expect("PROGRAM Main").body.clone()
    }

    fn only_stmt(body: &str) -> Stmt {
        let stmts = body_of(body);
        assert_eq!(stmts.len(), 1, "expected one statement, got {stmts:?}");
        stmts.into_iter().next().expect("one statement")
    }

    /// The right-hand side of `X := <text>;`, parsed under `dialect`.
    fn expr_in(dialect: &Dialect, text: &str) -> Expr {
        let src = in_program(&format!("X := {text};"));
        let (unit, diags, map) = parse_with(dialect, &src);
        assert!(
            !diags.has_errors(),
            "unexpected errors parsing {text:?}:\n{}",
            diags.render(&map)
        );
        let pou = unit.pou("Main").expect("PROGRAM Main");
        match &pou.body.first().expect("a statement").kind {
            StmtKind::Assign { value, .. } => value.clone(),
            other => panic!("expected an assignment, got {other:?}"),
        }
    }

    fn expr_of(text: &str) -> Expr {
        expr_in(&Dialect::generic(), text)
    }

    /// A parenthesised rendering of an expression tree, so that a precedence
    /// test states the shape it expects instead of picking through boxes.
    fn shape(expr: &Expr) -> String {
        match &expr.kind {
            ExprKind::Literal(LiteralValue::Int {
                magnitude,
                negative,
                ..
            }) => format!("{}{magnitude}", if *negative { "-" } else { "" }),
            ExprKind::Literal(LiteralValue::Bool(value)) => value.to_string(),
            ExprKind::Literal(LiteralValue::Real { value, .. }) => format!("r{value}"),
            ExprKind::Literal(_) => "lit".to_string(),
            ExprKind::Var(name) => name.as_str().to_string(),
            ExprKind::Direct(address) => format!("%{}", address.location.letter()),
            ExprKind::Member { base, field } => format!("(. {} {field})", shape(base)),
            ExprKind::Index { base, indices } => {
                let mut out = format!("(index {}", shape(base));
                for index in indices {
                    out.push(' ');
                    out.push_str(&shape(index));
                }
                out.push(')');
                out
            }
            ExprKind::Deref(base) => format!("(^ {})", shape(base)),
            ExprKind::Unary { op, operand } => format!("({} {})", op.text(), shape(operand)),
            ExprKind::Binary { op, lhs, rhs } => {
                format!("({} {} {})", op.text(), shape(lhs), shape(rhs))
            }
            ExprKind::Call { callee, args } => {
                let mut out = format!("(call {}", shape(callee));
                for arg in args {
                    out.push(' ');
                    out.push_str(&arg_shape(arg));
                }
                out.push(')');
                out
            }
            ExprKind::EnumValue { ty, value } => format!("{ty}#{value}"),
            ExprKind::Paren(inner) => format!("(paren {})", shape(inner)),
            ExprKind::Error => "<error>".to_string(),
        }
    }

    fn arg_shape(arg: &Arg) -> String {
        match arg {
            Arg::Positional(expr) => shape(expr),
            Arg::Input { name, value } => format!("{name}:={}", shape(value)),
            Arg::Output { name, target } => match target {
                Some(target) => format!("{name}=>{}", shape(target)),
                None => format!("{name}=>_"),
            },
        }
    }

    fn collect_expr_ids(expr: &Expr, out: &mut Vec<u32>) {
        out.push(expr.id.0);
        match &expr.kind {
            ExprKind::Member { base, .. } | ExprKind::Deref(base) => collect_expr_ids(base, out),
            ExprKind::Paren(inner) => collect_expr_ids(inner, out),
            ExprKind::Unary { operand, .. } => collect_expr_ids(operand, out),
            ExprKind::Binary { lhs, rhs, .. } => {
                collect_expr_ids(lhs, out);
                collect_expr_ids(rhs, out);
            }
            ExprKind::Index { base, indices } => {
                collect_expr_ids(base, out);
                for index in indices {
                    collect_expr_ids(index, out);
                }
            }
            ExprKind::Call { callee, args } => {
                collect_expr_ids(callee, out);
                for arg in args {
                    match arg {
                        Arg::Positional(expr) | Arg::Input { value: expr, .. } => {
                            collect_expr_ids(expr, out);
                        }
                        Arg::Output { target, .. } => {
                            if let Some(target) = target {
                                collect_expr_ids(target, out);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_stmt_ids(stmts: &[Stmt], out: &mut Vec<u32>) {
        for stmt in stmts {
            out.push(stmt.id.0);
            match &stmt.kind {
                StmtKind::Assign { target, value } | StmtKind::AssignAttempt { target, value } => {
                    collect_expr_ids(target, out);
                    collect_expr_ids(value, out);
                }
                StmtKind::Call(expr) => collect_expr_ids(expr, out),
                StmtKind::If {
                    branches,
                    else_body,
                } => {
                    for branch in branches {
                        collect_expr_ids(&branch.condition, out);
                        collect_stmt_ids(&branch.body, out);
                    }
                    if let Some(body) = else_body {
                        collect_stmt_ids(body, out);
                    }
                }
                StmtKind::Case {
                    selector,
                    arms,
                    else_body,
                } => {
                    collect_expr_ids(selector, out);
                    for arm in arms {
                        for label in &arm.labels {
                            match label {
                                CaseLabel::Single(expr) => collect_expr_ids(expr, out),
                                CaseLabel::Range { low, high } => {
                                    collect_expr_ids(low, out);
                                    collect_expr_ids(high, out);
                                }
                            }
                        }
                        collect_stmt_ids(&arm.body, out);
                    }
                    if let Some(body) = else_body {
                        collect_stmt_ids(body, out);
                    }
                }
                StmtKind::For {
                    from, to, by, body, ..
                } => {
                    collect_expr_ids(from, out);
                    collect_expr_ids(to, out);
                    if let Some(by) = by {
                        collect_expr_ids(by, out);
                    }
                    collect_stmt_ids(body, out);
                }
                StmtKind::While { condition, body } => {
                    collect_expr_ids(condition, out);
                    collect_stmt_ids(body, out);
                }
                StmtKind::Repeat { body, until } => {
                    collect_stmt_ids(body, out);
                    collect_expr_ids(until, out);
                }
                _ => {}
            }
        }
    }

    // -- precedence ----------------------------------------------------------

    #[test]
    fn or_binds_looser_than_and() {
        assert_eq!(shape(&expr_of("a OR b AND c")), "(OR a (AND b c))");
    }

    #[test]
    fn and_binds_looser_than_equality() {
        assert_eq!(shape(&expr_of("a AND b = c")), "(AND a (= b c))");
    }

    #[test]
    fn equality_binds_looser_than_the_ordering_comparisons() {
        // Annex A calls the `=`/`<>` level `comparison` and the `<`/`>` level
        // `equ_expression`, which is the opposite of what the names suggest.
        // This test states which one actually binds looser.
        assert_eq!(shape(&expr_of("a = b < c")), "(= a (< b c))");
        assert_eq!(shape(&expr_of("a <> b >= c")), "(<> a (>= b c))");
    }

    #[test]
    fn xor_sits_between_or_and_and() {
        assert_eq!(
            shape(&expr_of("a OR b XOR c AND e")),
            "(OR a (XOR b (AND c e)))"
        );
    }

    #[test]
    fn addition_binds_looser_than_multiplication() {
        assert_eq!(shape(&expr_of("a + b * c")), "(+ a (* b c))");
    }

    #[test]
    fn multiplication_binds_looser_than_exponentiation() {
        assert_eq!(shape(&expr_of("a * b ** c")), "(* a (** b c))");
    }

    #[test]
    fn mod_binds_as_tightly_as_multiplication() {
        assert_eq!(shape(&expr_of("a + b MOD c")), "(+ a (MOD b c))");
        assert_eq!(shape(&expr_of("a * b MOD c")), "(MOD (* a b) c)");
    }

    #[test]
    fn not_binds_tighter_than_equality() {
        // Table 71 puts NOT at row 6 and `=` far below it, so `NOT a = b`
        // compares the negation, it does not negate the comparison.
        assert_eq!(shape(&expr_of("NOT a = b")), "(= (NOT a) b)");
    }

    #[test]
    fn unary_minus_binds_tighter_than_exponentiation_as_edition_3_orders_them() {
        // `-2 ** 2` is `(-2) ** 2`, which is 4.
        //
        // IEC 61131-3:2013 Table 71 "Operators of the ST language" lists
        // negation above exponentiation, and the Edition 3 normative Annex A
        // grammar makes the operands of `**` unary expressions. Two independent
        // open implementations agree.
        //
        // DIVERGENCE: CODESYS and Beckhoff ship the Edition 2 ordering and give
        // -4 for the same text. That is why salman warns here — see
        // `an_unparenthesised_unary_operand_of_power_is_warned_about`.
        assert_eq!(shape(&expr_of("-2 ** 2")), "(** (- 2) 2)");
        assert_eq!(shape(&expr_of("NOT a ** b")), "(** (NOT a) b)");
    }

    #[test]
    fn exponentiation_is_left_associative() {
        // UNVERIFIED: no source salman could read states the associativity of
        // `**` itself. Three open implementations group it to the left, so
        // salman does; code that depends on this should use parentheses.
        assert_eq!(shape(&expr_of("2 ** 3 ** 2")), "(** (** 2 3) 2)");
    }

    #[test]
    fn every_binary_level_is_left_associative() {
        assert_eq!(shape(&expr_of("a - b - c")), "(- (- a b) c)");
        assert_eq!(shape(&expr_of("a / b / c")), "(/ (/ a b) c)");
        assert_eq!(shape(&expr_of("a OR b OR c")), "(OR (OR a b) c)");
        assert_eq!(shape(&expr_of("a AND b AND c")), "(AND (AND a b) c)");
        assert_eq!(shape(&expr_of("a < b < c")), "(< (< a b) c)");
    }

    #[test]
    fn ampersand_is_another_spelling_of_and() {
        assert_eq!(shape(&expr_of("a & b")), "(AND a b)");
        assert_eq!(shape(&expr_of("a & b AND c")), "(AND (AND a b) c)");
    }

    #[test]
    fn parentheses_stay_in_the_tree_rather_than_being_folded_away() {
        // A formatter that discards the author's parentheses rewrites code
        // nobody asked it to rewrite, and the `**` warning needs to know.
        assert_eq!(shape(&expr_of("(a + b) * c")), "(* (paren (+ a b)) c)");
    }

    #[test]
    fn an_unparenthesised_unary_operand_of_power_is_warned_about() {
        assert_eq!(warnings(&in_program("X := -2 ** 2;")), ["W0201"]);
        assert_eq!(warnings(&in_program("X := 2 ** -2;")), ["W0201"]);
        assert_eq!(warnings(&in_program("X := NOT a ** b;")), ["W0201"]);
    }

    #[test]
    fn a_parenthesised_power_operand_does_not_warn() {
        assert!(warnings(&in_program("X := (-2) ** 2;")).is_empty());
        assert!(warnings(&in_program("X := 2 ** (-2);")).is_empty());
        assert!(warnings(&in_program("X := 2 ** 2;")).is_empty());
    }

    #[test]
    fn the_power_warning_suggests_the_parentheses_that_would_settle_it() {
        let (_, diags, _) = parse_text(&in_program("X := -2 ** 2;"));
        let warning = diags
            .items()
            .iter()
            .find(|d| d.code == codes::W_POWER_OPERAND_BINDING)
            .expect("a binding warning");
        let suggestion = warning.suggestions.first().expect("a suggestion");
        assert_eq!(
            suggestion.edits.first().map(|e| e.replacement.as_str()),
            Some("(-2)")
        );
        assert!(
            warning
                .dialect_rule
                .as_deref()
                .is_some_and(|r| r.contains("unary operators against `**`")),
            "{:?}",
            warning.dialect_rule
        );
    }

    #[test]
    fn a_dialect_that_binds_power_tighter_lifts_the_unary_back_out() {
        // Nothing salman ships selects this, but the setting exists, so it has
        // to mean something. Under the Edition 2 ordering `-2 ** 2` is -4.
        let dialect = Dialect {
            unary_power_binding: UnaryPowerBinding::PowerTighter,
            ..Dialect::generic()
        };
        assert_eq!(shape(&expr_in(&dialect, "-2 ** 2")), "(- (** 2 2))");
        assert_eq!(
            shape(&expr_in(&dialect, "-2 ** 3 ** 2")),
            "(- (** (** 2 3) 2))"
        );
    }

    // -- primaries and postfix ----------------------------------------------

    #[test]
    fn member_access_chains_left_to_right() {
        assert_eq!(shape(&expr_of("a.b.c")), "(. (. a b) c)");
    }

    #[test]
    fn an_index_takes_one_expression_per_dimension() {
        assert_eq!(shape(&expr_of("Grid[i, j + 1]")), "(index Grid i (+ j 1))");
    }

    #[test]
    fn dereference_is_a_postfix_operator() {
        assert_eq!(shape(&expr_of("Handle^")), "(^ Handle)");
        assert_eq!(shape(&expr_of("Handle^.Field")), "(. (^ Handle) Field)");
    }

    #[test]
    fn member_index_and_call_chain_together_in_source_order() {
        assert_eq!(
            shape(&expr_of("Cell.Motors[2].Start(x)")),
            "(call (. (index (. Cell Motors) 2) Start) x)"
        );
    }

    #[test]
    fn an_enumeration_value_parses_as_a_qualified_pair() {
        assert_eq!(shape(&expr_of("Colour#Red")), "Colour#Red");
    }

    #[test]
    fn a_direct_address_arrives_as_one_token_and_one_node() {
        // The lexer hands `%IX0.0` over whole; the parser must not try to
        // reassemble it out of an identifier, a dot and a number.
        let expr = expr_of("%IX0.0");
        assert!(matches!(expr.kind, ExprKind::Direct(_)), "{expr:?}");
    }

    #[test]
    fn literals_reach_the_tree_with_their_values() {
        assert_eq!(shape(&expr_of("42")), "42");
        assert_eq!(shape(&expr_of("TRUE")), "true");
        assert_eq!(shape(&expr_of("16#FF")), "255");
        assert!(matches!(
            expr_of("T#5s").kind,
            ExprKind::Literal(LiteralValue::Duration { .. })
        ));
        assert!(matches!(
            expr_of("'text'").kind,
            ExprKind::Literal(LiteralValue::String(_))
        ));
    }

    #[test]
    fn a_chain_of_unary_operators_nests_outward() {
        assert_eq!(shape(&expr_of("- - a")), "(- (- a))");
        assert_eq!(shape(&expr_of("NOT NOT a")), "(NOT (NOT a))");
        assert_eq!(shape(&expr_of("-+a")), "(- (+ a))");
    }

    // -- calls ---------------------------------------------------------------

    #[test]
    fn a_call_may_take_no_arguments() {
        assert_eq!(shape(&expr_of("Reset()")), "(call Reset)");
    }

    #[test]
    fn a_call_may_take_positional_arguments() {
        assert_eq!(shape(&expr_of("MAX(a, b)")), "(call MAX a b)");
    }

    #[test]
    fn a_call_may_bind_inputs_by_name() {
        assert_eq!(
            shape(&expr_of("Timer(IN := Start, PT := Delay)")),
            "(call Timer IN:=Start PT:=Delay)"
        );
    }

    #[test]
    fn a_call_may_bind_outputs_by_name() {
        assert_eq!(
            shape(&expr_of("Timer(IN := Start, Q => Running)")),
            "(call Timer IN:=Start Q=>Running)"
        );
    }

    #[test]
    fn an_output_binding_with_nothing_after_it_discards_the_output() {
        assert_eq!(
            shape(&expr_of("Timer(Q =>, ET => E)")),
            "(call Timer Q=>_ ET=>E)"
        );
        assert_eq!(shape(&expr_of("Timer(Q =>)")), "(call Timer Q=>_)");
    }

    #[test]
    fn positional_named_and_output_arguments_may_be_mixed() {
        // Whether a particular callee accepts the mixture is the checker's
        // business; the parser's job is to keep what was written.
        assert_eq!(
            shape(&expr_of("F(a, IN := b, Q => c)")),
            "(call F a IN:=b Q=>c)"
        );
    }

    #[test]
    fn an_argument_may_itself_be_a_call() {
        assert_eq!(shape(&expr_of("F(G(x), y)")), "(call F (call G x) y)");
    }

    // -- statements ----------------------------------------------------------

    #[test]
    fn a_bare_semicolon_is_the_empty_statement() {
        assert!(matches!(only_stmt(";").kind, StmtKind::Empty));
    }

    #[test]
    fn an_assignment_keeps_its_target_and_its_value() {
        match only_stmt("Motor := Speed + 1;").kind {
            StmtKind::Assign { target, value } => {
                assert_eq!(shape(&target), "Motor");
                assert_eq!(shape(&value), "(+ Speed 1)");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_assignment_attempt_is_parsed_rather_than_refused() {
        // IEC 61131-3:2013 Table 52 "Assignment attempt". salman implements no
        // references, but parsing `?=` is what lets the message about that come
        // from the checker and read like an answer.
        assert!(matches!(
            only_stmt("Target ?= Source;").kind,
            StmtKind::AssignAttempt { .. }
        ));
    }

    #[test]
    fn a_call_on_its_own_is_a_statement() {
        match only_stmt("Timer(IN := Start);").kind {
            StmtKind::Call(expr) => assert_eq!(shape(&expr), "(call Timer IN:=Start)"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_bare_expression_is_not_a_statement() {
        assert_eq!(errors(&in_program("a + b;")), ["E0204"]);
    }

    #[test]
    fn if_then_end_if_has_one_branch_and_no_else() {
        match only_stmt("IF Ready THEN Go := 1; END_IF").kind {
            StmtKind::If {
                branches,
                else_body,
            } => {
                assert_eq!(branches.len(), 1);
                assert_eq!(shape(&branches[0].condition), "Ready");
                assert_eq!(branches[0].body.len(), 1);
                assert!(else_body.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn elsif_branches_are_kept_in_order_after_the_if() {
        match only_stmt(
            "IF a THEN X := 1; ELSIF b THEN X := 2; ELSIF c THEN X := 3; ELSE X := 4; END_IF",
        )
        .kind
        {
            StmtKind::If {
                branches,
                else_body,
            } => {
                let conditions: Vec<String> =
                    branches.iter().map(|b| shape(&b.condition)).collect();
                assert_eq!(conditions, ["a", "b", "c"]);
                assert_eq!(else_body.map(|b| b.len()), Some(1));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_trailing_semicolon_after_a_closing_keyword_is_accepted() {
        assert_eq!(body_of("IF a THEN X := 1; END_IF;").len(), 1);
        assert_eq!(body_of("WHILE a DO X := 1; END_WHILE;").len(), 1);
    }

    #[test]
    fn case_labels_may_be_single_values_lists_or_ranges() {
        match only_stmt("CASE Sel OF 1: A := 1; 2, 3: A := 2; 4..7: A := 3; END_CASE").kind {
            StmtKind::Case { arms, .. } => {
                assert_eq!(arms.len(), 3);
                assert_eq!(arms[0].labels.len(), 1);
                assert_eq!(arms[1].labels.len(), 2);
                assert!(matches!(arms[2].labels[0], CaseLabel::Range { .. }));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_case_may_have_an_else_arm() {
        match only_stmt("CASE Sel OF 1: A := 1; ELSE A := 0; END_CASE").kind {
            StmtKind::Case {
                arms, else_body, ..
            } => {
                assert_eq!(arms.len(), 1);
                assert_eq!(else_body.map(|b| b.len()), Some(1));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_case_arm_holds_every_statement_up_to_the_next_label() {
        match only_stmt("CASE S OF 1: A := 1; B := 2; C := 3; 2: E := 4; END_CASE").kind {
            StmtKind::Case { arms, .. } => {
                assert_eq!(arms[0].body.len(), 3);
                assert_eq!(arms[1].body.len(), 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_enumeration_value_may_be_a_case_label() {
        match only_stmt("CASE C OF Colour#Red: A := 1; Colour#Blue: A := 2; END_CASE").kind {
            StmtKind::Case { arms, .. } => assert_eq!(arms.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn for_keeps_its_control_variable_bounds_and_step() {
        match only_stmt("FOR i := 0 TO 10 BY 2 DO Total := Total + i; END_FOR").kind {
            StmtKind::For {
                variable,
                from,
                to,
                by,
                body,
            } => {
                assert_eq!(variable.as_str(), "i");
                assert_eq!(shape(&from), "0");
                assert_eq!(shape(&to), "10");
                assert_eq!(by.map(|e| shape(&e)), Some("2".to_string()));
                assert_eq!(body.len(), 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn for_without_by_records_no_step_rather_than_inventing_one() {
        match only_stmt("FOR i := 0 TO 10 DO A := i; END_FOR").kind {
            StmtKind::For { by, .. } => assert!(by.is_none()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn while_tests_before_the_body_and_repeat_tests_after_it() {
        match only_stmt("WHILE a DO X := 1; END_WHILE").kind {
            StmtKind::While { condition, body } => {
                assert_eq!(shape(&condition), "a");
                assert_eq!(body.len(), 1);
            }
            other => panic!("{other:?}"),
        }
        match only_stmt("REPEAT X := 1; UNTIL a END_REPEAT").kind {
            StmtKind::Repeat { body, until } => {
                assert_eq!(body.len(), 1);
                assert_eq!(shape(&until), "a");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn continue_is_a_standard_statement_in_edition_3() {
        // IEC 61131-3:2013 Table 72 "ST language statements" row 9. Beckhoff's
        // documentation still calls CONTINUE a non-standard extension, which is
        // Edition-2-era wording; salman treats it as standard and does not warn.
        let stmts = body_of("WHILE a DO CONTINUE; END_WHILE");
        match &stmts.first().expect("a WHILE").kind {
            StmtKind::While { body, .. } => {
                assert!(matches!(
                    body.first().map(|s| &s.kind),
                    Some(StmtKind::Continue)
                ));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn exit_and_return_are_statements_of_their_own() {
        assert!(matches!(only_stmt("EXIT;").kind, StmtKind::Exit));
        assert!(matches!(only_stmt("RETURN;").kind, StmtKind::Return));
    }

    #[test]
    fn control_constructs_nest_inside_one_another() {
        let stmt = only_stmt(
            "FOR i := 1 TO 3 DO \
               IF a THEN \
                 CASE i OF 1: WHILE b DO REPEAT X := 1; UNTIL c END_REPEAT END_WHILE END_CASE \
               END_IF \
             END_FOR",
        );
        let mut ids = Vec::new();
        collect_stmt_ids(std::slice::from_ref(&stmt), &mut ids);
        assert!(ids.len() >= 6, "the nest collapsed: {stmt:?}");
    }

    // -- salman's CASE rule --------------------------------------------------

    #[test]
    fn duplicate_case_labels_are_refused_by_a_salman_rule() {
        assert_eq!(
            errors(&in_program("CASE S OF 1: A := 1; 1: A := 2; END_CASE")),
            ["E0208"]
        );
    }

    #[test]
    fn overlapping_case_ranges_are_refused_by_a_salman_rule() {
        assert_eq!(
            errors(&in_program(
                "CASE S OF 1..5: A := 1; 4..8: A := 2; END_CASE"
            )),
            ["E0209"]
        );
    }

    #[test]
    fn the_case_label_rule_says_in_the_diagnostic_that_it_is_a_salman_rule() {
        let (_, diags, _) = parse_text(&in_program("CASE S OF 1: A := 1; 1: A := 2; END_CASE"));
        let note = diags
            .items()
            .iter()
            .find(|d| d.code == codes::E_DUPLICATE_CASE_LABEL)
            .and_then(|d| d.notes.first())
            .expect("a note");
        assert!(note.contains("salman rule"), "{note}");
        assert!(
            note.contains("IEC 61131-3:2013 Table 72 \"ST language statements\""),
            "{note}"
        );
    }

    #[test]
    fn case_labels_that_name_constants_are_compared_by_spelling_only() {
        // The parser can see that `Red` appears twice. It cannot see whether
        // `Colour#Red` and `Red` are the same value — that needs the
        // enumeration's declaration, so it is the checker's to decide.
        assert_eq!(
            errors(&in_program("CASE C OF Red: A := 1; Red: A := 2; END_CASE")),
            ["E0208"]
        );
        assert!(
            errors(&in_program(
                "CASE C OF Colour#Red: A := 1; Red: A := 2; END_CASE"
            ))
            .is_empty()
        );
    }

    #[test]
    fn distinct_case_labels_are_accepted() {
        assert!(
            errors(&in_program(
                "CASE S OF 1: A := 1; 2..4: A := 2; 5, 6: A := 3; ELSE A := 0; END_CASE"
            ))
            .is_empty()
        );
    }

    #[test]
    fn a_negative_case_label_is_evaluated_like_any_other_constant() {
        assert_eq!(
            errors(&in_program("CASE S OF -1: A := 1; -1: A := 2; END_CASE")),
            ["E0208"]
        );
        assert!(errors(&in_program("CASE S OF -1: A := 1; 1: A := 2; END_CASE")).is_empty());
    }

    // -- salman's FOR rule ---------------------------------------------------

    #[test]
    fn assigning_the_control_variable_in_a_for_body_is_refused_by_a_salman_rule() {
        assert_eq!(
            errors(&in_program("FOR i := 1 TO 10 DO i := 5; END_FOR")),
            ["E0210"]
        );
    }

    #[test]
    fn the_for_rule_finds_an_assignment_nested_inside_the_body() {
        assert_eq!(
            errors(&in_program(
                "FOR i := 1 TO 10 DO IF a THEN i := 5; END_IF END_FOR"
            )),
            ["E0210"]
        );
    }

    #[test]
    fn a_for_body_that_leaves_its_counter_alone_is_accepted() {
        assert!(
            errors(&in_program(
                "FOR i := 1 TO 10 DO j := i; Total := Total + i; END_FOR"
            ))
            .is_empty()
        );
    }

    // -- declarations --------------------------------------------------------

    #[test]
    fn a_program_carries_its_variable_blocks_and_its_body() {
        let unit = parse_ok(
            "PROGRAM Conveyor\n\
             VAR_INPUT Start : BOOL; END_VAR\n\
             VAR Count : DINT := 0; END_VAR\n\
             Count := Count + 1;\n\
             END_PROGRAM\n",
        );
        let pou = unit
            .pou("conveyor")
            .expect("names compare case-insensitively");
        assert_eq!(pou.kind, PouKind::Program);
        assert_eq!(pou.var_blocks.len(), 2);
        assert_eq!(pou.var_blocks[0].section, VarSection::Input);
        assert_eq!(pou.var_blocks[1].section, VarSection::Local);
        assert_eq!(pou.body.len(), 1);
    }

    #[test]
    fn every_variable_section_keyword_opens_its_section() {
        let sections = [
            ("VAR", VarSection::Local),
            ("VAR_INPUT", VarSection::Input),
            ("VAR_OUTPUT", VarSection::Output),
            ("VAR_IN_OUT", VarSection::InOut),
            ("VAR_TEMP", VarSection::Temp),
            ("VAR_EXTERNAL", VarSection::External),
            ("VAR_ACCESS", VarSection::Access),
            ("VAR_CONFIG", VarSection::Config),
        ];
        for (keyword, expected) in sections {
            let src = format!("PROGRAM P\n{keyword}\n A : BOOL;\nEND_VAR\nEND_PROGRAM\n");
            let unit = parse_ok(&src);
            let pou = unit.pou("P").expect("P");
            assert_eq!(
                pou.var_blocks.first().map(|b| b.section),
                Some(expected),
                "{keyword}"
            );
        }
    }

    #[test]
    fn a_var_global_block_at_file_scope_is_a_top_level_item() {
        let unit = parse_ok("VAR_GLOBAL Emergency : BOOL; END_VAR\n");
        match unit.items.first() {
            Some(Item::Globals(block)) => assert_eq!(block.section, VarSection::Global),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn variable_block_qualifiers_are_recorded() {
        let unit = parse_ok("PROGRAM P VAR RETAIN CONSTANT A : INT := 1; END_VAR END_PROGRAM");
        let block = &unit.pou("P").expect("P").var_blocks[0];
        assert_eq!(block.qualifiers.retention, Retention::Retain);
        assert!(block.qualifiers.constant);
        assert!(!block.qualifiers.persistent);

        let unit = parse_ok("PROGRAM P VAR NON_RETAIN PERSISTENT A : INT; END_VAR END_PROGRAM");
        let block = &unit.pou("P").expect("P").var_blocks[0];
        assert_eq!(block.qualifiers.retention, Retention::NonRetain);
        assert!(block.qualifiers.persistent);
    }

    #[test]
    fn one_declaration_may_name_several_variables() {
        let unit = parse_ok("PROGRAM P VAR A, B, C : DINT := 0; END_VAR END_PROGRAM");
        let decl = &unit.pou("P").expect("P").var_blocks[0].decls[0];
        let names: Vec<&str> = decl.names.iter().map(Name::as_str).collect();
        assert_eq!(names, ["A", "B", "C"]);
        assert!(decl.init.is_some());
    }

    #[test]
    fn a_located_variable_keeps_the_address_it_was_bound_to() {
        let unit = parse_ok("PROGRAM P VAR Sensor AT %IX0.0 : BOOL; END_VAR END_PROGRAM");
        let decl = &unit.pou("P").expect("P").var_blocks[0].decls[0];
        assert!(decl.located_at.is_some(), "{decl:?}");
        assert!(decl.located_at_span.is_some());
    }

    #[test]
    fn a_string_may_declare_its_maximum_length() {
        let unit = parse_ok(
            "PROGRAM P VAR A : STRING[80]; B : STRING; C : WSTRING(20); END_VAR END_PROGRAM",
        );
        let decls = &unit.pou("P").expect("P").var_blocks[0].decls;
        assert!(matches!(
            decls[0].ty,
            TypeRef::String {
                max_len: Some(_),
                ..
            }
        ));
        assert!(matches!(decls[1].ty, TypeRef::String { max_len: None, .. }));
        assert!(matches!(
            decls[2].ty,
            TypeRef::String {
                ty: ElementaryType::WString,
                max_len: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn an_array_declaration_keeps_one_dimension_per_bound_pair() {
        let unit = parse_ok("PROGRAM P VAR Grid : ARRAY [0..9, 1..4] OF REAL; END_VAR END_PROGRAM");
        match &unit.pou("P").expect("P").var_blocks[0].decls[0].ty {
            TypeRef::Array { dims, element, .. } => {
                assert_eq!(dims.len(), 2);
                assert_eq!(shape(&dims[0].low), "0");
                assert_eq!(shape(&dims[1].high), "4");
                assert!(matches!(
                    **element,
                    TypeRef::Elementary {
                        ty: ElementaryType::Real,
                        ..
                    }
                ));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_subrange_declaration_keeps_its_base_type_and_both_bounds() {
        let unit = parse_ok("PROGRAM P VAR Level : INT (0..100); END_VAR END_PROGRAM");
        match &unit.pou("P").expect("P").var_blocks[0].decls[0].ty {
            TypeRef::Subrange {
                base, low, high, ..
            } => {
                assert!(matches!(
                    **base,
                    TypeRef::Elementary {
                        ty: ElementaryType::Int,
                        ..
                    }
                ));
                assert_eq!(shape(low), "0");
                assert_eq!(shape(high), "100");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_function_block_instance_is_declared_by_naming_its_type() {
        let unit = parse_ok("PROGRAM P VAR T1 : TON; END_VAR END_PROGRAM");
        match &unit.pou("P").expect("P").var_blocks[0].decls[0].ty {
            TypeRef::Named(name) => assert_eq!(name.as_str(), "TON"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_function_declares_the_type_of_the_value_it_returns() {
        let unit = parse_ok(
            "FUNCTION Average : REAL\nVAR_INPUT A, B : REAL; END_VAR\nAverage := (A + B) / 2.0;\nEND_FUNCTION\n",
        );
        let pou = unit.pou("Average").expect("Average");
        assert_eq!(pou.kind, PouKind::Function);
        assert!(matches!(
            pou.return_type,
            Some(TypeRef::Elementary {
                ty: ElementaryType::Real,
                ..
            })
        ));
    }

    #[test]
    fn a_function_block_has_no_return_type() {
        let unit = parse_ok(
            "FUNCTION_BLOCK Latch\nVAR_INPUT S : BOOL; END_VAR\nVAR_OUTPUT Q : BOOL; END_VAR\nQ := S;\nEND_FUNCTION_BLOCK\n",
        );
        let pou = unit.pou("Latch").expect("Latch");
        assert_eq!(pou.kind, PouKind::FunctionBlock);
        assert!(pou.return_type.is_none());
        assert_eq!(pou.var_blocks.len(), 2);
    }

    #[test]
    fn a_type_block_holds_aliases_structures_enumerations_subranges_and_arrays() {
        let unit = parse_ok(
            "TYPE\n\
               Speed : DINT := 7;\n\
               Point : STRUCT X : INT; Y : INT; END_STRUCT;\n\
               Colour : (Red, Green := 5, Blue);\n\
               Percent : INT (0..100);\n\
               Table : ARRAY [1..10] OF REAL;\n\
             END_TYPE\n",
        );
        let Some(Item::Types(decls)) = unit.items.first() else {
            panic!("{:?}", unit.items);
        };
        assert_eq!(decls.len(), 5);
        assert!(matches!(decls[0].kind, TypeDeclKind::Alias(_)));
        assert!(decls[0].init.is_some());
        match &decls[1].kind {
            TypeDeclKind::Struct(fields) => assert_eq!(fields.len(), 2),
            other => panic!("{other:?}"),
        }
        match &decls[2].kind {
            TypeDeclKind::Enum { base, values } => {
                assert!(base.is_none());
                assert_eq!(values.len(), 3);
                assert_eq!(values[1].name.as_str(), "Green");
                assert!(values[1].value.is_some());
                assert!(values[0].value.is_none());
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(decls[3].kind, TypeDeclKind::Subrange { .. }));
        assert!(matches!(decls[4].kind, TypeDeclKind::Array { .. }));
    }

    #[test]
    fn an_enumeration_may_name_the_type_its_values_are_stored_in() {
        // `Colour : INT (Red, Green)` is an enumeration; `Percent : INT (0..100)`
        // is a subrange. They differ only after the opening parenthesis.
        let unit = parse_ok("TYPE Colour : INT (Red, Green); END_TYPE");
        let Some(Item::Types(decls)) = unit.items.first() else {
            panic!("{:?}", unit.items);
        };
        match &decls[0].kind {
            TypeDeclKind::Enum { base, values } => {
                assert_eq!(*base, Some(ElementaryType::Int));
                assert_eq!(values.len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_configuration_holds_globals_resources_tasks_and_program_instances() {
        let unit = parse_ok(
            "CONFIGURATION Plant\n\
               VAR_GLOBAL Emergency : BOOL; END_VAR\n\
               RESOURCE Station ON CPU_001\n\
                 VAR_GLOBAL Local : INT; END_VAR\n\
                 TASK Fast (INTERVAL := T#10ms, PRIORITY := 1);\n\
                 TASK OnEvent (SINGLE := Emergency, PRIORITY := 0);\n\
                 PROGRAM Main WITH Fast : Conveyor;\n\
                 PROGRAM Spare : Conveyor;\n\
               END_RESOURCE\n\
             END_CONFIGURATION\n",
        );
        let config = unit.configurations().next().expect("a configuration");
        assert_eq!(config.name.as_str(), "Plant");
        assert_eq!(config.var_blocks.len(), 1);
        let resource = config.resources.first().expect("a resource");
        assert_eq!(resource.on_type.as_ref().map(Name::as_str), Some("CPU_001"));
        assert_eq!(resource.var_blocks.len(), 1);
        assert_eq!(resource.tasks.len(), 2);
        assert!(resource.tasks[0].interval.is_some());
        assert!(resource.tasks[0].priority.is_some());
        assert!(resource.tasks[1].single.is_some());
        assert_eq!(resource.programs.len(), 2);
        assert_eq!(
            resource.programs[0].task.as_ref().map(Name::as_str),
            Some("Fast")
        );
        assert!(resource.programs[1].task.is_none());
        assert_eq!(resource.programs[1].program_type.as_str(), "Conveyor");
    }

    #[test]
    fn several_top_level_items_parse_into_one_unit_in_source_order() {
        let unit = parse_ok(
            "TYPE Speed : INT; END_TYPE\n\
             FUNCTION F : INT F := 1; END_FUNCTION\n\
             FUNCTION_BLOCK B B_Out := 1; END_FUNCTION_BLOCK\n\
             PROGRAM P X := 1; END_PROGRAM\n",
        );
        assert_eq!(unit.items.len(), 4);
        let names: Vec<&str> = unit.pous().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["F", "B", "P"]);
    }

    // -- unimplemented constructs -------------------------------------------

    #[test]
    fn a_reserved_but_unimplemented_construct_says_so_rather_than_failing_to_parse() {
        assert_eq!(errors("CLASS Motor END_CLASS"), ["U0201"]);
        assert_eq!(errors(&in_program("X := NULL;")), ["U0201"]);
    }

    #[test]
    fn an_inline_structure_or_enumeration_asks_for_a_named_type() {
        assert_eq!(
            errors("PROGRAM P VAR C : (Red, Green); END_VAR END_PROGRAM"),
            ["U0201"]
        );
        assert_eq!(
            errors("PROGRAM P VAR C : STRUCT X : INT; END_STRUCT; END_VAR END_PROGRAM"),
            ["U0201"]
        );
    }

    #[test]
    fn an_instance_path_in_a_declaration_says_it_is_not_implemented() {
        assert_eq!(
            errors(
                "PROGRAM P VAR_CONFIG Station.Main.Sensor AT %IX1.0 : BOOL; END_VAR END_PROGRAM"
            ),
            ["U0201"]
        );
    }

    // -- error recovery ------------------------------------------------------

    #[test]
    fn a_file_with_ten_broken_statements_reports_about_ten_errors_not_one() {
        let mut body = String::new();
        for index in 0..10 {
            // Each line is missing its right-hand side, which is a fresh error
            // and not a consequence of the one before it.
            body.push('V');
            body.push_str(&index.to_string());
            body.push_str(" := ;\n");
        }
        let found = errors(&in_program(&body));
        assert!(
            found.len() >= 10,
            "recovery collapsed: got {} errors: {found:?}",
            found.len()
        );
        assert!(
            found.len() <= 14,
            "recovery is producing consequences: got {} errors: {found:?}",
            found.len()
        );
    }

    #[test]
    fn a_broken_statement_does_not_hide_the_good_ones_after_it() {
        let stmts = {
            let src = in_program("A := ;\nB := 2;\nC := 3;");
            let (unit, _, _) = parse_text(&src);
            unit.pou("Main").expect("Main").body.clone()
        };
        assert_eq!(stmts.len(), 3, "{stmts:?}");
        // The first statement survives as an assignment whose value could not
        // be built; the two after it are untouched.
        match &stmts[0].kind {
            StmtKind::Assign { value, .. } => assert!(matches!(value.kind, ExprKind::Error)),
            other => panic!("{other:?}"),
        }
        assert!(matches!(stmts[1].kind, StmtKind::Assign { .. }));
        assert!(matches!(stmts[2].kind, StmtKind::Assign { .. }));
    }

    #[test]
    fn an_unclosed_block_says_where_it_started() {
        let (_, diags, map) = parse_text("PROGRAM P\nIF a THEN\nX := 1;\nEND_PROGRAM\n");
        let text = diags.render(&map);
        assert!(text.contains("`IF` is never closed"), "{text}");
        assert!(text.contains("`IF` starts here"), "{text}");
    }

    #[test]
    fn a_mismatched_closing_keyword_is_reported_where_it_is_not_swallowed() {
        let found = errors("PROGRAM P\nIF a THEN\nX := 1;\nEND_WHILE\nEND_PROGRAM\n");
        assert!(found.contains(&"E0207"), "{found:?}");
    }

    #[test]
    fn a_missing_semicolon_resynchronises_at_the_next_one() {
        let found = errors(&in_program("A := 1\nB := 2;\nC := 3;"));
        assert_eq!(found, ["E0202"], "{found:?}");
    }

    #[test]
    fn a_broken_declaration_does_not_stop_the_ones_after_it() {
        let (unit, diags, _) =
            parse_text("PROGRAM P VAR A : ; B : INT; C : BOOL; END_VAR END_PROGRAM");
        assert!(diags.has_errors());
        let decls = &unit.pou("P").expect("P").var_blocks[0].decls;
        assert!(decls.len() >= 2, "{decls:?}");
    }

    #[test]
    fn a_broken_top_level_item_does_not_stop_the_ones_after_it() {
        let (unit, diags, _) = parse_text("nonsense here\nPROGRAM P X := 1; END_PROGRAM\n");
        assert!(diags.has_errors());
        assert!(unit.pou("P").is_some(), "{:?}", unit.items);
    }

    #[test]
    fn an_error_node_never_appears_without_a_diagnostic_beside_it() {
        for src in [
            in_program("A := ;"),
            in_program("A := ) ;"),
            in_program("FOR := 1 TO 2 DO END_FOR"),
            "PROGRAM".to_string(),
        ] {
            let (_, diags, _) = parse_text(&src);
            assert!(diags.has_errors(), "no diagnostic for {src:?}");
        }
    }

    // -- safety --------------------------------------------------------------

    #[test]
    fn ten_thousand_nested_parentheses_produce_a_diagnostic_rather_than_a_stack_overflow() {
        let src = in_program(&format!(
            "X := {}1{};",
            "(".repeat(10_000),
            ")".repeat(10_000)
        ));
        let found = errors(&src);
        assert_eq!(found, ["E0201"], "{found:?}");
    }

    #[test]
    fn ten_thousand_nested_if_statements_produce_a_diagnostic_rather_than_a_stack_overflow() {
        let src = in_program(&format!(
            "{}X := 1;{}",
            "IF a THEN ".repeat(10_000),
            "END_IF ".repeat(10_000)
        ));
        let found = errors(&src);
        assert_eq!(found, ["E0201"], "{found:?}");
    }

    #[test]
    fn a_long_operator_chain_is_bounded_too_because_its_tree_is_just_as_deep() {
        let short = format!("X := {};", vec!["1"; 50].join(" + "));
        assert!(errors(&in_program(&short)).is_empty());

        let long = format!("X := {};", vec!["1"; 5_000].join(" + "));
        assert_eq!(errors(&in_program(&long)), ["E0201"]);
    }

    #[test]
    fn a_long_member_chain_is_bounded_too() {
        let long = format!("X := a{};", ".b".repeat(5_000));
        assert_eq!(errors(&in_program(&long)), ["E0201"]);
    }

    #[test]
    fn a_deeper_bound_accepts_what_the_default_refuses() {
        // Proves the bound really is the dialect's setting rather than a
        // constant hidden in the parser.
        let deep = Dialect {
            max_nesting_depth: 200,
            ..Dialect::generic()
        };
        let src = in_program(&format!("X := {}1{};", "(".repeat(160), ")".repeat(160)));
        let (_, diags, _) = parse_with(&deep, &src);
        assert!(
            !diags.has_errors(),
            "{:?}",
            codes_of(&diags, Severity::Error)
        );
        assert_eq!(errors(&src), ["E0201"]);
    }

    #[test]
    fn pathological_input_terminates_and_never_panics() {
        let cases = [
            "",
            ";",
            ";;;;;;;;",
            "((((((((",
            "))))))))",
            "PROGRAM",
            "PROGRAM P",
            "PROGRAM P VAR",
            "IF",
            "CASE",
            "CASE OF",
            "FOR",
            "REPEAT",
            "END_IF",
            "TYPE",
            "TYPE X :",
            "CONFIGURATION",
            "RESOURCE",
            "1,1,1,1,1,1,1,1",
            "X := ",
            "X ?= ",
            ", , , ,",
            ".....",
            "**",
            "a ** ** b",
            "F(,)",
            "F(",
            "a[",
            "a[]",
            "%IX0.0 := ;",
            "VAR_GLOBAL",
            "&&&&&&&&",
            "PROGRAM P PROGRAM Q END_PROGRAM",
        ];
        for case in cases {
            // The assertion is that this returns at all. A hang or a panic here
            // is the failure; the diagnostics are not the point.
            let (unit, _, _) = parse_text(case);
            let _ = unit.items.len();
            let wrapped = in_program(case);
            let (unit, _, _) = parse_text(&wrapped);
            let _ = unit.items.len();
        }
    }

    #[test]
    fn a_case_label_scan_that_finds_no_colon_still_terminates() {
        // The one place the parser looks ahead without a bound on the grammar
        // is the CASE arm scanner; this is the input that would make it walk.
        let commas = vec!["1"; 20_000].join(",");
        let src = in_program(&format!("CASE S OF {commas} END_CASE"));
        let (_, diags, _) = parse_text(&src);
        assert!(diags.has_errors());
    }

    #[test]
    fn parsing_is_deterministic() {
        let src = "TYPE Speed : INT (0..100); END_TYPE\n\
                   PROGRAM Main\n\
                   VAR RETAIN Count : DINT := 0; T1 : TON; END_VAR\n\
                   IF Count < 10 THEN Count := Count + 1; ELSE Count := 0; END_IF\n\
                   CASE Count OF 0: A := 1; 1..3: A := 2; ELSE A := 0; END_CASE\n\
                   T1(IN := TRUE, PT := T#5s, Q => Running);\n\
                   END_PROGRAM\n";
        let (first_unit, first_diags, map) = parse_text(src);
        let (second_unit, second_diags, _) = parse_text(src);
        assert_eq!(first_unit, second_unit);
        assert_eq!(first_diags.render(&map), second_diags.render(&map));
        assert_eq!(first_unit.node_count, second_unit.node_count);
    }

    #[test]
    fn node_ids_are_unique_and_counted() {
        let unit = parse_ok(
            "PROGRAM Main\n\
             VAR Count : DINT := 0; END_VAR\n\
             FOR i := 1 TO 10 BY 2 DO\n\
               IF Count > 3 THEN Count := Count + F(a, b); ELSE Count := 0; END_IF\n\
             END_FOR\n\
             CASE Count OF 1, 2: A := 1; 3..4: A := 2; ELSE A := 0; END_CASE\n\
             END_PROGRAM\n",
        );
        let mut ids = Vec::new();
        for pou in unit.pous() {
            collect_stmt_ids(&pou.body, &mut ids);
        }
        assert!(ids.len() > 20, "the walk found almost nothing: {ids:?}");
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "a node id was reused");
        assert!(
            ids.iter().all(|id| *id < unit.node_count),
            "an id fell outside node_count {}",
            unit.node_count
        );
    }

    #[test]
    fn parse_and_parse_source_agree_on_the_same_text() {
        let src = "PROGRAM P X := a + b * c; END_PROGRAM\n";
        let mut map = SourceMap::new();
        let file = map.add("t.st", src).unwrap();
        let dialect = Dialect::generic();
        let (stream, lex_diags) = lex(file, src, &dialect);
        assert!(!lex_diags.has_errors());
        let (from_stream, _) = parse(file, src, &stream, &dialect);
        let (from_source, _) = parse_source(file, src, &dialect);
        assert_eq!(from_stream, from_source);
    }

    #[test]
    fn the_lexers_diagnostics_survive_into_the_merged_list() {
        // `2#2` is a lexical error, `X := ;` a syntactic one. Both must be in
        // the list a caller of parse_source gets.
        let found = errors(&in_program("A := 2#2;\nB := ;"));
        assert!(found.contains(&"E0106"), "{found:?}");
        assert!(found.contains(&"E0203"), "{found:?}");
    }

    #[test]
    fn comments_and_pragmas_do_not_reach_the_tree_but_do_not_derail_it() {
        let unit = parse_ok(
            "PROGRAM P (* a comment *) VAR {attribute 'x'} A : INT; END_VAR // trailing\n\
             A := 1;\n\
             END_PROGRAM\n",
        );
        let pou = unit.pou("P").expect("P");
        assert_eq!(pou.var_blocks.len(), 1);
        assert_eq!(pou.body.len(), 1);
    }
}
