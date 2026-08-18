// SPDX-License-Identifier: Apache-2.0
//! The parser must not panic, hang or overflow the stack on any input.
//!
//! This target matters more than the lexer ones: the parser is recursive, and
//! its nesting bound is the only thing standing between a hostile file and a
//! stack overflow. It asserts the invariants that must hold whatever arrives,
//! not merely that nothing crashed.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_core::span::SourceMap;
use salman_lang::ast::{Arg, Expr, ExprKind, Stmt, StmtKind};
use salman_lang::dialect::Dialect;

fuzz_target!(|source: &str| {
    // Bound the input. A fuzzer will happily feed megabytes, and time spent on
    // one enormous input is time not spent on a thousand interesting ones.
    if source.len() > 64 * 1024 {
        return;
    }

    let mut map = SourceMap::new();
    let Ok(file) = map.add("fuzz.st", source) else {
        return;
    };
    let (unit, diagnostics) = salman_lang::parse_source(file, source, &Dialect::generic());

    // Every node id must be usable as an index into a side table of the
    // declared size, or a later pass reads out of bounds.
    let limit = unit.node_count;
    for pou in unit.pous() {
        for statement in &pou.body {
            walk_statement(statement, limit);
        }
    }

    // Rendering must never panic, whatever spans the parser produced.
    let _ = diagnostics.render(&map);

    // Parsing is deterministic: the same bytes give the same tree.
    let (again, _) = salman_lang::parse_source(file, source, &Dialect::generic());
    assert!(unit == again, "parsing the same input twice gave two trees");
});

fn walk_statement(statement: &Stmt, limit: u32) {
    assert!(statement.id.0 < limit, "statement id past node_count");
    match &statement.kind {
        StmtKind::Assign { target, value } | StmtKind::AssignAttempt { target, value } => {
            walk_expr(target, limit);
            walk_expr(value, limit);
        }
        StmtKind::Call(call) => walk_expr(call, limit),
        StmtKind::If { branches, else_body } => {
            for branch in branches {
                walk_expr(&branch.condition, limit);
                for s in &branch.body {
                    walk_statement(s, limit);
                }
            }
            if let Some(body) = else_body {
                for s in body {
                    walk_statement(s, limit);
                }
            }
        }
        StmtKind::Case { selector, arms, else_body } => {
            walk_expr(selector, limit);
            for arm in arms {
                for s in &arm.body {
                    walk_statement(s, limit);
                }
            }
            if let Some(body) = else_body {
                for s in body {
                    walk_statement(s, limit);
                }
            }
        }
        StmtKind::For { from, to, by, body, .. } => {
            walk_expr(from, limit);
            walk_expr(to, limit);
            if let Some(e) = by {
                walk_expr(e, limit);
            }
            for s in body {
                walk_statement(s, limit);
            }
        }
        StmtKind::While { condition, body } => {
            walk_expr(condition, limit);
            for s in body {
                walk_statement(s, limit);
            }
        }
        StmtKind::Repeat { body, until } => {
            for s in body {
                walk_statement(s, limit);
            }
            walk_expr(until, limit);
        }
        StmtKind::Empty
        | StmtKind::Continue
        | StmtKind::Exit
        | StmtKind::Return
        | StmtKind::Error => {}
    }
}

fn walk_expr(expr: &Expr, limit: u32) {
    assert!(expr.id.0 < limit, "expression id past node_count");
    assert!(expr.span.start <= expr.span.end, "inverted span");
    match &expr.kind {
        ExprKind::Paren(inner) | ExprKind::Deref(inner) => walk_expr(inner, limit),
        ExprKind::Member { base, .. } => walk_expr(base, limit),
        ExprKind::Index { base, indices } => {
            walk_expr(base, limit);
            for index in indices {
                walk_expr(index, limit);
            }
        }
        ExprKind::Unary { operand, .. } => walk_expr(operand, limit),
        ExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, limit);
            walk_expr(rhs, limit);
        }
        ExprKind::Call { callee, args } => {
            walk_expr(callee, limit);
            for arg in args {
                match arg {
                    Arg::Positional(e) | Arg::Input { value: e, .. } => walk_expr(e, limit),
                    Arg::Output { target, .. } => {
                        if let Some(e) = target {
                            walk_expr(e, limit);
                        }
                    }
                }
            }
        }
        ExprKind::Literal(_)
        | ExprKind::Var(_)
        | ExprKind::Direct(_)
        | ExprKind::EnumValue { .. }
        | ExprKind::Error => {}
    }
}
