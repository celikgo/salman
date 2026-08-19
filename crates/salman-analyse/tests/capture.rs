// SPDX-License-Identifier: Apache-2.0
//! What salman says about a Modbus capture.
//!
//! Each test builds a capture that contains exactly one interesting thing and
//! checks that salman says exactly that about it — including, in several
//! cases, that salman says it *cannot tell*. Those are the tests worth having:
//! a tool that reports only faults cannot distinguish "nothing is wrong here"
//! from "I did not look", and both render as silence.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_analyse::modbus::{Options, analyse_capture};
use salman_capture::pcap::{LinkType, Writer};
use salman_findings::finding::{Finding, Group, Justification, Kind, Severity};
use salman_modbus::function::ExceptionCode;
use salman_modbus::pdu::{Request, Response, Words};
use salman_modbus::tcp::TcpAdu;

const CLIENT: [u8; 4] = [192, 168, 1, 10];
const SERVER: [u8; 4] = [192, 168, 1, 20];

/// Wraps a TCP payload in TCP, IPv4 and Ethernet headers.
fn frame(from: [u8; 4], to: [u8; 4], sport: u16, dport: u16, seq: u32, payload: &[u8]) -> Vec<u8> {
    let mut tcp = Vec::new();
    tcp.extend_from_slice(&sport.to_be_bytes());
    tcp.extend_from_slice(&dport.to_be_bytes());
    tcp.extend_from_slice(&seq.to_be_bytes());
    tcp.extend_from_slice(&0_u32.to_be_bytes());
    tcp.push(0x50);
    tcp.push(0x18);
    tcp.extend_from_slice(&[0xFF, 0xFF, 0, 0, 0, 0]);
    tcp.extend_from_slice(payload);

    let total = 20 + tcp.len();
    let mut ip = vec![0x45, 0x00];
    ip.extend_from_slice(&(total as u16).to_be_bytes());
    ip.extend_from_slice(&[0, 0, 0, 0, 64, 6, 0, 0]);
    ip.extend_from_slice(&from);
    ip.extend_from_slice(&to);
    ip.extend_from_slice(&tcp);

    let mut ethernet = vec![0x11; 6];
    ethernet.extend_from_slice(&[0x22; 6]);
    ethernet.extend_from_slice(&0x0800_u16.to_be_bytes());
    ethernet.extend_from_slice(&ip);
    ethernet
}

/// Builds a capture from a list of (outgoing?, sequence, payload).
fn capture(frames: &[(bool, u32, Vec<u8>)]) -> Vec<u8> {
    let mut writer = Writer::new(LinkType::ETHERNET, 262_144);
    let mut time = 1_700_000_000_000_000_000_u64;
    for (outgoing, seq, payload) in frames {
        let bytes = if *outgoing {
            frame(CLIENT, SERVER, 51_000, 502, *seq, payload)
        } else {
            frame(SERVER, CLIENT, 502, 51_000, *seq, payload)
        };
        writer.write(time, bytes.len() as u32, &bytes);
        time += 5_000_000;
    }
    writer.finish()
}

fn request(transaction: u16, request: &Request) -> Vec<u8> {
    TcpAdu::new(transaction, 1, request.encode().unwrap()).to_vec()
}

fn response(transaction: u16, response: &Response) -> Vec<u8> {
    TcpAdu::new(transaction, 1, response.encode()).to_vec()
}

fn read() -> Request {
    Request::ReadHoldingRegisters {
        start: 0,
        quantity: 2,
    }
}

fn find<'a>(findings: &'a [Finding], id: &str) -> Option<&'a Finding> {
    findings.iter().find(|f| f.id() == id)
}

// -- the ordinary case ---------------------------------------------------

#[test]
fn a_healthy_exchange_produces_a_pass_and_nothing_else() {
    // Proof of coverage. A report of only failures cannot tell "nothing is
    // wrong here" from "salman did not look", so the pass has to exist.
    let answer = Response::ReadHoldingRegisters(Words::new(&[0x1234, 0x5678]).unwrap());
    let bytes = capture(&[
        (true, 1, request(1, &read())),
        (false, 1, response(1, &answer)),
    ]);

    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();
    assert_eq!(analysis.frames, 2);
    assert_eq!(analysis.adus, 2);
    assert_eq!(analysis.paired, 1);

    let pass = find(&analysis.findings, "mbtcp.transactions.paired")
        .expect("a healthy exchange must say it was checked");
    assert_eq!(pass.kind(), Kind::Pass);
    assert!(pass.severity().is_none());

    // Joining mid-stream is expected of any capture that does not start at the
    // SYN, so it is reported once and is not a fault.
    let others: Vec<&str> = analysis
        .findings
        .iter()
        .filter(|f| f.kind() == Kind::Fail)
        .map(salman_findings::finding::Finding::id)
        .collect();
    assert!(others.is_empty(), "unexpected faults: {others:?}");
}

#[test]
fn a_capture_that_starts_mid_conversation_says_so_once() {
    let answer = Response::ReadHoldingRegisters(Words::new(&[1, 2]).unwrap());
    let bytes = capture(&[
        (true, 1, request(1, &read())),
        (false, 1, response(1, &answer)),
        (true, 100, request(2, &read())),
        (false, 100, response(2, &answer)),
    ]);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();
    let reported = analysis
        .findings
        .iter()
        .filter(|f| f.id() == "tcp.stream.started_mid_connection")
        .count();
    assert_eq!(
        reported, 1,
        "it is a fact about the capture, not about each stream"
    );
    let finding = find(&analysis.findings, "tcp.stream.started_mid_connection").unwrap();
    assert_eq!(finding.kind(), Kind::CannotDetermine);
    assert_eq!(
        finding.justification(),
        Some(Justification::StreamStartedMidConnection)
    );
}

// -- things salman cannot tell -------------------------------------------

#[test]
fn a_request_nobody_answered_says_it_cannot_tell_which_happened() {
    // A device that never answered and a capture that ended first look
    // identical from here. Saying which would be a guess presented as a fact.
    let bytes = capture(&[(true, 1, request(7, &read()))]);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();

    let finding =
        find(&analysis.findings, "mbtcp.request.unanswered").expect("an unanswered request");
    assert_eq!(finding.kind(), Kind::CannotDetermine);
    assert_eq!(
        finding.justification(),
        Some(Justification::ResponseNotCaptured)
    );
    assert!(
        finding.message().contains("cannot be told from here"),
        "{}",
        finding.message()
    );
    assert!(
        finding.next_check().is_some(),
        "it must say what to do next"
    );
    let reference = finding.evidence().transaction.unwrap();
    assert_eq!(reference.transaction, 7);
    assert_eq!(reference.response_frame, None);
}

#[test]
fn a_response_whose_request_was_not_captured_says_what_that_costs() {
    // Not merely "unmatched": a read response carries a byte count and never
    // the quantity, so without its request nobody can decode it — not salman,
    // not anyone. The finding says so, because a reader who does not know that
    // will think salman is being lazy.
    let answer = Response::ReadHoldingRegisters(Words::new(&[9]).unwrap());
    let bytes = capture(&[(false, 1, response(3, &answer))]);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();

    let finding =
        find(&analysis.findings, "mbtcp.response.unmatched").expect("an unmatched response");
    assert_eq!(finding.kind(), Kind::CannotDetermine);
    assert_eq!(
        finding.justification(),
        Some(Justification::RequestNotCaptured)
    );
    assert!(
        finding.message().contains("never the quantity"),
        "{}",
        finding.message()
    );
}

#[test]
fn a_capture_with_no_modbus_on_the_port_suggests_another_port() {
    // The most useful finding Wireshark's Modbus dissector has: the reader
    // almost always has the right capture and the wrong port.
    let mut writer = Writer::new(LinkType::ETHERNET, 262_144);
    let http = frame(CLIENT, SERVER, 51_000, 80, 1, b"GET / HTTP/1.1\r\n\r\n");
    writer.write(0, http.len() as u32, &http);
    let bytes = writer.finish();

    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();
    assert_eq!(analysis.adus, 0);
    assert_eq!(analysis.other_streams, 1);

    let finding = find(&analysis.findings, "mbtcp.no_traffic_on_port").expect("a hint");
    assert_eq!(finding.kind(), Kind::CannotDetermine);
    assert_eq!(finding.group(), Group::Assumption);
    assert!(
        finding.next_check().is_some(),
        "it must say what to try instead"
    );
}

// -- faults --------------------------------------------------------------

#[test]
fn a_device_that_refuses_produces_a_finding_with_what_was_asked_for() {
    // The exception alone is not useful. What makes it actionable is which
    // registers were asked for, which is why the request has to be paired
    // before the finding can be made.
    let refusal = Response::Exception {
        function: salman_modbus::function::FunctionCode::READ_HOLDING_REGISTERS,
        code: ExceptionCode::ILLEGAL_DATA_ADDRESS,
    };
    let bytes = capture(&[
        (true, 1, request(1, &read())),
        (false, 1, response(1, &refusal)),
    ]);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();

    let finding = find(&analysis.findings, "mbtcp.exception").expect("the refusal");
    assert_eq!(finding.kind(), Kind::Fail);
    assert_eq!(finding.severity(), Some(Severity::Warning));
    let observed = finding.evidence().observed.as_ref().unwrap();
    assert_eq!(observed.expected, "2 items from address 0");
    assert!(
        observed.actual.contains("Illegal Data Address"),
        "{observed}"
    );
    assert!(
        finding.message().contains("unit 1"),
        "{}",
        finding.message()
    );
    assert_eq!(analysis.paired, 1, "a refusal is still an answer");
}

#[test]
fn a_length_field_that_disagrees_with_its_own_frame_is_an_error() {
    let mut adu = request(1, &read());
    // Claim one byte fewer than is there. The framer still delivers a frame —
    // it believes the length — and the unit behind it is one byte short.
    adu[5] -= 1;
    adu.truncate(adu.len() - 1);
    let bytes = capture(&[(true, 1, adu)]);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();

    // The frame is short, so it does not decode as a request either; what
    // matters is that salman reports the shortened unit rather than accepting
    // it silently.
    assert!(
        analysis.findings.iter().any(|f| f.kind() == Kind::Fail),
        "a truncated request produced no fault: {:?}",
        analysis
            .findings
            .iter()
            .map(salman_findings::finding::Finding::id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_request_sent_again_before_anything_answered_it_is_named_as_a_retry() {
    // pymodbus reuses one transaction identifier across every retry. Reported
    // as ten lost requests it is alarming and wrong; reported as a retry it is
    // the truth.
    let one = request(5, &read());
    let bytes = capture(&[
        (true, 1, one.clone()),
        (true, 1 + one.len() as u32, one.clone()),
        (true, 1 + 2 * one.len() as u32, one),
    ]);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();

    let finding = find(&analysis.findings, "mbtcp.request.repeated").expect("a retry");
    assert_eq!(finding.kind(), Kind::Open);
    assert!(finding.message().contains("retry"), "{}", finding.message());
    // Reported once for the transaction, not once per repeat.
    assert_eq!(
        analysis
            .findings
            .iter()
            .filter(|f| f.id() == "mbtcp.request.repeated")
            .count(),
        1
    );
}

// -- what every finding must carry ---------------------------------------

#[test]
fn every_finding_is_well_formed_and_names_its_evidence() {
    // Over every capture in this file: the structural rules hold, every
    // finding says which artefact it is about, and every one identifies that
    // artefact by contents rather than only by name.
    let answer = Response::ReadHoldingRegisters(Words::new(&[1, 2]).unwrap());
    let captures = [
        capture(&[(true, 1, request(1, &read()))]),
        capture(&[(false, 1, response(9, &answer))]),
        capture(&[
            (true, 1, request(1, &read())),
            (false, 1, response(1, &answer)),
        ]),
    ];
    for (index, bytes) in captures.iter().enumerate() {
        let analysis = analyse_capture("plant.pcap", bytes, Options::default()).unwrap();
        assert!(
            !analysis.findings.is_empty(),
            "capture {index} produced nothing at all"
        );
        for finding in &analysis.findings {
            assert!(finding.is_well_formed(), "{}: {finding}", finding.id());
            assert_eq!(finding.evidence().artifact.name, "plant.pcap");
            assert!(
                finding.evidence().artifact.sha256.is_some(),
                "{} cannot say which capture it is about",
                finding.id()
            );
            assert!(!finding.message().is_empty(), "{}", finding.id());
            assert!(finding.source().starts_with("salman-analyse"));
            // And it renders.
            let rendered = finding.to_string();
            assert!(rendered.contains(finding.id()), "{rendered}");
        }
    }
}

#[test]
fn analysing_the_same_capture_twice_gives_the_same_findings() {
    // Determinism, as everywhere else. A findings report that varied between
    // runs could not be used as a golden test or compared across a change.
    let answer = Response::ReadHoldingRegisters(Words::new(&[1, 2]).unwrap());
    let bytes = capture(&[
        (true, 1, request(1, &read())),
        (false, 1, response(1, &answer)),
        (true, 20, request(2, &read())),
    ]);
    let first = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();
    let second = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();
    let render = |a: &salman_analyse::modbus::Analysis| {
        a.findings
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(render(&first), render(&second));
}

#[test]
fn a_file_that_is_not_a_capture_is_an_error_rather_than_an_empty_analysis() {
    // An empty report about a file salman could not read would say "nothing
    // wrong here" about something it never looked at.
    assert!(analyse_capture("not.pcap", b"this is not a capture", Options::default()).is_err());
}

#[test]
fn a_stream_that_never_framed_says_it_is_not_modbus_rather_than_that_modbus_broke() {
    // Found by pointing the command at an HTTP capture and reading what came
    // out: ten confident `framing lost` errors about somebody's web traffic.
    // Technically true and the wrong finding. A person who has the wrong port
    // should be told they have the wrong port.
    let http = frame(
        CLIENT,
        SERVER,
        51_000,
        502,
        1,
        b"GET / HTTP/1.1\r\nHost: x\r\n\r\n",
    );
    let more = frame(
        CLIENT,
        SERVER,
        51_000,
        502,
        1 + 26,
        b"GET /again HTTP/1.1\r\n\r\n",
    );
    let mut writer = Writer::new(LinkType::ETHERNET, 262_144);
    writer.write(0, http.len() as u32, &http);
    writer.write(1_000_000, more.len() as u32, &more);
    let bytes = writer.finish();

    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();

    let finding = find(&analysis.findings, "mbtcp.stream.not_modbus")
        .expect("salman should say these bytes are not Modbus");
    assert_eq!(finding.kind(), Kind::CannotDetermine);
    assert_eq!(finding.group(), Group::Assumption);
    assert_eq!(
        finding.justification(),
        Some(Justification::ProtocolAssumedFromPort)
    );
    assert!(
        finding.next_check().is_some(),
        "it must say what to try instead"
    );

    // And it says it once for the stream rather than once per frame.
    assert_eq!(
        analysis
            .findings
            .iter()
            .filter(|f| f.id() == "mbtcp.stream.not_modbus")
            .count(),
        1
    );
    assert!(
        find(&analysis.findings, "mbtcp.framing.lost").is_none(),
        "nothing here ever framed, so nothing was lost"
    );
}

#[test]
fn a_stream_that_framed_and_then_broke_is_a_fault() {
    // The other side of the same decision. This one really is a fault: it was
    // Modbus, and then it was not.
    let good = request(1, &read());
    let answer = Response::ReadHoldingRegisters(Words::new(&[1, 2]).unwrap());
    let mut second = response(2, &answer);
    // Break the protocol identifier of the second unit.
    second[3] = 0x10;

    let bytes = capture(&[
        (true, 1, good.clone()),
        (false, 1, response(1, &answer)),
        (false, 1 + response(1, &answer).len() as u32, second),
    ]);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();

    let finding = find(&analysis.findings, "mbtcp.framing.lost")
        .expect("a stream that framed and then broke is a fault");
    assert_eq!(finding.kind(), Kind::Fail);
    assert_eq!(finding.severity(), Some(Severity::Error));
    assert!(
        finding.message().contains("and then lost framing"),
        "{}",
        finding.message()
    );
    assert!(
        find(&analysis.findings, "mbtcp.stream.not_modbus").is_none(),
        "it framed cleanly first, so it is Modbus"
    );
}
