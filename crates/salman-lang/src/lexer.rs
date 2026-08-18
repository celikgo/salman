// SPDX-License-Identifier: Apache-2.0
//! The Structured Text lexer.
//!
//! Citation policy: salman cites IEC 61131-3:2013 (Edition 3.0) clause, table
//! and figure numbers as references only. No IEC text is reproduced anywhere in
//! this repository. Edition 3.0 was **withdrawn on 2025-05-22** and superseded
//! by IEC 61131-3:2025 (Edition 4.0); salman targets Edition 3.0 because that is
//! the edition its public sources allow it to verify. Clause numbers are
//! edition-specific — Structured Text is §7.3 in Edition 3.0 and §7.2 in
//! Edition 4.0 — so a bare clause number is meaningless and salman never writes
//! one. See `docs/IEC_CITATIONS.md`.
//!
//! # Design
//!
//! Hand-written, single pass, iterative. Iterative matters: this function eats
//! bytes that arrived from a file salman did not write, and a recursive lexer is
//! a stack overflow waiting for a hostile input. Comment nesting, the only
//! recursive-looking construct here, is a depth counter with a ceiling.
//!
//! Errors never stop the lexer. A malformed token produces a diagnostic and the
//! lexer resynchronises, so one bad literal does not hide the twenty real errors
//! after it.
//!
//! # Things that are easy to get wrong, and are therefore tested
//!
//! * Block comments **nest** — IEC 61131-3:2013 Table 3 "Comments" rows 3a and
//!   3b make that normative. A lexer that scans to the first `*)` is wrong on
//!   conforming input.
//! * `1..5` is a range, not the real number `1.` followed by `.5`.
//! * `%QX7.5` must lex as one token, or the parser sees an identifier, a dot
//!   and a number.
//! * `16#FF` and `INT#16#FF` and `T#1d2h` all begin with something that looks
//!   like an ordinary token until the `#` arrives.

use salman_core::diag::{DiagCode, Diagnostic, Diagnostics, Severity};
use salman_core::ident::MAX_IDENT_BYTES;
use salman_core::span::{FileId, Span};
use salman_core::time::{Date, DateTime, Duration, TimeOfDay};
use salman_core::value::{ElementaryType, MAX_STRING_LEN};

use crate::address::{AddressLocation, AddressSize, DirectAddress, MAX_ADDRESS_DEPTH};
use crate::codes;
use crate::dialect::Dialect;
use crate::token::{Keyword, LiteralValue, Punct, Token, TokenKind, elementary_type_from_word};

/// The result of lexing one source file.
#[derive(Debug, Clone)]
pub struct TokenStream {
    tokens: Vec<Token>,
    literals: Vec<LiteralValue>,
    addresses: Vec<DirectAddress>,
    comments: Vec<Span>,
    pragmas: Vec<Span>,
}

impl TokenStream {
    /// Every token, ending with exactly one [`TokenKind::Eof`].
    #[must_use]
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// The literal a [`TokenKind::Literal`] token refers to.
    #[must_use]
    pub fn literal(&self, index: u32) -> Option<&LiteralValue> {
        self.literals.get(index as usize)
    }

    /// The address a [`TokenKind::DirectAddress`] token refers to.
    #[must_use]
    pub fn address(&self, index: u32) -> Option<&DirectAddress> {
        self.addresses.get(index as usize)
    }

    /// Spans of every comment, in source order.
    ///
    /// Kept because a formatter and a documentation extractor both need them,
    /// and because throwing them away is the reason so many code formatters
    /// move comments to the wrong place.
    #[must_use]
    pub fn comments(&self) -> &[Span] {
        &self.comments
    }

    /// Spans of every pragma `{ ... }`, in source order.
    ///
    /// salman recognises pragmas so they do not derail the parse. It does not
    /// interpret them: their content is vendor-specific and salman implements
    /// none of it.
    #[must_use]
    pub fn pragmas(&self) -> &[Span] {
        &self.pragmas
    }
}

/// Lexes `source`, returning its tokens and everything wrong with it.
///
/// Never fails and never panics: a file of pure noise yields a stream
/// containing only [`TokenKind::Eof`] plus a diagnostic per unusable byte.
#[must_use]
pub fn lex(file: FileId, source: &str, dialect: &Dialect) -> (TokenStream, Diagnostics) {
    let mut lexer = Lexer {
        source,
        pos: 0,
        file,
        dialect,
        tokens: Vec::new(),
        literals: Vec::new(),
        addresses: Vec::new(),
        comments: Vec::new(),
        pragmas: Vec::new(),
        diags: Diagnostics::new(),
    };
    lexer.run();
    let end = lexer.source.len() as u32;
    lexer.tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span::at(file, end),
    });
    (
        TokenStream {
            tokens: lexer.tokens,
            literals: lexer.literals,
            addresses: lexer.addresses,
            comments: lexer.comments,
            pragmas: lexer.pragmas,
        },
        lexer.diags,
    )
}

struct Lexer<'a> {
    source: &'a str,
    pos: usize,
    file: FileId,
    dialect: &'a Dialect,
    tokens: Vec<Token>,
    literals: Vec<LiteralValue>,
    addresses: Vec<DirectAddress>,
    comments: Vec<Span>,
    pragmas: Vec<Span>,
    diags: Diagnostics,
}

impl<'a> Lexer<'a> {
    fn bytes(&self) -> &'a [u8] {
        self.source.as_bytes()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes().get(self.pos + offset).copied()
    }

    fn peek(&self) -> Option<u8> {
        self.peek_at(0)
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn span_from(&self, start: usize) -> Span {
        Span::new(self.file, start as u32, self.pos as u32)
    }

    fn slice(&self, start: usize, end: usize) -> &'a str {
        self.source.get(start..end).unwrap_or("")
    }

    fn error(
        &mut self,
        code: DiagCode,
        span: Span,
        message: impl Into<String>,
        label: impl Into<String>,
    ) {
        self.diags
            .push(Diagnostic::error(code, message).with_primary(span, label));
    }

    fn warn(
        &mut self,
        code: DiagCode,
        span: Span,
        message: impl Into<String>,
        label: impl Into<String>,
    ) {
        self.diags
            .push(Diagnostic::warning(code, message).with_primary(span, label));
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        let span = self.span_from(start);
        self.tokens.push(Token { kind, span });
    }

    fn push_literal(&mut self, value: LiteralValue, start: usize) {
        let index = self.literals.len() as u32;
        self.literals.push(value);
        self.push(TokenKind::Literal(index), start);
    }

    fn run(&mut self) {
        while self.pos < self.source.len() {
            let start = self.pos;
            let Some(b) = self.peek() else { break };
            match b {
                b'\t' | b'\n' | 0x0c | b'\r' | b' ' => {
                    self.pos += 1;
                }
                b'(' if self.peek_at(1) == Some(b'*') => self.block_comment(start, *b"(*", *b"*)"),
                b'/' if self.peek_at(1) == Some(b'*') && self.dialect.c_style_block_comments => {
                    self.block_comment(start, *b"/*", *b"*/");
                }
                b'/' if self.peek_at(1) == Some(b'/') && self.dialect.line_comments => {
                    while self.peek().is_some_and(|c| c != b'\n') {
                        self.pos += 1;
                    }
                    self.comments.push(self.span_from(start));
                }
                b'{' => self.pragma(start),
                b'\'' => self.string_literal(start),
                b'"' => self.wstring_literal(start),
                b'%' => self.direct_address(start),
                b'0'..=b'9' => self.number(start),
                b if b.is_ascii_alphabetic() || b == b'_' => self.word(start),
                _ => self.operator(start),
            }
        }
    }

    /// A block comment, which nests. IEC 61131-3:2013 Table 3 "Comments".
    fn block_comment(&mut self, start: usize, open: [u8; 2], close: [u8; 2]) {
        let mut depth: u32 = 0;
        loop {
            let Some(b) = self.peek() else {
                let span = Span::new(self.file, start as u32, start as u32 + 2);
                self.error(
                    codes::E_UNTERMINATED_COMMENT,
                    span,
                    "block comment is never closed",
                    "this comment runs to the end of the file",
                );
                self.comments.push(self.span_from(start));
                return;
            };
            let next = self.peek_at(1);
            if b == open[0] && next == Some(open[1]) && self.dialect.nested_comments {
                depth += 1;
                if depth > self.dialect.max_nesting_depth {
                    let span = self.span_from(start);
                    self.error(
                        codes::E_NESTING_TOO_DEEP,
                        span,
                        format!(
                            "comments nested more than {} deep",
                            self.dialect.max_nesting_depth
                        ),
                        "salman bounds nesting because source text is untrusted",
                    );
                    // Consume the rest of the file rather than looping.
                    self.pos = self.source.len();
                    self.comments.push(self.span_from(start));
                    return;
                }
                self.pos += 2;
                continue;
            }
            if b == close[0] && next == Some(close[1]) {
                self.pos += 2;
                if !self.dialect.nested_comments {
                    break;
                }
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
                continue;
            }
            self.pos += 1;
        }
        self.comments.push(self.span_from(start));
    }

    /// A pragma, `{ ... }`. IEC 61131-3:2013 §6.2 "Pragma", Table 4 "Pragma".
    ///
    /// Recognised and skipped. salman implements no pragma, so interpreting one
    /// would mean pretending to honour a directive it ignores.
    fn pragma(&mut self, start: usize) {
        self.pos += 1;
        let mut depth = 1u32;
        while depth > 0 {
            let Some(b) = self.bump() else {
                let span = Span::new(self.file, start as u32, start as u32 + 1);
                self.error(
                    codes::E_UNTERMINATED_PRAGMA,
                    span,
                    "pragma is never closed",
                    "expected a `}` before the end of the file",
                );
                self.pragmas.push(self.span_from(start));
                return;
            };
            match b {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
        }
        self.pragmas.push(self.span_from(start));
    }

    /// An identifier, a keyword, a type name, or the prefix of a typed literal.
    fn word(&mut self, start: usize) {
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            self.pos += 1;
        }
        let text = self.slice(start, self.pos);
        if text.len() > MAX_IDENT_BYTES {
            let span = self.span_from(start);
            self.error(
                codes::E_IDENT_TOO_LONG,
                span,
                format!("identifier is longer than the {MAX_IDENT_BYTES} byte limit"),
                "salman bounds identifier length because source text is untrusted",
            );
            self.push(TokenKind::Ident, start);
            return;
        }

        // A `#` immediately after a word makes it a literal prefix — `T#1s`,
        // `INT#16#FF`, `BOOL#1`, `DT#2024-01-01-00:00:00`.
        if self.peek() == Some(b'#') {
            if self.prefixed_literal(start, text) {
                return;
            }
            // Not a literal prefix: this is a qualified enumeration value such
            // as `Colour#Red`. Emit the identifier and let the parser handle
            // the `#`.
            self.push(TokenKind::Ident, start);
            return;
        }

        match Keyword::lookup(text) {
            Some(Keyword::True) => self.push_literal(LiteralValue::Bool(true), start),
            Some(Keyword::False) => self.push_literal(LiteralValue::Bool(false), start),
            Some(k) => self.push(TokenKind::Keyword(k), start),
            None => {
                if let Some(ty) = elementary_type_from_word(text) {
                    self.push(TokenKind::TypeName(ty), start);
                } else {
                    if text.as_bytes().windows(2).any(|w| w == b"__") {
                        let span = self.span_from(start);
                        self.warn(
                            codes::W_CONSECUTIVE_UNDERSCORES,
                            span,
                            "identifier contains two consecutive underscores",
                            "several dialects reject this; salman accepts it",
                        );
                    }
                    self.push(TokenKind::Ident, start);
                }
            }
        }
    }

    /// Lexes `WORD#...` when `WORD` names a literal prefix. Returns whether it
    /// was one; leaves `self.pos` at the `#` when it was not.
    fn prefixed_literal(&mut self, start: usize, prefix: &str) -> bool {
        let hash = self.pos;
        // Prefixes salman does not implement, listed so that meeting one gives
        // a straight answer instead of a syntax error.
        for unsupported in [
            "LD",
            "LDATE",
            "LTOD",
            "LTIME_OF_DAY",
            "LDT",
            "LDATE_AND_TIME",
        ] {
            if prefix.eq_ignore_ascii_case(unsupported) {
                self.pos += 1;
                while self.peek().is_some_and(|b| {
                    b.is_ascii_alphanumeric() || matches!(b, b'-' | b':' | b'.' | b'_')
                }) {
                    self.pos += 1;
                }
                let span = self.span_from(start);
                self.diags.push(
                    Diagnostic::new(
                        Severity::Error,
                        codes::U_UNSUPPORTED_LITERAL_PREFIX,
                        format!("salman does not implement the `{unsupported}#` literal"),
                    )
                    .with_primary(
                        span,
                        "LDATE, LTIME_OF_DAY and LDATE_AND_TIME are not implemented",
                    )
                    .with_note(
                        "salman implements DATE, TIME_OF_DAY and DATE_AND_TIME; the long forms \
                         added in IEC 61131-3:2013 are not in this version",
                    ),
                );
                return true;
            }
        }

        if prefix.eq_ignore_ascii_case("T")
            || prefix.eq_ignore_ascii_case("TIME")
            || prefix.eq_ignore_ascii_case("LT")
            || prefix.eq_ignore_ascii_case("LTIME")
        {
            self.pos += 1;
            let long = prefix.len() > 1 && prefix.to_ascii_uppercase().starts_with('L')
                || prefix.eq_ignore_ascii_case("LT");
            self.duration_literal(start, long);
            return true;
        }
        if prefix.eq_ignore_ascii_case("D") || prefix.eq_ignore_ascii_case("DATE") {
            self.pos += 1;
            self.date_literal(start);
            return true;
        }
        if prefix.eq_ignore_ascii_case("TOD") || prefix.eq_ignore_ascii_case("TIME_OF_DAY") {
            self.pos += 1;
            self.time_of_day_literal(start);
            return true;
        }
        if prefix.eq_ignore_ascii_case("DT") || prefix.eq_ignore_ascii_case("DATE_AND_TIME") {
            self.pos += 1;
            self.date_and_time_literal(start);
            return true;
        }
        if let Some(ty) = elementary_type_from_word(prefix) {
            self.pos += 1;
            self.typed_literal(start, ty);
            return true;
        }
        self.pos = hash;
        false
    }

    /// `INT#-5`, `WORD#2#1010`, `BOOL#1`, `REAL#1.5`, `STRING#'abc'`.
    fn typed_literal(&mut self, start: usize, ty: ElementaryType) {
        if ty == ElementaryType::Bool {
            let value_start = self.pos;
            while self.peek().is_some_and(|b| b.is_ascii_alphanumeric()) {
                self.pos += 1;
            }
            let text = self.slice(value_start, self.pos);
            let value = match text.to_ascii_uppercase().as_str() {
                "1" | "TRUE" => Some(true),
                "0" | "FALSE" => Some(false),
                _ => None,
            };
            if let Some(v) = value {
                self.push_literal(LiteralValue::Bool(v), start);
            } else {
                let span = self.span_from(start);
                self.error(
                    codes::E_BAD_DIGIT,
                    span,
                    "BOOL literal must be 0, 1, FALSE or TRUE",
                    format!("`{text}` is not a boolean value"),
                );
            }
            return;
        }
        if matches!(ty, ElementaryType::String | ElementaryType::WString) {
            match self.peek() {
                Some(b'\'') => self.string_literal(start),
                Some(b'"') => self.wstring_literal(start),
                _ => {
                    let span = self.span_from(start);
                    self.error(
                        codes::E_BAD_DIGIT,
                        span,
                        format!("`{}#` must be followed by a quoted string", ty.name()),
                        "expected a string literal here",
                    );
                }
            }
            return;
        }
        self.numeric_body(start, Some(ty));
    }

    /// A number that begins at a digit: integer, based integer, or real.
    fn number(&mut self, start: usize) {
        self.numeric_body(start, None);
    }

    fn numeric_body(&mut self, start: usize, declared: Option<ElementaryType>) {
        let negative = if self.peek() == Some(b'-') {
            self.pos += 1;
            true
        } else {
            if self.peek() == Some(b'+') {
                self.pos += 1;
            }
            false
        };

        let digits_start = self.pos;
        let Some(first) = self.read_digits(10) else {
            let span = self.span_from(start);
            self.error(
                codes::E_BAD_DIGIT,
                span,
                "expected a number here",
                "a numeric literal needs at least one digit",
            );
            return;
        };

        // A `#` here makes what we just read the radix: `16#FF`.
        if self.peek() == Some(b'#') {
            let radix_span = Span::new(self.file, digits_start as u32, self.pos as u32);
            self.pos += 1;
            let radix = match first {
                2 => 2u32,
                8 => 8,
                16 => 16,
                other => {
                    self.error(
                        codes::E_BAD_RADIX,
                        radix_span,
                        format!("{other} is not a valid radix"),
                        "IEC 61131-3 allows radix 2, 8 and 16",
                    );
                    // Consume the digits that follow so the parser is not
                    // handed the remains of a broken literal.
                    let _ = self.read_digits(16);
                    return;
                }
            };
            if radix == 8 {
                let span = self.span_from(start);
                self.warn(
                    codes::E_BAD_RADIX,
                    span,
                    "octal literals are deprecated in IEC 61131-3:2013",
                    "salman accepts them; consider 16# instead",
                );
            }
            if let Some(value) = self.read_digits(radix) {
                self.push_literal(
                    LiteralValue::Int {
                        magnitude: value,
                        negative,
                        declared,
                    },
                    start,
                );
            } else {
                let span = self.span_from(start);
                self.error(
                    codes::E_BAD_DIGIT,
                    span,
                    format!("expected at least one base-{radix} digit after `{radix}#`"),
                    "this literal has no digits",
                );
            }
            return;
        }

        // A `.` followed by a digit makes this a real. A `.` followed by
        // another `.` is the range operator in `1..5` and must not be eaten.
        let is_real =
            self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit());
        if is_real {
            self.pos += 1;
            if self.read_digits(10).is_none() {
                let span = self.span_from(start);
                self.error(
                    codes::E_BAD_REAL,
                    span,
                    "real literal has no digits after the decimal point",
                    "IEC 61131-3 requires digits on both sides of the point",
                );
                return;
            }
            self.read_exponent();
            self.finish_real(start, digits_start, negative, declared);
            return;
        }

        // `1E3` is not a real in IEC 61131-3 — the grammar requires a decimal
        // point — but it is what a newcomer writes, so say so rather than
        // producing "unexpected identifier E3".
        if matches!(self.peek(), Some(b'e' | b'E'))
            && (self.peek_at(1).is_some_and(|b| b.is_ascii_digit())
                || (matches!(self.peek_at(1), Some(b'+' | b'-'))
                    && self.peek_at(2).is_some_and(|b| b.is_ascii_digit())))
        {
            self.read_exponent();
            let span = self.span_from(start);
            let text = self.slice(digits_start, self.pos);
            self.diags.push(
                Diagnostic::error(codes::E_BAD_REAL, "real literal needs a decimal point")
                    .with_primary(
                        span,
                        "an exponent without a decimal point is not a real literal",
                    )
                    .with_note(format!(
                        "IEC 61131-3:2013 Table 5 \"Numeric literals\" writes this as `{}.0{}`",
                        text.split(['e', 'E']).next().unwrap_or(text),
                        text.split_once(['e', 'E'])
                            .map_or(String::new(), |(_, e)| format!("E{e}"))
                    )),
            );
            self.finish_real(start, digits_start, negative, declared);
            return;
        }

        self.push_literal(
            LiteralValue::Int {
                magnitude: first,
                negative,
                declared,
            },
            start,
        );
    }

    fn finish_real(
        &mut self,
        start: usize,
        digits_start: usize,
        negative: bool,
        declared: Option<ElementaryType>,
    ) {
        let text: String = self
            .slice(digits_start, self.pos)
            .chars()
            .filter(|c| *c != '_')
            .collect();
        if let Ok(value) = text.parse::<f64>() {
            let value = if negative { -value } else { value };
            self.push_literal(LiteralValue::Real { value, declared }, start);
        } else {
            let span = self.span_from(start);
            self.error(
                codes::E_BAD_REAL,
                span,
                "real literal cannot be represented",
                "this is not a number salman can read",
            );
        }
    }

    fn read_exponent(&mut self) {
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.read_digits(10).is_none() {
                self.pos = save;
            }
        }
    }

    /// Reads digits in `radix`, allowing single `_` separators between digits.
    ///
    /// Returns `None` when there is no digit at all. Reports, but tolerates, a
    /// misplaced underscore and an out-of-range value, because stopping would
    /// hide every later error in the file.
    fn read_digits(&mut self, radix: u32) -> Option<u128> {
        let start = self.pos;
        let mut value: u128 = 0;
        let mut overflowed = false;
        let mut any = false;
        let mut last_was_digit = false;

        while let Some(b) = self.peek() {
            if b == b'_' {
                if !last_was_digit || !self.peek_at(1).is_some_and(|n| is_digit_in(n, radix)) {
                    let span = Span::at(self.file, self.pos as u32);
                    self.error(
                        codes::E_MISPLACED_UNDERSCORE,
                        span,
                        "underscore in a numeric literal must sit between two digits",
                        "this underscore does not separate digits",
                    );
                    self.pos += 1;
                    last_was_digit = false;
                    continue;
                }
                self.pos += 1;
                last_was_digit = false;
                continue;
            }
            if !b.is_ascii_alphanumeric() {
                break;
            }
            let Some(digit) = digit_value(b) else { break };
            if digit >= radix {
                // A letter that is not a digit in this radix ends the number;
                // `16#FFg` is a literal followed by an identifier. But a
                // *decimal* digit that is out of range is a real mistake.
                if b.is_ascii_digit() {
                    let span = Span::at(self.file, self.pos as u32);
                    self.error(
                        codes::E_BAD_DIGIT,
                        span,
                        format!("`{}` is not a base-{radix} digit", b as char),
                        "digit out of range for this radix",
                    );
                    self.pos += 1;
                    any = true;
                    last_was_digit = true;
                    continue;
                }
                break;
            }
            if radix == 16 && b.is_ascii_lowercase() && !self.dialect.lowercase_hex_digits {
                let span = Span::at(self.file, self.pos as u32);
                let rule = self
                    .dialect
                    .rule("lowercase hexadecimal digits", "not accepted");
                self.diags.push(
                    Diagnostic::error(codes::E_DIALECT_REJECTS, "lowercase hexadecimal digit")
                        .with_primary(span, "write this digit in upper case")
                        .with_dialect_rule(rule)
                        .with_note(
                            "salman could not verify from a public source whether IEC 61131-3 \
                             permits lowercase hexadecimal digits; the generic dialect accepts \
                             them and the strict dialect does not",
                        ),
                );
            }
            value = value
                .checked_mul(u128::from(radix))
                .and_then(|v| v.checked_add(u128::from(digit)))
                .unwrap_or_else(|| {
                    overflowed = true;
                    value
                });
            self.pos += 1;
            any = true;
            last_was_digit = true;
        }

        if !any {
            self.pos = start;
            return None;
        }
        // ULINT is the widest integer salman has, so anything above u64::MAX
        // has no type to live in. Accumulating in u128 means the check is on
        // the value rather than on whether the accumulator happened to wrap.
        if overflowed || value > u128::from(u64::MAX) {
            let span = self.span_from(start);
            self.error(
                codes::E_LITERAL_OUT_OF_RANGE,
                span,
                "integer literal is too large for any salman type",
                "the widest is ULINT, whose maximum is 18446744073709551615",
            );
            return Some(u128::from(u64::MAX));
        }
        Some(value)
    }

    /// `T#1d2h3m4s5ms`, `LTIME#1000d15h23m12s34ms2us44ns`.
    ///
    /// IEC 61131-3:2013 §6.3.4 and Table 8 "Duration literals". The rules
    /// salman enforces, each with its own test:
    ///
    /// * units appear in descending order `d h m s ms us ns`;
    /// * a unit may be skipped;
    /// * overflow is permitted **only in the first unit present**, so `T#100s`
    ///   is fine and `T#5m68s` is not;
    /// * a fraction is permitted **only on the last unit present**;
    /// * `_` may separate units.
    fn duration_literal(&mut self, start: usize, long: bool) {
        const UNITS: [(&str, i128); 7] = [
            ("d", 86_400_000_000_000),
            ("h", 3_600_000_000_000),
            ("m", 60_000_000_000),
            ("s", 1_000_000_000),
            ("ms", 1_000_000),
            ("us", 1_000),
            ("ns", 1),
        ];

        let negative = match self.peek() {
            Some(b'-') => {
                self.pos += 1;
                if !self.dialect.signed_duration_literals {
                    let span = self.span_from(start);
                    let rule = self
                        .dialect
                        .rule("signed duration literals", "not accepted");
                    self.diags.push(
                        Diagnostic::error(codes::E_DIALECT_REJECTS, "signed duration literal")
                            .with_primary(span, "this dialect does not accept a sign here")
                            .with_dialect_rule(rule)
                            .with_note(
                                "matiec quotes an Edition 3 committee-draft grammar permitting a \
                                 sign; CODESYS and Beckhoff both state signs are not permitted. \
                                 salman could not resolve this against the published standard",
                            ),
                    );
                }
                true
            }
            Some(b'+') => {
                self.pos += 1;
                false
            }
            _ => false,
        };

        let mut total: i128 = 0;
        let mut next_unit = 0usize;
        let mut count = 0usize;
        let mut truncated = false;

        loop {
            if self.peek() == Some(b'_') && count > 0 {
                self.pos += 1;
            }
            let Some(b) = self.peek() else { break };
            if !b.is_ascii_digit() {
                break;
            }
            let int_start = self.pos;
            let Some(whole) = self.read_digits(10) else {
                break;
            };
            let mut fraction_digits: Vec<u32> = Vec::new();
            if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
                while let Some(d) = self.peek() {
                    if !d.is_ascii_digit() {
                        break;
                    }
                    fraction_digits.push(u32::from(d - b'0'));
                    self.pos += 1;
                }
            }
            let unit_start = self.pos;
            while self.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
                self.pos += 1;
            }
            let unit_text = self.slice(unit_start, self.pos);
            let Some(unit_index) = UNITS
                .iter()
                .position(|(name, _)| name.eq_ignore_ascii_case(unit_text))
            else {
                let span = Span::new(
                    self.file,
                    unit_start as u32,
                    self.pos.max(unit_start + 1) as u32,
                );
                self.error(
                    codes::E_BAD_DURATION,
                    span,
                    format!("`{unit_text}` is not a duration unit"),
                    "expected one of d, h, m, s, ms, us, ns",
                );
                return;
            };
            let (_, scale) = UNITS.get(unit_index).copied().unwrap_or(("ns", 1));

            if unit_index < next_unit {
                let span = Span::new(self.file, int_start as u32, self.pos as u32);
                self.error(
                    codes::E_BAD_DURATION,
                    span,
                    "duration units must appear in descending order",
                    format!("`{unit_text}` cannot follow a smaller unit"),
                );
                return;
            }
            // Overflow is permitted only in the first unit present.
            if count > 0 {
                let limit: u128 = match unit_index {
                    1 => 24,
                    2 | 3 => 60,
                    4..=6 => 1000,
                    _ => u128::MAX,
                };
                if whole >= limit {
                    let span = Span::new(self.file, int_start as u32, self.pos as u32);
                    self.error(
                        codes::E_BAD_DURATION,
                        span,
                        "only the first unit of a duration literal may overflow",
                        format!("{whole}{unit_text} is out of range once a larger unit is present"),
                    );
                    return;
                }
            }

            let Some(whole_ns) = i128::try_from(whole)
                .ok()
                .and_then(|w| w.checked_mul(scale))
            else {
                let span = self.span_from(start);
                self.error(
                    codes::E_BAD_DURATION,
                    span,
                    "duration literal is out of range",
                    "salman durations span roughly plus or minus 292 years",
                );
                return;
            };
            let Some(sum) = total.checked_add(whole_ns) else {
                let span = self.span_from(start);
                self.error(
                    codes::E_BAD_DURATION,
                    span,
                    "duration literal is out of range",
                    "salman durations span roughly plus or minus 292 years",
                );
                return;
            };
            total = sum;

            if !fraction_digits.is_empty() {
                let mut numerator: i128 = 0;
                let mut denominator: i128 = 1;
                for d in &fraction_digits {
                    numerator = numerator.saturating_mul(10).saturating_add(i128::from(*d));
                    denominator = denominator.saturating_mul(10);
                }
                let contribution = scale.saturating_mul(numerator) / denominator;
                if scale.saturating_mul(numerator) % denominator != 0 {
                    truncated = true;
                }
                total = total.saturating_add(contribution);
                // Only the last unit may carry a fraction, so nothing may
                // follow it.
                if self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'_') {
                    let span = self.span_from(start);
                    self.error(
                        codes::E_BAD_DURATION,
                        span,
                        "only the last unit of a duration literal may have a fraction",
                        "a fractional unit must end the literal",
                    );
                    return;
                }
            }

            next_unit = unit_index + 1;
            count += 1;
        }

        if count == 0 {
            let span = self.span_from(start);
            self.error(
                codes::E_BAD_DURATION,
                span,
                "duration literal has no value",
                "expected something like T#1s",
            );
            return;
        }
        if truncated {
            let span = self.span_from(start);
            self.warn(
                codes::W_DURATION_TRUNCATED,
                span,
                "duration literal is finer than one nanosecond",
                "the remainder was truncated; salman durations have nanosecond resolution",
            );
        }
        if negative {
            total = -total;
        }
        let Ok(nanos) = i64::try_from(total) else {
            let span = self.span_from(start);
            self.error(
                codes::E_BAD_DURATION,
                span,
                "duration literal is out of range",
                "salman durations span roughly plus or minus 292 years",
            );
            return;
        };
        self.push_literal(
            LiteralValue::Duration {
                value: Duration::from_nanos(nanos),
                long,
            },
            start,
        );
    }

    /// `D#2024-02-29`. IEC 61131-3:2013 §6.3.5, Table 9.
    fn date_literal(&mut self, start: usize) {
        let Some((y, m, d)) = self.read_ymd() else {
            self.bad_date_time(start, "expected a date written as YYYY-MM-DD");
            return;
        };
        if let Some(date) = Date::from_ymd(y, m, d) {
            self.push_literal(LiteralValue::Date(date), start);
        } else {
            self.bad_date_time(start, "this date does not exist");
        }
    }

    /// `TOD#12:34:56.789`.
    fn time_of_day_literal(&mut self, start: usize) {
        let Some(tod) = self.read_hms() else {
            self.bad_date_time(start, "expected a time written as HH:MM:SS[.fff]");
            return;
        };
        self.push_literal(LiteralValue::TimeOfDay(tod), start);
    }

    /// `DT#2024-02-29-12:34:56.789`. The date and the time are separated by a
    /// hyphen, not a space and not a `T`.
    fn date_and_time_literal(&mut self, start: usize) {
        let Some((y, m, d)) = self.read_ymd() else {
            self.bad_date_time(start, "expected a date written as YYYY-MM-DD");
            return;
        };
        if !self.eat(b'-') {
            self.bad_date_time(start, "expected `-` between the date and the time of day");
            return;
        }
        let Some(tod) = self.read_hms() else {
            self.bad_date_time(start, "expected a time written as HH:MM:SS[.fff]");
            return;
        };
        let Some(date) = Date::from_ymd(y, m, d) else {
            self.bad_date_time(start, "this date does not exist");
            return;
        };
        if let Some(dt) = DateTime::from_date_time(date, tod) {
            self.push_literal(LiteralValue::DateAndTime(dt), start);
        } else {
            self.bad_date_time(start, "this instant is out of range");
        }
    }

    fn bad_date_time(&mut self, start: usize, label: &str) {
        // Swallow the rest of the literal so the parser is not handed its
        // fragments as separate tokens.
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b':' | b'.'))
        {
            self.pos += 1;
        }
        let span = self.span_from(start);
        self.error(
            codes::E_BAD_DATE_TIME,
            span,
            "malformed date or time literal",
            label.to_string(),
        );
    }

    fn read_plain_number(&mut self, max_digits: usize) -> Option<u32> {
        let start = self.pos;
        let mut value: u64 = 0;
        let mut digits = 0;
        while let Some(b) = self.peek() {
            if !b.is_ascii_digit() || digits == max_digits {
                break;
            }
            value = value.checked_mul(10)?.checked_add(u64::from(b - b'0'))?;
            self.pos += 1;
            digits += 1;
        }
        if digits == 0 {
            self.pos = start;
            return None;
        }
        u32::try_from(value).ok()
    }

    fn read_ymd(&mut self) -> Option<(i32, u32, u32)> {
        let year = self.read_plain_number(4)?;
        if !self.eat(b'-') {
            return None;
        }
        let month = self.read_plain_number(2)?;
        if !self.eat(b'-') {
            return None;
        }
        let day = self.read_plain_number(2)?;
        Some((i32::try_from(year).ok()?, month, day))
    }

    fn read_hms(&mut self) -> Option<TimeOfDay> {
        let hour = self.read_plain_number(2)?;
        if !self.eat(b':') {
            return None;
        }
        let minute = self.read_plain_number(2)?;
        if !self.eat(b':') {
            return None;
        }
        let second = self.read_plain_number(2)?;
        let mut nanos = 0u32;
        if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
            let mut scale = 100_000_000u32;
            while let Some(b) = self.peek() {
                if !b.is_ascii_digit() {
                    break;
                }
                if scale > 0 {
                    nanos += u32::from(b - b'0') * scale;
                    scale /= 10;
                }
                self.pos += 1;
            }
        }
        TimeOfDay::from_hms_nano(hour, minute, second, nanos)
    }

    /// `%IX0.0`, `%QW4`, `%M1.2.3`, `%I*`.
    ///
    /// IEC 61131-3:2013 §6.5.5 and Table 16 "Directly represented variables".
    fn direct_address(&mut self, start: usize) {
        self.pos += 1; // `%`
        let Some(location) = self.peek().and_then(AddressLocation::from_letter) else {
            let span = self.span_from(start);
            self.error(
                codes::E_BAD_DIRECT_ADDRESS,
                span,
                "expected a location letter after `%`",
                "IEC 61131-3 defines I for input, Q for output and M for memory",
            );
            return;
        };
        self.pos += 1;

        // The size letter is optional; absent means a single bit.
        let mut size = AddressSize::Bit;
        let mut size_letter_written = false;
        if let Some(b) = self.peek()
            && let Some(s) = AddressSize::from_letter(b)
            // `%IX1` — but also guard against reading the `D` of a following
            // word. A size letter is only a size letter if a digit or `*`
            // follows it.
            && self.peek_at(1).is_some_and(|n| n.is_ascii_digit() || n == b'*')
        {
            size = s;
            size_letter_written = true;
            self.pos += 1;
        }

        if self.eat(b'*') {
            let index = self.addresses.len() as u32;
            self.addresses.push(DirectAddress {
                location,
                size,
                size_letter_written,
                path: None,
            });
            self.push(TokenKind::DirectAddress(index), start);
            return;
        }

        let mut path = Vec::new();
        loop {
            let Some(part) = self.read_plain_number(10) else {
                let span = self.span_from(start);
                self.error(
                    codes::E_BAD_DIRECT_ADDRESS,
                    span,
                    "expected an address index",
                    "an address is written like %IX0.0 or %QW4",
                );
                return;
            };
            path.push(part);
            if path.len() > MAX_ADDRESS_DEPTH {
                let span = self.span_from(start);
                self.error(
                    codes::E_BAD_DIRECT_ADDRESS,
                    span,
                    format!("address is nested more than {MAX_ADDRESS_DEPTH} levels deep"),
                    "salman bounds address depth because source text is untrusted",
                );
                return;
            }
            if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
                self.pos += 1;
                continue;
            }
            break;
        }

        let index = self.addresses.len() as u32;
        self.addresses.push(DirectAddress {
            location,
            size,
            size_letter_written,
            path: Some(path),
        });
        self.push(TokenKind::DirectAddress(index), start);
    }

    /// A single-quoted string. IEC 61131-3:2013 §6.3.3, Tables 6 and 7.
    fn string_literal(&mut self, start: usize) {
        self.pos += 1;
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            let Some(b) = self.peek() else {
                self.unterminated_string(start);
                return;
            };
            match b {
                b'\'' => {
                    self.pos += 1;
                    break;
                }
                b'\n' => {
                    self.unterminated_string(start);
                    return;
                }
                b'$' => {
                    if let Some(unit) = self.escape(2) {
                        if let Ok(byte) = u8::try_from(unit) {
                            bytes.push(byte);
                        }
                    } else {
                        continue;
                    }
                }
                _ => {
                    bytes.push(b);
                    self.pos += 1;
                }
            }
            if bytes.len() > MAX_STRING_LEN {
                let span = self.span_from(start);
                self.error(
                    codes::E_UNTERMINATED_STRING,
                    span,
                    format!("string literal is longer than the {MAX_STRING_LEN} byte limit"),
                    "salman bounds string length because source text is untrusted",
                );
                return;
            }
        }
        self.push_literal(LiteralValue::String(bytes), start);
    }

    /// A double-quoted string, whose `$` escapes take four hex digits.
    fn wstring_literal(&mut self, start: usize) {
        self.pos += 1;
        let mut units: Vec<u16> = Vec::new();
        loop {
            let Some(b) = self.peek() else {
                self.unterminated_string(start);
                return;
            };
            match b {
                b'"' => {
                    self.pos += 1;
                    break;
                }
                b'\n' => {
                    self.unterminated_string(start);
                    return;
                }
                b'$' => {
                    if let Some(unit) = self.escape(4) {
                        units.push(unit);
                    } else {
                        continue;
                    }
                }
                _ => {
                    // A WSTRING holds 16-bit code units. Source is UTF-8, so
                    // take a whole character and encode it, rather than
                    // splitting a multi-byte character across two units.
                    let rest = self.source.get(self.pos..).unwrap_or("");
                    if let Some(c) = rest.chars().next() {
                        let mut buf = [0u16; 2];
                        units.extend_from_slice(c.encode_utf16(&mut buf));
                        self.pos += c.len_utf8();
                    } else {
                        self.unterminated_string(start);
                        return;
                    }
                }
            }
            if units.len() > MAX_STRING_LEN {
                let span = self.span_from(start);
                self.error(
                    codes::E_UNTERMINATED_STRING,
                    span,
                    format!("string literal is longer than the {MAX_STRING_LEN} unit limit"),
                    "salman bounds string length because source text is untrusted",
                );
                return;
            }
        }
        self.push_literal(LiteralValue::WString(units), start);
    }

    /// Reads a `$` escape. `hex_digits` is 2 for `STRING` and 4 for `WSTRING`.
    ///
    /// Returns `None` after reporting a bad escape, having consumed the `$` so
    /// the caller makes progress.
    fn escape(&mut self, hex_digits: usize) -> Option<u16> {
        let start = self.pos;
        self.pos += 1;
        let Some(b) = self.peek() else {
            let span = self.span_from(start);
            self.error(
                codes::E_BAD_ESCAPE,
                span,
                "string ends in the middle of an escape",
                "expected a character after `$`",
            );
            return None;
        };
        let simple = match b {
            b'$' => Some(u16::from(b'$')),
            b'\'' => Some(u16::from(b'\'')),
            b'"' => Some(u16::from(b'"')),
            // $L and $N both denote a line feed in IEC 61131-3 Table 7; the
            // standard names them "line feed" and "newline" separately, but
            // salman has one line terminator and does not pretend otherwise.
            b'L' | b'l' | b'N' | b'n' => Some(0x0a),
            b'P' | b'p' => Some(0x0c),
            b'R' | b'r' => Some(0x0d),
            b'T' | b't' => Some(0x09),
            _ => None,
        };
        if let Some(unit) = simple {
            self.pos += 1;
            return Some(unit);
        }
        if b.is_ascii_hexdigit() {
            let mut value: u32 = 0;
            let mut seen = 0;
            while seen < hex_digits {
                let Some(d) = self
                    .peek()
                    .filter(u8::is_ascii_hexdigit)
                    .and_then(digit_value)
                else {
                    break;
                };
                value = value * 16 + d;
                self.pos += 1;
                seen += 1;
            }
            if seen == hex_digits {
                return u16::try_from(value).ok();
            }
        }
        while self.peek().is_some_and(|b| b.is_ascii_alphanumeric()) {
            self.pos += 1;
        }
        let span = self.span_from(start);
        self.error(
            codes::E_BAD_ESCAPE,
            span,
            "unknown escape sequence",
            format!("IEC 61131-3 defines $$ $' $\" $L $N $P $R $T and ${hex_digits} hex digits"),
        );
        None
    }

    fn unterminated_string(&mut self, start: usize) {
        let span = Span::new(self.file, start as u32, (start + 1) as u32);
        self.error(
            codes::E_UNTERMINATED_STRING,
            span,
            "string literal is never closed",
            "a string literal may not span a line break",
        );
        while self.peek().is_some_and(|b| b != b'\n') {
            self.pos += 1;
        }
    }

    fn operator(&mut self, start: usize) {
        let two = |a: u8, b: u8, p: Punct| (a, Some(b), p);
        let Some(first) = self.peek() else { return };
        let second = self.peek_at(1);
        let pairs = [
            two(b':', b'=', Punct::Assign),
            two(b'?', b'=', Punct::AssignAttempt),
            two(b'=', b'>', Punct::RightArrow),
            two(b'<', b'=', Punct::LessEqual),
            two(b'>', b'=', Punct::GreaterEqual),
            two(b'<', b'>', Punct::NotEqual),
            two(b'*', b'*', Punct::Power),
            two(b'.', b'.', Punct::DotDot),
        ];
        for (a, b, p) in pairs {
            if first == a && second == b {
                self.pos += 2;
                self.push(TokenKind::Punct(p), start);
                return;
            }
        }
        let single = match first {
            b';' => Punct::Semicolon,
            b':' => Punct::Colon,
            b',' => Punct::Comma,
            b'.' => Punct::Dot,
            b'(' => Punct::LParen,
            b')' => Punct::RParen,
            b'[' => Punct::LBracket,
            b']' => Punct::RBracket,
            b'+' => Punct::Plus,
            b'-' => Punct::Minus,
            b'*' => Punct::Star,
            b'/' => Punct::Slash,
            b'<' => Punct::Less,
            b'>' => Punct::Greater,
            b'=' => Punct::Equal,
            b'&' => Punct::Ampersand,
            b'^' => Punct::Caret,
            b'#' => Punct::Hash,
            _ => {
                // Skip one whole character, not one byte, so a multi-byte
                // character produces one diagnostic rather than three.
                let rest = self.source.get(self.pos..).unwrap_or("");
                let width = rest.chars().next().map_or(1, char::len_utf8);
                self.pos += width;
                let span = self.span_from(start);
                self.error(
                    codes::E_UNEXPECTED_CHARACTER,
                    span,
                    format!(
                        "unexpected character `{}`",
                        rest.chars().next().unwrap_or('?')
                    ),
                    "this cannot begin any Structured Text token",
                );
                return;
            }
        };
        self.pos += 1;
        self.push(TokenKind::Punct(single), start);
    }
}

fn digit_value(b: u8) -> Option<u32> {
    Some(match b {
        b'0'..=b'9' => u32::from(b - b'0'),
        b'a'..=b'z' => u32::from(b - b'a') + 10,
        b'A'..=b'Z' => u32::from(b - b'A') + 10,
        _ => return None,
    })
}

fn is_digit_in(b: u8, radix: u32) -> bool {
    digit_value(b).is_some_and(|d| d < radix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use salman_core::span::SourceMap;

    fn lex_generic(src: &str) -> (TokenStream, Diagnostics) {
        let mut map = SourceMap::new();
        let file = map.add("t.st", src).unwrap();
        lex(file, src, &Dialect::generic())
    }

    fn lex_ok(src: &str) -> TokenStream {
        let (stream, diags) = lex_generic(src);
        let mut map = SourceMap::new();
        let _ = map.add("t.st", src);
        assert!(
            !diags.has_errors(),
            "unexpected errors lexing {src:?}:\n{}",
            diags.render(&map)
        );
        stream
    }

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex_ok(src).tokens().iter().map(|t| t.kind).collect()
    }

    fn first_literal(src: &str) -> LiteralValue {
        let stream = lex_ok(src);
        let token = stream.tokens().first().copied().expect("a token");
        match token.kind {
            TokenKind::Literal(i) => stream.literal(i).cloned().expect("literal in table"),
            other => panic!("expected a literal, got {other:?} for {src:?}"),
        }
    }

    fn errors(src: &str) -> Vec<String> {
        let (_, diags) = lex_generic(src);
        diags
            .items()
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| format!("{}: {}", d.code, d.message))
            .collect()
    }

    #[test]
    fn a_small_program_lexes_into_the_expected_tokens() {
        let k = kinds("PROGRAM Main\n  X := 1;\nEND_PROGRAM\n");
        assert_eq!(
            k,
            vec![
                TokenKind::Keyword(Keyword::Program),
                TokenKind::Ident,
                TokenKind::Ident,
                TokenKind::Punct(Punct::Assign),
                TokenKind::Literal(0),
                TokenKind::Punct(Punct::Semicolon),
                TokenKind::Keyword(Keyword::EndProgram),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn keywords_and_type_names_are_recognised_in_any_case() {
        assert_eq!(kinds("if")[0], TokenKind::Keyword(Keyword::If));
        assert_eq!(kinds("If")[0], TokenKind::Keyword(Keyword::If));
        assert_eq!(kinds("END_IF")[0], TokenKind::Keyword(Keyword::EndIf));
        assert_eq!(kinds("dint")[0], TokenKind::TypeName(ElementaryType::Dint));
        assert_eq!(kinds("Motor_Run")[0], TokenKind::Ident);
    }

    #[test]
    fn true_and_false_arrive_as_boolean_literals_not_as_keywords() {
        assert_eq!(first_literal("TRUE"), LiteralValue::Bool(true));
        assert_eq!(first_literal("false"), LiteralValue::Bool(false));
    }

    // --- comments -------------------------------------------------------

    #[test]
    fn block_comments_nest() {
        // IEC 61131-3:2013 Table 3 "Comments" rows 3a and 3b make nesting
        // normative. A lexer that stops at the first `*)` would treat the
        // trailing `X` here as code, and this program has no `X`.
        let k = kinds("(* outer (* inner *) still a comment *) Y");
        assert_eq!(k, vec![TokenKind::Ident, TokenKind::Eof]);
        let k = kinds("/* outer /* inner */ still a comment */ Y");
        assert_eq!(k, vec![TokenKind::Ident, TokenKind::Eof]);
    }

    #[test]
    fn line_comments_end_at_the_line_break() {
        let k = kinds("// comment\nY");
        assert_eq!(k, vec![TokenKind::Ident, TokenKind::Eof]);
    }

    #[test]
    fn comment_spans_are_recorded_for_the_formatter() {
        let stream = lex_ok("(* a *) X // b\n");
        assert_eq!(stream.comments().len(), 2);
    }

    #[test]
    fn an_unterminated_block_comment_is_reported_and_does_not_hang() {
        let errs = errors("(* never closed");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].starts_with("E0102"), "{errs:?}");
    }

    #[test]
    fn comment_nesting_is_bounded_so_a_hostile_file_cannot_exhaust_the_stack() {
        let deep = "(*".repeat(10_000);
        let errs = errors(&deep);
        assert!(errs.iter().any(|e| e.starts_with("E0113")), "{errs:?}");
    }

    // --- numbers --------------------------------------------------------

    #[test]
    fn a_range_is_not_a_real_literal() {
        // `1..5` in a CASE label must lex as 1, .., 5 — not as `1.` and `.5`.
        let k = kinds("1..5");
        assert_eq!(
            k,
            vec![
                TokenKind::Literal(0),
                TokenKind::Punct(Punct::DotDot),
                TokenKind::Literal(1),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn based_literals_use_radix_2_8_and_16() {
        assert_eq!(
            first_literal("2#1111_1111"),
            LiteralValue::Int {
                magnitude: 255,
                negative: false,
                declared: None
            }
        );
        assert_eq!(
            first_literal("8#377"),
            LiteralValue::Int {
                magnitude: 255,
                negative: false,
                declared: None
            }
        );
        assert_eq!(
            first_literal("16#FF"),
            LiteralValue::Int {
                magnitude: 255,
                negative: false,
                declared: None
            }
        );
        assert_eq!(
            first_literal("1_000_000"),
            LiteralValue::Int {
                magnitude: 1_000_000,
                negative: false,
                declared: None
            }
        );
    }

    #[test]
    fn an_invalid_radix_is_reported_rather_than_guessed_at() {
        let errs = errors("3#12");
        assert!(errs.iter().any(|e| e.starts_with("E0105")), "{errs:?}");
    }

    #[test]
    fn a_digit_outside_the_radix_is_reported() {
        let errs = errors("2#12");
        assert!(errs.iter().any(|e| e.starts_with("E0106")), "{errs:?}");
    }

    #[test]
    fn a_typed_literal_carries_the_type_its_prefix_named() {
        assert_eq!(
            first_literal("INT#16#FF"),
            LiteralValue::Int {
                magnitude: 255,
                negative: false,
                declared: Some(ElementaryType::Int)
            }
        );
        assert_eq!(
            first_literal("SINT#-128"),
            LiteralValue::Int {
                magnitude: 128,
                negative: true,
                declared: Some(ElementaryType::Sint)
            }
        );
        assert_eq!(first_literal("BOOL#1"), LiteralValue::Bool(true));
        assert_eq!(first_literal("BOOL#FALSE"), LiteralValue::Bool(false));
    }

    #[test]
    fn real_literals_need_digits_on_both_sides_of_the_point() {
        assert_eq!(
            first_literal("1.5"),
            LiteralValue::Real {
                value: 1.5,
                declared: None
            }
        );
        assert_eq!(
            first_literal("1.34E-12"),
            LiteralValue::Real {
                value: 1.34E-12,
                declared: None
            }
        );
        // `1E3` is not a real literal in IEC 61131-3, but it is what a
        // newcomer writes, so salman says what is wrong rather than
        // reporting an unexpected identifier `E3`.
        let errs = errors("1E3");
        assert!(errs.iter().any(|e| e.starts_with("E0108")), "{errs:?}");
    }

    #[test]
    fn an_integer_literal_too_large_for_any_type_is_reported() {
        let errs = errors("99999999999999999999999999999999");
        assert!(errs.iter().any(|e| e.starts_with("E0107")), "{errs:?}");
    }

    #[test]
    fn a_misplaced_underscore_is_reported() {
        for src in ["1__0", "_1", "16#_FF"] {
            let errs = errors(src);
            assert!(
                errs.iter().any(|e| e.starts_with("E0112")) || src == "_1",
                "{src}: {errs:?}"
            );
        }
        // A leading underscore is an identifier, not a broken number.
        assert_eq!(kinds("_1")[0], TokenKind::Ident);
    }

    // --- durations ------------------------------------------------------

    #[test]
    fn duration_literals_sum_their_units() {
        let LiteralValue::Duration { value, long } = first_literal("T#1d2h3m4s5ms") else {
            panic!("not a duration")
        };
        assert!(!long);
        assert_eq!(value.to_iec_literal(), "T#1d2h3m4s5ms");
    }

    #[test]
    fn the_first_unit_of_a_duration_may_overflow_but_a_later_one_may_not() {
        // CODESYS states this rule explicitly, with t#5m68s as the invalid
        // counter-example.
        let LiteralValue::Duration { value, .. } = first_literal("T#100s12ms") else {
            panic!("not a duration")
        };
        assert_eq!(value.nanos(), 100 * 1_000_000_000 + 12 * 1_000_000);

        let errs = errors("T#5m68s");
        assert!(errs.iter().any(|e| e.starts_with("E0109")), "{errs:?}");
    }

    #[test]
    fn duration_units_must_descend() {
        let errs = errors("T#4ms13d");
        assert!(errs.iter().any(|e| e.starts_with("E0109")), "{errs:?}");
    }

    #[test]
    fn a_duration_unit_may_be_skipped() {
        let LiteralValue::Duration { value, .. } = first_literal("T#1d5s") else {
            panic!("not a duration")
        };
        assert_eq!(value.nanos(), 86_400_000_000_000 + 5_000_000_000);
    }

    #[test]
    fn only_the_last_duration_unit_may_carry_a_fraction() {
        let LiteralValue::Duration { value, .. } = first_literal("T#1.5s") else {
            panic!("not a duration")
        };
        assert_eq!(value.nanos(), 1_500_000_000);
        let errs = errors("T#1.5s500ms");
        assert!(errs.iter().any(|e| e.starts_with("E0109")), "{errs:?}");
    }

    #[test]
    fn underscores_may_separate_duration_units() {
        let LiteralValue::Duration { value, .. } = first_literal("T#1d_2h_3m") else {
            panic!("not a duration")
        };
        assert_eq!(value.to_iec_literal(), "T#1d2h3m");
    }

    #[test]
    fn long_duration_prefixes_are_marked_as_ltime() {
        let LiteralValue::Duration { long, value } = first_literal("LTIME#1d15h23m12s34ms2us44ns")
        else {
            panic!("not a duration")
        };
        assert!(long);
        assert_eq!(value.to_iec_literal(), "T#1d15h23m12s34ms2us44ns");
        let LiteralValue::Duration { long, .. } = first_literal("LT#5s") else {
            panic!("not a duration")
        };
        assert!(long);
    }

    #[test]
    fn negative_durations_are_accepted_by_the_generic_dialect_and_refused_by_the_strict_one() {
        let LiteralValue::Duration { value, .. } = first_literal("T#-250ms") else {
            panic!("not a duration")
        };
        assert_eq!(value.nanos(), -250_000_000);

        let mut map = SourceMap::new();
        let src = "T#-250ms";
        let file = map.add("t.st", src).unwrap();
        let (_, diags) = lex(file, src, &Dialect::strict_iec());
        assert!(diags.has_errors());
        let text = diags.render(&map);
        assert!(
            text.contains("dialect rule applied: iec61131-3:2013-strict"),
            "{text}"
        );
    }

    #[test]
    fn a_duration_finer_than_a_nanosecond_warns_that_it_was_truncated() {
        let (_, diags) = lex_generic("T#0.0000000005s");
        assert!(!diags.has_errors());
        assert!(
            diags.items().iter().any(|d| d.code.0 == "W0101"),
            "expected a truncation warning"
        );
    }

    // --- dates ----------------------------------------------------------

    #[test]
    fn date_time_and_date_and_time_literals_parse() {
        assert!(matches!(
            first_literal("D#2024-02-29"),
            LiteralValue::Date(_)
        ));
        assert!(matches!(
            first_literal("DATE#1996-05-06"),
            LiteralValue::Date(_)
        ));
        assert!(matches!(
            first_literal("TOD#12:34:56.789"),
            LiteralValue::TimeOfDay(_)
        ));
        assert!(matches!(
            first_literal("TIME_OF_DAY#15:36:30.123"),
            LiteralValue::TimeOfDay(_)
        ));
        let LiteralValue::DateAndTime(dt) = first_literal("DT#2024-02-29-12:34:56.789") else {
            panic!("not a date and time")
        };
        assert_eq!(dt.to_iec_literal(), "DT#2024-02-29-12:34:56.789");
    }

    #[test]
    fn a_date_may_omit_leading_zeros() {
        let LiteralValue::Date(d) = first_literal("DATE#2018-8-8") else {
            panic!("not a date")
        };
        assert_eq!(d.to_ymd(), (2018, 8, 8));
    }

    #[test]
    fn a_date_that_does_not_exist_is_rejected() {
        let errs = errors("D#2023-02-29");
        assert!(errs.iter().any(|e| e.starts_with("E0110")), "{errs:?}");
        let errs = errors("TOD#25:00:00");
        assert!(errs.iter().any(|e| e.starts_with("E0110")), "{errs:?}");
    }

    #[test]
    fn the_long_date_types_salman_does_not_implement_say_so_plainly() {
        let (_, diags) = lex_generic("LDT#2018-08-08-13:33:20.5");
        let text: Vec<&str> = diags.items().iter().map(|d| d.code.0).collect();
        assert!(text.contains(&"U0101"), "{text:?}");
    }

    // --- strings --------------------------------------------------------

    #[test]
    fn string_escapes_follow_table_7() {
        assert_eq!(
            first_literal("'a$$b'"),
            LiteralValue::String(b"a$b".to_vec())
        );
        assert_eq!(
            first_literal("'it$'s'"),
            LiteralValue::String(b"it's".to_vec())
        );
        assert_eq!(
            first_literal("'$R$L'"),
            LiteralValue::String(vec![0x0d, 0x0a])
        );
        assert_eq!(first_literal("'$t'"), LiteralValue::String(vec![0x09]));
        assert_eq!(first_literal("'$41'"), LiteralValue::String(b"A".to_vec()));
        // A bare double quote is legal inside a single-quoted string.
        assert_eq!(
            first_literal("'a\"b'"),
            LiteralValue::String(b"a\"b".to_vec())
        );
    }

    #[test]
    fn wstring_escapes_take_four_hex_digits() {
        assert_eq!(
            first_literal("\"$0041\""),
            LiteralValue::WString(vec![0x41])
        );
        assert_eq!(
            first_literal("\"a$\"b\""),
            LiteralValue::WString(vec![0x61, 0x22, 0x62])
        );
    }

    #[test]
    fn a_multi_byte_character_becomes_whole_code_units_in_a_wstring() {
        let LiteralValue::WString(units) = first_literal("\"ä\"") else {
            panic!("not a wstring")
        };
        assert_eq!(units, vec![0x00e4]);
    }

    #[test]
    fn an_unterminated_string_is_reported_and_does_not_swallow_the_next_line() {
        let (stream, diags) = lex_generic("X := 'oops\nY := 1;\n");
        assert!(diags.items().iter().any(|d| d.code.0 == "E0103"));
        // The lexer resynchronised at the line break, so the second line is
        // still tokenised and its errors are still found.
        assert!(
            stream.tokens().len() > 4,
            "lexer gave up after the bad string"
        );
    }

    #[test]
    fn an_unknown_escape_is_reported() {
        let errs = errors("'$q'");
        assert!(errs.iter().any(|e| e.starts_with("E0104")), "{errs:?}");
    }

    // --- direct addresses -----------------------------------------------

    #[test]
    fn a_hierarchical_direct_address_lexes_as_one_token() {
        let stream = lex_ok("%QX7.5");
        let tokens = stream.tokens();
        assert_eq!(tokens.len(), 2, "expected one address token and EOF");
        let TokenKind::DirectAddress(i) = tokens[0].kind else {
            panic!("not an address")
        };
        assert_eq!(stream.address(i).unwrap().to_string(), "%QX7.5");
    }

    #[test]
    fn the_size_letter_of_a_direct_address_is_optional() {
        let stream = lex_ok("%I1");
        let TokenKind::DirectAddress(i) = stream.tokens()[0].kind else {
            panic!("not an address")
        };
        let a = stream.address(i).unwrap();
        assert_eq!(a.size, crate::address::AddressSize::Bit);
        assert!(!a.size_letter_written);
        assert_eq!(a.to_string(), "%I1");
    }

    #[test]
    fn every_location_and_size_letter_is_accepted() {
        for src in ["%IX0.0", "%QB1", "%MW2", "%MD3", "%ML4", "%iw5"] {
            let stream = lex_ok(src);
            assert!(
                matches!(stream.tokens()[0].kind, TokenKind::DirectAddress(_)),
                "{src} did not lex as an address"
            );
        }
    }

    #[test]
    fn a_partly_specified_address_is_accepted() {
        let stream = lex_ok("%MW*");
        let TokenKind::DirectAddress(i) = stream.tokens()[0].kind else {
            panic!("not an address")
        };
        assert!(stream.address(i).unwrap().path.is_none());
    }

    #[test]
    fn a_malformed_address_is_reported() {
        for src in ["%Z1", "%I", "%"] {
            let errs = errors(src);
            assert!(
                errs.iter().any(|e| e.starts_with("E0111")),
                "{src}: {errs:?}"
            );
        }
    }

    #[test]
    fn address_depth_is_bounded() {
        let deep = format!("%IX{}", vec!["1"; 40].join("."));
        let errs = errors(&deep);
        assert!(errs.iter().any(|e| e.starts_with("E0111")), "{errs:?}");
    }

    // --- operators and pragmas -------------------------------------------

    #[test]
    fn multi_character_operators_win_over_their_prefixes() {
        assert_eq!(kinds(":=")[0], TokenKind::Punct(Punct::Assign));
        assert_eq!(kinds("**")[0], TokenKind::Punct(Punct::Power));
        assert_eq!(kinds("<=")[0], TokenKind::Punct(Punct::LessEqual));
        assert_eq!(kinds("<>")[0], TokenKind::Punct(Punct::NotEqual));
        assert_eq!(kinds("=>")[0], TokenKind::Punct(Punct::RightArrow));
        assert_eq!(kinds("?=")[0], TokenKind::Punct(Punct::AssignAttempt));
        assert_eq!(kinds("..")[0], TokenKind::Punct(Punct::DotDot));
        assert_eq!(kinds("<")[0], TokenKind::Punct(Punct::Less));
        assert_eq!(kinds("*")[0], TokenKind::Punct(Punct::Star));
    }

    #[test]
    fn pragmas_are_recognised_and_skipped_without_being_interpreted() {
        let stream = lex_ok("{attribute 'qualified_only'} X");
        assert_eq!(stream.pragmas().len(), 1);
        assert_eq!(
            stream.tokens().iter().map(|t| t.kind).collect::<Vec<_>>(),
            vec![TokenKind::Ident, TokenKind::Eof]
        );
    }

    #[test]
    fn an_unterminated_pragma_is_reported() {
        let errs = errors("{oops");
        assert!(errs.iter().any(|e| e.starts_with("E0114")), "{errs:?}");
    }

    #[test]
    fn a_qualified_enumeration_value_keeps_its_hash_for_the_parser() {
        let k = kinds("Colour#Red");
        assert_eq!(
            k,
            vec![
                TokenKind::Ident,
                TokenKind::Punct(Punct::Hash),
                TokenKind::Ident,
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn an_unexpected_character_is_reported_once_per_character_not_per_byte() {
        let errs = errors("€");
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].starts_with("E0101"));
    }

    // --- invariants -----------------------------------------------------

    #[test]
    fn token_spans_are_ordered_and_inside_the_source() {
        let src = "PROGRAM P VAR x : DINT := 16#FF; END_VAR x := %IW1 + T#1s; END_PROGRAM";
        let stream = lex_ok(src);
        let mut previous = 0u32;
        for t in stream.tokens() {
            assert!(t.span.start >= previous, "spans went backwards at {t:?}");
            assert!(
                t.span.end <= src.len() as u32,
                "span past end of file: {t:?}"
            );
            assert!(t.span.start <= t.span.end);
            previous = t.span.start;
        }
    }

    #[test]
    fn the_stream_always_ends_with_exactly_one_eof() {
        for src in ["", "   ", "X", "(* unterminated", "'unterminated"] {
            let (stream, _) = lex_generic(src);
            let eofs = stream
                .tokens()
                .iter()
                .filter(|t| t.kind == TokenKind::Eof)
                .count();
            assert_eq!(eofs, 1, "{src:?}");
            assert_eq!(stream.tokens().last().map(|t| t.kind), Some(TokenKind::Eof));
        }
    }

    #[test]
    fn the_lexer_terminates_and_never_panics_on_hostile_input() {
        // Not a substitute for the fuzz target, but it catches the obvious
        // ways a hand-written lexer fails to make progress.
        let cases: Vec<String> = vec![
            String::new(),
            "\0\0\0".to_string(),
            "\u{feff}PROGRAM".to_string(),
            "#".repeat(500),
            "'".repeat(500),
            "\"".repeat(500),
            "%".repeat(500),
            "$".repeat(500),
            "16#".repeat(300),
            "T#".repeat(300),
            "D#".repeat(300),
            "(*".repeat(200),
            "*)".repeat(200),
            "{".repeat(200),
            "1.".repeat(300),
            "..".repeat(300),
            "\u{1F600}\u{1F600}".to_string(),
            "T#1d".repeat(200),
            "_".repeat(2000),
            "a".repeat(5000),
        ];
        for src in cases {
            let (stream, _) = lex_generic(&src);
            assert_eq!(stream.tokens().last().map(|t| t.kind), Some(TokenKind::Eof));
        }
    }

    #[test]
    fn lexing_is_deterministic() {
        let src = "PROGRAM P x := 16#ff + T#1s; END_PROGRAM";
        let a = format!("{:?}", lex_generic(src).0.tokens());
        let b = format!("{:?}", lex_generic(src).0.tokens());
        assert_eq!(a, b);
    }

    #[test]
    fn the_strict_dialect_rejects_lowercase_hexadecimal_digits() {
        let src = "16#ff";
        let mut map = SourceMap::new();
        let file = map.add("t.st", src).unwrap();
        let (_, strict) = lex(file, src, &Dialect::strict_iec());
        assert!(strict.has_errors());
        let (_, generic) = lex(file, src, &Dialect::generic());
        assert!(!generic.has_errors());
    }
}
