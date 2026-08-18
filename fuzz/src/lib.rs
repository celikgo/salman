// SPDX-License-Identifier: Apache-2.0
//! Invariants every lexer fuzz target checks, in one place.
//!
//! # Why this is not four copies of the same asserts
//!
//! A fuzz target that only calls the function under test and returns finds
//! exactly one class of bug: the ones that abort the process. Everything else —
//! a token whose span points past the end of the file, a `Literal(7)` in a
//! stream whose literal table holds six entries, a stream with two `Eof`s —
//! survives the run and reaches the parser, which is where it turns into a
//! panic in code that did nothing wrong. Rule 7 is about the decoder, so the
//! decoder's postconditions have to be stated somewhere the fuzzer can falsify
//! them.
//!
//! Stating them once means a new target gets the whole set, and means a
//! postcondition that turns out to be false is corrected in one place rather
//! than in whichever three of four files somebody remembered.
//!
//! # Panicking is the reporting channel
//!
//! Everything here asserts. Under libFuzzer a panic is how a target says "this
//! input is the bug", and the message is what the person reproducing it reads
//! first, so each one names the input, the offending index and the value —
//! never a bare `assert!(cond)`. The workspace's ban on `panic!` in library
//! code does not reach here, and should not: this crate is not linked into
//! anything salman ships.

#![deny(unsafe_code)]
#![deny(missing_docs)]

use salman_core::diag::Diagnostics;
use salman_core::span::{FileId, SourceMap};
use salman_lang::dialect::Dialect;
use salman_lang::lexer::{TokenStream, lex};
use salman_lang::token::TokenKind;

/// The file name every target registers its input under.
///
/// Fixed so a crash message names the same file whichever target produced it.
pub const FUZZ_FILE_NAME: &str = "fuzz.st";

/// Lexes `source` under `dialect` and checks every invariant in
/// [`check_stream_invariants`].
///
/// Returns the stream and its diagnostics so a caller can assert more — the
/// differential target does — or `None` when the source map refused the input.
/// A refusal is not a bug: `SourceMap::add` rejects anything over
/// `salman_core::span::MAX_SOURCE_BYTES`, which is a bound working as intended
/// rather than a failure to report.
///
/// # Panics
///
/// On any violated invariant. That is the point of the function.
#[must_use]
pub fn lex_and_check(source: &str, dialect: &Dialect) -> Option<(TokenStream, Diagnostics)> {
    let mut sources = SourceMap::new();
    let file = sources.add(FUZZ_FILE_NAME, source).ok()?;
    let (stream, diags) = lex(file, source, dialect);
    check_stream_invariants(source, file, &stream);
    Some((stream, diags))
}

/// Asserts everything that must hold of a [`TokenStream`] for **any** input.
///
/// None of these depend on the input being valid Structured Text, or valid
/// anything. A file of random bytes must satisfy all of them.
///
/// # Panics
///
/// On any violated invariant.
pub fn check_stream_invariants(source: &str, file: FileId, stream: &TokenStream) {
    // Spans are `u32` byte offsets, so a source that cannot be described by one
    // could not be spanned at all. `SourceMap::add` refuses anything over 64 MiB
    // long before this, so arriving here would itself be the bug — which is why
    // it asserts rather than returning quietly.
    let source_len = u32::try_from(source.len()).unwrap_or_else(|_| {
        panic!(
            "source of {} bytes cannot be described by u32 spans; MAX_SOURCE_BYTES \
             should have refused it",
            source.len()
        )
    });

    let tokens = stream.tokens();

    // --- the stream is terminated, exactly once -------------------------
    //
    // `TokenKind::Eof` documents itself as "always present exactly once, as the
    // last token", and every consumer written against that will index the last
    // token or loop until it. Two of them, or none, is a hang or an
    // out-of-bounds read in the parser rather than here.
    assert!(
        !tokens.is_empty(),
        "lex produced no tokens at all for {source:?}; the stream must always end with Eof"
    );
    let eof_count = tokens.iter().filter(|t| t.kind == TokenKind::Eof).count();
    assert_eq!(
        eof_count, 1,
        "lex produced {eof_count} Eof tokens for {source:?}; exactly one is required"
    );
    let last = tokens.last().map(|t| t.kind);
    assert_eq!(
        last,
        Some(TokenKind::Eof),
        "the last token for {source:?} is {last:?}, not Eof"
    );

    // --- spans are ordered, non-inverted and inside the file ------------
    //
    // Reaching this file means the lexer terminated: a target that loops
    // forever is killed by libFuzzer's `-timeout` and reported as a hang, so
    // "the lexer terminated" needs no assertion of its own — it is a
    // precondition of every assertion below.
    let mut previous = None;
    for (index, token) in tokens.iter().enumerate() {
        assert_eq!(
            token.span.file, file,
            "token {index} of {source:?} carries file {:?}, not the file it was lexed from",
            token.span.file
        );
        assert!(
            token.span.start <= token.span.end,
            "token {index} of {source:?} has an inverted span {}..{}",
            token.span.start,
            token.span.end
        );
        assert!(
            token.span.end <= source_len,
            "token {index} of {source:?} ends at {} but the source is {source_len} bytes; \
             rendering this diagnostic would slice out of bounds",
            token.span.end
        );
        // Non-decreasing rather than strictly increasing: zero-width tokens are
        // legitimate — `Eof` is one, and so is a token recovered at a position
        // the lexer could not advance past.
        if let Some(prev) = previous {
            assert!(
                prev <= token.span.start,
                "token {index} of {source:?} starts at {} after a token that started at {prev}; \
                 spans must be non-decreasing or every binary search over them is wrong",
                token.span.start
            );
        }
        previous = Some(token.span.start);

        // --- side-table indices resolve --------------------------------
        //
        // `TokenKind::Literal(i)` and `TokenKind::DirectAddress(i)` are indices
        // into tables the same stream owns. `TokenStream::literal` and
        // `::address` return `Option`, so a stale index does not panic here —
        // it produces a `None` that a caller written against the documented
        // invariant will unwrap or silently mishandle. This is the assertion
        // that turns that into a finding.
        match token.kind {
            TokenKind::Literal(i) => assert!(
                stream.literal(i).is_some(),
                "token {index} of {source:?} refers to literal {i}, which is not in the \
                 stream's literal table"
            ),
            TokenKind::DirectAddress(i) => assert!(
                stream.address(i).is_some(),
                "token {index} of {source:?} refers to address {i}, which is not in the \
                 stream's address table"
            ),
            _ => {}
        }
    }

    // --- trivia spans are inside the file too ---------------------------
    //
    // Comment and pragma spans never reach the parser, but a formatter and the
    // documentation extractor both slice the source with them, and an
    // out-of-range span there is the same bug arriving somewhere quieter.
    for (index, span) in stream.comments().iter().enumerate() {
        assert!(
            span.start <= span.end && span.end <= source_len,
            "comment {index} of {source:?} spans {}..{}, outside a {source_len}-byte source",
            span.start,
            span.end
        );
    }
    for (index, span) in stream.pragmas().iter().enumerate() {
        assert!(
            span.start <= span.end && span.end <= source_len,
            "pragma {index} of {source:?} spans {}..{}, outside a {source_len}-byte source",
            span.start,
            span.end
        );
    }
}
