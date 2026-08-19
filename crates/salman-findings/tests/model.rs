// SPDX-License-Identifier: Apache-2.0
//! The shape of a finding, and the two rules that are structural.
//!
//! Most of these tests exist to say, once, in a place a reader will find, that
//! the model's central rules are not conventions anybody has to remember. A
//! `Pass` carrying a severity of `Error` is not forbidden; it cannot be
//! written down, because [`Finding::pass`] takes no severity. A
//! `CannotDetermine` with no reason is not forbidden; the constructor requires
//! one, from a closed list with no free-text escape.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_findings::evidence::{Artifact, Evidence, Observed, TransactionRef};
use salman_findings::finding::{
    Confidence, Dedup, DedupScope, Finding, Group, Justification, Kind, NextCheck, Severity,
};

fn evidence() -> Evidence {
    Evidence::from(Artifact::file("plant.pcap", b"pretend this is a capture"))
        .at_frame(41)
        .at_offset(1234)
        .with_bytes(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x07, 0x01, 0x03, 0x02])
        .about(TransactionRef {
            transaction: 1,
            unit: 1,
            request_frame: Some(40),
            response_frame: Some(41),
        })
        .observing(Observed::new("length 5", "length 7"))
        .at_time(1_700_000_000_000_000_000)
}

// -- the structural rules ------------------------------------------------

#[test]
fn only_an_assertion_of_fault_carries_a_severity() {
    // Not a rule anybody has to remember: `pass`, `open`, `cannot_determine`,
    // `not_applicable` and `informational` take no severity argument, so there
    // is nothing to pass and nothing to get wrong.
    let failure = Finding::fail(
        "mbtcp.length.disagrees_with_pdu",
        "salman-modbus",
        Group::Framing,
        Severity::Error,
        Confidence::Certain,
        "the length field and the protocol data unit disagree",
        evidence(),
    );
    assert_eq!(failure.severity(), Some(Severity::Error));

    for finding in [
        Finding::pass("x.pass", "s", Group::Protocol, "fine", evidence()),
        Finding::open(
            "x.open",
            "s",
            Group::Sequence,
            Confidence::Possible,
            Justification::RequestNotCaptured,
            "cannot say",
            evidence(),
        ),
        Finding::cannot_determine(
            "x.cannot",
            "s",
            Group::Timing,
            Justification::NotObservableFromThisSource,
            "not visible here",
            evidence(),
        ),
        Finding::not_applicable(
            "x.na",
            "s",
            Group::Checksum,
            Justification::TransportDoesNotCarryField,
            "no CRC on TCP",
            evidence(),
        ),
        Finding::informational("x.info", "s", Group::Protocol, "worth knowing", evidence()),
    ] {
        assert_eq!(
            finding.severity(),
            None,
            "{} carries a severity and does not assert a fault",
            finding.id()
        );
        assert!(finding.is_well_formed());
    }
}

#[test]
fn anything_that_is_not_an_assertion_of_fault_says_why() {
    // The mechanism that stops salman reporting "no framing errors detected"
    // when the truth is "inter-frame timing cannot be measured from a TCP
    // capture". There is no free-text escape: the reason comes from a closed
    // list, and every entry names a limit somebody can act on.
    for finding in [
        Finding::open(
            "x.open",
            "s",
            Group::Sequence,
            Confidence::Possible,
            Justification::StreamStartedMidConnection,
            "m",
            evidence(),
        ),
        Finding::cannot_determine(
            "x.cannot",
            "s",
            Group::Timing,
            Justification::NotObservableFromThisSource,
            "m",
            evidence(),
        ),
        Finding::not_applicable(
            "x.na",
            "s",
            Group::Checksum,
            Justification::TransportDoesNotCarryField,
            "m",
            evidence(),
        ),
    ] {
        assert!(finding.justification().is_some(), "{}", finding.id());
        assert!(finding.is_well_formed());
    }
}

#[test]
fn every_justification_explains_itself_in_a_sentence() {
    // A reason a reader cannot act on is not a reason.
    for justification in [
        Justification::NotObservableFromThisSource,
        Justification::StreamTruncatedBySnaplen,
        Justification::BytesMissingFromCapture,
        Justification::StreamStartedMidConnection,
        Justification::FeatureNotImplemented,
        Justification::ProtocolAssumedFromPort,
        Justification::RequestNotCaptured,
        Justification::ResponseNotCaptured,
        Justification::TransportDoesNotCarryField,
        Justification::DeviceDidNotAdvertiseCapability,
    ] {
        let said = justification.explanation();
        assert!(said.len() > 20, "{justification:?}: {said:?}");
        assert_eq!(justification.to_string(), said);
    }
}

// -- the honesty channel -------------------------------------------------

#[test]
fn four_groups_are_salman_talking_about_itself() {
    // The most transferable idea in the model. A tool that cannot say "I did
    // not decode this" says nothing instead, and silence reads as "nothing was
    // there".
    for group in [
        Group::Undecoded,
        Group::Assumption,
        Group::Malformed,
        Group::DecoderBug,
    ] {
        assert!(group.is_about_salman(), "{group}");
    }
    for group in [
        Group::Checksum,
        Group::Framing,
        Group::Protocol,
        Group::Sequence,
        Group::Timing,
        Group::Addressing,
        Group::Value,
    ] {
        assert!(!group.is_about_salman(), "{group}");
    }
}

#[test]
fn a_finding_about_salman_says_so_when_it_is_rendered() {
    // A reader must not have to know which groups mean "the wire is wrong" and
    // which mean "salman is incomplete".
    let finding = Finding::cannot_determine(
        "modbus.function.not_implemented",
        "salman-modbus",
        Group::Undecoded,
        Justification::FeatureNotImplemented,
        "salman does not decode function code 0x18",
        evidence(),
    );
    let rendered = finding.to_string();
    assert!(
        rendered.contains("this is about salman, not the wire"),
        "{rendered}"
    );
}

// -- what a reader gets --------------------------------------------------

#[test]
fn a_rendered_finding_carries_everything_needed_to_check_it() {
    let finding = Finding::fail(
        "mbtcp.length.disagrees_with_pdu",
        "salman-modbus",
        Group::Framing,
        Severity::Error,
        Confidence::Certain,
        "the length field and the protocol data unit disagree",
        evidence(),
    )
    .suggesting(NextCheck::InspectFrame { frame: 41 });

    let rendered = finding.to_string();
    for expected in [
        "error[mbtcp.length.disagrees_with_pdu]",
        "plant.pcap",
        "frame 41",
        "byte 1234",
        "transaction 1 unit 1",
        "expected length 5, found length 7",
        "confidence: certain",
        "next: look at frame 41",
    ] {
        assert!(
            rendered.contains(expected),
            "{expected:?} missing:\n{rendered}"
        );
    }
}

#[test]
fn an_artifact_is_identified_by_its_contents_and_not_only_its_name() {
    // A finding about a capture that cannot say *which* capture is a finding
    // nobody can reproduce, and a path is not enough because files get edited.
    let first = Artifact::file("plant.pcap", b"one");
    let second = Artifact::file("plant.pcap", b"two");
    assert_ne!(first.sha256, second.sha256);
    assert_eq!(first.length, Some(3));
    assert_eq!(first.sha256_hex().unwrap().len(), 64);
    assert!(first.to_string().starts_with("plant.pcap ("));

    // A live connection has no contents to hash, and says so by having none.
    let live = Artifact::named("10.4.2.7:502");
    assert_eq!(live.sha256, None);
    assert_eq!(live.to_string(), "10.4.2.7:502");
}

#[test]
fn evidence_bytes_are_bounded() {
    // A finding is a pointer into evidence, not a copy of it. One carrying a
    // whole frame would make a report of a busy capture unreadable.
    let evidence = Evidence::from(Artifact::named("live")).with_bytes(&[0xAB; 4096]);
    assert_eq!(
        evidence.bytes.len(),
        salman_findings::evidence::MAX_EVIDENCE_BYTES
    );
}

#[test]
fn rendering_is_deterministic_and_has_no_colour() {
    // The same rule as salman's compiler diagnostics: a golden test compares
    // bytes, and meaning never depends on a colour a reader may not see.
    let finding = Finding::fail(
        "x.y",
        "s",
        Group::Value,
        Severity::Warning,
        Confidence::Probable,
        "m",
        evidence(),
    );
    assert_eq!(finding.to_string(), finding.to_string());
    assert!(
        !finding.to_string().contains('\u{1b}'),
        "an escape sequence reached the output"
    );
}

// -- deduplication -------------------------------------------------------

#[test]
fn deduplication_is_part_of_the_finding_rather_than_the_reporter() {
    // A device that misbehaves produces the same finding ten thousand times.
    // Retrofitting this after people have seen ten-thousand-line reports is
    // painful, so the finding carries its own identity.
    let finding = Finding::fail(
        "mbtcp.exception",
        "salman-modbus",
        Group::Protocol,
        Severity::Note,
        Confidence::Certain,
        "the device refused",
        evidence(),
    )
    .deduplicated(Dedup::per(
        "unit 1 registers 0..4",
        DedupScope::PerRegisterRange,
    ));

    assert_eq!(finding.dedup().scope, DedupScope::PerRegisterRange);
    assert_eq!(finding.dedup().key, "unit 1 registers 0..4");
}

#[test]
fn a_finding_defaults_to_being_reported_once() {
    let finding = Finding::pass("x.y", "s", Group::Protocol, "m", evidence());
    assert_eq!(finding.dedup().scope, DedupScope::Once);
}

// -- the kinds themselves ------------------------------------------------

#[test]
fn only_one_kind_asserts_a_fault() {
    assert!(Kind::Fail.asserts_a_fault());
    for kind in [
        Kind::Pass,
        Kind::NotApplicable,
        Kind::Open,
        Kind::CannotDetermine,
        Kind::Informational,
    ] {
        assert!(!kind.asserts_a_fault(), "{kind}");
    }
}

#[test]
fn a_pass_is_a_real_answer_and_not_the_absence_of_a_failure() {
    // The distinction that makes a report trustworthy: without it, "nothing
    // wrong here" and "salman did not look" render identically, which is to
    // say they render as nothing at all.
    let checked = Finding::pass(
        "mbtcp.length.agrees_with_pdu",
        "salman-modbus",
        Group::Framing,
        "every frame's length field agreed with its protocol data unit",
        evidence(),
    );
    assert_eq!(checked.kind(), Kind::Pass);
    assert_eq!(checked.confidence(), Confidence::Certain);
    assert!(checked.to_string().contains("pass["));
}
