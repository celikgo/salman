// SPDX-License-Identifier: Apache-2.0
//! Diagnostics: what went wrong, where, and what to do about it.
//!
//! salman renders its own diagnostics rather than taking a dependency on a
//! rendering crate. Three reasons, in order of weight:
//!
//! 1. Diagnostic text is compared byte-for-byte by golden tests, so its exact
//!    shape is a thing salman must own and version.
//! 2. The installer budget is 120 MB and the cold-start budget is 2 s; a
//!    renderer is about 200 lines here.
//! 3. Colour must never be the only carrier of meaning — a red/green table a
//!    reader with deuteranopia cannot distinguish is a defect — so the plain
//!    text alone has to be complete.
//!
//! Two fields exist here that a general-purpose compiler would not have:
//!
//! * [`Diagnostic::clause`] cites the IEC clause a rule comes from, so an
//!   engineer can check salman against the standard rather than trusting it.
//! * [`Diagnostic::dialect_rule`] names *which dialect's* rule was applied,
//!   which is what someone porting code between vendors actually needs to know.

use std::fmt::Write as _;

use crate::clause::ClauseRef;
use crate::span::{SourceMap, Span};

/// Width a tab is rendered as, so carets line up under the text above them.
const TAB_WIDTH: usize = 4;

/// How many diagnostics are kept before salman stops recording them.
///
/// A hostile or badly corrupted file can produce an error per byte. The cap
/// keeps that from turning into unbounded memory; the count of what was dropped
/// is still reported.
pub const DEFAULT_DIAGNOSTIC_LIMIT: usize = 1000;

/// How serious a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Compilation or execution cannot proceed correctly.
    Error,
    /// Legal, but probably not what was meant.
    Warning,
    /// Extra context attached to another diagnostic.
    Note,
    /// A suggested course of action.
    Help,
}

impl Severity {
    /// The word used at the head of a rendered diagnostic.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
            Self::Help => "help",
        }
    }
}

/// A stable diagnostic code, e.g. `"E0101"`.
///
/// Codes are stable across releases: they end up in users' lint suppressions
/// and in CI filters, so renaming one is a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiagCode(pub &'static str);

impl std::fmt::Display for DiagCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// Whether a label marks the thing that is wrong, or something related.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelStyle {
    /// The span the diagnostic is actually about.
    Primary,
    /// Context: the declaration, the other branch, the earlier definition.
    Secondary,
}

/// A span with something to say about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// Where.
    pub span: Span,
    /// What about it.
    pub message: String,
    /// Whether this is the point of the diagnostic.
    pub style: LabelStyle,
}

impl Label {
    /// A primary label.
    pub fn primary(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            style: LabelStyle::Primary,
        }
    }

    /// A secondary label.
    pub fn secondary(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            style: LabelStyle::Secondary,
        }
    }
}

/// One textual replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The range to replace.
    pub span: Span,
    /// What to put there.
    pub replacement: String,
}

/// A machine-applicable fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// What the fix does, in an engineer's words.
    pub message: String,
    /// The edits, which must not overlap.
    pub edits: Vec<Edit>,
}

/// One thing salman has to say about a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// How serious it is.
    pub severity: Severity,
    /// The stable code.
    pub code: DiagCode,
    /// The headline, which must make sense on its own in a CI log.
    pub message: String,
    /// Spans, with the primary one first when rendered.
    pub labels: Vec<Label>,
    /// Extra prose.
    pub notes: Vec<String>,
    /// The IEC clause this rule comes from, if it comes from one.
    pub clause: Option<ClauseRef>,
    /// Which dialect rule was applied, when dialects differ here.
    ///
    /// This is what makes a port between vendors reviewable: the engineer sees
    /// not only that something changed but under whose rule.
    pub dialect_rule: Option<String>,
    /// Machine-applicable fixes.
    pub suggestions: Vec<Suggestion>,
}

impl Diagnostic {
    /// A diagnostic with no labels yet.
    pub fn new(severity: Severity, code: DiagCode, message: impl Into<String>) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            clause: None,
            dialect_rule: None,
            suggestions: Vec::new(),
        }
    }

    /// An error.
    pub fn error(code: DiagCode, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }

    /// A warning.
    pub fn warning(code: DiagCode, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    /// Adds a primary label.
    #[must_use]
    pub fn with_primary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label::primary(span, message));
        self
    }

    /// Adds a secondary label.
    #[must_use]
    pub fn with_secondary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label::secondary(span, message));
        self
    }

    /// Adds a note.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Attaches the IEC clause behind this rule.
    #[must_use]
    pub const fn with_clause(mut self, clause: ClauseRef) -> Self {
        self.clause = Some(clause);
        self
    }

    /// Records which dialect rule produced this diagnostic.
    #[must_use]
    pub fn with_dialect_rule(mut self, rule: impl Into<String>) -> Self {
        self.dialect_rule = Some(rule.into());
        self
    }

    /// Adds a machine-applicable fix.
    #[must_use]
    pub fn with_suggestion(mut self, message: impl Into<String>, edits: Vec<Edit>) -> Self {
        self.suggestions.push(Suggestion {
            message: message.into(),
            edits,
        });
        self
    }

    /// The span the diagnostic is primarily about, if it has one.
    #[must_use]
    pub fn primary_span(&self) -> Option<Span> {
        self.labels
            .iter()
            .find(|l| l.style == LabelStyle::Primary)
            .or_else(|| self.labels.first())
            .map(|l| l.span)
    }

    /// A one-line form for CI logs and JUnit messages.
    #[must_use]
    pub fn one_line(&self, sources: &SourceMap) -> String {
        let mut out = String::new();
        if let Some(span) = self.primary_span()
            && let Some(file) = sources.file(span.file)
        {
            let _ = write!(out, "{}:{}: ", file.name(), file.line_col(span.start));
        }
        let _ = write!(
            out,
            "{}[{}]: {}",
            self.severity.label(),
            self.code,
            self.message
        );
        out
    }
}

/// A collection of diagnostics, capped so hostile input cannot exhaust memory.
#[derive(Debug, Clone)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
    limit: usize,
    dropped: usize,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

impl Diagnostics {
    /// An empty collection with the default cap.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            items: Vec::new(),
            limit: DEFAULT_DIAGNOSTIC_LIMIT,
            dropped: 0,
        }
    }

    /// An empty collection with an explicit cap.
    #[must_use]
    pub const fn with_limit(limit: usize) -> Self {
        Self {
            items: Vec::new(),
            limit,
            dropped: 0,
        }
    }

    /// Records a diagnostic, or counts it as dropped if the cap is reached.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        if self.items.len() >= self.limit {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.items.push(diagnostic);
    }

    /// Everything recorded, in the order it was recorded.
    #[must_use]
    pub fn items(&self) -> &[Diagnostic] {
        &self.items
    }

    /// How many diagnostics were discarded because the cap was reached.
    #[must_use]
    pub const fn dropped(&self) -> usize {
        self.dropped
    }

    /// Whether anything at all was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many were recorded (not counting dropped ones).
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether any recorded diagnostic is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.severity == Severity::Error)
    }

    /// How many errors were recorded.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.items
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    /// Appends everything from `other`, respecting this collection's cap.
    pub fn extend(&mut self, other: Self) {
        for d in other.items {
            self.push(d);
        }
        self.dropped = self.dropped.saturating_add(other.dropped);
    }

    /// Sorts into the order diagnostics are printed and compared in.
    ///
    /// The order is file, then start offset, then end offset, then code, then
    /// message. It is total, so two runs over the same input print the same
    /// bytes even if the diagnostics were produced in a different order — which
    /// is what the determinism gate requires.
    pub fn sort(&mut self) {
        self.items.sort_by(|a, b| {
            let ka = a.primary_span();
            let kb = b.primary_span();
            let file = ka.map(|s| s.file).cmp(&kb.map(|s| s.file));
            let start = ka.map(|s| s.start).cmp(&kb.map(|s| s.start));
            let end = ka.map(|s| s.end).cmp(&kb.map(|s| s.end));
            file.then(start)
                .then(end)
                .then_with(|| a.code.cmp(&b.code))
                .then_with(|| a.message.cmp(&b.message))
        });
    }

    /// Renders every diagnostic as plain text.
    #[must_use]
    pub fn render(&self, sources: &SourceMap) -> String {
        let mut out = String::new();
        for d in &self.items {
            out.push_str(&render(d, sources));
            out.push('\n');
        }
        if self.dropped > 0 {
            let _ = writeln!(
                out,
                "note: {} further diagnostics were not recorded (limit {})",
                self.dropped, self.limit
            );
        }
        out
    }
}

/// Renders one diagnostic as plain text, in the shape rustc made familiar.
///
/// Colour is deliberately absent: this text is compared byte-for-byte by golden
/// tests, and meaning must never depend on colour.
#[must_use]
pub fn render(d: &Diagnostic, sources: &SourceMap) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}[{}]: {}", d.severity.label(), d.code, d.message);

    // Primary labels first, then secondary, each group in the order given.
    let ordered = d
        .labels
        .iter()
        .filter(|l| l.style == LabelStyle::Primary)
        .chain(d.labels.iter().filter(|l| l.style == LabelStyle::Secondary));

    let gutter = label_gutter_width(d, sources);
    let pad = " ".repeat(gutter);

    for (index, label) in ordered.enumerate() {
        let Some(file) = sources.file(label.span.file) else {
            continue;
        };
        let start = file.line_col(label.span.start);
        let end = file.line_col(label.span.end);
        let arrow = if index == 0 { "-->" } else { "..." };
        let _ = writeln!(out, "{pad}{arrow} {}:{start}", file.name());
        let _ = writeln!(out, "{pad} |");

        let line_text = file.line_text(start.line);
        let (rendered, caret_start) = expand_tabs(line_text, start.column as usize);
        let caret_len = if end.line == start.line {
            let (_, caret_end) = expand_tabs(line_text, end.column as usize);
            caret_end.saturating_sub(caret_start).max(1)
        } else {
            // A span that crosses lines is marked to the end of its first line;
            // the closing position is stated in the note instead of drawn.
            rendered.chars().count().saturating_sub(caret_start).max(1)
        };

        let _ = writeln!(
            out,
            "{:>width$} | {rendered}",
            start.line,
            width = gutter.max(1)
        );
        let marker = if label.style == LabelStyle::Primary {
            '^'
        } else {
            '-'
        };
        let _ = writeln!(
            out,
            "{pad} | {}{} {}",
            " ".repeat(caret_start),
            marker.to_string().repeat(caret_len),
            label.message
        );
        if end.line != start.line {
            let _ = writeln!(out, "{pad} | (continues to {}:{end})", file.name());
        }
    }

    if !d.labels.is_empty() {
        let _ = writeln!(out, "{pad} |");
    }
    for note in &d.notes {
        let _ = writeln!(out, "{pad} = note: {note}");
    }
    if let Some(rule) = &d.dialect_rule {
        let _ = writeln!(out, "{pad} = dialect rule applied: {rule}");
    }
    if let Some(clause) = &d.clause {
        let _ = writeln!(out, "{pad} = standard: {clause}");
        let _ = writeln!(out, "{pad} = requirement: {}", clause.requirement);
    }
    for s in &d.suggestions {
        let _ = writeln!(out, "{pad} = help: {}", s.message);
        for edit in &s.edits {
            let _ = writeln!(out, "{pad} =       replace with: {}", edit.replacement);
        }
    }
    out
}

/// Width of the line-number gutter, so every line number in one diagnostic
/// lines up.
fn label_gutter_width(d: &Diagnostic, sources: &SourceMap) -> usize {
    let widest = d
        .labels
        .iter()
        .filter_map(|l| {
            sources
                .file(l.span.file)
                .map(|f| f.line_col(l.span.start).line)
        })
        .max()
        .unwrap_or(1);
    widest.to_string().len()
}

/// Expands tabs so that a caret drawn under a line lands under the right
/// character, and reports where a one-based column ends up after expansion.
fn expand_tabs(line: &str, one_based_column: usize) -> (String, usize) {
    let mut rendered = String::with_capacity(line.len());
    let mut mapped = 0usize;
    let target = one_based_column.saturating_sub(1);
    for (index, ch) in line.chars().enumerate() {
        if index == target {
            mapped = rendered.chars().count();
        }
        if ch == '\t' {
            let width = TAB_WIDTH - (rendered.chars().count() % TAB_WIDTH);
            for _ in 0..width {
                rendered.push(' ');
            }
        } else {
            rendered.push(ch);
        }
    }
    if target >= line.chars().count() {
        mapped = rendered.chars().count();
    }
    (rendered, mapped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clause::{CitationKind, CitedTest, ClauseRef, Provenance};
    use crate::span::{FileId, SourceMap, Span};

    const E_TEST: DiagCode = DiagCode("E0001");
    const W_TEST: DiagCode = DiagCode("W0001");

    fn sources(text: &str) -> (SourceMap, FileId) {
        let mut map = SourceMap::new();
        let id = map.add("conveyor.st", text).unwrap();
        (map, id)
    }

    #[test]
    fn a_rendered_error_points_at_the_offending_text() {
        let src = "PROGRAM Main\n    Motor_Run := Count;\nEND_PROGRAM\n";
        let (map, id) = sources(src);
        let start = src.find("Motor_Run").unwrap() as u32;
        let end = src.find(';').unwrap() as u32;
        let d = Diagnostic::error(E_TEST, "cannot assign INT to BOOL").with_primary(
            Span::new(id, start, end),
            "INT is not implicitly convertible to BOOL",
        );
        let text = render(&d, &map);
        assert!(
            text.starts_with("error[E0001]: cannot assign INT to BOOL\n"),
            "{text}"
        );
        assert!(text.contains("--> conveyor.st:2:5"), "{text}");
        assert!(text.contains("Motor_Run := Count"), "{text}");
        assert!(text.contains("^^^^^^^^^^^^^^^^^^"), "{text}");
    }

    #[test]
    fn rendering_carries_the_iec_clause_so_a_reader_can_check_salman() {
        let (map, id) = sources("X := 1;\n");
        let clause = ClauseRef {
            standard: "IEC 61131-3:2013",
            edition: "3.0",
            kind: CitationKind::Clause,
            number: "0.0",
            title: "Example clause",
            requirement: "an example requirement paraphrase for the test",
            provenance: Provenance::NumberUnconfirmed,
            tests: &[CitedTest {
                file: "crates/salman-core/src/diag.rs",
                test: "rendering_carries_the_iec_clause_so_a_reader_can_check_salman",
            }],
        };
        let d = Diagnostic::error(E_TEST, "example")
            .with_primary(Span::new(id, 0, 1), "here")
            .with_clause(clause);
        let text = render(&d, &map);
        assert!(
            text.contains("= standard: IEC 61131-3:2013 §0.0 \"Example clause\" (Ed 3.0)"),
            "{text}"
        );
        assert!(text.contains("clause number unconfirmed"), "{text}");
        assert!(
            text.contains("= requirement: an example requirement"),
            "{text}"
        );
    }

    #[test]
    fn rendering_names_the_dialect_rule_that_was_applied() {
        let (map, id) = sources("X := 1;\n");
        let d = Diagnostic::warning(W_TEST, "integer division truncates")
            .with_primary(Span::new(id, 0, 1), "here")
            .with_dialect_rule("codesys: DINT / DINT truncates toward zero");
        let text = render(&d, &map);
        assert!(
            text.contains("= dialect rule applied: codesys: DINT / DINT truncates toward zero"),
            "{text}"
        );
    }

    #[test]
    fn carets_land_under_the_right_character_when_the_line_uses_tabs() {
        let src = "\t\tMotor := 1;\n";
        let (map, id) = sources(src);
        let start = src.find("Motor").unwrap() as u32;
        let d = Diagnostic::error(E_TEST, "example")
            .with_primary(Span::new(id, start, start + 5), "here");
        let text = render(&d, &map);
        let lines: Vec<&str> = text.lines().collect();
        let source_line = lines.iter().find(|l| l.contains("Motor := 1;")).unwrap();
        let caret_line = lines.iter().find(|l| l.contains("^^^^^")).unwrap();
        let code_col = source_line.find("Motor").unwrap();
        let caret_col = caret_line.find('^').unwrap();
        assert_eq!(code_col, caret_col, "caret misaligned:\n{text}");
    }

    #[test]
    fn a_span_crossing_lines_renders_without_panicking_and_says_where_it_ends() {
        let src = "IF A THEN\n  B := 1;\nEND_IF\n";
        let (map, id) = sources(src);
        let d = Diagnostic::error(E_TEST, "example").with_primary(
            Span::new(id, 0, src.len() as u32 - 1),
            "this whole statement",
        );
        let text = render(&d, &map);
        assert!(text.contains("continues to conveyor.st:3:7"), "{text}");
    }

    #[test]
    fn a_zero_width_span_still_draws_one_caret() {
        let (map, id) = sources("X := ;\n");
        let d = Diagnostic::error(E_TEST, "expected an expression")
            .with_primary(Span::at(id, 5), "expected here");
        let text = render(&d, &map);
        assert!(text.contains('^'), "{text}");
    }

    #[test]
    fn diagnostics_are_capped_so_hostile_input_cannot_exhaust_memory() {
        let (_, id) = sources("X");
        let mut diags = Diagnostics::with_limit(3);
        for _ in 0..100 {
            diags.push(Diagnostic::error(E_TEST, "boom").with_primary(Span::at(id, 0), "here"));
        }
        assert_eq!(diags.len(), 3);
        assert_eq!(diags.dropped(), 97);
    }

    #[test]
    fn the_dropped_count_is_reported_rather_than_hidden() {
        let (map, id) = sources("X");
        let mut diags = Diagnostics::with_limit(1);
        for _ in 0..5 {
            diags.push(Diagnostic::error(E_TEST, "boom").with_primary(Span::at(id, 0), "here"));
        }
        let text = diags.render(&map);
        assert!(
            text.contains("4 further diagnostics were not recorded"),
            "{text}"
        );
    }

    #[test]
    fn sorting_is_total_so_rendering_is_byte_stable_whatever_order_errors_arrive_in() {
        let src = "AAAA\nBBBB\nCCCC\n";
        let (map, id) = sources(src);
        let make = |offset: u32, code: DiagCode, msg: &str| {
            Diagnostic::error(code, msg.to_string())
                .with_primary(Span::new(id, offset, offset + 1), "x")
        };

        let mut forwards = Diagnostics::new();
        forwards.push(make(0, E_TEST, "a"));
        forwards.push(make(5, E_TEST, "b"));
        forwards.push(make(5, W_TEST, "c"));
        forwards.push(make(10, E_TEST, "d"));
        forwards.sort();

        let mut backwards = Diagnostics::new();
        backwards.push(make(10, E_TEST, "d"));
        backwards.push(make(5, W_TEST, "c"));
        backwards.push(make(5, E_TEST, "b"));
        backwards.push(make(0, E_TEST, "a"));
        backwards.sort();

        assert_eq!(forwards.render(&map), backwards.render(&map));
    }

    #[test]
    fn one_line_form_is_usable_in_a_ci_log() {
        let src = "PROGRAM Main\n    X := 1;\nEND_PROGRAM\n";
        let (map, id) = sources(src);
        let offset = src.find('X').unwrap() as u32;
        let d = Diagnostic::error(E_TEST, "undeclared variable `X`")
            .with_primary(Span::new(id, offset, offset + 1), "not declared");
        assert_eq!(
            d.one_line(&map),
            "conveyor.st:2:5: error[E0001]: undeclared variable `X`"
        );
    }

    #[test]
    fn error_count_ignores_warnings() {
        let (_, id) = sources("X");
        let mut diags = Diagnostics::new();
        diags.push(Diagnostic::error(E_TEST, "e").with_primary(Span::at(id, 0), ""));
        diags.push(Diagnostic::warning(W_TEST, "w").with_primary(Span::at(id, 0), ""));
        assert_eq!(diags.error_count(), 1);
        assert_eq!(diags.len(), 2);
        assert!(diags.has_errors());
    }

    #[test]
    fn rendering_never_panics_on_a_span_that_points_outside_its_file() {
        let (map, id) = sources("short");
        let d = Diagnostic::error(E_TEST, "example")
            .with_primary(Span::new(id, 1_000, 2_000), "way out there");
        let _ = render(&d, &map);
    }
}
