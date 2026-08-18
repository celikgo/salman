// SPDX-License-Identifier: Apache-2.0
//! Lex, parse, check and compile in one call.
//!
//! Every front end — the command line, the test harness, and later the language
//! server — wants the same pipeline, and each stage's diagnostics belong in one
//! list. Doing that in one place means the pipeline cannot be assembled
//! slightly differently in two of them.
//!
//! # One source file
//!
//! [`build`] compiles **one** source file. A project spanning several files is
//! not implemented: node identity is allocated per parse, so merging units
//! means renumbering, and that is work for the project model rather than a
//! quiet approximation now. `salman` says so rather than silently compiling
//! only the first file.

use salman_core::diag::Diagnostics;
use salman_core::span::{FileId, SourceError, SourceMap};
use salman_lang::dialect::Dialect;
use salman_lang::sema::{self, Checked};

use crate::compile::{Compiled, compile};

/// The result of building a source file.
#[derive(Debug)]
pub struct Build {
    /// The source map, so diagnostics can be rendered.
    pub sources: SourceMap,
    /// The file that was built.
    pub file: FileId,
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
    let mut sources = SourceMap::new();
    let file = sources.add(name, text)?;

    let (unit, mut diagnostics) = salman_lang::parse_source(file, text, dialect);
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
        file,
        checked,
        compiled,
        diagnostics,
    })
}
