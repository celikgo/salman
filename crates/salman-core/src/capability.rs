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
        evidence: &[
            Evidence {
                file: "crates/salman-core/src/clause.rs",
                test: "citation_display_flags_unconfirmed_numbers_so_a_reader_cannot_miss_it",
            },
            Evidence {
                file: "crates/salman-core/src/clause.rs",
                test: "no_clause_number_goes_deeper_than_the_three_levels_the_contents_publish",
            },
            Evidence {
                file: "crates/salman-core/src/clause.rs",
                test: "the_committed_citation_document_matches_what_the_registry_renders",
            },
        ],
        note: "43 citations are registered — 22 clauses, 18 tables and 3 figures of \
               IEC 61131-3:2013 (Edition 3.0) — each with a number cross-checked against a \
               public source and a requirement paraphrased in salman's own words. \
               docs/IEC_CITATIONS.md is generated from the registry and cannot drift from it. \
               A citation being registered does not mean the behaviour it names is implemented.",
    },
    Capability {
        id: "core.deterministic-rng",
        area: "Determinism",
        title: "Seeded xoshiro256++ generator, pinned and recorded in every trace header",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-core/src/rng.rs",
                test: "splitmix64_reproduces_the_published_vectors_from_state_zero",
            },
            Evidence {
                file: "crates/salman-core/src/rng.rs",
                test: "xoshiro256plusplus_reproduces_the_published_vectors_for_seed_100",
            },
            Evidence {
                file: "crates/salman-core/src/rng.rs",
                test: "next_below_never_reaches_its_bound",
            },
        ],
        note: "Written out in-crate rather than taken from rand, whose StdRng and SmallRng are \
               documented as non-portable. Not cryptographic: never use it for a key or a token.",
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
        id: "core.trace-fingerprint",
        area: "Determinism",
        title: "In-crate SHA-256 fingerprint of simulation traces, with NIST known-answer tests",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-core/src/hash.rs",
                test: "the_published_fips_180_4_vectors_hash_to_their_published_digests",
            },
            Evidence {
                file: "crates/salman-core/src/hash.rs",
                test: "splitting_the_input_into_chunks_never_changes_the_digest",
            },
        ],
        note: "A content fingerprint, not a security primitive: not constant-time, and not \
               to be used where an attacker picks the input and the comparison is secret. \
               Written in-crate so there is no runtime CPU-feature dispatch and no C toolchain.",
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
    Capability {
        id: "lang.st.dialects",
        area: "Language",
        title: "Dialects as configuration, with every diagnostic naming the rule it applied",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-lang/src/dialect.rs",
                test: "the_strict_dialect_differs_from_generic_on_the_unverified_points",
            },
            Evidence {
                file: "crates/salman-cli/tests/diagnostics.rs",
                test: "the_strict_dialect_names_the_rule_it_applied",
            },
        ],
        note: "Two profiles ship: generic and iec61131-3:2013-strict. No vendor profile exists, \
               and DialectId does not contain one.",
    },
    Capability {
        id: "lang.st.lexer-fuzzing",
        area: "Language",
        title: "libFuzzer targets for the Structured Text lexer, asserting its postconditions",
        status: Status::ImplementedUntested,
        milestone: "v0.1",
        evidence: &[],
        note: "Four targets in fuzz/fuzz_targets: valid UTF-8, raw bytes decoded the way the \
               loader will decode them, the strict dialect, and a differential run of both \
               dialects. Each asserts what must hold for any input — exactly one Eof, \
               non-decreasing spans inside the source, every literal and address index \
               resolving — rather than only that nothing panicked. All four build and run \
               under nightly, and .github/workflows/fuzz.yml runs each for 60 s daily. Not \
               ImplementedTested, for two reasons that both matter: a fuzzing run shows that \
               nothing was found, which is not the same as showing anything is right, and \
               this registry's evidence rule wants a named test function, which a libFuzzer \
               target is not. Only the lexer is covered.",
    },
    Capability {
        id: "lang.st.parser",
        area: "Language",
        title: "Recursive-descent Structured Text parser with error recovery and bounded nesting",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-lang/src/parser.rs",
                test: "unary_minus_binds_tighter_than_exponentiation_as_edition_3_orders_them",
            },
            Evidence {
                file: "crates/salman-lang/src/parser.rs",
                test: "a_file_with_ten_broken_statements_reports_about_ten_errors_not_one",
            },
            Evidence {
                file: "crates/salman-lang/src/parser.rs",
                test: "ten_thousand_nested_parentheses_produce_a_diagnostic_rather_than_a_stack_overflow",
            },
        ],
        note: "Every statement and declaration form of Structured Text, with the Edition 3 \
               operator precedence: unary binds tighter than `**`, so `-2 ** 2` is 4, and \
               salman warns where CODESYS and Beckhoff would give -4. Errors produce error \
               nodes and resynchronise rather than stopping the parse. Nesting, including \
               left-associative operator chains, is bounded by the dialect. Three things are \
               salman rules rather than verified requirements and say so in the diagnostic: \
               duplicate and overlapping CASE labels are refused, a FOR body may not assign \
               to its control variable, and the value of that variable after the loop is \
               unspecified. Inline structures and enumerations, VAR_CONFIG instance paths, \
               single-resource configurations, references and the object-oriented extensions \
               are parsed far enough to be named and are not implemented.",
    },
    Capability {
        id: "test.declarative-tests",
        area: "Testing",
        title: "Declarative unit tests for POUs, on a virtual clock, with no vendor runtime",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-cli/tests/conveyor_example.rs",
                test: "every_test_in_the_example_passes",
            },
            Evidence {
                file: "crates/salman-test/src/value.rs",
                test: "every_literal_form_the_language_accepts_works_in_a_test_file",
            },
            Evidence {
                file: "crates/salman-test/src/spec.rs",
                test: "an_unknown_key_is_rejected_rather_than_ignored",
            },
        ],
        note: "One source file per run. Multi-file projects are not implemented.",
    },
    Capability {
        id: "test.golden-traces",
        area: "Testing",
        title: "Golden-trace tests against a reviewable text file",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-cli/tests/conveyor_example.rs",
                test: "the_recorded_trace_matches_the_committed_golden_file",
            },
            Evidence {
                file: "crates/salman-cli/tests/conveyor_example.rs",
                test: "a_golden_trace_file_contains_no_carriage_returns",
            },
        ],
        note: "--update-golden rewrites them. Read the diff before committing it.",
    },
    Capability {
        id: "test.junit-report",
        area: "Testing",
        title: "JUnit XML report and a real exit code, for a build server",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-test/src/report.rs",
                test: "junit_output_reports_failures_and_errors_as_different_elements",
            },
            Evidence {
                file: "crates/salman-test/src/report.rs",
                test: "xml_escaping_survives_anything_an_engineer_might_type",
            },
        ],
        note: "Targets the Jenkins junit-10 schema, the strictest of the three in circulation.",
    },
    Capability {
        id: "vm.compiler",
        area: "Runtime",
        title: "Bytecode compiler with static instance layout and no run-time allocation",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-cli/tests/conveyor_example.rs",
                test: "the_compiled_program_is_byte_identical_across_two_compilations",
            },
            Evidence {
                file: "crates/salman-cli/tests/diagnostics.rs",
                test: "every_statement_form_compiles",
            },
            Evidence {
                file: "crates/salman-cli/tests/diagnostics.rs",
                test: "a_constant_subscript_outside_the_declared_bounds_is_rejected_at_compile_time",
            },
        ],
        note: "Exponentiation and AT %-located variables are reported as not implemented rather \
               than compiled to something approximate.",
    },
    Capability {
        id: "vm.interpreter",
        area: "Runtime",
        title: "Bytecode interpreter that faults rather than panics, with a scan watchdog",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-vm/src/exec.rs",
                test: "integer_overflow_wraps_because_that_is_what_a_controller_does",
            },
            Evidence {
                file: "crates/salman-vm/src/exec.rs",
                test: "the_most_negative_integer_divided_by_minus_one_does_not_abort",
            },
            Evidence {
                file: "crates/salman-vm/src/exec.rs",
                test: "the_watchdog_stops_a_routine_that_jumps_to_itself",
            },
        ],
        note: "Integer overflow wraps and division by zero faults; both are salman decisions, \
               documented in docs/CONFORMANCE.md.",
    },
    Capability {
        id: "vm.process-image",
        area: "Runtime",
        title: "Scan semantics with a correct process image, and a visible force list",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-vm/src/memory.rs",
                test: "an_input_read_mid_scan_sees_the_value_it_had_at_scan_start",
            },
            Evidence {
                file: "crates/salman-vm/src/memory.rs",
                test: "bit_byte_and_word_addresses_overlay_each_other_as_they_do_on_a_controller",
            },
            Evidence {
                file: "crates/salman-vm/src/memory.rs",
                test: "a_force_records_what_the_program_wanted_so_the_difference_is_visible",
            },
        ],
        note: "Nothing maps a located variable to the image yet; the image is reachable only \
               through a directly represented variable in an expression.",
    },
    Capability {
        id: "vm.retain-simulation",
        area: "Runtime",
        title: "RETAIN and PERSISTENT across simulated warm and cold restarts",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-vm/src/memory.rs",
                test: "a_warm_restart_keeps_retain_and_persistent_and_clears_the_rest",
            },
            Evidence {
                file: "crates/salman-vm/src/memory.rs",
                test: "a_cold_restart_keeps_only_persistent",
            },
        ],
        note: "The runtime models it; no command line surface exposes a restart yet.",
    },
    Capability {
        id: "vm.scan-scheduler",
        area: "Runtime",
        title: "Cyclic, event and freewheeling tasks with priority and overrun detection",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-vm/src/task.rs",
                test: "tasks_released_together_run_in_priority_order_lower_number_first",
            },
            Evidence {
                file: "crates/salman-vm/src/task.rs",
                test: "a_scan_that_outlasts_its_period_is_counted_as_an_overrun",
            },
            Evidence {
                file: "crates/salman-vm/src/task.rs",
                test: "an_event_task_runs_on_a_rising_edge_and_not_otherwise",
            },
        ],
        note: "Pre-emption is NOT modelled: a scan is atomic. A race that depends on being \
               interrupted mid-scan cannot be reproduced here.",
    },
    Capability {
        id: "vm.standard-function-blocks",
        area: "Runtime",
        title: "All ten IEC standard function blocks, with their awkward edge cases asserted",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-vm/src/stdfb.rs",
                test: "a_fresh_f_trig_emits_one_spurious_pulse_with_its_clock_low",
            },
            Evidence {
                file: "crates/salman-vm/src/stdfb.rs",
                test: "a_fresh_tof_with_its_input_low_does_not_start_an_off_delay",
            },
            Evidence {
                file: "crates/salman-vm/src/stdfb.rs",
                test: "ctu_keeps_counting_past_its_preset_and_saturates_at_the_type_limit",
            },
        ],
        note: "SEMA is also provided and is NOT an IEC standard function block; see \
               docs/CONFORMANCE.md.",
    },
    Capability {
        id: "vm.virtual-clock",
        area: "Runtime",
        title: "Virtual clock, so a ten-minute sequence tests in milliseconds, identically",
        status: Status::ImplementedTested,
        milestone: "v0.1",
        evidence: &[
            Evidence {
                file: "crates/salman-vm/src/clock.rs",
                test: "the_clock_never_runs_backwards",
            },
            Evidence {
                file: "crates/salman-vm/src/clock.rs",
                test: "the_wall_clock_comes_from_a_configured_epoch_not_from_the_host",
            },
        ],
        note: "A real-time mode exists in the type and reports its measured jitter; nothing \
               drives it yet, because there is no hardware to be in the loop with.",
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

    #[test]
    fn the_committed_status_document_matches_what_the_registry_renders() {
        // docs/STATUS.md is generated by `salman status --markdown`. Checking
        // it here rather than only in CI means the drift is caught by
        // `cargo test`, which is what a contributor runs before pushing.
        let path = repo_root().join("docs/STATUS.md");
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "cannot read {}: {err}. Regenerate it with `salman status --markdown > docs/STATUS.md`",
                path.display()
            )
        });
        assert_eq!(
            committed,
            render_markdown(),
            "docs/STATUS.md is out of date. Regenerate it with \
             `salman status --markdown > docs/STATUS.md`"
        );
    }
}
