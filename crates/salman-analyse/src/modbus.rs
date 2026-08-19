// SPDX-License-Identifier: Apache-2.0
//! What happened on a Modbus TCP capture.
//!
//! # The transaction is the unit, not the frame
//!
//! Almost every question worth asking about Modbus is about a request paired
//! with its response, or about a request with no response. A tool that reports
//! per frame makes the reader do the pairing, and pairing is where the
//! interesting failures live: a late answer, an answer to a question that was
//! given up on, a request nobody answered.
//!
//! Both directions are decoded, which matters more here than for the protocols
//! most capture tools were built for: in industrial protocols the interesting
//! information is usually in the **response**.
//!
//! # Which streams are Modbus
//!
//! Whichever ones use the port salman was told to look at, 502 by default.
//! There is nothing in a TCP stream that says "this is Modbus", and salman
//! does not guess: it says which port it looked at, and a finding suggests
//! trying another when it saw traffic it did not classify.

use std::collections::BTreeMap;

use salman_capture::frame::{Decoded, Endpoint, decode};
use salman_capture::pcap::{CaptureError, Reader};
use salman_capture::reassemble::{Note, Reassembler};
use salman_findings::evidence::{Artifact, Evidence, Observed, TransactionRef};
use salman_findings::finding::{
    Confidence, Dedup, DedupScope, Finding, Group, Justification, NextCheck, Severity,
};
use salman_modbus::pdu::{Request, Response};
use salman_modbus::tcp::{Framer, TcpAdu};

/// Where salman looks for Modbus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// The TCP port that carries Modbus.
    pub port: u16,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            port: salman_modbus::limits::TCP_PORT,
        }
    }
}

/// What one request looked like while it was outstanding.
#[derive(Debug, Clone)]
struct Outstanding {
    request: Request,
    frame: u64,
    timestamp: u64,
    unit: u8,
    repeats: u32,
}

/// What a capture turned out to contain.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    /// Everything salman is willing to say about it.
    pub findings: Vec<Finding>,
    /// How many frames were read.
    pub frames: u64,
    /// How many Modbus application data units were assembled.
    pub adus: u64,
    /// How many requests were paired with a response.
    pub paired: u64,
    /// How many TCP streams were seen that did not use the Modbus port.
    pub other_streams: usize,
}

/// The crate that made these claims, recorded on every finding.
const SOURCE: &str = "salman-analyse::modbus";

/// Reads a capture and says what happened on it.
///
/// # Errors
///
/// Returns [`CaptureError`] if the file is not a capture salman reads. A frame
/// salman cannot decode is not an error — it is a finding, or nothing at all.
pub fn analyse_capture(
    name: &str,
    bytes: &[u8],
    options: Options,
) -> Result<Analysis, CaptureError> {
    let artifact = Artifact::file(name, bytes);
    let mut reader = Reader::new(bytes)?;
    let link = reader.link_type();
    let scale = reader.scale();

    let mut analysis = Analysis::default();
    let mut reassembler = Reassembler::new();
    // One framer per direction, because each direction is its own byte stream.
    let mut framers: BTreeMap<(Endpoint, Endpoint), Framer> = BTreeMap::new();
    // Outstanding requests, keyed by the connection and the transaction id —
    // which is the only thing that pairs a response with its request.
    let mut outstanding: BTreeMap<((Endpoint, Endpoint), u16), Outstanding> = BTreeMap::new();
    let mut other_streams: BTreeMap<(Endpoint, Endpoint), u64> = BTreeMap::new();
    let mut mid_stream_reported = false;
    // How many units each direction has produced, and whether salman has
    // already given up on it. Both are needed to say the right thing when
    // framing fails: a stream that never framed anything is probably not
    // Modbus at all, and one that framed something and then broke is a fault.
    let mut framed_ok: BTreeMap<(Endpoint, Endpoint), u64> = BTreeMap::new();
    let mut abandoned: BTreeMap<(Endpoint, Endpoint), bool> = BTreeMap::new();

    loop {
        let record = match reader.next_record() {
            Ok(Some(record)) => record,
            Ok(None) => break,
            Err(error) => {
                analysis.findings.push(
                    Finding::cannot_determine(
                        "capture.record.unreadable",
                        SOURCE,
                        Group::Malformed,
                        Justification::BytesMissingFromCapture,
                        format!("the capture stops here: {error}"),
                        Evidence::from(artifact.clone()),
                    )
                    .suggesting(NextCheck::CaptureWith {
                        hint: "the file may have been truncated while it was being written"
                            .to_string(),
                    }),
                );
                break;
            }
        };
        analysis.frames += 1;
        let timestamp = record.nanos(scale);

        let segment = match decode(link, record.data, record.truncated) {
            Ok(Decoded::Tcp(segment)) => segment,
            Ok(Decoded::NotDecoded { .. }) => continue,
            Err(_) => continue,
        };

        if segment.source.port != options.port && segment.destination.port != options.port {
            *other_streams.entry(segment.connection()).or_insert(0) += 1;
            continue;
        }

        let key = (segment.source, segment.destination);
        let delivery = reassembler.push(&segment);

        for note in &delivery.notes {
            if let Some(finding) = note_finding(
                note,
                &artifact,
                record.index,
                timestamp,
                &mut mid_stream_reported,
            ) {
                analysis.findings.push(finding);
            }
        }

        if abandoned.get(&key).copied().unwrap_or(false) {
            // Framing on this direction is lost for good, and saying so once
            // per frame would bury everything else in the report.
            continue;
        }
        let framer = framers.entry(key).or_default();
        let mut rest = &delivery.bytes[..];
        loop {
            let (used, outcome) = framer.advance(rest);
            rest = rest.get(used..).unwrap_or(&[]);
            match outcome {
                Ok(Some(adu)) => {
                    analysis.adus += 1;
                    *framed_ok.entry(key).or_insert(0) += 1;
                    handle_adu(
                        &adu,
                        &segment.source,
                        &segment.destination,
                        record.index,
                        timestamp,
                        &artifact,
                        options,
                        &mut outstanding,
                        &mut analysis,
                    );
                }
                Ok(None) => break,
                Err(error) => {
                    abandoned.insert(key, true);
                    let framed = framed_ok.get(&key).copied().unwrap_or(0);
                    analysis.findings.push(
                        if framed == 0 {
                            // Nothing on this stream ever framed as Modbus. The
                            // honest reading is that it is not Modbus, not that
                            // Modbus broke — a person who pointed salman at the
                            // wrong port should be told that, not handed a wall of
                            // confident framing errors about somebody's HTTP.
                            Finding::cannot_determine(
                                "mbtcp.stream.not_modbus",
                                SOURCE,
                                Group::Assumption,
                                Justification::ProtocolAssumedFromPort,
                                format!(
                                    "{} carries something that is not Modbus TCP: {error}. \
                                 Nothing on this stream ever framed, so salman was probably \
                                 pointed at the wrong port rather than watching a fault",
                                    segment.source
                                ),
                                Evidence::from(artifact.clone())
                                    .at_frame(record.index)
                                    .at_time(timestamp)
                                    .with_bytes(rest.get(..rest.len().min(16)).unwrap_or(&[])),
                            )
                            .suggesting(NextCheck::RerunWith {
                                flag: "--modbus-port".to_string(),
                                value: "the port this device actually uses".to_string(),
                            })
                        } else {
                            // It framed cleanly and then stopped. That is a fault.
                            Finding::fail(
                                "mbtcp.framing.lost",
                                SOURCE,
                                Group::Framing,
                                Severity::Error,
                                Confidence::Certain,
                                format!(
                                    "{} framed {framed} unit(s) and then lost framing: {error}. \
                                 A Modbus TCP stream carries no sync word, so nothing after \
                                 this point on this stream can be read",
                                    segment.source
                                ),
                                Evidence::from(artifact.clone())
                                    .at_frame(record.index)
                                    .at_time(timestamp)
                                    .with_bytes(rest.get(..rest.len().min(16)).unwrap_or(&[])),
                            )
                            .suggesting(NextCheck::InspectFrame {
                                frame: record.index,
                            })
                        }
                        .deduplicated(Dedup::per(
                            format!("{} -> {}", segment.source, segment.destination),
                            DedupScope::PerConnection,
                        )),
                    );
                    break;
                }
            }
        }
    }

    // Whatever is still outstanding was never answered inside the capture.
    for ((_, transaction), pending) in outstanding {
        analysis
            .findings
            .push(unanswered(&artifact, transaction, &pending));
    }

    analysis.other_streams = other_streams.len();
    if analysis.adus == 0 && !other_streams.is_empty() {
        // Wireshark's Modbus dissector has exactly this finding, and it is the
        // most useful one it has: the reader almost always has the right
        // capture and the wrong port.
        analysis.findings.push(
            Finding::cannot_determine(
                "mbtcp.no_traffic_on_port",
                SOURCE,
                Group::Assumption,
                Justification::ProtocolAssumedFromPort,
                format!(
                    "nothing on port {} in this capture, and {} other TCP stream(s) were seen",
                    options.port,
                    other_streams.len()
                ),
                Evidence::from(artifact.clone()),
            )
            .suggesting(NextCheck::RerunWith {
                flag: "--modbus-port".to_string(),
                value: "the port this device actually uses".to_string(),
            }),
        );
    }

    if analysis.paired > 0 {
        // Proof of coverage. Without it, "nothing wrong here" and "salman did
        // not look" render identically, which is to say as nothing at all.
        analysis.findings.push(Finding::pass(
            "mbtcp.transactions.paired",
            SOURCE,
            Group::Protocol,
            format!(
                "{} request(s) were paired with a response and decoded",
                analysis.paired
            ),
            Evidence::from(artifact.clone()),
        ));
    }

    Ok(analysis)
}

/// Turns a stream observation into a finding, where it deserves one.
fn note_finding(
    note: &Note,
    artifact: &Artifact,
    frame: u64,
    timestamp: u64,
    mid_stream_reported: &mut bool,
) -> Option<Finding> {
    match note {
        Note::MidStream { base } => {
            // Once per capture: it is a fact about the capture, not about each
            // stream, and repeating it teaches nobody anything.
            if core::mem::replace(mid_stream_reported, true) {
                return None;
            }
            Some(
                Finding::cannot_determine(
                    "tcp.stream.started_mid_connection",
                    SOURCE,
                    Group::Sequence,
                    Justification::StreamStartedMidConnection,
                    format!(
                        "the capture joined a connection already in progress at sequence \
                         {base}; whatever the device sent before this is not here"
                    ),
                    Evidence::from(artifact.clone())
                        .at_frame(frame)
                        .at_time(timestamp),
                )
                .suggesting(NextCheck::CaptureWith {
                    hint: "start the capture before the client connects to see the whole \
                           conversation"
                        .to_string(),
                }),
            )
        }
        Note::Gap { from, bytes } => Some(
            Finding::cannot_determine(
                "tcp.stream.gap",
                SOURCE,
                Group::Sequence,
                Justification::BytesMissingFromCapture,
                format!(
                    "{bytes} bytes from sequence {from} never reached this file, so whatever \
                     they carried cannot be decoded"
                ),
                Evidence::from(artifact.clone())
                    .at_frame(frame)
                    .at_time(timestamp),
            )
            .suggesting(NextCheck::CaptureWith {
                hint: "the capturing tool was probably dropping packets; capture on a quieter \
                       link or with a larger buffer"
                    .to_string(),
            }),
        ),
        Note::OverlapDisagreed { sequence, bytes } => Some(Finding::open(
            "tcp.stream.overlap_disagreed",
            SOURCE,
            Group::Sequence,
            Confidence::Certain,
            Justification::NotObservableFromThisSource,
            format!(
                "{bytes} bytes at sequence {sequence} were sent again with different contents. \
                 salman kept what it had already delivered; which bytes the device acted on \
                 cannot be told from here"
            ),
            Evidence::from(artifact.clone())
                .at_frame(frame)
                .at_time(timestamp),
        )),
        // Ordinary on any real network, and reporting them as problems is how
        // a diagnostic tool loses the reader's trust. `Unverified` is among
        // them: bytes arriving again from further back than salman's window is
        // a limit of salman's, not a fault of the device's, and it does not
        // affect the stream that was delivered.
        Note::Retransmission { .. }
        | Note::Duplicate { .. }
        | Note::Unverified { .. }
        | Note::OutOfOrder { .. }
        | Note::Finished
        | Note::Reset => None,
    }
}

/// Handles one assembled application data unit.
#[allow(clippy::too_many_arguments)]
fn handle_adu(
    adu: &TcpAdu,
    source: &Endpoint,
    destination: &Endpoint,
    frame: u64,
    timestamp: u64,
    artifact: &Artifact,
    options: Options,
    outstanding: &mut BTreeMap<((Endpoint, Endpoint), u16), Outstanding>,
    analysis: &mut Analysis,
) {
    check_declared_length(adu, artifact, frame, timestamp, analysis);

    let transaction = adu.header.transaction;
    let to_server = destination.port == options.port;

    if to_server {
        let key = ((*source, *destination), transaction);
        match Request::decode(adu.pdu.as_bytes()) {
            Ok(request) => {
                if let Some(previous) = outstanding.get_mut(&key) {
                    // The same identifier, outstanding again, before anything
                    // answered it. pymodbus reuses one identifier across every
                    // retry, so this is what a retry burst looks like from that
                    // client — not ten lost requests.
                    previous.repeats += 1;
                    if previous.repeats == 1 {
                        analysis.findings.push(
                            Finding::open(
                                "mbtcp.request.repeated",
                                SOURCE,
                                Group::Sequence,
                                Confidence::Probable,
                                Justification::ResponseNotCaptured,
                                format!(
                                    "transaction {transaction} was sent again before anything \
                                     answered it, which is what a client's retry looks like \
                                     when it reuses the identifier"
                                ),
                                Evidence::from(artifact.clone())
                                    .at_frame(frame)
                                    .at_time(timestamp)
                                    .about(TransactionRef {
                                        transaction,
                                        unit: adu.header.unit,
                                        request_frame: Some(previous.frame),
                                        response_frame: None,
                                    }),
                            )
                            .deduplicated(Dedup::per(
                                format!("transaction {transaction}"),
                                DedupScope::PerTransaction,
                            )),
                        );
                    }
                    return;
                }
                outstanding.insert(
                    key,
                    Outstanding {
                        request,
                        frame,
                        timestamp,
                        unit: adu.header.unit,
                        repeats: 0,
                    },
                );
            }
            Err(error) => analysis.findings.push(Finding::fail(
                "mbtcp.request.undecodable",
                SOURCE,
                Group::Protocol,
                Severity::Warning,
                Confidence::Certain,
                format!("a request salman could not read: {error}"),
                Evidence::from(artifact.clone())
                    .at_frame(frame)
                    .at_time(timestamp)
                    .with_bytes(adu.pdu.as_bytes()),
            )),
        }
        return;
    }

    // A response. Its request went the other way, so the key is reversed.
    let key = ((*destination, *source), transaction);
    let Some(pending) = outstanding.remove(&key) else {
        analysis.findings.push(Finding::cannot_determine(
            "mbtcp.response.unmatched",
            SOURCE,
            Group::Sequence,
            Justification::RequestNotCaptured,
            format!(
                "a response to transaction {transaction} arrived and the request it answers \
                 is not in this capture, so what it means cannot be decoded: a read response \
                 carries a byte count and never the quantity"
            ),
            Evidence::from(artifact.clone())
                .at_frame(frame)
                .at_time(timestamp)
                .with_bytes(adu.pdu.as_bytes())
                .about(TransactionRef {
                    transaction,
                    unit: adu.header.unit,
                    request_frame: None,
                    response_frame: Some(frame),
                }),
        ));
        return;
    };

    let reference = TransactionRef {
        transaction,
        unit: pending.unit,
        request_frame: Some(pending.frame),
        response_frame: Some(frame),
    };

    match Response::decode(adu.pdu.as_bytes(), &pending.request) {
        Ok(Response::Exception { function, code }) => {
            analysis.paired += 1;
            analysis.findings.push(
                Finding::fail(
                    "mbtcp.exception",
                    SOURCE,
                    Group::Protocol,
                    Severity::Warning,
                    Confidence::Certain,
                    format!("unit {} refused {function}: {code}", pending.unit),
                    Evidence::from(artifact.clone())
                        .at_frame(frame)
                        .at_time(timestamp)
                        .with_bytes(adu.pdu.as_bytes())
                        .about(reference)
                        .observing(Observed::new(
                            format!(
                                "{} items from address {}",
                                pending.request.quantity(),
                                pending.request.start()
                            ),
                            format!("{code}"),
                        )),
                )
                .deduplicated(Dedup::per(
                    format!(
                        "unit {} {} {}..{}",
                        pending.unit,
                        function.0,
                        pending.request.start(),
                        u32::from(pending.request.start()) + u32::from(pending.request.quantity())
                    ),
                    DedupScope::PerRegisterRange,
                )),
            );
        }
        Ok(_) => analysis.paired += 1,
        Err(error) => analysis.findings.push(Finding::fail(
            "mbtcp.response.undecodable",
            SOURCE,
            Group::Protocol,
            Severity::Warning,
            Confidence::Certain,
            format!(
                "the answer to transaction {transaction} does not decode against the request \
                 it answers: {error}"
            ),
            Evidence::from(artifact.clone())
                .at_frame(frame)
                .at_time(timestamp)
                .with_bytes(adu.pdu.as_bytes())
                .about(reference),
        )),
    }
}

/// Checks the MBAP length against the unit that followed it.
fn check_declared_length(
    adu: &TcpAdu,
    artifact: &Artifact,
    frame: u64,
    timestamp: u64,
    analysis: &mut Analysis,
) {
    let declared = adu.header.claimed_pdu_len();
    if declared == Some(adu.pdu.len()) {
        return;
    }
    analysis.findings.push(Finding::fail(
        "mbtcp.length.disagrees_with_pdu",
        SOURCE,
        Group::Framing,
        Severity::Error,
        Confidence::Certain,
        "the MBAP length field and the protocol data unit behind it disagree",
        Evidence::from(artifact.clone())
            .at_frame(frame)
            .at_time(timestamp)
            .with_bytes(&adu.header.to_bytes())
            .observing(Observed::new(
                format!("{} bytes", adu.pdu.len() + 1),
                format!("{}", adu.header.length),
            )),
    ));
}

/// The finding for a request nothing answered.
fn unanswered(artifact: &Artifact, transaction: u16, pending: &Outstanding) -> Finding {
    Finding::cannot_determine(
        "mbtcp.request.unanswered",
        SOURCE,
        Group::Sequence,
        Justification::ResponseNotCaptured,
        format!(
            "unit {} was asked for {} items from address {} and no answer to transaction \
             {transaction} is in this capture. Whether the device never answered or the \
             capture ended first cannot be told from here",
            pending.unit,
            pending.request.quantity(),
            pending.request.start()
        ),
        Evidence::from(artifact.clone())
            .at_frame(pending.frame)
            .at_time(pending.timestamp)
            .about(TransactionRef {
                transaction,
                unit: pending.unit,
                request_frame: Some(pending.frame),
                response_frame: None,
            }),
    )
    .suggesting(NextCheck::CaptureWith {
        hint: "let the capture run past the last request to see whether an answer arrives"
            .to_string(),
    })
}
