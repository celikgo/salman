// SPDX-License-Identifier: Apache-2.0
//! Reporting results, for a person and for a build server.
//!
//! The JUnit XML written here targets the Jenkins `junit-10` schema, which is
//! the strictest of the three formats in circulation; conforming to it means
//! Jenkins, GitLab and the GitHub marketplace reporters all read it. The schema
//! is at
//! <https://raw.githubusercontent.com/jenkinsci/xunit-plugin/master/src/main/resources/org/jenkinsci/plugins/xunit/types/model/xsd/junit-10.xsd>.
//!
//! It is written here rather than taken as a dependency because the consumed
//! subset is about eight elements, and because the escaper is one of the
//! highest-value small fuzz targets in the project: failure messages contain
//! whatever an engineer typed, including bytes that are not legal in XML at
//! all.

use std::fmt::Write as _;

use crate::runner::{Outcome, Status};

/// How a whole run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Summary {
    /// Tests that passed.
    pub passed: usize,
    /// Tests whose expectations did not hold.
    pub failed: usize,
    /// Tests that could not be run.
    pub errored: usize,
    /// Tests that declared a reason to skip.
    pub skipped: usize,
}

impl Summary {
    /// Counts a set of outcomes.
    #[must_use]
    pub fn of(outcomes: &[Outcome]) -> Self {
        let mut summary = Self::default();
        for outcome in outcomes {
            match outcome.status {
                Status::Passed => summary.passed += 1,
                Status::Failed => summary.failed += 1,
                Status::Errored => summary.errored += 1,
                Status::Skipped => summary.skipped += 1,
            }
        }
        summary
    }

    /// How many tests there were.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.passed + self.failed + self.errored + self.skipped
    }

    /// Whether the run should be treated as a success.
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        self.failed == 0 && self.errored == 0
    }

    /// The process exit code: zero for success, one otherwise.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        if self.is_ok() { 0 } else { 1 }
    }
}

/// Renders results for a person reading a terminal.
#[must_use]
pub fn render_text(outcomes: &[Outcome]) -> String {
    let mut out = String::new();
    for outcome in outcomes {
        let mark = match outcome.status {
            Status::Passed => "pass",
            Status::Failed => "FAIL",
            Status::Errored => "ERROR",
            Status::Skipped => "skip",
        };
        let _ = writeln!(
            out,
            "{mark:>5}  {}  ({} scans, {})",
            outcome.name,
            outcome.scans,
            outcome.elapsed.to_iec_literal()
        );
        for problem in &outcome.problems {
            match problem.step {
                Some(step) => {
                    let _ = writeln!(out, "         step {step}: {}", problem.message);
                }
                None => {
                    let _ = writeln!(out, "         {}", problem.message);
                }
            }
        }
    }
    let summary = Summary::of(outcomes);
    let _ = writeln!(
        out,
        "\n{} tests: {} passed, {} failed, {} errored, {} skipped",
        summary.total(),
        summary.passed,
        summary.failed,
        summary.errored,
        summary.skipped
    );
    out
}

/// Renders results as JUnit XML.
#[must_use]
pub fn render_junit(suite: &str, outcomes: &[Outcome]) -> String {
    let summary = Summary::of(outcomes);
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<testsuites tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\">",
        summary.total(),
        summary.failed,
        summary.errored,
        summary.skipped
    );
    let _ = writeln!(
        out,
        "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\">",
        escape_attribute(suite),
        summary.total(),
        summary.failed,
        summary.errored,
        summary.skipped
    );
    for outcome in outcomes {
        let _ = write!(
            out,
            "    <testcase name=\"{}\" classname=\"{}\"",
            escape_attribute(&outcome.name),
            escape_attribute(suite)
        );
        match outcome.status {
            Status::Passed => out.push_str(" />\n"),
            Status::Skipped => {
                let _ = writeln!(
                    out,
                    ">\n      <skipped message=\"{}\" />\n    </testcase>",
                    escape_attribute(&joined(outcome))
                );
            }
            Status::Failed | Status::Errored => {
                let element = if outcome.status == Status::Failed { "failure" } else { "error" };
                let _ = writeln!(
                    out,
                    ">\n      <{element} message=\"{}\">{}</{element}>\n    </testcase>",
                    escape_attribute(&first_line(outcome)),
                    escape_text(&joined(outcome))
                );
            }
        }
    }
    out.push_str("  </testsuite>\n</testsuites>\n");
    out
}

fn first_line(outcome: &Outcome) -> String {
    outcome.problems.first().map_or_else(String::new, |p| p.message.clone())
}

fn joined(outcome: &Outcome) -> String {
    outcome
        .problems
        .iter()
        .map(|p| match p.step {
            Some(step) => format!("step {step}: {}", p.message),
            None => p.message.clone(),
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// Escapes text for an XML element body.
///
/// XML 1.0 permits only tab, line feed, carriage return and the characters
/// above U+001F. Anything else is replaced rather than emitted, because a
/// control character in a failure message would make the whole report
/// unparseable — and failure messages contain whatever an engineer typed.
#[must_use]
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || (0xd800..=0xdfff).contains(&(c as u32)) => {
                out.push('\u{fffd}');
            }
            c => out.push(c),
        }
    }
    out
}

/// Escapes text for an XML attribute value.
#[must_use]
pub fn escape_attribute(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if (c as u32) < 0x20 || (0xd800..=0xdfff).contains(&(c as u32)) => {
                out.push('\u{fffd}');
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::Problem;
    use salman_core::time::Duration;

    fn outcome(name: &str, status: Status, problems: Vec<&str>) -> Outcome {
        Outcome {
            name: name.into(),
            status,
            problems: problems
                .into_iter()
                .map(|m| Problem { step: Some(1), message: m.into() })
                .collect(),
            scans: 3,
            elapsed: Duration::from_nanos(3_000_000),
            trace: None,
            golden: None,
        }
    }

    #[test]
    fn a_summary_counts_every_kind_of_outcome() {
        let outcomes = [
            outcome("a", Status::Passed, vec![]),
            outcome("b", Status::Failed, vec!["x"]),
            outcome("c", Status::Errored, vec!["y"]),
            outcome("d", Status::Skipped, vec!["waiting"]),
        ];
        let summary = Summary::of(&outcomes);
        assert_eq!(summary.total(), 4);
        assert_eq!((summary.passed, summary.failed, summary.errored, summary.skipped), (1, 1, 1, 1));
        assert!(!summary.is_ok());
        assert_eq!(summary.exit_code(), 1);
    }

    #[test]
    fn a_run_with_only_passes_and_skips_succeeds() {
        let outcomes = [
            outcome("a", Status::Passed, vec![]),
            outcome("b", Status::Skipped, vec!["waiting"]),
        ];
        let summary = Summary::of(&outcomes);
        assert!(summary.is_ok());
        assert_eq!(summary.exit_code(), 0);
    }

    #[test]
    fn junit_output_reports_failures_and_errors_as_different_elements() {
        let xml = render_junit(
            "conveyor",
            &[
                outcome("passes", Status::Passed, vec![]),
                outcome("fails", Status::Failed, vec!["Motor_Run is FALSE, expected TRUE"]),
                outcome("errors", Status::Errored, vec!["no variable called Nope"]),
                outcome("skipped", Status::Skipped, vec!["waiting"]),
            ],
        );
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"), "{xml}");
        assert!(xml.contains("<failure message="), "{xml}");
        assert!(xml.contains("<error message="), "{xml}");
        assert!(xml.contains("<skipped message="), "{xml}");
        assert!(xml.contains("tests=\"4\" failures=\"1\" errors=\"1\" skipped=\"1\""), "{xml}");
    }

    #[test]
    fn xml_escaping_survives_anything_an_engineer_might_type() {
        let nasty = "a<b>c&d\"e'f\u{0}g\u{1f}h\ti";
        let text = escape_text(nasty);
        assert!(!text.contains('<') && !text.contains('>'));
        assert!(text.contains("&amp;"));
        assert!(!text.contains('\u{0}'), "a control character would break the whole report");
        let attribute = escape_attribute(nasty);
        assert!(attribute.contains("&quot;") && attribute.contains("&apos;"));
        assert!(!attribute.contains('\u{0}'));
        assert!(attribute.contains("&#9;"), "a tab in an attribute must be a reference");
    }

    #[test]
    fn a_failure_message_containing_xml_does_not_escape_its_element() {
        let xml = render_junit("s", &[outcome("t", Status::Failed, vec!["</failure><script>"])]);
        assert!(!xml.contains("<script>"), "{xml}");
    }

    #[test]
    fn the_text_report_names_every_failing_step() {
        let text = render_text(&[outcome("t", Status::Failed, vec!["Motor_Run is FALSE"])]);
        assert!(text.contains(" FAIL  t"), "{text}");
        assert!(text.contains("step 1: Motor_Run is FALSE"), "{text}");
        assert!(text.contains("1 tests: 0 passed, 1 failed"), "{text}");
    }
}
