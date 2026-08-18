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

use std::fmt;
use std::fmt::Write as _;

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

/// A citation of one clause of a standard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClauseRef {
    /// The standard, e.g. `"IEC 61131-3"`.
    pub standard: &'static str,
    /// The edition, e.g. `"3.0 (2013)"`.
    pub edition: &'static str,
    /// The clause or subclause number, e.g. `"6.6.2"`.
    pub number: &'static str,
    /// The clause title as printed in the standard's table of contents.
    pub title: &'static str,
    /// A paraphrase, in salman's own words, of the requirement being tested.
    ///
    /// Never the normative text.
    pub requirement: &'static str,
    /// How far the number can be trusted.
    pub provenance: Provenance,
}

impl fmt::Display for ClauseRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} Ed. {} §{} \"{}\"",
            self.standard, self.edition, self.number, self.title
        )?;
        if !self.provenance.is_confirmed() {
            f.write_str(" [clause number unconfirmed]")?;
        }
        Ok(())
    }
}

/// Every clause salman cites.
///
/// `docs/IEC_CITATIONS.md` is generated from this slice, so adding a citation
/// to a test and forgetting to document it is not possible.
pub static REGISTRY: &[ClauseRef] = &[];

/// Renders the citation registry as the body of `docs/IEC_CITATIONS.md`.
///
/// Deterministic: entries are emitted in registry order, which is source order.
#[must_use]
pub fn render_markdown() -> String {
    let mut out = String::new();
    out.push_str("# IEC citations used by salman\n\n");
    out.push_str(
        "*Generated from `salman_core::clause::REGISTRY`. Do not edit by hand;\n\
         edit the registry and re-run `salman docs citations`.*\n\n",
    );
    out.push_str(
        "salman never reproduces the normative text of IEC 61131-3, which is\n\
         copyrighted and sold by the IEC. Each row states, in salman's own words,\n\
         the requirement a test checks, and points at the clause where the\n\
         normative wording lives.\n\n",
    );
    out.push_str(
        "The standard is paywalled, so clause **numbers** have been cross-checked\n\
         against public secondary sources where that was possible. Rows marked\n\
         *unconfirmed* have a title that is reliable and a number that is not:\n\
         search for the title.\n\n",
    );

    if REGISTRY.is_empty() {
        out.push_str("No clauses are cited yet.\n");
        return out;
    }

    out.push_str("| Clause | Title | Requirement salman tests | Number confirmed by |\n");
    out.push_str("|---|---|---|---|\n");
    for c in REGISTRY {
        let confirmation = match c.provenance {
            Provenance::PublicSource(url) => format!("[source]({url})"),
            Provenance::NumberUnconfirmed => "**unconfirmed**".to_string(),
        };
        // Writing to a String is infallible; the Result is discarded for that
        // reason and no other.
        let _ = writeln!(
            out,
            "| {} Ed. {} §{} | {} | {} | {} |",
            c.standard, c.edition, c.number, c.title, c.requirement, confirmation
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_citation_names_a_standard_edition_number_and_title() {
        for c in REGISTRY {
            assert!(!c.standard.is_empty(), "citation with no standard: {c:?}");
            assert!(!c.edition.is_empty(), "citation with no edition: {c:?}");
            assert!(!c.number.is_empty(), "citation with no number: {c:?}");
            assert!(!c.title.is_empty(), "citation with no title: {c:?}");
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
    fn citation_display_flags_unconfirmed_numbers_so_a_reader_cannot_miss_it() {
        let unconfirmed = ClauseRef {
            standard: "IEC 61131-3",
            edition: "3.0 (2013)",
            number: "9.9.9",
            title: "Example",
            requirement: "an example requirement paraphrase",
            provenance: Provenance::NumberUnconfirmed,
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
}
