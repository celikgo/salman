//! What salman can actually do, and how you can check.
//!
//! Every status claim salman publishes — the README table, `salman status`,
//! `docs/STATUS.md` — is generated from [`REGISTRY`]. It therefore cannot drift
//! away from the code, and a capability cannot be described as tested unless it
//! names the tests that test it.
//!
//! Two tests in this module enforce that:
//!
//! * a capability claiming [`Status::ImplementedTested`] must cite at least one
//!   piece of [`Evidence`], and
//! * every cited test must exist in the source tree, at the named path, spelled
//!   exactly that way.
//!
//! Deleting a test therefore fails the build of the crate that claims it.

use std::fmt;
use std::fmt::Write as _;

/// How far along a capability is.
///
/// The four values are deliberately blunt. "Mostly working" is not a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    /// It works, and tests in this repository demonstrate that it works.
    ImplementedTested,
    /// The code exists and runs, but nothing proves it is right. Using this
    /// status is an admission, not a milestone.
    ImplementedUntested,
    /// A placeholder exists that says, at runtime, that it is a placeholder.
    Stub,
    /// Not written. No code, no partial behaviour, nothing to call.
    Planned,
}

impl Status {
    /// The wording used in generated documentation.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ImplementedTested => "implemented and tested",
            Self::ImplementedUntested => "implemented, untested",
            Self::Stub => "stub",
            Self::Planned => "planned",
        }
    }

    /// A shape, not a colour. A red/green table some readers cannot
    /// distinguish is a defect, so generated tables lead with these.
    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::ImplementedTested => "[x]",
            Self::ImplementedUntested => "[~]",
            Self::Stub => "[-]",
            Self::Planned => "[ ]",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A test that demonstrates a capability works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Evidence {
    /// Path from the repository root, using forward slashes.
    pub file: &'static str,
    /// The exact name of the test function.
    pub test: &'static str,
}

/// One thing salman does, or does not yet do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    /// Stable dotted identifier, e.g. `"lang.st.parser"`. Never renamed once
    /// published; it is what release notes and issues refer to.
    pub id: &'static str,
    /// Grouping used for the generated table, e.g. `"Language"`.
    pub area: &'static str,
    /// One line, in an engineer's words, about what this is.
    pub title: &'static str,
    /// How far along it is.
    pub status: Status,
    /// The milestone it belongs to, e.g. `"v0.1"`.
    pub milestone: &'static str,
    /// Tests that demonstrate it. Required when `status` is
    /// [`Status::ImplementedTested`].
    pub evidence: &'static [Evidence],
    /// Anything a reader needs in order not to be misled. Limitations go here.
    pub note: &'static str,
}

/// Everything salman claims, in one place.
///
/// Ordered by `id`; a test enforces that, so generated documents are stable.
pub static REGISTRY: &[Capability] = &[
    Capability {
        id: "core.capability-registry",
        area: "Project infrastructure",
        title: "Generated capability status, with tests cited as evidence",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-core/src/capability.rs",
                test: "tested_capabilities_must_cite_evidence",
            },
            Evidence {
                file: "crates/salman-core/src/capability.rs",
                test: "every_cited_test_exists_in_the_source_tree",
            },
        ],
        note: "Status tables in the README and docs are generated from this registry.",
    },
    Capability {
        id: "core.clause-citations",
        area: "Project infrastructure",
        title: "IEC clause citation registry with explicit provenance",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[Evidence {
            file: "crates/salman-core/src/clause.rs",
            test: "citation_display_flags_unconfirmed_numbers_so_a_reader_cannot_miss_it",
        }],
        note: "No clauses are cited yet; the mechanism and its honesty rules are in place.",
    },
    Capability {
        id: "core.diagnostics",
        area: "Language",
        title: "Diagnostics with spans, IEC clause citations and the dialect rule applied",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-core/src/diag.rs",
                test: "a_rendered_error_points_at_the_offending_text",
            },
            Evidence {
                file: "crates/salman-core/src/diag.rs",
                test: "sorting_is_total_so_rendering_is_byte_stable_whatever_order_errors_arrive_in",
            },
            Evidence {
                file: "crates/salman-core/src/diag.rs",
                test: "diagnostics_are_capped_so_hostile_input_cannot_exhaust_memory",
            },
        ],
        note: "Rendered in plain text with no colour, so meaning never depends on colour \
               and golden tests can compare bytes.",
    },
    Capability {
        id: "core.identifiers",
        area: "Language",
        title: "Case-insensitive, case-preserving IEC identifiers",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-core/src/ident.rs",
                test: "identifiers_compare_case_insensitively",
            },
            Evidence {
                file: "crates/salman-core/src/ident.rs",
                test: "identifier_ordering_ignores_case_so_generated_output_is_stable",
            },
        ],
        note: "ASCII case rules only, so identifier identity cannot shift with a Unicode version.",
    },
    Capability {
        id: "core.posture",
        area: "Safety",
        title: "OBSERVE / SIMULATE / ARMED posture model with categorical refusals",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-core/src/posture.rs",
                test: "the_default_posture_is_observe",
            },
            Evidence {
                file: "crates/salman-core/src/posture.rs",
                test: "firmware_credential_and_dos_effects_are_refused_at_every_posture",
            },
            Evidence {
                file: "crates/salman-core/src/posture.rs",
                test: "armed_still_requires_per_call_confirmation_for_live_writes",
            },
        ],
        note: "No code path in salman writes to a device yet, so nothing calls this. \
               It exists first so that the first write path cannot avoid it.",
    },
    Capability {
        id: "core.source-map",
        area: "Language",
        title: "Source map, spans and line/column resolution for diagnostics",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-core/src/span.rs",
                test: "line_col_counts_columns_in_characters_not_bytes",
            },
            Evidence {
                file: "crates/salman-core/src/span.rs",
                test: "oversized_sources_are_rejected_rather_than_loaded",
            },
        ],
        note: "Source files above 64 MiB are refused rather than loaded.",
    },
    Capability {
        id: "core.time",
        area: "Language",
        title: "TIME, LTIME, DATE, TIME_OF_DAY and DATE_AND_TIME values",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-core/src/time.rs",
                test: "date_conversion_round_trips_over_a_wide_range",
            },
            Evidence {
                file: "crates/salman-core/src/time.rs",
                test: "duration_literal_components_are_summed_so_a_unit_may_overflow",
            },
            Evidence {
                file: "crates/salman-core/src/time.rs",
                test: "duration_arithmetic_reports_overflow_rather_than_wrapping",
            },
        ],
        note: "Leap seconds, time zones and daylight saving are not modelled: every day \
               here is exactly 86 400 s.",
    },
    Capability {
        id: "core.value-model",
        area: "Language",
        title: "Elementary types, the ANY generic hierarchy, and runtime values",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-core/src/value.rs",
                test: "the_generic_hierarchy_matches_the_standard_groupings",
            },
            Evidence {
                file: "crates/salman-core/src/value.rs",
                test: "nan_is_canonicalised_so_traces_cannot_differ_between_architectures",
            },
            Evidence {
                file: "crates/salman-core/src/value.rs",
                test: "strings_hold_arbitrary_bytes_without_corrupting_them",
            },
        ],
        note: "CHAR, WCHAR, LDATE, LTOD and LDT are not implemented. NaN is canonicalised \
               on entry so that a trace cannot differ between architectures.",
    },
    Capability {
        id: "core.version-truth",
        area: "Project infrastructure",
        title: "One source of version truth, checked when the crate compiles",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[Evidence {
            file: "crates/salman-core/src/version.rs",
            test: "version_is_read_from_the_version_file_and_matches_cargo",
        }],
        note: "The root VERSION file and Cargo's version cannot disagree: the mismatch \
               is a compile error, not a CI job.",
    },
];

/// Capabilities in `area`, in registry order.
pub fn in_area(area: &str) -> impl Iterator<Item = &'static Capability> {
    REGISTRY.iter().filter(move |c| c.area == area)
}

/// Distinct areas, in first-appearance order.
#[must_use]
pub fn areas() -> Vec<&'static str> {
    let mut seen: Vec<&'static str> = Vec::new();
    for c in REGISTRY {
        if !seen.contains(&c.area) {
            seen.push(c.area);
        }
    }
    seen.sort_unstable();
    seen
}

/// Renders the status table used in `docs/STATUS.md` and by `salman status`.
///
/// Deterministic: areas are sorted, and capabilities inside an area keep
/// registry order, which is enforced to be `id` order.
#[must_use]
pub fn render_markdown() -> String {
    let mut out = String::new();
    out.push_str("# salman capability status\n\n");
    out.push_str("*Generated from `salman_core::capability::REGISTRY`. Do not edit by hand.*\n\n");
    out.push_str(
        "`[x]` implemented and tested  ·  `[~]` implemented, untested  ·  \
         `[-]` stub  ·  `[ ]` planned\n\n",
    );
    out.push_str(
        "A capability is only marked *implemented and tested* if it names tests that \
         exist in this repository. A test in this crate checks that they do.\n",
    );
    for area in areas() {
        // Writing to a String is infallible; the Result is discarded for that
        // reason and no other.
        let _ = write!(out, "\n## {area}\n\n");
        out.push_str("| | Capability | Status | Milestone | Notes |\n");
        out.push_str("|---|---|---|---|---|\n");
        for c in in_area(area) {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} | {} |",
                c.status.marker(),
                c.title,
                c.status,
                c.milestone,
                c.note
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn repo_root() -> PathBuf {
        // CARGO_MANIFEST_DIR is <root>/crates/salman-core.
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("crate is two levels below the repository root")
            .to_path_buf()
    }

    #[test]
    fn capability_ids_are_unique_and_sorted() {
        let ids: Vec<&str> = REGISTRY.iter().map(|c| c.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "REGISTRY must be kept in id order");
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "duplicate capability id");
    }

    #[test]
    fn tested_capabilities_must_cite_evidence() {
        for c in REGISTRY {
            if c.status == Status::ImplementedTested {
                assert!(
                    !c.evidence.is_empty(),
                    "{} claims to be tested but cites no test",
                    c.id
                );
            }
        }
    }

    #[test]
    fn planned_capabilities_cite_nothing_because_nothing_exists_to_cite() {
        for c in REGISTRY {
            if matches!(c.status, Status::Planned) {
                assert!(
                    c.evidence.is_empty(),
                    "{} is planned but cites evidence, which cannot exist",
                    c.id
                );
            }
        }
    }

    #[test]
    fn every_cited_test_exists_in_the_source_tree() {
        let root = repo_root();
        for c in REGISTRY {
            for e in c.evidence {
                let path = root.join(e.file);
                let source = std::fs::read_to_string(&path).unwrap_or_else(|err| {
                    panic!("{} cites {}, which cannot be read: {err}", c.id, e.file)
                });
                let needle = format!("fn {}(", e.test);
                assert!(
                    source.contains(&needle),
                    "{} cites test `{}` in {}, which is not there",
                    c.id,
                    e.test,
                    e.file
                );
            }
        }
    }

    #[test]
    fn every_capability_is_described_and_placed_in_a_milestone() {
        for c in REGISTRY {
            assert!(!c.id.is_empty());
            assert!(!c.title.is_empty(), "{} has no title", c.id);
            assert!(!c.area.is_empty(), "{} has no area", c.id);
            assert!(
                c.milestone.starts_with('v'),
                "{} has milestone {:?}, which is not a version",
                c.id,
                c.milestone
            );
        }
    }

    #[test]
    fn status_markers_are_distinguishable_without_colour() {
        let markers = [
            Status::ImplementedTested.marker(),
            Status::ImplementedUntested.marker(),
            Status::Stub.marker(),
            Status::Planned.marker(),
        ];
        let mut unique = markers.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), markers.len());
    }

    #[test]
    fn rendered_status_is_deterministic() {
        assert_eq!(render_markdown(), render_markdown());
    }
}
