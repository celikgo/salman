// SPDX-License-Identifier: Apache-2.0
//! A capture and a scan trace on one axis.
//!
//! The test that matters most is the one about the alignment: salman requires
//! it and will not infer it. Guessing produces a timeline where every ordering
//! is plausible and every conclusion drawn from it is wrong, which is worse
//! than having no timeline at all.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_analyse::modbus::{Options, analyse_capture};
use salman_analyse::timeline::{Alignment, Event, Timeline};
use salman_capture::pcap::{LinkType, Writer};
use salman_core::time::Duration;
use salman_core::value::Value;
use salman_modbus::pdu::{Request, Response};
use salman_modbus::tcp::TcpAdu;
use salman_vm::memory::SlotId;
use salman_vm::trace::{Sample, Signal, Trace};

const CLIENT: [u8; 4] = [192, 168, 1, 10];
const SERVER: [u8; 4] = [192, 168, 1, 20];

/// The wall-clock instant the capture starts at.
const CAPTURE_START: u64 = 1_700_000_000_000_000_000;

fn frame(outgoing: bool, seq: u32, payload: &[u8]) -> Vec<u8> {
    let (from, to, sport, dport) = if outgoing {
        (CLIENT, SERVER, 51_000_u16, 502_u16)
    } else {
        (SERVER, CLIENT, 502, 51_000)
    };
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

/// A capture holding one refused read, at a known instant.
fn capture(refusal_at: u64) -> Vec<u8> {
    let request = TcpAdu::new(
        1,
        1,
        Request::ReadHoldingRegisters {
            start: 9_000,
            quantity: 4,
        }
        .encode()
        .unwrap(),
    )
    .to_vec();
    let refusal = TcpAdu::new(
        1,
        1,
        Response::Exception {
            function: salman_modbus::function::FunctionCode::READ_HOLDING_REGISTERS,
            code: salman_modbus::function::ExceptionCode::ILLEGAL_DATA_ADDRESS,
        }
        .encode(),
    )
    .to_vec();

    let mut writer = Writer::new(LinkType::ETHERNET, 262_144);
    writer.write(CAPTURE_START, 0, &frame(true, 1, &request));
    writer.write(refusal_at, 0, &frame(false, 1, &refusal));
    writer.finish()
}

/// A trace of four scans, one millisecond apart.
fn trace() -> Trace {
    let mut trace = Trace::new(vec![Signal::slot(SlotId(0), "Level")], 0, true);
    for scan in 0..4_i64 {
        trace.push(Sample {
            scan: scan.cast_unsigned(),
            time: Duration::from_nanos(scan * 1_000_000),
            task: 0,
            values: vec![Value::Int(scan as i16 * 10)],
        });
    }
    trace
}

// -- the alignment -------------------------------------------------------

#[test]
fn an_alignment_is_stated_by_naming_a_scan_and_when_it_happened() {
    // The form a person can actually supply. salman cannot work this out: two
    // runs of the same program produce identical traces, and the same traffic
    // captured twice produces different timestamps.
    let alignment =
        Alignment::from_correspondence(Duration::from_nanos(2_000_000), CAPTURE_START + 5_000_000)
            .expect("scan 2 ran three milliseconds after the capture started");
    assert_eq!(alignment.scan_zero_at_nanos, CAPTURE_START + 3_000_000);

    // And it maps back the way it came.
    assert_eq!(
        alignment.wall_nanos(Duration::from_nanos(2_000_000)),
        CAPTURE_START + 5_000_000
    );
    assert_eq!(
        alignment.wall_nanos(Duration::ZERO),
        CAPTURE_START + 3_000_000
    );
}

#[test]
fn a_correspondence_that_would_put_scan_zero_before_the_epoch_is_refused() {
    // One of the two numbers is not what the caller thinks it is, and
    // producing a timeline from it would put every row somewhere plausible
    // and wrong.
    assert!(Alignment::from_correspondence(Duration::from_nanos(1_000), 500).is_none());
}

// -- merging -------------------------------------------------------------

#[test]
fn a_wire_event_is_labelled_with_the_scan_that_acted_on_it() {
    // The whole reason to merge the two. A refusal that arrived between scan 1
    // and scan 2 was seen by scan 2, and a reader should not have to work that
    // out by subtracting timestamps.
    let refusal_at = CAPTURE_START + 1_500_000; // between scan 1 and scan 2
    let bytes = capture(refusal_at);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();
    let alignment = Alignment {
        scan_zero_at_nanos: CAPTURE_START,
    };

    let timeline = Timeline::merge(&trace(), &analysis, alignment);
    let refusal = timeline
        .entries
        .iter()
        .find(|entry| matches!(&entry.event, Event::Finding { id, .. } if *id == "mbtcp.exception"))
        .expect("the refusal is on the timeline");
    assert_eq!(
        refusal.during_scan,
        Some(2),
        "a refusal at 1.5 ms was seen by the scan that ended at 2 ms"
    );
}

#[test]
fn moving_the_alignment_moves_which_scan_saw_the_event() {
    // The point of requiring the alignment: it changes the answer. A tool that
    // guessed would produce one of these and present it as fact.
    let refusal_at = CAPTURE_START + 1_500_000;
    let bytes = capture(refusal_at);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();

    let seen_by = |offset: u64| {
        let timeline = Timeline::merge(
            &trace(),
            &analysis,
            Alignment {
                scan_zero_at_nanos: CAPTURE_START + offset,
            },
        );
        timeline
            .entries
            .iter()
            .find(
                |entry| matches!(&entry.event, Event::Finding { id, .. } if *id == "mbtcp.exception"),
            )
            .and_then(|entry| entry.during_scan)
    };

    assert_eq!(seen_by(0), Some(2), "scans at 0, 1, 2, 3 ms");
    assert_eq!(
        seen_by(1_000_000),
        Some(1),
        "the same scans a millisecond later see it a scan earlier"
    );
}

#[test]
fn everything_is_in_time_order() {
    let bytes = capture(CAPTURE_START + 2_500_000);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();
    let timeline = Timeline::merge(
        &trace(),
        &analysis,
        Alignment {
            scan_zero_at_nanos: CAPTURE_START,
        },
    );
    let times: Vec<u64> = timeline.entries.iter().map(|e| e.at_nanos).collect();
    let mut sorted = times.clone();
    sorted.sort_unstable();
    assert_eq!(times, sorted);
    assert!(timeline.len() >= 4, "the four scans are all there");
}

#[test]
fn a_finding_about_the_whole_capture_is_left_off_the_axis() {
    // A finding with no timestamp did not happen at a moment. Putting it at
    // zero would sort it before everything and imply it happened first.
    let bytes = capture(CAPTURE_START + 1_000_000);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();
    assert!(
        analysis
            .findings
            .iter()
            .any(|f| f.evidence().timestamp.is_none()),
        "this capture should produce at least one finding about the whole file"
    );

    let timeline = Timeline::merge(
        &trace(),
        &analysis,
        Alignment {
            scan_zero_at_nanos: CAPTURE_START,
        },
    );
    let on_axis = timeline
        .entries
        .iter()
        .filter(|e| matches!(e.event, Event::Finding { .. }))
        .count();
    let timestamped = analysis
        .findings
        .iter()
        .filter(|f| f.evidence().timestamp.is_some())
        .count();
    assert_eq!(on_axis, timestamped);
}

// -- rendering -----------------------------------------------------------

#[test]
fn the_rendering_says_what_the_alignment_was() {
    // A timeline whose alignment is not visible is a timeline nobody can
    // check, because the alignment is the one thing a person supplied.
    let bytes = capture(CAPTURE_START + 1_000_000);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();
    let timeline = Timeline::merge(
        &trace(),
        &analysis,
        Alignment {
            scan_zero_at_nanos: CAPTURE_START,
        },
    );
    let rendered = timeline.render();
    assert!(
        rendered.contains("as the caller stated"),
        "the alignment must be visible: {rendered}"
    );
    assert!(rendered.contains(&CAPTURE_START.to_string()));
    assert!(rendered.contains("Level="));
    assert!(rendered.contains("mbtcp.exception"));
}

#[test]
fn rendering_is_deterministic_and_has_no_colour() {
    let bytes = capture(CAPTURE_START + 1_000_000);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();
    let timeline = Timeline::merge(
        &trace(),
        &analysis,
        Alignment {
            scan_zero_at_nanos: CAPTURE_START,
        },
    );
    assert_eq!(timeline.render(), timeline.render());
    assert!(!timeline.render().contains('\u{1b}'));
}

#[test]
fn intervals_are_shown_relative_to_the_first_row() {
    // An absolute nanosecond count is unreadable, and the interval is what a
    // reader is after.
    let bytes = capture(CAPTURE_START + 2_000_000);
    let analysis = analyse_capture("plant.pcap", &bytes, Options::default()).unwrap();
    let timeline = Timeline::merge(
        &trace(),
        &analysis,
        Alignment {
            scan_zero_at_nanos: CAPTURE_START,
        },
    );
    let rendered = timeline.render();
    let rows: Vec<&str> = rendered.lines().filter(|l| !l.starts_with('#')).collect();
    // The header, then the first data row starting at zero.
    assert_eq!(rows.first().copied(), Some("time\tscan\tsource\twhat"));
    assert!(
        rows.get(1).is_some_and(|row| row.starts_with('0')),
        "{rows:?}"
    );
    assert!(rendered.contains("1.000ms"), "{rendered}");
}

#[test]
fn an_empty_trace_and_an_empty_analysis_make_an_empty_timeline() {
    let timeline = Timeline::merge(
        &Trace::new(Vec::new(), 0, true),
        &salman_analyse::modbus::Analysis::default(),
        Alignment {
            scan_zero_at_nanos: 0,
        },
    );
    assert!(timeline.is_empty());
    assert!(timeline.render().contains("time\tscan\tsource\twhat"));
}
