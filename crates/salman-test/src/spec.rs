// SPDX-License-Identifier: Apache-2.0
//! The declarative test format.
//!
//! A test file is YAML, holding either one test or a list of them:
//!
//! ```yaml
//! - test: "TON does not fire early"
//!   pou: Conveyor_Ctrl
//!   given: { Start: false, Delay.PT: "T#5s" }
//!   steps:
//!     - { set: { Start: true }, scans: 1 }
//!     - { advance: "T#4s999ms", expect: { Motor_Run: false } }
//!     - { advance: "T#2ms",     expect: { Motor_Run: true } }
//! ```
//!
//! Values are written as IEC literals, lexed with salman's own lexer, so
//! `T#5s`, `16#FF` and `D#2024-02-29` all mean here exactly what they mean in
//! source code.
//!
//! Unknown keys are **rejected**, not ignored. A misspelled `expects:` that was
//! silently skipped would leave a test passing while asserting nothing, which
//! is the worst failure a test harness has.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::value::ValueSpec;

/// One step of a test.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Variables to write before running.
    #[serde(default)]
    pub set: BTreeMap<String, ValueSpec>,
    /// Variables to force, which the program cannot then overwrite.
    #[serde(default)]
    pub force: BTreeMap<String, ValueSpec>,
    /// Forces to release.
    #[serde(default)]
    pub release: Vec<String>,
    /// Scans to run.
    #[serde(default)]
    pub scans: Option<u64>,
    /// Simulation time to advance, as a duration literal such as `"T#5s"`.
    #[serde(default)]
    pub advance: Option<String>,
    /// Variables to check after running.
    #[serde(default)]
    pub expect: BTreeMap<String, ValueSpec>,
    /// A note shown when this step fails.
    #[serde(default)]
    pub note: Option<String>,
}

/// One test.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestCase {
    /// What the test is called. Shown in the report and in the JUnit output.
    pub test: String,
    /// The POU instance names are resolved against, when a name is ambiguous.
    #[serde(default)]
    pub pou: Option<String>,
    /// Variables to write before the first step.
    #[serde(default)]
    pub given: BTreeMap<String, ValueSpec>,
    /// The steps, in order.
    #[serde(default)]
    pub steps: Vec<Step>,
    /// Signals to record, for a golden-trace test.
    #[serde(default)]
    pub record: Vec<String>,
    /// The golden trace file this run is compared against, relative to the
    /// test file.
    #[serde(default)]
    pub golden: Option<String>,
    /// The seed recorded in the trace. Defaults to zero.
    #[serde(default)]
    pub seed: Option<u64>,
    /// A reason this test is not run. Present means skipped, and the reason is
    /// reported — a skipped test with no reason is a test nobody will fix.
    #[serde(default)]
    pub skip: Option<String>,
}

/// A test file: one test, or a list of them.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum TestFile {
    /// A single test.
    One(Box<TestCase>),
    /// Several tests.
    Many(Vec<TestCase>),
}

impl TestFile {
    /// The tests it holds, in file order.
    #[must_use]
    pub fn cases(self) -> Vec<TestCase> {
        match self {
            Self::One(case) => vec![*case],
            Self::Many(cases) => cases,
        }
    }
}

/// Parses a test file.
///
/// # Errors
///
/// Returns the parser's message, which carries the line and column.
pub fn parse(text: &str) -> Result<Vec<TestCase>, String> {
    let file: TestFile = serde_saphyr::from_str(text).map_err(|e| e.to_string())?;
    Ok(file.cases())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_test_parses() {
        let cases = parse(
            r#"
test: "TON does not fire early"
pou: Conveyor_Ctrl
given: { Start: false, "Delay.PT": "T#5s" }
steps:
  - { set: { Start: true }, scans: 1 }
  - { advance: "T#4s999ms", expect: { Motor_Run: false } }
"#,
        )
        .expect("parses");
        assert_eq!(cases.len(), 1);
        let case = cases.first().expect("one case");
        assert_eq!(case.test, "TON does not fire early");
        assert_eq!(case.pou.as_deref(), Some("Conveyor_Ctrl"));
        assert_eq!(case.given.len(), 2);
        assert_eq!(case.steps.len(), 2);
        assert_eq!(case.steps.first().and_then(|s| s.scans), Some(1));
        assert_eq!(
            case.steps.get(1).and_then(|s| s.advance.clone()).as_deref(),
            Some("T#4s999ms")
        );
    }

    #[test]
    fn a_list_of_tests_parses() {
        let cases = parse(
            r"
- test: first
  steps: []
- test: second
  steps: []
",
        )
        .expect("parses");
        assert_eq!(cases.len(), 2);
    }

    #[test]
    fn an_unknown_key_is_rejected_rather_than_ignored() {
        // A misspelled `expects:` that was silently skipped would leave the
        // test passing while asserting nothing.
        let error = parse(
            r"
test: typo
steps:
  - { expects: { X: true } }
",
        )
        .expect_err("must be rejected");
        assert!(!error.is_empty());
    }

    #[test]
    fn a_test_with_no_name_is_rejected() {
        assert!(parse("steps: []\n").is_err());
    }

    #[test]
    fn values_keep_their_written_form_for_later_conversion() {
        let cases =
            parse("test: t\ngiven: { A: 3, B: true, C: 1.5, D: \"T#1s\" }\n").expect("parses");
        let given = &cases.first().expect("case").given;
        assert_eq!(given.get("A"), Some(&ValueSpec::Int(3)));
        assert_eq!(given.get("B"), Some(&ValueSpec::Bool(true)));
        assert_eq!(given.get("C"), Some(&ValueSpec::Real(1.5)));
        assert_eq!(given.get("D"), Some(&ValueSpec::Text("T#1s".into())));
    }

    #[test]
    fn a_skipped_test_must_say_why() {
        let cases = parse("test: t\nskip: \"waiting on the OPC UA client\"\n").expect("parses");
        assert_eq!(
            cases.first().and_then(|c| c.skip.clone()).as_deref(),
            Some("waiting on the OPC UA client")
        );
    }

    #[test]
    fn malformed_yaml_reports_rather_than_panicking() {
        for text in ["test: [", "\t- x", "{{{{", &"- ".repeat(5000)] {
            let _ = parse(text);
        }
    }
}
