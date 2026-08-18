#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use salman_capture::frame::{Decoded, decode};
use salman_capture::pcap::{LinkType, Reader, Writer};

fn ipv4(payload: &[u8], proto: u8, frag_word: u16, total_override: Option<u16>) -> Vec<u8> {
    let mut p = Vec::new();
    p.push(0x45);
    p.push(0);
    let total = total_override.unwrap_or((20 + payload.len()) as u16);
    p.extend_from_slice(&total.to_be_bytes());
    p.extend_from_slice(&[0xAB, 0xCD]);
    p.extend_from_slice(&frag_word.to_be_bytes());
    p.push(64);
    p.push(proto);
    p.extend_from_slice(&[0, 0]);
    p.extend_from_slice(&[192, 168, 1, 10]);
    p.extend_from_slice(&[192, 168, 1, 20]);
    p.extend_from_slice(payload);
    p
}

fn eth(t: u16, p: &[u8]) -> Vec<u8> {
    let mut f = vec![0x11; 6];
    f.extend_from_slice(&[0x22; 6]);
    f.extend_from_slice(&t.to_be_bytes());
    f.extend_from_slice(p);
    f
}

#[test]
fn a_non_first_fragment_is_decoded_as_if_it_were_a_tcp_header() {
    // A second fragment of a TCP packet: fragment offset 185 (=1480 bytes),
    // no MF. It carries raw application bytes, NOT a TCP header.
    let continuation: Vec<u8> = (0u8..40).collect();
    // frag word: flags=0, offset=185
    let packet = ipv4(&continuation, 6, 185, None);
    let frame = eth(0x0800, &packet);
    match decode(LinkType::ETHERNET, &frame, false).unwrap() {
        Decoded::Tcp(s) => {
            println!(
                "FRAGMENT DECODED AS TCP: src port {} dst port {} seq {:#x} payload {:?}",
                s.source.port, s.destination.port, s.sequence, s.payload
            );
        }
        Decoded::NotDecoded { what } => println!("not decoded: {what}"),
    }
}

#[test]
fn a_lying_total_length_is_blamed_on_the_capture() {
    // Fully captured 60-byte frame whose IP total length claims 1400.
    let mut packet = ipv4(&[], 6, 0, Some(1400));
    // append a 20 byte tcp header
    let mut tcp = Vec::new();
    tcp.extend_from_slice(&1234u16.to_be_bytes());
    tcp.extend_from_slice(&502u16.to_be_bytes());
    tcp.extend_from_slice(&[0; 8]);
    tcp.push(0x50);
    tcp.push(0x10);
    tcp.extend_from_slice(&[0; 6]);
    packet.extend_from_slice(&tcp);
    let frame = eth(0x0800, &packet);
    assert_eq!(frame.len(), 54);
    match decode(LinkType::ETHERNET, &frame, false).unwrap() {
        Decoded::Tcp(s) => println!("LYING TOTAL LENGTH -> truncated = {}", s.truncated),
        Decoded::NotDecoded { what } => println!("not decoded: {what}"),
    }
}

#[test]
fn a_frame_truncated_inside_a_vlan_tag() {
    let mut frame = vec![0x11; 6];
    frame.extend_from_slice(&[0x22; 6]);
    frame.extend_from_slice(&0x8100u16.to_be_bytes());
    frame.extend_from_slice(&[0x00, 0x64]); // TCI only, then cut
    match decode(LinkType::ETHERNET, &frame, true).unwrap() {
        Decoded::Tcp(_) => println!("tcp?"),
        Decoded::NotDecoded { what } => println!("VLAN CUT -> {what}"),
    }
}

#[test]
fn an_empty_raw_frame() {
    match decode(LinkType::RAW, &[], true) {
        Ok(d) => println!("empty raw -> {d:?}"),
        Err(e) => println!("EMPTY RAW FRAME -> Err: {e}"),
    }
}

#[test]
fn snaplen_zero_file_is_refused() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0xA1B2C3D4u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // snaplen 0
    bytes.extend_from_slice(&1u32.to_le_bytes());
    // one perfectly good 60-byte record
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&60u32.to_le_bytes());
    bytes.extend_from_slice(&60u32.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 60]);
    match Reader::new(&bytes) {
        Ok(_) => println!("snaplen 0 accepted"),
        Err(e) => println!("SNAPLEN 0 -> Err: {e}"),
    }
}

#[test]
fn writer_rewrites_a_legal_original_length() {
    let mut w = Writer::new(LinkType::ETHERNET, 262144);
    w.write(1_000_000_000, 5, &[0u8; 10]);
    let bytes = w.finish();
    let mut r = Reader::new(&bytes).unwrap();
    let rec = r.next_record().unwrap().unwrap();
    println!(
        "WRITER: asked origlen 5 caplen 10 -> got origlen {} caplen {} truncated {}",
        rec.original_length,
        rec.data.len(),
        rec.truncated
    );
}

#[test]
fn records_discards_what_it_read() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0xA1B2C3D4u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&65535u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    for _ in 0..3 {
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 4]);
    }
    // a fourth record header that lies
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&4000u32.to_le_bytes());
    bytes.extend_from_slice(&4000u32.to_le_bytes());
    let mut r = Reader::new(&bytes).unwrap();
    match r.records() {
        Ok(v) => println!("records ok: {}", v.len()),
        Err(e) => println!("RECORDS -> Err (three good records were dropped): {e}"),
    }
}
