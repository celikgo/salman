// SPDX-License-Identifier: Apache-2.0
//! A capture and a scan trace on one time axis.
//!
//! A control problem is almost never visible in either alone. The trace says
//! the program decided something and the capture says the device was asked
//! something, and the question is nearly always *which came first* — did the
//! scan act on the value that had arrived, or on the one before it?
//!
//! # The alignment cannot be guessed, so it is required
//!
//! A scan trace runs on salman's **virtual clock**, which starts at zero
//! because that is what makes a ten-minute sequence testable in
//! milliseconds. A capture carries **wall-clock** timestamps from the machine
//! that recorded it. Nothing in either says how they relate.
//!
//! There is no honest way to infer it. Two runs of the same program produce
//! identical traces; the same traffic captured twice produces different
//! timestamps. Lining up the first scan with the first frame would be a guess
//! that is wrong whenever the capture started first, which is the ordinary
//! case, and wrong in a way that looks perfectly reasonable — every event
//! shifted by a constant, every ordering plausible, every conclusion drawn
//! from it invalid.
//!
//! So [`Alignment`] is a required argument and there is no default. A caller
//! that does not know it has to say so, and the way to say so is
//! [`Alignment::from_correspondence`]: name one scan and the wall-clock
//! instant it happened at, which is something a person can establish and
//! salman cannot.

use core::fmt;

use salman_core::time::Duration;
use salman_core::value::Value;
use salman_findings::finding::Finding;
use salman_vm::trace::Trace;

use crate::modbus::Analysis;

/// How a scan trace's clock relates to a capture's.
///
/// Not inferable; see the module documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alignment {
    /// Where scan time zero sits, in nanoseconds since the Unix epoch.
    pub scan_zero_at_nanos: u64,
}

impl Alignment {
    /// States that a scan happened at a wall-clock instant.
    ///
    /// The form a person can actually supply: "scan 12 ran at 10:31:04.250".
    /// Returns `None` if that would put scan zero before the epoch, which
    /// means one of the two numbers is not what the caller thinks it is.
    #[must_use]
    pub fn from_correspondence(scan_time: Duration, wall_nanos: u64) -> Option<Self> {
        let offset = u64::try_from(scan_time.nanos()).ok()?;
        Some(Self {
            scan_zero_at_nanos: wall_nanos.checked_sub(offset)?,
        })
    }

    /// Where a scan-clock instant falls on the wall clock.
    #[must_use]
    pub fn wall_nanos(&self, scan_time: Duration) -> u64 {
        // A negative scan time cannot happen on the virtual clock, which
        // starts at zero and only moves forward; if one somehow arrives it
        // clamps to the alignment point rather than wrapping to an enormous
        // instant that would sort last.
        let offset = u64::try_from(scan_time.nanos()).unwrap_or(0);
        self.scan_zero_at_nanos.saturating_add(offset)
    }
}

/// What happened at one instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A scan of the program finished.
    Scan {
        /// Which scan, counting from zero.
        scan: u64,
        /// Which task ran.
        task: u16,
        /// The watched signals, as name and rendered value.
        values: Vec<(String, String)>,
    },
    /// Something salman is willing to say about the capture.
    Finding {
        /// Its stable identifier.
        id: &'static str,
        /// One line about it.
        summary: String,
    },
}

/// One row of the timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// When, in nanoseconds since the Unix epoch.
    pub at_nanos: u64,
    /// What.
    pub event: Event,
    /// Which scan this fell inside, for events that are not scans themselves.
    ///
    /// The whole reason to merge the two: a request that arrived between scan
    /// 5 and scan 6 was acted on by scan 6, and a reader should not have to
    /// work that out by subtracting timestamps.
    pub during_scan: Option<u64>,
}

/// A capture and a scan trace, merged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Timeline {
    /// Every row, in time order.
    pub entries: Vec<Entry>,
    /// How the two clocks were related, for the header.
    alignment_nanos: u64,
}

impl Timeline {
    /// Merges a scan trace and a capture analysis onto one axis.
    ///
    /// `alignment` says where the trace's clock sits on the capture's. It is a
    /// parameter rather than something salman works out, and the module
    /// documentation says why at length: guessing it produces a timeline where
    /// every ordering is plausible and every conclusion is wrong.
    #[must_use]
    pub fn merge(trace: &Trace, analysis: &Analysis, alignment: Alignment) -> Self {
        let mut entries: Vec<Entry> = Vec::new();

        for sample in &trace.samples {
            let values = trace
                .signals
                .iter()
                .zip(&sample.values)
                .map(|(signal, value)| (signal.name.clone(), render(value)))
                .collect();
            entries.push(Entry {
                at_nanos: alignment.wall_nanos(sample.time),
                event: Event::Scan {
                    scan: sample.scan,
                    task: sample.task,
                    values,
                },
                during_scan: Some(sample.scan),
            });
        }

        for finding in &analysis.findings {
            // A finding with no timestamp is about the capture as a whole
            // rather than about a moment in it, and putting it at zero would
            // sort it before everything and imply it happened first.
            let Some(at) = finding.evidence().timestamp else {
                continue;
            };
            entries.push(Entry {
                at_nanos: at,
                event: Event::Finding {
                    id: finding.id(),
                    summary: summarise(finding),
                },
                during_scan: None,
            });
        }

        // Stable by time, and by the order they were added within an instant,
        // so a timeline of the same inputs is always the same timeline.
        entries.sort_by_key(|entry| entry.at_nanos);

        // Now say which scan each non-scan event fell inside. A scan's sample
        // is timestamped at its *end*, so an event belongs to the first scan
        // that finished at or after it.
        let scan_ends: Vec<(u64, u64)> = entries
            .iter()
            .filter_map(|entry| match entry.event {
                Event::Scan { scan, .. } => Some((entry.at_nanos, scan)),
                Event::Finding { .. } => None,
            })
            .collect();
        for entry in &mut entries {
            if entry.during_scan.is_some() {
                continue;
            }
            entry.during_scan = scan_ends
                .iter()
                .find(|(end, _)| *end >= entry.at_nanos)
                .map(|(_, scan)| *scan);
        }

        Self {
            entries,
            alignment_nanos: alignment.scan_zero_at_nanos,
        }
    }

    /// Whether anything landed on the axis.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Renders the timeline as plain text.
    ///
    /// No colour, in the same style as salman's traces and diagnostics, so
    /// that a golden test compares bytes and meaning never depends on a colour
    /// a reader may not see.
    #[must_use]
    pub fn render(&self) -> String {
        use core::fmt::Write;
        let mut out = String::new();
        let _ = writeln!(out, "# salman timeline");
        let _ = writeln!(
            out,
            "# scan clock zero is {} ns since the epoch, as the caller stated",
            self.alignment_nanos
        );
        let _ = writeln!(out, "#");
        let _ = writeln!(out, "time\tscan\tsource\twhat");

        let first = self.entries.first().map_or(0, |entry| entry.at_nanos);
        for entry in &self.entries {
            // Relative to the first row, because an absolute nanosecond count
            // is unreadable and the interval is what the reader is after.
            let offset = entry.at_nanos.saturating_sub(first);
            let scan = entry
                .during_scan
                .map_or_else(|| "-".to_string(), |s| s.to_string());
            match &entry.event {
                Event::Scan { task, values, .. } => {
                    let rendered: Vec<String> = values
                        .iter()
                        .map(|(name, value)| format!("{name}={value}"))
                        .collect();
                    let _ = writeln!(
                        out,
                        "{}\t{scan}\tscan\ttask {task}: {}",
                        interval(offset),
                        rendered.join(" ")
                    );
                }
                Event::Finding { id, summary } => {
                    let _ = writeln!(out, "{}\t{scan}\twire\t{id}: {summary}", interval(offset));
                }
            }
        }
        out
    }
}

impl fmt::Display for Timeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// A nanosecond offset, in a form a person reads.
fn interval(nanos: u64) -> String {
    if nanos == 0 {
        return "0".to_string();
    }
    if nanos < 1_000 {
        return format!("{nanos}ns");
    }
    if nanos < 1_000_000 {
        return format!("{}.{:03}us", nanos / 1_000, nanos % 1_000);
    }
    if nanos < 1_000_000_000 {
        return format!("{}.{:03}ms", nanos / 1_000_000, (nanos % 1_000_000) / 1_000);
    }
    format!(
        "{}.{:03}s",
        nanos / 1_000_000_000,
        (nanos % 1_000_000_000) / 1_000_000
    )
}

/// One line about a finding, for a timeline row.
fn summarise(finding: &Finding) -> String {
    // The first sentence, or the whole message if it has none. A timeline row
    // that wrapped over four lines would stop being a timeline.
    let message = finding.message();
    match message.find(". ") {
        Some(stop) => message.get(..=stop).unwrap_or(message).to_string(),
        None => message.to_string(),
    }
}

/// A watched value, rendered the way the trace renders it.
fn render(value: &Value) -> String {
    format!("{value}")
}
