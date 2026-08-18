// SPDX-License-Identifier: Apache-2.0
//! Lex, parse, check and compile in one call.
//!
//! Every front end — the command line, the test harness, and later the language
//! server — wants the same pipeline, and each stage's diagnostics belong in one
//! list. Doing that in one place means the pipeline cannot be assembled
//! slightly differently in two of them.
//!
//! # One file or several
//!
//! [`build`] compiles one source file; [`build_all`] compiles several as one
//! program, so a POU declared in one file can be called from another. The files
//! share a single node-id space, handed out at parse time — see
//! [`parse_source_from`](salman_lang::parse_source_from) — because every side
//! table downstream is indexed by node id.

use salman_core::diag::Diagnostics;
use salman_core::span::{FileId, SourceError, SourceMap};
use salman_lang::ast::CompilationUnit;
use salman_lang::dialect::Dialect;
use salman_lang::sema::{self, Checked};

use crate::compile::{Compiled, compile};

/// The result of building a source file.
#[derive(Debug)]
pub struct Build {
    /// The source map, so diagnostics can be rendered.
    pub sources: SourceMap,
    /// The files that were built, in the order they were given.
    pub files: Vec<FileId>,
    /// What semantic analysis learned, whether or not compilation succeeded.
    pub checked: Checked,
    /// The compiled program, if there were no errors.
    pub compiled: Option<Compiled>,
    /// Everything that went wrong, in source order.
    pub diagnostics: Diagnostics,
}

impl Build {
    /// Whether the build produced something runnable.
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        self.compiled.is_some()
    }

    /// Renders every diagnostic.
    #[must_use]
    pub fn render_diagnostics(&self) -> String {
        self.diagnostics.render(&self.sources)
    }
}

/// Builds one source file.
///
/// Compilation is skipped when semantic analysis reported an error: generating
/// code from a program already known to be wrong produces confusing runtime
/// faults instead of the errors the engineer needs to read.
///
/// # Errors
///
/// Returns [`SourceError`] only when the text cannot be loaded at all — that
/// is, when it is larger than salman's source limit. Everything else is a
/// diagnostic.
pub fn build(name: &str, text: &str, dialect: &Dialect) -> Result<Build, SourceError> {
    build_all(&[(name, text)], dialect)
}

/// Builds several source files as one program.
///
/// The files are parsed independently and then joined, so a `PROGRAM` in one
/// file may call a `FUNCTION_BLOCK` declared in another, and a name declared
/// twice across two files is reported as a duplicate with both spans — which is
/// the reason to join before checking rather than check each file alone.
///
/// Order matters only for reading: declarations resolve across the whole
/// project, not from the top down.
///
/// # Errors
///
/// Returns [`SourceError`] only when a file cannot be loaded at all — that is,
/// when it is larger than salman's source limit. Everything else is a
/// diagnostic. An empty list is not an error: it produces a build with no
/// program and the diagnostic that says there is nothing to run.
pub fn build_all(files: &[(&str, &str)], dialect: &Dialect) -> Result<Build, SourceError> {
    let mut sources = SourceMap::new();
    let mut ids = Vec::with_capacity(files.len());
    let mut units = Vec::with_capacity(files.len());
    let mut diagnostics = Diagnostics::new();
    // Each file is handed the id space above every file before it, so no two
    // nodes in the project share an id and the side tables stay disjoint.
    let mut next_id = 0;
    for (name, text) in files {
        let file = sources.add(*name, *text)?;
        ids.push(file);
        let (unit, file_diagnostics) = salman_lang::parse_source_from(file, text, dialect, next_id);
        next_id = unit.node_count;
        diagnostics.extend(file_diagnostics);
        units.push(unit);
    }

    // No files at all still has to produce a build, and the honest one says
    // there is nothing to run. An empty source gives an empty unit without a
    // special case in the checker.
    let unit = if let Some(unit) = CompilationUnit::join(units) {
        unit
    } else {
        let file = sources.add("<project>", "")?;
        salman_lang::parse_source(file, "", dialect).0
    };

    let (checked, check_diagnostics) = sema::check(&unit, dialect);
    diagnostics.extend(check_diagnostics);

    let compiled = if diagnostics.has_errors() {
        None
    } else {
        let (compiled, compile_diagnostics) = compile(&unit, &checked, dialect);
        diagnostics.extend(compile_diagnostics);
        if diagnostics.has_errors() {
            None
        } else {
            compiled
        }
    };

    diagnostics.sort();
    Ok(Build {
        sources,
        files: ids,
        checked,
        compiled,
        diagnostics,
    })
}
