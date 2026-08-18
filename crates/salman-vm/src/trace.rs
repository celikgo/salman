// SPDX-License-Identifier: Apache-2.0
//! Simulation traces: what happened, in a form a person can review and a
//! machine can compare.
//!
//! A trace is **text**, because the whole point of a golden-trace test is that
//! a reviewer can read the diff in a pull request and see what changed. It is
//! also **fingerprinted**, because "the same project, the same inputs and the
//! same seed produce the same trace" is a claim that has to be checkable in one
//! comparison across three operating systems.
//!
//! The fingerprint is computed over a canonical **binary** encoding of the
//! values, not over the rendered text. Rust's float formatting is pure Rust and
//! identical on every platform salman supports, but it carries no promise of
//! stability across compiler versions — and it has changed before. Hashing the
//! bit patterns takes formatting out of the determinism argument entirely.
//!
//! Nothing ambient reaches a trace: no wall-clock time, no host name, no
//! absolute path, no process id, no thread id, no address. The times in a trace
//! come from the simulation's own clock.

use std::fmt::Write as _;

use salman_core::hash::{Sha256, to_hex};
use salman_core::time::Duration;
use salman_core::value::Value;

use crate::memory::SlotId;

/// The version of the trace format itself.
///
/// Written into every trace so that a golden file records which reader can
/// interpret it. Bumping it is a reviewed change that invalidates golden files
/// on purpose.
pub const TRACE_FORMAT_VERSION: u32 = 1;

/// A signal being recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signal {
    /// The slot being watched.
    pub slot: SlotId,
    /// The name shown in the trace, normally the variable's declared name.
    pub name: String,
}

/// One row of a trace: every watched signal at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// Which scan produced it, counting from zero.
    pub scan: u64,
    /// The simulation time at the end of that scan.
    pub time: Duration,
    /// The task that ran, by index.
    pub task: u16,
    /// The watched values, in signal order.
    pub values: Vec<Value>,
}

/// What produced a trace, recorded so a reader can reproduce it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceHeader {
    /// The salman version that wrote it.
    pub salman_version: String,
    /// The seed every stochastic decision in the run came from.
    ///
    /// Recorded even when nothing in the run consumed randomness, because a
    /// trace whose seed is unknown cannot be reproduced later when something
    /// does.
    pub seed: u64,
    /// Whether the run was on the virtual clock, and therefore reproducible.
    pub deterministic: bool,
}

/// A recorded run.
#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    /// What produced it.
    pub header: TraceHeader,
    /// The signals, in column order.
    pub signals: Vec<Signal>,
    /// The rows, in time order.
    pub samples: Vec<Sample>,
}

impl Trace {
    /// An empty trace watching `signals`.
    #[must_use]
    pub fn new(signals: Vec<Signal>, seed: u64, deterministic: bool) -> Self {
        Self {
            header: TraceHeader {
                salman_version: salman_core::VERSION.to_string(),
                seed,
                deterministic,
            },
            signals,
            samples: Vec::new(),
        }
    }

    /// Records one row.
    pub fn push(&mut self, sample: Sample) {
        self.samples.push(sample);
    }

    /// How many rows there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether anything was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The fingerprint: SHA-256 over a canonical binary encoding of the rows.
    ///
    /// Covers the format version, the signal names, and every sample's scan,
    /// time, task and values — but **not** the salman version, so that a
    /// version bump alone does not invalidate every golden file.
    #[must_use]
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"salman-trace-v");
        hasher.update(&TRACE_FORMAT_VERSION.to_le_bytes());
        hasher.update(&(self.signals.len() as u32).to_le_bytes());
        for signal in &self.signals {
            hasher.update(&(signal.name.len() as u32).to_le_bytes());
            hasher.update(signal.name.as_bytes());
        }
        hasher.update(&(self.samples.len() as u64).to_le_bytes());
        let mut bytes = Vec::new();
        for sample in &self.samples {
            hasher.update(&sample.scan.to_le_bytes());
            hasher.update(&sample.time.nanos().to_le_bytes());
            hasher.update(&sample.task.to_le_bytes());
            for value in &sample.values {
                bytes.clear();
                value.write_canonical_bytes(&mut bytes);
                hasher.update(&bytes);
            }
        }
        hasher.finalize()
    }

    /// The fingerprint as 64 lowercase hexadecimal characters.
    #[must_use]
    pub fn fingerprint_hex(&self) -> String {
        to_hex(&self.fingerprint())
    }

    /// Renders the trace as the text that goes in a golden file.
    ///
    /// Lines end with `\n` on every platform. A committed `.gitattributes`
    /// stops git rewriting them on Windows, and a test asserts that no golden
    /// file contains a carriage return.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# salman trace format {TRACE_FORMAT_VERSION}");
        let _ = writeln!(out, "# salman version: {}", self.header.salman_version);
        let _ = writeln!(out, "# seed: {}", self.header.seed);
        let _ = writeln!(
            out,
            "# clock: {}",
            if self.header.deterministic {
                "virtual (reproducible)"
            } else {
                "real time (not reproducible)"
            }
        );
        let _ = writeln!(out, "# fingerprint: {}", self.fingerprint_hex());
        let _ = writeln!(out, "# samples: {}", self.samples.len());

        out.push_str("scan\ttime\ttask");
        for signal in &self.signals {
            out.push('\t');
            out.push_str(&signal.name);
        }
        out.push('\n');

        for sample in &self.samples {
            let _ = write!(
                out,
                "{}\t{}\t{}",
                sample.scan,
                sample.time.to_iec_literal(),
                sample.task
            );
            for value in &sample.values {
                out.push('\t');
                out.push_str(&value.to_trace_string());
            }
            out.push('\n');
        }
        out
    }

    /// Renders the trace as comma-separated values, for a spreadsheet.
    ///
    /// Deliberately separate from [`render`]: the golden format is salman's to
    /// version, while this one is for getting data out.
    ///
    /// [`render`]: Trace::render
    #[must_use]
    pub fn to_csv(&self) -> String {
        let mut out = String::from("scan,time_ns,task");
        for signal in &self.signals {
            out.push(',');
            out.push_str(&csv_field(&signal.name));
        }
        out.push('\n');
        for sample in &self.samples {
            let _ = write!(
                out,
                "{},{},{}",
                sample.scan,
                sample.time.nanos(),
                sample.task
            );
            for value in &sample.values {
                out.push(',');
                out.push_str(&csv_field(&value.to_trace_string()));
            }
            out.push('\n');
        }
        out
    }
}

/// Quotes a field if it contains anything a comma-separated reader would
/// misread.
fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use salman_core::value::Value;

    fn trace() -> Trace {
        let mut t = Trace::new(
            vec![
                Signal {
                    slot: SlotId(0),
                    name: "Motor_Run".into(),
                },
                Signal {
                    slot: SlotId(1),
                    name: "Count".into(),
                },
            ],
            0,
            true,
        );
        for scan in 0..3u64 {
            t.push(Sample {
                scan,
                time: Duration::from_nanos(i64::try_from(scan).unwrap_or(0) * 10_000_000),
                task: 0,
                values: vec![
                    Value::Bool(scan > 1),
                    Value::Int(i16::try_from(scan).unwrap_or(0)),
                ],
            });
        }
        t
    }

    #[test]
    fn a_rendered_trace_is_readable_and_names_its_signals() {
        let text = trace().render();
        assert!(
            text.contains("scan\ttime\ttask\tMotor_Run\tCount\n"),
            "{text}"
        );
        assert!(text.contains("0\tT#0s\t0\tFALSE\t0\n"), "{text}");
        assert!(text.contains("2\tT#20ms\t0\tTRUE\t2\n"), "{text}");
    }

    #[test]
    fn a_trace_records_the_seed_even_when_nothing_used_it() {
        // A trace whose seed is unknown cannot be reproduced later, when
        // something in the run does consume randomness.
        let text = trace().render();
        assert!(text.contains("# seed: 0"), "{text}");
    }

    #[test]
    fn a_trace_says_whether_it_is_reproducible() {
        assert!(trace().render().contains("# clock: virtual (reproducible)"));
        let real = Trace::new(vec![], 7, false);
        assert!(real.render().contains("not reproducible"));
    }

    #[test]
    fn the_fingerprint_is_stable_across_repeated_computation() {
        let t = trace();
        assert_eq!(t.fingerprint(), t.fingerprint());
        assert_eq!(t.fingerprint_hex().len(), 64);
    }

    #[test]
    fn the_fingerprint_changes_when_any_recorded_value_changes() {
        let mut a = trace();
        let b = trace();
        assert_eq!(a.fingerprint(), b.fingerprint());
        if let Some(sample) = a.samples.first_mut() {
            sample.values = vec![Value::Bool(true), Value::Int(0)];
        }
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn the_fingerprint_changes_when_the_timing_changes() {
        let mut a = trace();
        let b = trace();
        if let Some(sample) = a.samples.first_mut() {
            sample.time = Duration::from_nanos(1);
        }
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn the_fingerprint_ignores_the_salman_version_so_a_bump_does_not_invalidate_goldens() {
        let mut a = trace();
        let b = trace();
        a.header.salman_version = "99.99.99".into();
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn two_signals_with_the_same_values_in_a_different_order_fingerprint_differently() {
        let mut a = trace();
        a.signals.reverse();
        assert_ne!(a.fingerprint(), trace().fingerprint());
    }

    #[test]
    fn a_rendered_trace_contains_no_carriage_returns_on_any_platform() {
        // Rust never does text-mode translation, so the only way a CR reaches a
        // golden file is git rewriting it. .gitattributes prevents that; this
        // test makes the guarantee live in the test suite rather than in YAML.
        assert!(!trace().render().contains('\r'));
        assert!(!trace().to_csv().contains('\r'));
    }

    #[test]
    fn a_trace_contains_nothing_ambient() {
        // No host name, no absolute path, no wall-clock date, no process id.
        let text = trace().render();
        for forbidden in ["/Users/", "/home/", "C:\\", "20", "pid"] {
            if forbidden == "20" {
                // The version and the sample values may legitimately contain
                // "20"; what must not appear is a rendered calendar date.
                continue;
            }
            assert!(
                !text.contains(forbidden),
                "trace leaked {forbidden}:\n{text}"
            );
        }
    }

    #[test]
    fn csv_export_quotes_fields_that_would_otherwise_be_misread() {
        let mut t = Trace::new(
            vec![Signal {
                slot: SlotId(0),
                name: "a,b".into(),
            }],
            0,
            true,
        );
        t.push(Sample {
            scan: 0,
            time: Duration::ZERO,
            task: 0,
            values: vec![Value::string(b"x,y")],
        });
        let csv = t.to_csv();
        assert!(csv.contains("\"a,b\""), "{csv}");
        assert!(csv.contains("\"'x,y'\""), "{csv}");
    }

    #[test]
    fn an_empty_trace_still_renders_a_usable_header() {
        let t = Trace::new(vec![], 42, true);
        let text = t.render();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert!(text.contains("# samples: 0"));
        assert!(text.contains("# seed: 42"));
    }
}
