// SPDX-License-Identifier: Apache-2.0
//! Lexes arbitrary raw bytes, decoded the way the loader will decode them.
//!
//! Source text reaches salman as bytes — from a file on disk, from an export a
//! vendor tool wrote, from a capture. None of those are guaranteed to be valid
//! UTF-8, and a loader that refuses to open a file because byte 4000 is a stray
//! `0x80` is not usable. So the loader will decode lossily, and this target
//! decodes lossily too: fuzzing the lexer against input the loader could never
//! hand it would be fuzzing a function nobody calls.
//!
//! What this reaches that `lex_utf8` does not: `U+FFFD` is three bytes, so a
//! single invalid byte becomes three, and a run of them moves every subsequent
//! offset. Any place the lexer reasons about byte positions across a multi-byte
//! character is exercised here far more heavily, because libFuzzer's byte-level
//! mutations produce invalid sequences constantly and `arbitrary` no longer
//! throws them away.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_fuzz::lex_and_check;
use salman_lang::dialect::Dialect;

fuzz_target!(|bytes: &[u8]| {
    let source = String::from_utf8_lossy(bytes);
    let _ = lex_and_check(&source, &Dialect::generic());
});
