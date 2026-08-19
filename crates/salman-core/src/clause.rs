// SPDX-License-Identifier: Apache-2.0
//! Citing IEC 61131-3.
//!
//! salman's language and runtime tests are only worth anything if you can check
//! them against the standard. Every behavioural test therefore names the clause
//! it comes from, through a [`ClauseRef`] declared here rather than a string
//! typed into a test somewhere.
//!
//! # Two honesty rules, enforced by tests in this module
//!
//! 1. **salman never reproduces the normative text of IEC 61131-3.** The
//!    standard is copyrighted and sold by the IEC. Every [`ClauseRef`] carries
//!    a `requirement` field which is a *paraphrase written by salman's
//!    authors* saying what behaviour is being tested. To read the normative
//!    wording, buy the standard.
//!
//! 2. **A clause number salman could not check is labelled as such.** The
//!    standard is paywalled, so clause numbering has been cross-checked against
//!    public secondary sources where possible. Where it could not be, the
//!    citation carries [`Provenance::NumberUnconfirmed`] and the subclause
//!    *title* — which is stable and searchable — is the part you should trust.
//!    `docs/IEC_CITATIONS.md` is generated from this registry and lists which
//!    is which.
//!
//! A confidently wrong citation is worse than no citation, which is why the
//! type makes uncertainty a field rather than a footnote.
//!
//! # Where these numbers come from
//!
//! Nobody here read a pirated copy of the standard. Every number and title in
//! [`REGISTRY`] comes from one of these, all of which are published to be read
//! for free:
//!
//! * The publisher's own front-matter preview, which contains the cover, the
//!   full table of contents, the list of figures and the list of tables:
//!   <https://cdn.standards.iteh.ai/samples/16899/c7907e9a7e624f2185ff1f8d94e93f9f/IEC-61131-3-2013.pdf>.
//!   This is the source for every clause number, every figure title, and for
//!   any table the vendor statements below do not mention.
//! * Siemens' implementer compliance statement for STEP 7, which enumerates
//!   the third edition's feature tables by number and title:
//!   <https://cache.industry.siemens.com/dl/files/748/109476748/att_845621/v1/IEC_61131_compliance_en_US.pdf>.
//! * Rockwell Automation's Logix 5000 compliance manual, which states
//!   compliance with the third edition and is used here as an independent
//!   second vendor confirming that the feature-table scheme this registry
//!   assumes is the one implementers actually work from:
//!   <https://literature.rockwellautomation.com/idc/groups/literature/documents/pm/1756-pm018_-en-p.pdf>.
//!   Its text does not enumerate table titles, so no entry cites it as
//!   provenance; it corroborates the edition, not the numbering.
//! * The IEC's product page for Edition 3.0, which is where its withdrawal
//!   date comes from: <https://webstore.iec.ch/en/publication/4552>.
//!
//! Titles are quoted from those contents listings because a citation without
//! the printed title cannot be looked up. Nothing normative is reproduced: a
//! title tells you where to look, not what the standard requires.

use std::fmt;
use std::fmt::Write as _;

/// What the withdrawal of Edition 3.0 means for everything cited here.
///
/// Rendered at the top of `docs/IEC_CITATIONS.md`. It is a constant rather than
/// a paragraph inside the renderer because it is also the answer to "why does
/// salman cite a withdrawn edition?", and that answer should be quotable from
/// code, an issue reply or a release note without being retyped — retyped facts
/// drift.
pub const EDITION_NOTE: &str = "\
IEC 61131-3:2013 (Edition 3.0) was **withdrawn on 2025-05-22** and superseded by \
IEC 61131-3:2025 (Edition 4.0). salman targets Edition 3.0 because it is the edition its \
public sources allow it to verify, and citing an edition salman cannot check would be \
guessing in a footnote's clothing. Clause numbers are edition-specific — Structured Text is \
§7.3 in Edition 3.0 and §7.2 in Edition 4.0, and the standard function block tables shift \
from 43-46 to 44-47 — so a citation without an edition is meaningless. Every entry below \
carries one.";

/// The standard every entry in [`REGISTRY`] cites.
///
/// The year is part of the name on purpose: `IEC 61131-3` alone does not
/// identify a document, because the clause numbering moved between editions.
const STANDARD: &str = "IEC 61131-3:2013";

/// The edition every entry in [`REGISTRY`] cites. See [`EDITION_NOTE`].
const EDITION: &str = "3.0";

/// The publisher's free front-matter preview: cover, contents, list of tables
/// and list of figures. Every clause and figure number here was read off it.
const PREVIEW: Provenance = Provenance::PublicSource(
    "https://cdn.standards.iteh.ai/samples/16899/c7907e9a7e624f2185ff1f8d94e93f9f/IEC-61131-3-2013.pdf",
);

/// Siemens' implementer compliance statement, which lists the third edition's
/// feature tables by number and title.
const SIEMENS: Provenance = Provenance::PublicSource(
    "https://cache.industry.siemens.com/dl/files/748/109476748/att_845621/v1/IEC_61131_compliance_en_US.pdf",
);

/// Where a clause number came from, and therefore how much to trust it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// The clause number and title were cross-checked against the public
    /// source at this URL. The URL is checked by the `docs-links` CI job.
    PublicSource(&'static str),
    /// The behaviour is well attested across dialect documentation and open
    /// implementations, but the clause *number* could not be confirmed from a
    /// public source. Trust the title, not the number.
    NumberUnconfirmed,
}

impl Provenance {
    /// Whether the clause number was cross-checked.
    #[must_use]
    pub const fn is_confirmed(self) -> bool {
        matches!(self, Self::PublicSource(_))
    }

    /// The corroborating URL, if there is one.
    #[must_use]
    pub const fn url(self) -> Option<&'static str> {
        match self {
            Self::PublicSource(u) => Some(u),
            Self::NumberUnconfirmed => None,
        }
    }
}

/// Which numbering scheme a citation belongs to.
///
/// A standard has three of them running in parallel, and they are not
/// interchangeable: §6.4.2, Table 10 and Figure 12 are three different places.
/// Rendering all of them as `§10` — which is what a single `number` field
/// invites — sends a reader to a clause that has nothing to do with the claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CitationKind {
    /// A numbered clause or subclause of the body text.
    Clause,
    /// A numbered table. In IEC 61131-3 the feature tables are where most of
    /// the language surface actually lives, which is why they are cited at
    /// least as often as prose clauses.
    Table,
    /// A numbered figure.
    Figure,
}

impl CitationKind {
    /// What precedes the number when the citation is written out.
    ///
    /// The spacing is baked in rather than added at the call site because the
    /// section sign binds tight to its number (`§7.3.3`) while `Table` and
    /// `Figure` are words and take a space. One string per kind means no
    /// formatter has to remember that difference.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Clause => "§",
            Self::Table => "Table ",
            Self::Figure => "Figure ",
        }
    }
}

/// A test in this repository that checks the requirement a citation names.
///
/// Rule 9 in spirit: a citation is a claim that salman implements what the
/// clause requires, and a claim with nothing behind it is decoration. This is
/// the same shape `capability::Evidence` uses, and for the same reason — a
/// name that a test can check beats a sentence that says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CitedTest {
    /// Repository-relative path of the file holding the test.
    pub file: &'static str,
    /// The test function's name, without `fn` and without parentheses.
    pub test: &'static str,
}

/// A citation of one clause, table or figure of a standard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClauseRef {
    /// The standard including its year, e.g. `"IEC 61131-3:2013"`.
    ///
    /// The year is not decoration: without it the number below identifies
    /// nothing, because editions renumber.
    pub standard: &'static str,
    /// The edition, e.g. `"3.0"`.
    pub edition: &'static str,
    /// Which numbering scheme [`ClauseRef::number`] belongs to.
    pub kind: CitationKind,
    /// The clause, table or figure number, e.g. `"6.6.2"` or `"71"`, without
    /// any `§`, `Table` or `Figure` prefix — that comes from
    /// [`ClauseRef::kind`].
    pub number: &'static str,
    /// The title as printed in the standard's contents, list of tables or list
    /// of figures. It is what you search for when the number is wrong.
    pub title: &'static str,
    /// A paraphrase, in salman's own words, of the requirement being tested.
    ///
    /// Never the normative text.
    pub requirement: &'static str,
    /// How far the number can be trusted.
    pub provenance: Provenance,
    /// Tests that check the requirement this citation paraphrases.
    ///
    /// Never empty: a citation nothing tests is removed rather than kept as
    /// decoration. `every_citation_names_at_least_one_test` enforces it.
    pub tests: &'static [CitedTest],
}

impl fmt::Display for ClauseRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}{} \"{}\" (Ed {})",
            self.standard,
            self.kind.prefix(),
            self.number,
            self.title,
            self.edition
        )?;
        if !self.provenance.is_confirmed() {
            f.write_str(" [clause number unconfirmed]")?;
        }
        Ok(())
    }
}

/// Builds a citation of a clause of IEC 61131-3:2013.
///
/// A private constructor rather than twenty-two literal structs: the standard,
/// the edition and the kind are the same in every entry, and a field that is
/// copy-pasted twenty-two times is a field that will eventually be pasted
/// wrong.
const fn clause(
    number: &'static str,
    title: &'static str,
    requirement: &'static str,
    provenance: Provenance,
    tests: &'static [CitedTest],
) -> ClauseRef {
    ClauseRef {
        standard: STANDARD,
        edition: EDITION,
        kind: CitationKind::Clause,
        number,
        title,
        requirement,
        provenance,
        tests,
    }
}

/// Builds a citation of a table of IEC 61131-3:2013. See [`clause`].
const fn table(
    number: &'static str,
    title: &'static str,
    requirement: &'static str,
    provenance: Provenance,
    tests: &'static [CitedTest],
) -> ClauseRef {
    ClauseRef {
        standard: STANDARD,
        edition: EDITION,
        kind: CitationKind::Table,
        number,
        title,
        requirement,
        provenance,
        tests,
    }
}

/// Builds a citation of a figure of IEC 61131-3:2013. See [`clause`].
const fn figure(
    number: &'static str,
    title: &'static str,
    requirement: &'static str,
    provenance: Provenance,
    tests: &'static [CitedTest],
) -> ClauseRef {
    ClauseRef {
        standard: STANDARD,
        edition: EDITION,
        kind: CitationKind::Figure,
        number,
        title,
        requirement,
        provenance,
        tests,
    }
}

// ---------------------------------------------------------------------------
// Clauses
//
// Numbers and titles are from the publisher's contents listing (`PREVIEW`).
// The published contents stop at three levels, so salman's citations stop
// there too: a fourth level would be invented, and a test below refuses one.
// ---------------------------------------------------------------------------

/// How literal values are written in source text.
pub const LITERALS: ClauseRef = clause(
    "6.3",
    "Literals – External representation of data",
    "How a value is written literally in source text, for every literal form the languages accept",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "a_typed_literal_carries_the_type_its_prefix_named",
        },
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "literals_reach_the_tree_with_their_values",
        },
    ],
);

/// Integer, real and typed numeric literals, and the string literal forms.
pub const NUMERIC_AND_STRING_LITERALS: ClauseRef = clause(
    "6.3.2",
    "Numeric literals and string literals",
    "Decimal, based and typed numeric literals, underscores as digit separators, and how a \
     literal's type is decided",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "a_misplaced_underscore_is_reported",
        },
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "an_invalid_radix_is_reported_rather_than_guessed_at",
        },
    ],
);

/// Single- and double-quoted strings and their escape mechanism.
pub const CHARACTER_STRING_LITERALS: ClauseRef = clause(
    "6.3.3",
    "Character string literals",
    "Single and double quoted strings, and the dollar-sign escape that lets a quote, a newline \
     or a byte value appear inside one",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "an_unterminated_string_is_reported_and_does_not_swallow_the_next_line",
        },
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "a_multi_byte_character_becomes_whole_code_units_in_a_wstring",
        },
    ],
);

/// `TIME`/`LTIME` literals and how their components combine.
pub const DURATION_LITERAL: ClauseRef = clause(
    "6.3.4",
    "Duration literal",
    "The T# form, the day/hour/minute/second/millisecond unit sequence, and the rule that only \
     the first unit present may overflow its natural range",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "duration_literals_sum_their_units",
        },
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "the_first_unit_of_a_duration_may_overflow_but_a_later_one_may_not",
        },
    ],
);

/// Calendar-date and wall-clock literals.
pub const DATE_AND_TIME_OF_DAY_LITERAL: ClauseRef = clause(
    "6.3.5",
    "Date and time of day literal",
    "The D#, TOD# and DT# literal forms for a calendar date, a time of day and the two combined",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "date_time_and_date_and_time_literals_parse",
        },
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "a_date_that_does_not_exist_is_rejected",
        },
    ],
);

/// The type system's top-level clause.
pub const DATA_TYPES: ClauseRef = clause(
    "6.4",
    "Data types",
    "What types exist, how they are declared, and what a value of each may hold",
    PREVIEW,
    &[CitedTest {
        file: "crates/salman-lang/src/sema.rs",
        test: "a_declared_type_resolves_without_complaint",
    }],
);

/// `BOOL`, the integers, the reals, the time types, the string types.
pub const ELEMENTARY_DATA_TYPES: ClauseRef = clause(
    "6.4.2",
    "Elementary data types (BOOL, INT, REAL, STRING, etc.)",
    "The built-in types, their widths and ranges, and the initial value each takes when a \
     declaration does not give one",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/types.rs",
            test: "signed_integers_widen_through_the_whole_chain",
        },
        CitedTest {
            file: "crates/salman-lang/src/sema.rs",
            test: "a_typed_literal_whose_value_does_not_fit_is_refused",
        },
    ],
);

/// The `ANY_*` hierarchy used to type overloaded functions.
pub const GENERIC_DATA_TYPES: ClauseRef = clause(
    "6.4.3",
    "Generic data types",
    "The ANY hierarchy that groups elementary types, so an overloaded function can be specified \
     once for a whole family",
    PREVIEW,
    &[CitedTest {
        file: "crates/salman-lang/src/sema.rs",
        test: "an_operand_outside_the_operators_domain_names_the_generic_type_it_accepts",
    }],
);

/// Enumerations, subranges, arrays, structures and aliases.
pub const USER_DEFINED_DATA_TYPES: ClauseRef = clause(
    "6.4.4",
    "User-defined data types",
    "Declaring enumerations, subranges, arrays, structures and type aliases, and how each is \
     initialised",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "a_type_block_holds_aliases_structures_enumerations_subranges_and_arrays",
        },
        CitedTest {
            file: "crates/salman-lang/src/sema.rs",
            test: "enumeration_values_continue_from_the_previous_one_starting_at_zero",
        },
    ],
);

/// Declaration, scope and lifetime of variables.
pub const VARIABLES: ClauseRef = clause(
    "6.5",
    "Variables",
    "Declaring a variable, giving it an initial value, and what its scope and lifetime are",
    PREVIEW,
    &[CitedTest {
        file: "crates/salman-lang/src/parser.rs",
        test: "a_program_carries_its_variable_blocks_and_its_body",
    }],
);

/// `VAR`, `VAR_INPUT`, `VAR_OUTPUT`, `VAR_IN_OUT`, `VAR_GLOBAL` and friends.
pub const VARIABLE_SECTIONS: ClauseRef = clause(
    "6.5.2",
    "Variable sections",
    "The VAR keyword families that say whether a variable is local, an input, an output, \
     in-out, global or external, and what each permits",
    PREVIEW,
    &[CitedTest {
        file: "crates/salman-lang/src/parser.rs",
        test: "every_variable_section_keyword_opens_its_section",
    }],
);

/// `%IX0.0`-style addresses that name real I/O.
pub const DIRECTLY_REPRESENTED_VARIABLES: ClauseRef = clause(
    "6.5.5",
    "Directly represented variables (%)",
    "The percent-sign address form: its location letter, its size prefix, and its \
     hierarchical numeric part",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "a_hierarchical_direct_address_lexes_as_one_token",
        },
        CitedTest {
            file: "crates/salman-lang/src/address.rs",
            test: "an_address_renders_back_the_way_it_was_written",
        },
    ],
);

/// Which variables survive a power cycle, and which are cleared.
pub const RETENTIVE_VARIABLES: ClauseRef = clause(
    "6.5.6",
    "Retentive variables (RETAIN, NON_RETAIN)",
    "Which variables keep their value across a power cycle or a warm restart, and which are \
     put back to their initial value",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/ast.rs",
            test: "retention_defaults_to_unspecified_rather_than_to_retain",
        },
        CitedTest {
            file: "crates/salman-vm/src/memory.rs",
            test: "a_warm_restart_keeps_retain_and_persistent_and_clears_the_rest",
        },
    ],
);

/// Functions, function blocks, programs and what they have in common.
pub const PROGRAM_ORGANIZATION_UNITS: ClauseRef = clause(
    "6.6",
    "Program organization units (POUs)",
    "The three units of program structure — function, function block, program — and the \
     declaration features they share",
    PREVIEW,
    &[CitedTest {
        file: "crates/salman-lang/src/sema.rs",
        test: "two_pous_of_one_name_are_refused_and_the_first_one_wins",
    }],
);

/// Functions: no state between calls, one return value.
pub const FUNCTIONS: ClauseRef = clause(
    "6.6.2",
    "Functions",
    "A callable unit that keeps no state between calls and yields one result, so the same \
     arguments always produce the same value",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "a_function_declares_the_type_of_the_value_it_returns",
        },
        CitedTest {
            file: "crates/salman-lang/src/sema.rs",
            test: "a_functions_result_type_is_the_type_of_the_call",
        },
    ],
);

/// Function blocks: instances with state that persists between calls.
pub const FUNCTION_BLOCKS: ClauseRef = clause(
    "6.6.3",
    "Function blocks",
    "A callable unit whose instance keeps its internal state between calls, which is what makes \
     timers, counters and edge detectors expressible",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "a_function_block_has_no_return_type",
        },
        CitedTest {
            file: "crates/salman-lang/src/sema.rs",
            test: "a_user_function_block_is_called_through_its_instance",
        },
    ],
);

/// Programs: the unit a task is assigned to run.
pub const PROGRAMS: ClauseRef = clause(
    "6.6.4",
    "Programs",
    "The outermost program unit, which is what a configuration assigns to a task and therefore \
     what actually gets executed",
    PREVIEW,
    &[CitedTest {
        file: "crates/salman-lang/src/sema.rs",
        test: "a_program_cannot_be_called",
    }],
);

/// Configurations, resources and the binding of programs to tasks.
pub const CONFIGURATION_ELEMENTS: ClauseRef = clause(
    "6.8",
    "Configuration elements",
    "How a project is bound to hardware: configurations, the resources inside them, and the \
     global variables and tasks they declare",
    PREVIEW,
    &[CitedTest {
        file: "crates/salman-lang/src/parser.rs",
        test: "a_configuration_holds_globals_resources_tasks_and_program_instances",
    }],
);

/// Task period, priority and single-shot triggering.
pub const TASKS: ClauseRef = clause(
    "6.8.2",
    "Tasks",
    "Declaring a task by period or by a triggering variable, giving it a priority, and \
     associating programs with it",
    PREVIEW,
    &[CitedTest {
        file: "crates/salman-vm/src/task.rs",
        test: "an_event_task_runs_on_a_rising_edge_and_not_otherwise",
    }],
);

/// Structured Text: the language salman implements first.
pub const STRUCTURED_TEXT: ClauseRef = clause(
    "7.3",
    "Structured Text (ST)",
    "The textual, block-structured language: its expressions, its statements and its lexical \
     conventions",
    PREVIEW,
    &[CitedTest {
        file: "crates/salman-lang/src/lexer.rs",
        test: "a_small_program_lexes_into_the_expected_tokens",
    }],
);

/// ST expressions, operator precedence and evaluation order.
pub const ST_EXPRESSIONS: ClauseRef = clause(
    "7.3.2",
    "Expressions",
    "How an ST expression is built and evaluated, including operator precedence and the order \
     operands are taken in",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "or_binds_looser_than_and",
        },
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "every_binary_level_is_left_associative",
        },
    ],
);

/// ST statements: assignment, selection, iteration, calls, `EXIT`, `RETURN`.
pub const ST_STATEMENTS: ClauseRef = clause(
    "7.3.3",
    "Statements",
    "The ST statement forms: assignment, IF and CASE selection, the three loops, calls, EXIT \
     and RETURN",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "if_then_end_if_has_one_branch_and_no_else",
        },
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "while_tests_before_the_body_and_repeat_tests_after_it",
        },
    ],
);

// ---------------------------------------------------------------------------
// Tables
//
// The feature tables are where most of the language surface is actually
// specified. Numbers and titles are corroborated by Siemens' compliance
// statement (`SIEMENS`) except where noted, and by the publisher's list of
// tables (`PREVIEW`).
// ---------------------------------------------------------------------------

/// Comment syntax, including the nesting rule salman's lexer depends on.
pub const TABLE_COMMENTS: ClauseRef = table(
    "3",
    "Comments",
    "The comment forms: the line comment, the block comment, and the rows that make block \
     comments nest rather than end at the first close",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "block_comments_nest",
        },
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "line_comments_end_at_the_line_break",
        },
    ],
);

/// The numeric literal forms, base by base.
pub const TABLE_NUMERIC_LITERALS: ClauseRef = table(
    "5",
    "Numeric literals",
    "Every numeric literal form: signed decimals, reals with exponents, base 2, 8 and 16 \
     literals, and the typed literal prefix",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "an_invalid_radix_is_reported_rather_than_guessed_at",
        },
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "a_digit_outside_the_radix_is_reported",
        },
    ],
);

/// String literal forms for single-byte and wide characters.
pub const TABLE_CHARACTER_STRING_LITERALS: ClauseRef = table(
    "6",
    "Character string literals",
    "Single quoted and double quoted string literals, including the empty string and the forms \
     used for wide characters",
    SIEMENS,
    &[CitedTest {
        file: "crates/salman-lang/src/lexer.rs",
        test: "wstring_escapes_take_four_hex_digits",
    }],
);

/// The `$`-escape combinations legal inside a string.
pub const TABLE_TWO_CHARACTER_COMBINATIONS: ClauseRef = table(
    "7",
    "Two-character combinations in character strings",
    "The dollar-sign escapes that put a quote, a dollar sign, a newline, a tab or a page break \
     inside a string literal",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "string_escapes_follow_table_7",
        },
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "an_unknown_escape_is_reported",
        },
    ],
);

/// Duration literal forms and their unit letters.
pub const TABLE_DURATION_LITERALS: ClauseRef = table(
    "8",
    "Duration literals",
    "The written forms a duration may take: the keyword or hash prefix, the unit letters, and \
     whether the units may be abbreviated",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "duration_units_must_descend",
        },
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "long_duration_prefixes_are_marked_as_ltime",
        },
    ],
);

/// Date, time-of-day and combined date-and-time literal forms.
pub const TABLE_DATE_AND_TIME_OF_DAY_LITERALS: ClauseRef = table(
    "9",
    "Date and time of day literals",
    "The written forms of a calendar date, a time of day and a combined date and time, with \
     their long and short keywords",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "date_time_and_date_and_time_literals_parse",
        },
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "a_date_may_omit_leading_zeros",
        },
    ],
);

/// The elementary types with their widths and ranges.
pub const TABLE_ELEMENTARY_DATA_TYPES: ClauseRef = table(
    "10",
    "Elementary data types",
    "The keyword, the bit width and the range of values of each built-in type",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/types.rs",
            test: "nothing_narrows_implicitly",
        },
        CitedTest {
            file: "crates/salman-lang/src/types.rs",
            test: "bit_strings_widen_among_themselves",
        },
    ],
);

/// The `%` address grammar: location letter, size prefix, numeric part.
pub const TABLE_DIRECTLY_REPRESENTED_VARIABLES: ClauseRef = table(
    "16",
    "Directly represented variables",
    "The letters that make up a direct address: input, output or memory location, and the bit, \
     byte, word, double or long word size",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-lang/src/lexer.rs",
            test: "every_location_and_size_letter_is_accepted",
        },
        CitedTest {
            file: "crates/salman-lang/src/address.rs",
            test: "a_partly_specified_address_renders_as_a_star",
        },
    ],
);

/// `EN`/`ENO` and what they do to execution of a call.
pub const TABLE_EN_ENO: ClauseRef = table(
    "18",
    "Execution control graphically using EN and ENO",
    "The enable input and enable output that let a call be skipped, and what the outputs hold \
     when it is",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-cli/tests/constraints.rs",
            test: "a_call_with_enable_false_does_not_happen_at_all",
        },
        CitedTest {
            file: "crates/salman-cli/tests/constraints.rs",
            test: "enable_out_reports_whether_the_call_happened",
        },
    ],
);

/// Formal and non-formal function call syntax.
pub const TABLE_FUNCTION_CALL: ClauseRef = table(
    "20",
    "Function call",
    "Calling a function with a formal argument list naming each parameter, or with a positional \
     list, and where each is allowed",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-lang/src/sema.rs",
            test: "a_function_takes_positional_or_named_arguments",
        },
        CitedTest {
            file: "crates/salman-lang/src/sema.rs",
            test: "a_call_may_not_mix_positional_and_named_arguments",
        },
    ],
);

/// Calling a function block instance and reading its outputs.
pub const TABLE_FUNCTION_BLOCK_CALL: ClauseRef = table(
    "42",
    "Function block call",
    "Calling an instance of a function block, passing inputs by name, and reading outputs back \
     off the instance afterwards",
    SIEMENS,
    &[CitedTest {
        file: "crates/salman-lang/src/sema.rs",
        test: "positional_arguments_to_a_function_block_are_refused_citing_the_call_table",
    }],
);

/// `SR` and `RS`, and which input wins when both are true.
pub const TABLE_BISTABLE_FUNCTION_BLOCKS: ClauseRef = table(
    "43",
    "Standard bistable function blocks",
    "The set-dominant and reset-dominant latches, and which input decides the output when both \
     are true at once",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-vm/src/stdfb.rs",
            test: "sr_is_set_dominant_when_both_inputs_are_true",
        },
        CitedTest {
            file: "crates/salman-vm/src/stdfb.rs",
            test: "rs_is_reset_dominant_when_both_inputs_are_true",
        },
    ],
);

/// `R_TRIG` and `F_TRIG`.
pub const TABLE_EDGE_DETECTION_FUNCTION_BLOCKS: ClauseRef = table(
    "44",
    "Standard edge detection function blocks",
    "The rising and falling edge detectors, whose output is true for exactly the one execution \
     after the input changed",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-vm/src/stdfb.rs",
            test: "r_trig_pulses_for_exactly_one_invocation_on_a_rising_edge",
        },
        CitedTest {
            file: "crates/salman-vm/src/stdfb.rs",
            test: "f_trig_pulses_on_a_falling_edge",
        },
    ],
);

/// `CTU`, `CTD` and `CTUD`.
pub const TABLE_COUNTER_FUNCTION_BLOCKS: ClauseRef = table(
    "45",
    "Standard counter function blocks",
    "The up, down and up-down counters: when they count, when they load or reset, and when the \
     limit output goes true",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-vm/src/stdfb.rs",
            test: "ctu_counts_rising_edges_only_not_levels",
        },
        CitedTest {
            file: "crates/salman-vm/src/stdfb.rs",
            test: "ctud_precedence_is_reset_then_load_then_counting",
        },
    ],
);

/// `TP`, `TON` and `TOF`.
pub const TABLE_TIMER_FUNCTION_BLOCKS: ClauseRef = table(
    "46",
    "Standard timer function blocks",
    "The pulse, on-delay and off-delay timers, their elapsed-time output, and what happens when \
     the input changes mid-interval",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-vm/src/stdfb.rs",
            test: "ton_does_not_fire_early",
        },
        CitedTest {
            file: "crates/salman-vm/src/stdfb.rs",
            test: "ton_elapsed_time_is_clamped_at_the_preset",
        },
    ],
);

/// The checked downcast (`?=`) used with references and interfaces.
pub const TABLE_ASSIGNMENT_ATTEMPT: ClauseRef = table(
    "52",
    "Assignment attempt",
    "The checked assignment that succeeds only when the source really is of the target type, \
     and leaves the target alone when it is not",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "an_assignment_attempt_is_parsed_rather_than_refused",
        },
        CitedTest {
            file: "crates/salman-lang/src/sema.rs",
            test: "the_assignment_attempt_is_reported_as_not_implemented",
        },
    ],
);

/// ST operators and their precedence, in one table.
pub const TABLE_ST_OPERATORS: ClauseRef = table(
    "71",
    "Operators of the ST language",
    "Every ST operator with its precedence, from parentheses and function calls down to OR, \
     which is what fixes how an unparenthesised expression parses",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "multiplication_binds_looser_than_exponentiation",
        },
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "not_binds_tighter_than_equality",
        },
    ],
);

/// The ST statement forms, in one table.
pub const TABLE_ST_STATEMENTS: ClauseRef = table(
    "72",
    "ST language statements",
    "Every ST statement form with its syntax: assignment, the calls, IF, CASE, FOR, WHILE, \
     REPEAT, EXIT, CONTINUE and RETURN",
    SIEMENS,
    &[
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "case_labels_may_be_single_values_lists_or_ranges",
        },
        CitedTest {
            file: "crates/salman-lang/src/parser.rs",
            test: "for_keeps_its_control_variable_bounds_and_step",
        },
    ],
);

// ---------------------------------------------------------------------------
// Figures
//
// Numbers and titles are from the publisher's list of figures (`PREVIEW`).
// ---------------------------------------------------------------------------

/// The overview of which conversions are implicit and which must be written.
pub const FIGURE_CONVERSION_RULES: ClauseRef = figure(
    "11",
    "Data type conversion rules – implicit and/or explicit (Summary)",
    "The summary of which type conversions happen on their own and which the programmer has to \
     write out as a call",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/types.rs",
            test: "nothing_narrows_implicitly",
        },
        CitedTest {
            file: "crates/salman-lang/src/sema.rs",
            test: "a_narrowing_assignment_names_the_conversion_function",
        },
    ],
);

/// Exactly which implicit conversions exist, as a graph.
pub const FIGURE_IMPLICIT_CONVERSIONS: ClauseRef = figure(
    "12",
    "Supported implicit type conversions",
    "The graph of conversions a conforming implementation performs without being asked, which \
     is the set salman's type checker must not widen",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-lang/src/types.rs",
            test: "int_widens_to_real_but_dint_does_not",
        },
        CitedTest {
            file: "crates/salman-lang/src/types.rs",
            test: "no_real_converts_implicitly_to_an_integer",
        },
    ],
);

/// Timing diagrams for `TP`, `TON` and `TOF`.
pub const FIGURE_TIMER_TIMING_DIAGRAMS: ClauseRef = figure(
    "15",
    "Standard timer function blocks – timing diagrams (Rules)",
    "The timing diagrams that fix, cycle by cycle, what the standard timers do when their input \
     changes during a running interval",
    PREVIEW,
    &[
        CitedTest {
            file: "crates/salman-vm/src/stdfb.rs",
            test: "ton_does_not_accumulate_elapsed_time_across_separate_input_pulses",
        },
        CitedTest {
            file: "crates/salman-vm/src/stdfb.rs",
            test: "ton_falls_immediately_when_its_input_does",
        },
    ],
);

/// Every clause, table and figure salman cites.
///
/// `docs/IEC_CITATIONS.md` is generated from this slice, so adding a citation
/// to a test and forgetting to document it is not possible.
///
/// Order is clauses by number, then tables by number, then figures by number.
/// It is source order because [`render_markdown`] emits it verbatim, and a
/// generated document whose row order depends on anything else is a document
/// that produces spurious diffs.
pub static REGISTRY: &[ClauseRef] = &[
    LITERALS,
    NUMERIC_AND_STRING_LITERALS,
    CHARACTER_STRING_LITERALS,
    DURATION_LITERAL,
    DATE_AND_TIME_OF_DAY_LITERAL,
    DATA_TYPES,
    ELEMENTARY_DATA_TYPES,
    GENERIC_DATA_TYPES,
    USER_DEFINED_DATA_TYPES,
    VARIABLES,
    VARIABLE_SECTIONS,
    DIRECTLY_REPRESENTED_VARIABLES,
    RETENTIVE_VARIABLES,
    PROGRAM_ORGANIZATION_UNITS,
    FUNCTIONS,
    FUNCTION_BLOCKS,
    PROGRAMS,
    CONFIGURATION_ELEMENTS,
    TASKS,
    STRUCTURED_TEXT,
    ST_EXPRESSIONS,
    ST_STATEMENTS,
    TABLE_COMMENTS,
    TABLE_NUMERIC_LITERALS,
    TABLE_CHARACTER_STRING_LITERALS,
    TABLE_TWO_CHARACTER_COMBINATIONS,
    TABLE_DURATION_LITERALS,
    TABLE_DATE_AND_TIME_OF_DAY_LITERALS,
    TABLE_ELEMENTARY_DATA_TYPES,
    TABLE_DIRECTLY_REPRESENTED_VARIABLES,
    TABLE_EN_ENO,
    TABLE_FUNCTION_CALL,
    TABLE_FUNCTION_BLOCK_CALL,
    TABLE_BISTABLE_FUNCTION_BLOCKS,
    TABLE_EDGE_DETECTION_FUNCTION_BLOCKS,
    TABLE_COUNTER_FUNCTION_BLOCKS,
    TABLE_TIMER_FUNCTION_BLOCKS,
    TABLE_ASSIGNMENT_ATTEMPT,
    TABLE_ST_OPERATORS,
    TABLE_ST_STATEMENTS,
    FIGURE_CONVERSION_RULES,
    FIGURE_IMPLICIT_CONVERSIONS,
    FIGURE_TIMER_TIMING_DIAGRAMS,
];

/// Renders the citation registry as the body of `docs/IEC_CITATIONS.md`.
///
/// Deterministic: entries are emitted in registry order, which is source order.
#[must_use]
pub fn render_markdown() -> String {
    let mut out = String::new();
    out.push_str("# IEC citations used by salman\n\n");
    out.push_str(
        "*Generated from `salman_core::clause::REGISTRY`. Do not edit by hand; edit the\n\
         registry. A test in `crates/salman-core/src/clause.rs` fails if this file and the\n\
         registry disagree.*\n\n",
    );
    out.push_str("## The edition, and why a withdrawn one\n\n");
    out.push_str(EDITION_NOTE);
    out.push_str("\n\n## What these rows are, and are not\n\n");
    out.push_str(
        "salman never reproduces the normative text of IEC 61131-3, which is\n\
         copyrighted and sold by the IEC. Each row states, in salman's own words,\n\
         the requirement a test checks, and points at the clause, table or figure\n\
         where the normative wording lives.\n\n",
    );
    out.push_str(
        "The last column names the tests. A citation with no test is not allowed\n\
         into the registry: `every_citation_names_at_least_one_test` fails, and\n\
         `every_cited_test_exists_in_the_source_tree` fails again if a named test\n\
         is renamed or deleted. So a row here cannot be decoration, and the way to\n\
         retire a clause salman no longer checks is to delete the row, not to let\n\
         it stand with nothing behind it.\n\n",
    );
    out.push_str(
        "The standard is paywalled, so **numbers** have been cross-checked against\n\
         public secondary sources where that was possible: the publisher's own\n\
         front-matter preview, which contains the contents and the lists of tables\n\
         and figures, and vendor implementer compliance statements, which enumerate\n\
         the feature tables. Rows marked *unconfirmed* have a title that is reliable\n\
         and a number that is not: search for the title.\n\n",
    );

    if REGISTRY.is_empty() {
        out.push_str("No clauses are cited yet.\n");
        return out;
    }

    out.push_str("## Citations\n\n");
    out.push_str(
        "| Citation | Title | Requirement salman tests | Number confirmed by | Tested by |\n",
    );
    out.push_str("|---|---|---|---|---|\n");
    for c in REGISTRY {
        let confirmation = match c.provenance {
            Provenance::PublicSource(url) => format!("[source]({url})"),
            Provenance::NumberUnconfirmed => "**unconfirmed**".to_string(),
        };
        // Writing to a String is infallible; the Result is discarded for that
        // reason and no other.
        let cited = c
            .tests
            .iter()
            .map(|t| format!("`{}`", t.test))
            .collect::<Vec<_>>()
            .join("<br>");
        let _ = writeln!(
            out,
            "| {} {}{} (Ed {}) | {} | {} | {} | {} |",
            c.standard,
            c.kind.prefix(),
            c.number,
            c.edition,
            c.title,
            c.requirement,
            confirmation,
            cited
        );
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
    fn every_citation_names_a_standard_edition_number_and_title() {
        for c in REGISTRY {
            assert!(!c.standard.is_empty(), "citation with no standard: {c:?}");
            assert!(!c.edition.is_empty(), "citation with no edition: {c:?}");
            assert!(!c.number.is_empty(), "citation with no number: {c:?}");
            assert!(!c.title.is_empty(), "citation with no title: {c:?}");
            assert!(
                !c.requirement.is_empty(),
                "citation with no requirement: {c:?}"
            );
        }
    }

    #[test]
    fn every_citation_carries_the_year_so_a_number_identifies_one_document() {
        // A bare "IEC 61131-3 §7.3" is ambiguous: Structured Text is §7.3 in
        // Edition 3.0 and §7.2 in Edition 4.0.
        for c in REGISTRY {
            assert!(
                c.standard.contains(":2013"),
                "citation {} names {:?}, which has no year",
                c.number,
                c.standard
            );
            assert_eq!(
                c.edition, "3.0",
                "citation {} cites another edition",
                c.number
            );
        }
    }

    #[test]
    fn every_citation_paraphrases_the_requirement_it_tests() {
        for c in REGISTRY {
            assert!(
                c.requirement.len() > 15,
                "citation {} has no usable requirement paraphrase",
                c.number
            );
        }
    }

    #[test]
    fn every_citation_names_at_least_one_test() {
        // A citation is a claim that salman implements what the clause
        // requires. Before this test existed, twenty of the forty-three
        // entries here were named nowhere outside this file: the paraphrase
        // said "the requirement salman tests" and nothing checked that
        // anything did. A number with no test behind it is decoration, and in
        // this domain decoration reads as conformance.
        for c in REGISTRY {
            assert!(
                !c.tests.is_empty(),
                "citation {}{} names no test; write one or delete the citation",
                c.kind.prefix(),
                c.number
            );
        }
    }

    #[test]
    fn every_cited_test_exists_in_the_source_tree() {
        // Same mechanism as `capability::every_cited_test_exists_in_the_source_tree`:
        // renaming a test must break the citation that rests on it, rather
        // than leaving the citation pointing at nothing.
        let root = repo_root();
        for c in REGISTRY {
            for t in c.tests {
                let path = root.join(t.file);
                let source = std::fs::read_to_string(&path).unwrap_or_else(|err| {
                    panic!(
                        "citation {}{} cites {}, which cannot be read: {err}",
                        c.kind.prefix(),
                        c.number,
                        t.file
                    )
                });
                assert!(
                    source.contains(&format!("fn {}(", t.test)),
                    "citation {}{} cites test `{}` in {}, which is not there",
                    c.kind.prefix(),
                    c.number,
                    t.test,
                    t.file
                );
            }
        }
    }

    #[test]
    fn confirmed_citations_carry_a_resolvable_url() {
        for c in REGISTRY {
            if let Provenance::PublicSource(url) = c.provenance {
                assert!(
                    url.starts_with("https://"),
                    "citation {} cites {url}, which is not an https URL",
                    c.number
                );
            }
        }
    }

    #[test]
    fn no_clause_number_goes_deeper_than_the_three_levels_the_contents_publish() {
        // The publisher's free contents listing stops at three levels. A fourth
        // level in this registry could not have been read anywhere: it would
        // have been invented, which is the exact failure this file exists to
        // make impossible.
        for c in REGISTRY {
            let levels = c.number.split('.').count();
            assert!(
                levels <= 3,
                "citation {} has {levels} numbering levels; the published contents stop at three",
                c.number
            );
        }
    }

    #[test]
    fn a_kind_and_a_number_together_identify_exactly_one_entry() {
        // §6.4 and Table 6.4 would be different places, so the number alone is
        // not an identity. Two entries sharing a kind and a number, though, are
        // a copy-paste that got edited only halfway.
        let mut seen: Vec<(CitationKind, &str)> =
            REGISTRY.iter().map(|c| (c.kind, c.number)).collect();
        let count = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), count, "two citations share a kind and a number");
    }

    #[test]
    fn tables_and_figures_are_not_rendered_as_clauses() {
        assert_eq!(
            TABLE_ST_OPERATORS.to_string(),
            "IEC 61131-3:2013 Table 71 \"Operators of the ST language\" (Ed 3.0)"
        );
        assert_eq!(
            FIGURE_IMPLICIT_CONVERSIONS.to_string(),
            "IEC 61131-3:2013 Figure 12 \"Supported implicit type conversions\" (Ed 3.0)"
        );
        assert_eq!(
            ST_STATEMENTS.to_string(),
            "IEC 61131-3:2013 §7.3.3 \"Statements\" (Ed 3.0)"
        );
    }

    #[test]
    fn citation_display_flags_unconfirmed_numbers_so_a_reader_cannot_miss_it() {
        let unconfirmed = ClauseRef {
            standard: "IEC 61131-3:2013",
            edition: "3.0",
            kind: CitationKind::Clause,
            number: "9.9.9",
            title: "Example",
            requirement: "an example requirement paraphrase",
            provenance: Provenance::NumberUnconfirmed,
            tests: &[CitedTest {
                file: "crates/salman-core/src/clause.rs",
                test: "citation_display_flags_unconfirmed_numbers_so_a_reader_cannot_miss_it",
            }],
        };
        assert!(
            unconfirmed
                .to_string()
                .contains("[clause number unconfirmed]")
        );

        let confirmed = ClauseRef {
            provenance: Provenance::PublicSource("https://example.invalid/spec"),
            ..unconfirmed
        };
        assert!(!confirmed.to_string().contains("unconfirmed"));
    }

    #[test]
    fn rendered_markdown_is_deterministic() {
        assert_eq!(render_markdown(), render_markdown());
    }

    #[test]
    fn rendered_markdown_says_the_cited_edition_is_withdrawn() {
        let md = render_markdown();
        assert!(md.contains("withdrawn on 2025-05-22"), "{md}");
        assert!(md.contains("IEC 61131-3:2025"), "{md}");
        assert!(md.contains("Edition 4.0"), "{md}");
    }

    #[test]
    fn the_committed_citation_document_matches_what_the_registry_renders() {
        // Rule 3: a generated document that can drift from its generator is a
        // document that will. The fix when this fails is to regenerate
        // docs/IEC_CITATIONS.md, never to edit it.
        let path = repo_root().join("docs/IEC_CITATIONS.md");
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("docs/IEC_CITATIONS.md cannot be read: {err}"));
        assert_eq!(
            committed,
            render_markdown(),
            "docs/IEC_CITATIONS.md has drifted from salman_core::clause::REGISTRY"
        );
    }
}
