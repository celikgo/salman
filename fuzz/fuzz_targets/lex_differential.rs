// SPDX-License-Identifier: Apache-2.0
//! Lexes one input under both dialects and compares the two results.
//!
//! # The bug this exists to catch
//!
//! `Dialect::strict_iec` is built from `Dialect::generic` by clearing flags, and
//! the lexer consults those flags in `if !self.dialect.…` form. A single missing
//! `!`, or a pair of flags swapped where the dialect is constructed, produces a
//! configuration that is wired backwards: the dialect that is meant to accept
//! only what salman could verify in the standard would accept *more* than the
//! permissive one. Nothing about that is visible in a unit test that checks one
//! dialect at a time, and it is invisible in the output as well — both dialects
//! still lex, still produce tokens, still emit diagnostics. It shows up as
//! salman quietly accepting `16#ff` in the dialect whose entire purpose is to
//! refuse it.
//!
//! The relation between the two is what makes this checkable without knowing
//! what the right answer is for any particular input: whatever the generic
//! dialect objects to, the strict one must object to as well.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_fuzz::lex_and_check;
use salman_lang::dialect::Dialect;

fuzz_target!(|source: &str| {
    let Some((generic_stream, generic_diags)) = lex_and_check(source, &Dialect::generic()) else {
        return;
    };
    let Some((strict_stream, strict_diags)) = lex_and_check(source, &Dialect::strict_iec()) else {
        return;
    };

    // --- the strict dialect never accepts more -------------------------
    //
    // Guarded on `dropped`, and the guard is not defensive noise. `Diagnostics`
    // caps what it stores at `DEFAULT_DIAGNOSTIC_LIMIT` and counts the rest as
    // dropped, so on a saturating input both counts are pinned to whatever
    // survived the cap. Because the strict dialect pushes a superset of the
    // generic one's diagnostics, its extra entries can push generic errors past
    // the cap, and `error_count()` can then legitimately come out lower for
    // strict. That is truncation, not a wiring bug, and asserting through it
    // would produce a crash artefact nobody can act on.
    //
    // Testing only strict's counter is enough: strict pushes at least as many
    // diagnostics as generic, so strict staying under the cap implies generic
    // did too.
    if strict_diags.dropped() == 0 {
        assert!(
            strict_diags.error_count() >= generic_diags.error_count(),
            "the strict dialect reported {} errors for {source:?} and the generic dialect \
             reported {} — strict accepted something permissive rejected, so the dialect \
             table is wired backwards",
            strict_diags.error_count(),
            generic_diags.error_count()
        );
    }

    // --- and it does not change what a token *is* ----------------------
    //
    // True of these two dialects specifically, and worth stating because it is
    // easy to break by accident. Of the flags that differ between them,
    // `bool_widens_to_bit_strings` belongs to the type checker and the lexer
    // never reads it, while `lowercase_hex_digits` and `signed_duration_literals`
    // guard arms that push a diagnostic and then fall through to exactly the
    // same scanning: `16#ff` still accumulates to 255 under the strict dialect,
    // and `T#-5s` still yields a negative duration. Every flag the lexer uses to
    // decide what a token *is* — comment nesting, `//`, `/* */`, the nesting
    // ceiling — holds the same value in both.
    //
    // So a difference here means one of two things, and both want a human:
    // either a dialect flag started affecting tokenisation, or the strict
    // dialect drifted away from being generic-with-flags-cleared. If a future
    // dialect legitimately changes tokenisation, this assertion moves behind a
    // check on the specific flags rather than being deleted.
    assert_eq!(
        generic_stream.tokens(),
        strict_stream.tokens(),
        "the two dialects tokenised {source:?} differently; no flag that differs between \
         Dialect::generic and Dialect::strict_iec is supposed to affect tokenisation"
    );
});
