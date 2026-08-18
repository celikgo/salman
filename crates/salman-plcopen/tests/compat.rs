// SPDX-License-Identifier: Apache-2.0
//! The compatibility matrix, and the rule that keeps it honest.
//!
//! `docs/PLCOPEN_COMPATIBILITY.md` is generated, and this file is what stops
//! it drifting: one test rewrites it and another fails if the committed copy
//! disagrees with what the code does now. The same arrangement the capability
//! registry uses, for the same reason — a hand-written statement of what works
//! is a statement of what worked once.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use salman_plcopen::compat::{Outcome, matrix, render_markdown};

fn committed() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the workspace root")
        .join("docs/PLCOPEN_COMPATIBILITY.md")
}

#[test]
fn the_committed_matrix_matches_what_the_code_does_now() {
    let rendered = render_markdown();
    let path = committed();

    if std::env::var_os("SALMAN_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &rendered).unwrap();
        return;
    }

    let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} does not exist. Run with SALMAN_UPDATE_GOLDEN=1 to write it, then read it \
             before committing",
            path.display()
        )
    });
    assert_eq!(
        rendered,
        on_disk,
        "{} is out of date. Regenerate it with SALMAN_UPDATE_GOLDEN=1 and read the diff",
        path.display()
    );
}

#[test]
fn every_row_was_actually_tried() {
    // The rule that makes the matrix worth reading. A row is not a claim about
    // what the code looks as though it should do; it is what happened when the
    // construct was written, read back and compared, moments ago.
    let rows = matrix();
    assert!(rows.len() >= 20, "the matrix covers almost nothing");

    // Running it twice must give the same answers, or the table records the
    // weather rather than the behaviour.
    let again = matrix();
    assert_eq!(rows.len(), again.len());
    for (first, second) in rows.iter().zip(&again) {
        assert_eq!(first.construct, second.construct);
        assert_eq!(
            first.outcome, second.outcome,
            "{} gave two answers",
            first.construct
        );
    }
}

#[test]
fn the_matrix_says_plainly_what_it_cannot_tell_you() {
    // Without this, a reader takes a page of green marks as evidence salman
    // agrees with the ecosystem, when it is evidence salman agrees with
    // itself. That is the difference between a compatibility matrix and a
    // marketing table.
    let rendered = render_markdown();
    for expected in [
        "salman against salman",
        "no such corpus exists",
        "does not validate against the schema",
        "No conformance is claimed",
    ] {
        assert!(
            rendered.contains(expected),
            "{expected:?} missing from the matrix"
        );
    }
}

#[test]
fn a_language_salman_does_not_read_is_not_marked_as_round_tripping() {
    // The rows most likely to be flattered. salman keeps the element and loses
    // its contents, and the matrix must not call that a clean round trip.
    let rows = matrix();
    for language in [
        "a Ladder body",
        "a Function Block Diagram body",
        "a Sequential Function Chart body",
        "an Instruction List body",
    ] {
        let row = rows
            .iter()
            .find(|r| r.construct == language)
            .unwrap_or_else(|| panic!("{language} is not in the matrix"));
        assert_eq!(
            row.outcome,
            Outcome::Changed,
            "{language} is marked as a clean round trip, and its contents are discarded"
        );
        assert!(
            row.note.contains("contents do not"),
            "{language}: the note must say what is lost, and says {:?}",
            row.note
        );
    }
}

#[test]
fn the_markers_are_shapes_rather_than_colours() {
    // The same rule the capability table follows: a red and green table some
    // readers cannot distinguish is a defect.
    assert_eq!(Outcome::RoundTrips.marker(), "`[x]`");
    assert_eq!(Outcome::Changed.marker(), "`[~]`");
    assert_eq!(Outcome::Failed.marker(), "`[ ]`");
}

#[test]
fn a_type_the_version_has_no_element_for_is_not_marked_as_a_clean_round_trip() {
    // The other row the model would flatter. `LTIME` comes back as `LTIME`
    // because salman wrote it as a named type reference and read the name
    // straight back — while the document now says a user-declared type where
    // an elementary one was meant. Another reader sees something different.
    let rows = matrix();
    let row = rows
        .iter()
        .find(|r| r.construct.starts_with("`LTIME`"))
        .expect("the row is in the matrix");
    assert_eq!(row.outcome, Outcome::Changed);
    assert!(
        row.note.contains("not the elementary one"),
        "the note must say what changed: {:?}",
        row.note
    );
}

#[test]
fn the_matrix_is_not_all_clean_marks() {
    // A page of unbroken green is what a compatibility matrix looks like when
    // nobody is checking it, and this format has known losses.
    let rows = matrix();
    let altered = rows
        .iter()
        .filter(|r| r.outcome == Outcome::Changed)
        .count();
    assert!(
        altered >= 5,
        "every row round-trips cleanly, which for this format is not credible"
    );
}
