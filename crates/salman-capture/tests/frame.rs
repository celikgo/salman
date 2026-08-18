// SPDX-License-Identifier: Apache-2.0
//! Decoding a captured frame down to a TCP payload.
//!
//! The test that justifies the whole module is
//! `ethernet_padding_on_a_short_frame_is_not_payload`. An Ethernet frame has a
//! minimum size, so a bare acknowledgement arrives padded, and a decoder that
//! took everything after the TCP header would hand six bytes of padding to
//! whatever is above it. Fed to a Modbus framer those six bytes read as
//! transaction 0, protocol 0, length 0 — a phantom frame, on every
//! acknowledgement, in a capture where nothing is wrong.
//!
//! # Checked against a capture salman did not write
//!
//! These tests build their own frames. The decoder was additionally run over
//! Homebrew's `test.pcap` — a complete HTTP exchange — and agreed with
//! `tcpdump -r` on every frame: payload lengths 77, 215 and 95 for the three
//! data-carrying segments and zero for all nine others, with the handshake and
//! teardown flags in the right places.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_capture::frame::{Address, Decoded, FrameError, NotDecoded, decode};
use salman_capture::pcap::LinkType;

/// Builds an IPv4 + TCP packet with the given payload and header sizes.
fn ipv4_tcp(payload: &[u8], ip_options: usize, tcp_options: usize) -> Vec<u8> {
    let ip_header = 20 + ip_options;
    let tcp_header = 20 + tcp_options;
    let total = ip_header + tcp_header + payload.len();

    let mut packet = Vec::new();
    packet.push(0x40 | ((ip_header / 4) as u8)); // version 4, IHL
    packet.push(0); // DSCP/ECN
    packet.extend_from_slice(&(total as u16).to_be_bytes());
    packet.extend_from_slice(&[0, 0, 0, 0]); // id, flags, fragment
    packet.push(64); // TTL
    packet.push(6); // TCP
    packet.extend_from_slice(&[0, 0]); // checksum
    packet.extend_from_slice(&[192, 168, 1, 10]);
    packet.extend_from_slice(&[192, 168, 1, 20]);
    packet.extend(std::iter::repeat_n(0_u8, ip_options));

    packet.extend_from_slice(&1234_u16.to_be_bytes());
    packet.extend_from_slice(&502_u16.to_be_bytes());
    packet.extend_from_slice(&0x1111_2222_u32.to_be_bytes());
    packet.extend_from_slice(&0x3333_4444_u32.to_be_bytes());
    packet.push(((tcp_header / 4) as u8) << 4);
    packet.push(0x18); // PSH + ACK
    packet.extend_from_slice(&[0xFF, 0xFF]); // window
    packet.extend_from_slice(&[0, 0]); // checksum
    packet.extend_from_slice(&[0, 0]); // urgent
    packet.extend(std::iter::repeat_n(0_u8, tcp_options));
    packet.extend_from_slice(payload);
    packet
}

/// Wraps a packet in an Ethernet header.
fn ethernet(ether_type: u16, packet: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x11; 6];
    frame.extend_from_slice(&[0x22; 6]);
    frame.extend_from_slice(&ether_type.to_be_bytes());
    frame.extend_from_slice(packet);
    frame
}

fn tcp_of(decoded: Decoded<'_>) -> salman_capture::frame::Segment<'_> {
    match decoded {
        Decoded::Tcp(segment) => segment,
        Decoded::NotDecoded { what } => panic!("expected TCP, got: {what}"),
    }
}

// -- the trap ------------------------------------------------------------

#[test]
fn ethernet_padding_on_a_short_frame_is_not_payload() {
    // A bare acknowledgement: 14 + 20 + 20 = 54 bytes, padded to the 60-byte
    // Ethernet minimum. The six padding octets are not TCP payload, and a
    // decoder that took everything after the headers would say they were.
    let mut frame = ethernet(0x0800, &ipv4_tcp(&[], 0, 0));
    assert_eq!(frame.len(), 54);
    frame.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE]);
    assert_eq!(frame.len(), 60);

    let segment = tcp_of(decode(LinkType::ETHERNET, &frame, false).unwrap());
    assert_eq!(
        segment.payload.len(),
        0,
        "six bytes of Ethernet padding were reported as payload"
    );
    assert!(!segment.truncated);
}

#[test]
fn padding_that_would_look_like_a_modbus_header_is_still_not_payload() {
    // The concrete harm: six zero bytes read as an MBAP header with
    // transaction 0, protocol 0 and length 0. A framer above would report a
    // phantom frame on every acknowledgement in the capture.
    let mut frame = ethernet(0x0800, &ipv4_tcp(&[], 0, 0));
    frame.extend_from_slice(&[0x00; 6]);
    let segment = tcp_of(decode(LinkType::ETHERNET, &frame, false).unwrap());
    assert!(segment.payload.is_empty());
}

// -- variable-length headers ---------------------------------------------

#[test]
fn ip_options_move_the_tcp_header_and_are_not_read_as_one() {
    // IHL is a count of 32-bit words with a minimum of five. Twenty bytes is
    // the common case and not the rule: a decoder that hardcoded it would read
    // the IP options as the start of the TCP header and report a nonsense port.
    let frame = ethernet(0x0800, &ipv4_tcp(b"hello", 8, 0));
    let segment = tcp_of(decode(LinkType::ETHERNET, &frame, false).unwrap());
    assert_eq!(
        segment.source.port, 1234,
        "the ports moved with the options"
    );
    assert_eq!(segment.destination.port, 502);
    assert_eq!(segment.payload, b"hello");
}

#[test]
fn tcp_options_move_the_payload_and_are_not_read_as_one() {
    // A SYN almost always carries options, so this is the ordinary case rather
    // than an exotic one.
    let frame = ethernet(0x0800, &ipv4_tcp(b"hello", 0, 12));
    let segment = tcp_of(decode(LinkType::ETHERNET, &frame, false).unwrap());
    assert_eq!(segment.payload, b"hello");
}

#[test]
fn both_sets_of_options_at_once() {
    let frame = ethernet(0x0800, &ipv4_tcp(b"payload here", 4, 8));
    let segment = tcp_of(decode(LinkType::ETHERNET, &frame, false).unwrap());
    assert_eq!(segment.payload, b"payload here");
}

#[test]
fn a_header_length_below_its_own_minimum_is_refused() {
    let mut packet = ipv4_tcp(b"x", 0, 0);
    packet[0] = 0x44; // IHL = 4, which is 16 bytes
    let frame = ethernet(0x0800, &packet);
    assert!(matches!(
        decode(LinkType::ETHERNET, &frame, false),
        Err(FrameError::HeaderTooShort { layer: "IPv4", .. })
    ));

    let mut packet = ipv4_tcp(b"x", 0, 0);
    packet[20 + 12] = 0x40; // TCP data offset = 4, which is 16 bytes
    let frame = ethernet(0x0800, &packet);
    assert!(matches!(
        decode(LinkType::ETHERNET, &frame, false),
        Err(FrameError::HeaderTooShort { layer: "TCP", .. })
    ));
}

// -- VLAN ----------------------------------------------------------------

#[test]
fn a_vlan_tag_is_stepped_past() {
    let inner = ipv4_tcp(b"tagged", 0, 0);
    let mut frame = vec![0x11; 6];
    frame.extend_from_slice(&[0x22; 6]);
    frame.extend_from_slice(&0x8100_u16.to_be_bytes());
    frame.extend_from_slice(&[0x00, 0x64]); // VLAN 100
    frame.extend_from_slice(&0x0800_u16.to_be_bytes());
    frame.extend_from_slice(&inner);
    let segment = tcp_of(decode(LinkType::ETHERNET, &frame, false).unwrap());
    assert_eq!(segment.payload, b"tagged");
}

#[test]
fn stacked_vlan_tags_are_stepped_past() {
    // QinQ: a customer tag inside a service tag, which is ordinary in a
    // carrier network. The EtherType is a loop, not an `if`.
    let inner = ipv4_tcp(b"qinq", 0, 0);
    let mut frame = vec![0x11; 6];
    frame.extend_from_slice(&[0x22; 6]);
    frame.extend_from_slice(&0x88A8_u16.to_be_bytes());
    frame.extend_from_slice(&[0x00, 0x0A]);
    frame.extend_from_slice(&0x8100_u16.to_be_bytes());
    frame.extend_from_slice(&[0x00, 0x64]);
    frame.extend_from_slice(&0x0800_u16.to_be_bytes());
    frame.extend_from_slice(&inner);
    let segment = tcp_of(decode(LinkType::ETHERNET, &frame, false).unwrap());
    assert_eq!(segment.payload, b"qinq");
}

#[test]
fn a_frame_that_is_nothing_but_vlan_tags_is_refused_rather_than_followed_for_ever() {
    let mut frame = vec![0x11; 6];
    frame.extend_from_slice(&[0x22; 6]);
    for _ in 0..40 {
        frame.extend_from_slice(&0x8100_u16.to_be_bytes());
        frame.extend_from_slice(&[0x00, 0x64]);
    }
    frame.extend_from_slice(&0x0800_u16.to_be_bytes());
    assert!(matches!(
        decode(LinkType::ETHERNET, &frame, false),
        Err(FrameError::TooManyVlanTags { .. })
    ));
}

// -- link types ----------------------------------------------------------

#[test]
fn a_raw_ip_frame_has_no_link_header_and_is_told_apart_by_its_version_nibble() {
    let packet = ipv4_tcp(b"raw", 0, 0);
    let segment = tcp_of(decode(LinkType::RAW, &packet, false).unwrap());
    assert_eq!(segment.payload, b"raw");
}

#[test]
fn a_loopback_frame_is_read_in_whichever_byte_order_names_a_protocol() {
    // LINKTYPE_NULL's four-octet protocol field is in the **capturing host's**
    // byte order, and the file's magic says nothing about it. Both orders have
    // to be tried, which looks like a bug and is the documented behaviour.
    let packet = ipv4_tcp(b"loop", 0, 0);
    for header in [2_u32.to_le_bytes(), 2_u32.to_be_bytes()] {
        let mut frame = header.to_vec();
        frame.extend_from_slice(&packet);
        let segment = tcp_of(decode(LinkType::NULL, &frame, false).unwrap());
        assert_eq!(segment.payload, b"loop", "byte order {header:02X?}");
    }
}

#[test]
fn a_linux_cooked_capture_is_decoded_in_both_of_its_versions() {
    let packet = ipv4_tcp(b"cooked", 0, 0);

    // SLL: sixteen octets, with the EtherType in the last two.
    let mut sll = vec![0_u8; 14];
    sll.extend_from_slice(&0x0800_u16.to_be_bytes());
    sll.extend_from_slice(&packet);
    assert_eq!(
        tcp_of(decode(LinkType::LINUX_SLL, &sll, false).unwrap()).payload,
        b"cooked"
    );

    // SLL2: twenty octets, with the protocol first.
    let mut sll2 = 0x0800_u16.to_be_bytes().to_vec();
    sll2.extend_from_slice(&[0_u8; 18]);
    sll2.extend_from_slice(&packet);
    assert_eq!(
        tcp_of(decode(LinkType::LINUX_SLL2, &sll2, false).unwrap()).payload,
        b"cooked"
    );
}

#[test]
fn an_ipv6_segment_is_decoded() {
    let mut packet = vec![0x60, 0, 0, 0];
    let payload = b"six";
    let tcp_len = 20 + payload.len();
    packet.extend_from_slice(&(tcp_len as u16).to_be_bytes());
    packet.push(6); // next header: TCP
    packet.push(64); // hop limit
    packet.extend_from_slice(&[0x20; 16]);
    packet.extend_from_slice(&[0x30; 16]);
    packet.extend_from_slice(&1234_u16.to_be_bytes());
    packet.extend_from_slice(&502_u16.to_be_bytes());
    packet.extend_from_slice(&[0; 8]);
    packet.push(0x50);
    packet.push(0x10);
    packet.extend_from_slice(&[0; 6]);
    packet.extend_from_slice(payload);

    let frame = ethernet(0x86DD, &packet);
    let segment = tcp_of(decode(LinkType::ETHERNET, &frame, false).unwrap());
    assert_eq!(segment.payload, b"six");
    assert!(matches!(segment.source.address, Address::V6(_)));
    assert_eq!(
        segment.source.to_string(),
        "[2020:2020:2020:2020:2020:2020:2020:2020]:1234"
    );
}

// -- answers that are not failures ---------------------------------------

#[test]
fn a_frame_that_is_not_ip_is_named_rather_than_reported_as_broken() {
    // A capture is full of frames that are not what is being looked for. A
    // decoder that returned an error for each would drown the real findings.
    let frame = ethernet(0x0806, &[0; 28]); // ARP
    assert!(matches!(
        decode(LinkType::ETHERNET, &frame, false).unwrap(),
        Decoded::NotDecoded {
            what: NotDecoded::EtherType(0x0806)
        }
    ));
}

#[test]
fn udp_is_named_rather_than_decoded() {
    let mut packet = ipv4_tcp(b"x", 0, 0);
    packet[9] = 17;
    let frame = ethernet(0x0800, &packet);
    let decoded = decode(LinkType::ETHERNET, &frame, false).unwrap();
    assert!(matches!(
        decoded,
        Decoded::NotDecoded {
            what: NotDecoded::Protocol(17)
        }
    ));
    let Decoded::NotDecoded { what } = decoded else {
        panic!()
    };
    assert!(what.to_string().contains("UDP"), "{what}");
}

#[test]
fn an_ipv6_extension_header_is_named_rather_than_guessed_through() {
    // Walking an extension chain wrongly produces a payload that looks like
    // data and is not, which is worse than saying salman did not decode it.
    let mut packet = vec![0x60, 0, 0, 0, 0, 8];
    packet.push(0); // next header: hop-by-hop options
    packet.push(64);
    packet.extend_from_slice(&[0x20; 16]);
    packet.extend_from_slice(&[0x30; 16]);
    packet.extend_from_slice(&[0; 8]);
    let frame = ethernet(0x86DD, &packet);
    assert!(matches!(
        decode(LinkType::ETHERNET, &frame, false).unwrap(),
        Decoded::NotDecoded {
            what: NotDecoded::Protocol(0)
        }
    ));
}

#[test]
fn a_link_type_salman_does_not_decode_is_named() {
    let decoded = decode(LinkType(147), &[0; 64], false).unwrap();
    assert!(matches!(
        decoded,
        Decoded::NotDecoded {
            what: NotDecoded::LinkType(_)
        }
    ));
}

#[test]
fn a_frame_cut_short_before_its_headers_is_not_an_error() {
    // The capturing tool kept too little. Nothing is wrong with the frame.
    for length in 0..14_usize {
        let frame = vec![0_u8; length];
        assert!(
            matches!(
                decode(LinkType::ETHERNET, &frame, true).unwrap(),
                Decoded::NotDecoded { .. }
            ),
            "{length} bytes"
        );
    }
}

#[test]
fn a_payload_cut_short_by_the_snapshot_length_is_marked_truncated() {
    // The IP header says more than arrived, which is what a snaplen does. The
    // payload that did arrive is still usable and the flag says not to trust
    // its length.
    let mut frame = ethernet(0x0800, &ipv4_tcp(b"0123456789", 0, 0));
    frame.truncate(frame.len() - 4);
    let segment = tcp_of(decode(LinkType::ETHERNET, &frame, false).unwrap());
    assert_eq!(segment.payload, b"012345");
    assert!(
        segment.truncated,
        "the header claimed more than the capture kept"
    );
}

// -- the shape of a conversation -----------------------------------------

#[test]
fn both_directions_of_one_conversation_share_a_key() {
    // The reassembler keys on this, so it has to be the same key whichever way
    // the segment was going.
    let out_frame = ethernet(0x0800, &ipv4_tcp(b"a", 0, 0));
    let out = tcp_of(decode(LinkType::ETHERNET, &out_frame, false).unwrap());
    let mut back_packet = ipv4_tcp(b"b", 0, 0);
    back_packet[12..16].copy_from_slice(&[192, 168, 1, 20]);
    back_packet[16..20].copy_from_slice(&[192, 168, 1, 10]);
    back_packet[20..22].copy_from_slice(&502_u16.to_be_bytes());
    back_packet[22..24].copy_from_slice(&1234_u16.to_be_bytes());
    let back_frame = ethernet(0x0800, &back_packet);
    let back = tcp_of(decode(LinkType::ETHERNET, &back_frame, false).unwrap());

    assert_ne!(out.source, back.source);
    assert_eq!(
        out.connection(),
        back.connection(),
        "the two directions must key the same"
    );
}

#[test]
fn the_flags_are_read_from_the_right_bits() {
    let mut packet = ipv4_tcp(&[], 0, 0);
    let flags_at = 20 + 13;
    for (bits, syn, ack, fin, rst) in [
        (0x02_u8, true, false, false, false),
        (0x12, true, true, false, false),
        (0x11, false, true, true, false),
        (0x04, false, false, false, true),
        (0x10, false, true, false, false),
    ] {
        packet[flags_at] = bits;
        let frame = ethernet(0x0800, &packet);
        let segment = tcp_of(decode(LinkType::ETHERNET, &frame, false).unwrap());
        assert_eq!(
            (segment.syn, segment.ack, segment.fin, segment.rst),
            (syn, ack, fin, rst),
            "0x{bits:02X}"
        );
    }
}

// -- robustness ----------------------------------------------------------

#[test]
fn no_frame_makes_the_decoder_panic() {
    let mut seed = 0xACE1_ACE1_ACE1_ACE1_u64;
    let mut next = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let good = ethernet(0x0800, &ipv4_tcp(b"payload", 4, 8));
    for round in 0..20_000 {
        let bytes: Vec<u8> = if round % 2 == 0 {
            let length = (next() % 128) as usize;
            (0..length).map(|_| (next() >> 33) as u8).collect()
        } else {
            let mut corrupted = good.clone();
            for _ in 0..=(next() % 3) {
                let at = (next() as usize) % corrupted.len();
                corrupted[at] = (next() >> 33) as u8;
            }
            corrupted
        };
        for link in [
            LinkType::ETHERNET,
            LinkType::RAW,
            LinkType::NULL,
            LinkType::LINUX_SLL,
            LinkType::LINUX_SLL2,
            LinkType::IPV4,
            LinkType::IPV6,
        ] {
            let _ = decode(link, &bytes, false);
        }
    }
}

#[test]
fn a_payload_is_never_longer_than_the_frame_it_came_from() {
    // The property that stops a lying length field reading past the buffer.
    // Asserted over corrupted frames, because that is where a length field
    // lies.
    let mut seed = 0xBEEF_1234_5678_9ABC_u64;
    let mut next = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let good = ethernet(0x0800, &ipv4_tcp(b"payload", 0, 0));
    for _ in 0..20_000 {
        let mut frame = good.clone();
        let at = (next() as usize) % frame.len();
        frame[at] = (next() >> 33) as u8;
        if let Ok(Decoded::Tcp(segment)) = decode(LinkType::ETHERNET, &frame, false) {
            assert!(
                segment.payload.len() <= frame.len(),
                "a payload longer than its frame: {} > {}",
                segment.payload.len(),
                frame.len()
            );
        }
    }
}

// -- fragments -----------------------------------------------------------

/// Sets the flags-and-offset word of an IPv4 packet inside an Ethernet frame.
fn with_fragment_word(frame: &mut [u8], word: u16) {
    // 14 bytes of Ethernet, then the word is at offset 6 of the IP header.
    frame[14 + 6..14 + 8].copy_from_slice(&word.to_be_bytes());
}

#[test]
fn a_fragment_that_is_not_the_first_is_not_decoded_as_tcp() {
    // Found by review. A fragment with a non-zero offset carries **no
    // transport header at all** — the bytes where a TCP header would be are
    // application data from the middle of the packet. Decoding it produces a
    // segment with invented ports, an invented sequence number and a payload
    // cut from the middle of something: entirely plausible and entirely false.
    let mut frame = ethernet(0x0800, &ipv4_tcp(b"continuation bytes", 0, 0));
    // Offset 185 eight-byte units, which is 1480 bytes in.
    with_fragment_word(&mut frame, 185);

    let decoded = decode(LinkType::ETHERNET, &frame, false).unwrap();
    let Decoded::NotDecoded { what } = decoded else {
        panic!("a non-first fragment was decoded as TCP: {decoded:?}")
    };
    assert!(
        matches!(
            what,
            NotDecoded::Fragmented {
                offset: 1480,
                more_fragments: false
            }
        ),
        "{what:?}"
    );
    assert!(
        what.to_string().contains("no transport header at all"),
        "{what}"
    );
}

#[test]
fn the_first_of_several_fragments_is_not_decoded_either() {
    // Less obvious and just as wrong. A first fragment does carry a TCP
    // header, and carries only part of the payload — handing that to a stream
    // reassembler puts a hole in the middle of the byte stream that the
    // sequence numbers do not account for.
    let mut frame = ethernet(0x0800, &ipv4_tcp(b"the first part", 0, 0));
    // More fragments, offset zero.
    with_fragment_word(&mut frame, 0x2000);

    let decoded = decode(LinkType::ETHERNET, &frame, false).unwrap();
    let Decoded::NotDecoded { what } = decoded else {
        panic!("a first fragment was decoded as a whole segment: {decoded:?}")
    };
    assert!(
        matches!(
            what,
            NotDecoded::Fragmented {
                offset: 0,
                more_fragments: true
            }
        ),
        "{what:?}"
    );
    assert!(
        what.to_string().contains("only part of its payload"),
        "{what}"
    );
}

#[test]
fn an_unfragmented_packet_is_still_decoded_however_its_flags_are_set() {
    // Don't-fragment is set on a great deal of ordinary traffic, and it does
    // not make a packet a fragment. Refusing those would be worse than the bug
    // this fix is for.
    for word in [0x0000_u16, 0x4000] {
        let mut frame = ethernet(0x0800, &ipv4_tcp(b"whole", 0, 0));
        with_fragment_word(&mut frame, word);
        let segment = tcp_of(decode(LinkType::ETHERNET, &frame, false).unwrap());
        assert_eq!(segment.payload, b"whole", "flags 0x{word:04X}");
    }
}
