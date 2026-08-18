// SPDX-License-Identifier: Apache-2.0
//! Lexes arbitrary valid UTF-8 under the generic dialect.
//!
//! The baseline target. `lex` takes a `&str`, so this is the shape of input it
//! is actually written against, and the one where a violated postcondition is
//! unambiguously the lexer's fault rather than an artefact of how the bytes
//! were decoded on the way in.
//!
//! libFuzzer's mutations are byte-oriented, so asking for a `&str` means
//! `arbitrary` discards or repairs a large fraction of the inputs it is handed.
//! That is a real cost in throughput, and it is why `lex_bytes` exists
//! alongside this target rather than instead of it: the two reach the lexer's
//! interesting states by different routes.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_fuzz::lex_and_check;
use salman_lang::dialect::Dialect;

fuzz_target!(|source: &str| {
    let _ = lex_and_check(source, &Dialect::generic());
});
