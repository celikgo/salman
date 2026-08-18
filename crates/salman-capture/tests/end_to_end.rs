// SPDX-License-Identifier: Apache-2.0
//! A capture file, decoded to Modbus transactions.
//!
//! This is the shape §7 of the build specification asks for: the same decoder
//! runs on a live socket and on a capture file, and cannot tell which. The
//! capture is built here with salman's own writer, framed the way a real
//! network frames things — one request split across two segments, a
//! duplicated packet from a mirror port, an acknowledgement padded to the
//! Ethernet minimum — and the transactions have to come out whole regardless.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_capture::frame::{Decoded, decode};
use salman_capture::pcap::{LinkType, Reader, Writer};
use salman_capture::reassemble::Reassembler;
use salman_modbus::pdu::Request;
use salman_modbus::tcp::{Framer, TcpAdu};

/// Wraps a TCP payload in TCP, IPv4 and Ethernet headers.
fn frame(
    from: [u8; 4],
    to: [u8; 4],
    sport: u16,
    dport: u16,
    seq: u32,
    payload: &[u8],
    pad: bool,
) -> Vec<u8> {
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
    if pad {
        // An Ethernet frame has a minimum size. These bytes are not payload,
        // and a decoder that thought they were would report a phantom Modbus
        // frame here.
        while ethernet.len() < 60 {
            ethernet.push(0x00);
        }
    }
    ethernet
}

const CLIENT: [u8; 4] = [192, 168, 1, 10];
const SERVER: [u8; 4] = [192, 168, 1, 20];

#[test]
fn a_capture_file_becomes_modbus_transactions() {
    let request_one = TcpAdu::new(
        1,
        0xFF,
        Request::ReadHoldingRegisters {
            start: 0,
            quantity: 3,
        }
        .encode()
        .unwrap(),
    )
    .to_vec();
    let request_two = TcpAdu::new(
        2,
        0xFF,
        Request::ReadCoils {
            start: 8,
            quantity: 4,
        }
        .encode()
        .unwrap(),
    )
    .to_vec();

    let mut writer = Writer::new(LinkType::ETHERNET, 262_144);
    let mut time = 1_700_000_000_000_000_000_u64;
    let mut push = |writer: &mut Writer, bytes: &[u8]| {
        writer.write(time, bytes.len() as u32, bytes);
        time += 1_000_000;
    };

    // A padded acknowledgement carrying nothing.
    let ack = frame(CLIENT, SERVER, 51_000, 502, 1, &[], true);
    push(&mut writer, &ack);

    // The first request, split across two segments mid-frame.
    let (head, tail) = request_one.split_at(4);
    push(
        &mut writer,
        &frame(CLIENT, SERVER, 51_000, 502, 1, head, false),
    );
    push(
        &mut writer,
        &frame(CLIENT, SERVER, 51_000, 502, 5, tail, false),
    );

    // The second request, delivered twice by a mirror port.
    let second_seq = 1 + request_one.len() as u32;
    let second = frame(CLIENT, SERVER, 51_000, 502, second_seq, &request_two, false);
    push(&mut writer, &second);
    push(&mut writer, &second);

    let capture = writer.finish();

    // Now read it back the way salman would read any capture.
    let mut reader = Reader::new(&capture).unwrap();
    let link = reader.link_type();
    let mut reassembler = Reassembler::new();
    let mut framer = Framer::new();
    let mut requests = Vec::new();
    let mut duplicates = 0;

    for record in reader.records().unwrap() {
        let Ok(Decoded::Tcp(segment)) = decode(link, record.data, record.truncated) else {
            continue;
        };
        let delivery = reassembler.push(&segment);
        duplicates += delivery
            .notes
            .iter()
            .filter(|n| matches!(n, salman_capture::reassemble::Note::Duplicate { .. }))
            .count();

        let mut rest = &delivery.bytes[..];
        loop {
            let (used, outcome) = framer.advance(rest);
            rest = rest.get(used..).unwrap_or(&[]);
            match outcome {
                Ok(Some(adu)) => {
                    requests.push(Request::decode(adu.pdu.as_bytes()).unwrap());
                }
                Ok(None) => break,
                Err(error) => panic!("framing was lost: {error}"),
            }
        }
    }

    assert_eq!(
        requests,
        [
            Request::ReadHoldingRegisters {
                start: 0,
                quantity: 3
            },
            Request::ReadCoils {
                start: 8,
                quantity: 4
            },
        ],
        "the split request and the mirrored one both came out exactly once"
    );
    assert_eq!(duplicates, 1, "the mirrored packet was recognised");
}

#[test]
fn the_padding_on_an_acknowledgement_produces_no_transaction_at_all() {
    // Stated on its own because it is the failure that would otherwise appear
    // as a phantom Modbus frame on every acknowledgement in a capture.
    let ack = frame(CLIENT, SERVER, 51_000, 502, 1, &[], true);
    assert_eq!(ack.len(), 60, "this test needs a padded frame");

    let mut writer = Writer::new(LinkType::ETHERNET, 262_144);
    writer.write(0, ack.len() as u32, &ack);
    let capture = writer.finish();

    let mut reader = Reader::new(&capture).unwrap();
    let link = reader.link_type();
    let mut reassembler = Reassembler::new();
    let mut framer = Framer::new();

    for record in reader.records().unwrap() {
        let Ok(Decoded::Tcp(segment)) = decode(link, record.data, record.truncated) else {
            continue;
        };
        assert!(segment.payload.is_empty(), "padding became payload");
        let delivery = reassembler.push(&segment);
        let (_, outcome) = framer.advance(&delivery.bytes);
        assert_eq!(outcome.unwrap(), None, "a phantom frame was produced");
    }
}

#[test]
fn a_capture_salman_wrote_reads_back_identically() {
    // Determinism, over the whole path: the bytes salman writes are the bytes
    // salman reads, and two captures of the same traffic are byte-identical.
    let build = || {
        let mut writer = Writer::new(LinkType::ETHERNET, 262_144);
        for index in 0..8_u32 {
            let adu = TcpAdu::new(
                index as u16,
                0xFF,
                Request::ReadHoldingRegisters {
                    start: index as u16,
                    quantity: 1,
                }
                .encode()
                .unwrap(),
            )
            .to_vec();
            let bytes = frame(CLIENT, SERVER, 51_000, 502, 1 + index * 12, &adu, false);
            writer.write(u64::from(index) * 1_000_000, bytes.len() as u32, &bytes);
        }
        writer.finish()
    };
    assert_eq!(build(), build());

    let capture = build();
    let mut reader = Reader::new(&capture).unwrap();
    assert_eq!(reader.records().unwrap().len(), 8);
}
