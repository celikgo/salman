// SPDX-License-Identifier: Apache-2.0
//! Lexes arbitrary valid UTF-8 under the strict IEC dialect.
//!
//! Not a duplicate of `lex_utf8` with one argument changed. The two dialects
//! take different branches inside the lexer: `Dialect::strict_iec` clears
//! `lowercase_hex_digits` and `signed_duration_literals`, and each of those
//! flags guards a diagnostic-emitting arm that the generic dialect never
//! enters. Code that no target reaches is code no fuzzer tests, and those arms
//! run in the middle of numeric and duration scanning — the two places in this
//! lexer where offsets, radices and accumulators are being juggled at once.
//!
//! Kept as a separate target rather than folded into `lex_utf8` behind a
//! coin flip so that each dialect gets its own corpus. A corpus is a record of
//! which inputs reached which code, and merging two dialects into one corpus
//! loses that.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_fuzz::lex_and_check;
use salman_lang::dialect::Dialect;

fuzz_target!(|source: &str| {
    let _ = lex_and_check(source, &Dialect::strict_iec());
});
