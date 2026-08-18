// SPDX-License-Identifier: Apache-2.0
//! The whole front end — lex, parse, check — must survive any input.
//!
//! Semantic analysis walks a tree a hostile file shaped, resolves names it
//! chose, and folds constants it wrote. It has more ways to go wrong than the
//! parser does, and every one of them is reachable from a file salman did not
//! write.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_core::span::SourceMap;
use salman_lang::dialect::Dialect;
use salman_lang::sema;

fuzz_target!(|source: &str| {
    if source.len() > 32 * 1024 {
        return;
    }

    for dialect in [Dialect::generic(), Dialect::strict_iec()] {
        let mut map = SourceMap::new();
        let Ok(file) = map.add("fuzz.st", source) else {
            return;
        };
        let (unit, mut diagnostics) = salman_lang::parse_source(file, source, &dialect);
        let (checked, check_diagnostics) = sema::check(&unit, &dialect);
        diagnostics.extend(check_diagnostics);

        // Side tables must cover every node the parser allocated, or a later
        // pass indexes past the end of one.
        assert!(
            checked.expr_types.len() >= unit.node_count as usize || checked.expr_types.is_empty(),
            "the type side table is shorter than the tree"
        );

        // Rendering the report must never panic, whatever spans arrived.
        let _ = diagnostics.render(&map);
        diagnostics.sort();
        let _ = diagnostics.render(&map);
    }
});
