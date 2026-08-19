// SPDX-License-Identifier: Apache-2.0
//! The finding itself.

use core::fmt;
use core::time::Duration;

use crate::evidence::Evidence;

/// What kind of claim a finding is.
///
/// Separate from how bad it is, because they are genuinely different
/// questions. A tool that has only "error, warning, info" cannot say "I
/// checked this and it conformed", and a tool that cannot say that has no way
/// to distinguish "nothing is wrong here" from "I did not look".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// salman asserts a fault.
    Fail,
    /// salman checked this and it conformed. Proof of coverage.
    Pass,
    /// This check does not apply here.
    NotApplicable,
    /// salman found something and cannot decide. A person must.
    Open,
    /// salman could not check. The justification says why.
    CannotDetermine,
    /// Worth knowing and not a judgement.
    Informational,
}

impl Kind {
    /// Whether this kind asserts that something is wrong.
    #[must_use]
    pub const fn asserts_a_fault(self) -> bool {
        matches!(self, Self::Fail)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Fail => "fail",
            Self::Pass => "pass",
            Self::NotApplicable => "not applicable",
            Self::Open => "open",
            Self::CannotDetermine => "cannot determine",
            Self::Informational => "informational",
        })
    }
}

/// How bad an asserted fault is.
///
/// There is no `None`. A severity belongs to an assertion of fault and to
/// nothing else, and leaving the variant out is what makes a `Pass` with a
/// severity of `Error` impossible to write down rather than merely forbidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Worth mentioning.
    Note,
    /// Likely to matter.
    Warning,
    /// Certain to matter.
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Note => "note",
            Self::Warning => "warning",
            Self::Error => "error",
        })
    }
}

/// What sort of thing was observed.
///
/// The last four are the honesty channel, and they are the most transferable
/// idea in this whole model: four of Wireshark's seventeen groups exist purely
/// so a dissector can confess its own limits. A tool that cannot say "I did
/// not decode this" says nothing instead, and silence reads as "nothing was
/// there".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// A checksum did not match.
    Checksum,
    /// Something about where frames begin and end.
    Framing,
    /// A protocol rule was broken.
    Protocol,
    /// Ordering, duplication, retransmission.
    Sequence,
    /// How long something took.
    Timing,
    /// An address, or a range of them.
    Addressing,
    /// A value that is out of range or unexpected.
    Value,

    // -- what salman says about itself --
    /// salman's decoder is incomplete for this construct.
    Undecoded,
    /// salman decoded this using an assumption, which is named.
    Assumption,
    /// salman's decoder gave up on these bytes.
    Malformed,
    /// This is salman's fault rather than the wire's.
    DecoderBug,
}

impl Group {
    /// Whether this group is salman describing its own limits rather than the
    /// traffic.
    #[must_use]
    pub const fn is_about_salman(self) -> bool {
        matches!(
            self,
            Self::Undecoded | Self::Assumption | Self::Malformed | Self::DecoderBug
        )
    }

    /// The name used in reports and in filters.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Checksum => "checksum",
            Self::Framing => "framing",
            Self::Protocol => "protocol",
            Self::Sequence => "sequence",
            Self::Timing => "timing",
            Self::Addressing => "addressing",
            Self::Value => "value",
            Self::Undecoded => "undecoded",
            Self::Assumption => "assumption",
            Self::Malformed => "malformed",
            Self::DecoderBug => "decoder-bug",
        }
    }
}

impl fmt::Display for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How sure salman is.
///
/// First-class, and the reason this model is not SARIF: that schema has no way
/// to express it. A diagnostic tool for industrial networks that cannot say "I
/// think so" has to either overstate or stay silent, and both are worse than
/// saying how sure it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confidence {
    /// The bytes settle it. A CRC either matched or it did not.
    Certain,
    /// Very likely, and something salman cannot see could explain it.
    Probable,
    /// Consistent with what was seen, and so are other explanations.
    Possible,
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Certain => "certain",
            Self::Probable => "probable",
            Self::Possible => "possible",
        })
    }
}

/// Why a finding is not an assertion of fault.
///
/// A closed list, deliberately. The mechanism that stops salman reporting "no
/// framing errors detected" when the truth is "inter-frame timing cannot be
/// measured from a TCP capture" is that there is no free-text option: a
/// non-fault finding has to name one of these, and every one of them is a real
/// limit somebody can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Justification {
    /// The thing being checked cannot be seen from this kind of source — RTU
    /// inter-frame timing from a TCP capture, for instance.
    NotObservableFromThisSource,
    /// The capture kept too few bytes of the frame.
    StreamTruncatedBySnaplen,
    /// Bytes that were on the wire are missing from the capture.
    ///
    /// Different from a snapshot length, which cuts a frame short: this is a
    /// frame that never reached the file at all, which happens when a
    /// capturing tool drops packets under load.
    BytesMissingFromCapture,
    /// The capture joined the connection after it had started.
    StreamStartedMidConnection,
    /// salman does not implement this yet.
    FeatureNotImplemented,
    /// salman assumed which protocol these bytes are, from the port they were
    /// on, and nothing in the bytes confirms it.
    ///
    /// There is nothing in a TCP stream that says what protocol it carries.
    /// Every tool that decodes one has made this assumption; the difference is
    /// whether it says so.
    ProtocolAssumedFromPort,
    /// A response was seen and its request was not.
    RequestNotCaptured,
    /// A request was seen and no response was.
    ResponseNotCaptured,
    /// The transport does not carry the field — there is no CRC on Modbus TCP.
    TransportDoesNotCarryField,
    /// The device never said whether it supports this.
    DeviceDidNotAdvertiseCapability,
}

impl Justification {
    /// What it means, in a sentence a person can act on.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::NotObservableFromThisSource => "this cannot be seen from a capture of this kind",
            Self::StreamTruncatedBySnaplen => "the capture kept too few bytes of the frame to tell",
            Self::BytesMissingFromCapture => {
                "bytes that were on the wire are missing from the capture"
            }
            Self::StreamStartedMidConnection => {
                "the capture joined this connection after it had started"
            }
            Self::FeatureNotImplemented => "salman does not implement this check yet",
            Self::ProtocolAssumedFromPort => {
                "salman assumed these bytes are Modbus because of the port they are on, and \
                 nothing in the bytes themselves says so"
            }
            Self::RequestNotCaptured => "the request this answers was not captured",
            Self::ResponseNotCaptured => "no response to this was captured",
            Self::TransportDoesNotCarryField => {
                "this transport does not carry the field being checked"
            }
            Self::DeviceDidNotAdvertiseCapability => {
                "the device never said whether it supports this"
            }
        }
    }
}

impl fmt::Display for Justification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.explanation())
    }
}

/// What to do next, as something executable rather than a sentence.
///
/// Wireshark's Modbus dissector already does this — `mbtcp.cannot_classify`
/// tells the reader to set the port preference — and it is the difference
/// between a finding a person can act on and one they have to think about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextCheck {
    /// Run salman again with a different setting.
    RerunWith {
        /// The flag, as it is typed.
        flag: String,
        /// What to set it to.
        value: String,
    },
    /// Capture again, differently.
    CaptureWith {
        /// What to change about the capture.
        hint: String,
    },
    /// Look at a particular frame.
    InspectFrame {
        /// Which one.
        frame: u64,
    },
    /// Compare against something else.
    CompareAgainst {
        /// What.
        oracle: String,
    },
    /// Read a specific part of a specification.
    ConsultSpecification {
        /// Which document.
        document: &'static str,
        /// Which section.
        section: &'static str,
    },
}

impl fmt::Display for NextCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RerunWith { flag, value } => write!(f, "run again with {flag} {value}"),
            Self::CaptureWith { hint } => write!(f, "capture again: {hint}"),
            Self::InspectFrame { frame } => write!(f, "look at frame {frame}"),
            Self::CompareAgainst { oracle } => write!(f, "compare against {oracle}"),
            Self::ConsultSpecification { document, section } => {
                write!(f, "see {document} {section}")
            }
        }
    }
}

/// What makes two findings the same finding.
///
/// Part of the model rather than a detail of the reporter, because a device
/// that misbehaves produces the same finding ten thousand times, and
/// retrofitting deduplication after people have seen ten-thousand-line reports
/// is painful.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dedup {
    /// What identity to collapse on, beyond the finding's id.
    pub key: String,
    /// How widely to collapse.
    pub scope: DedupScope,
    /// How long a window to collapse over, if any.
    pub window: Option<Duration>,
}

impl Dedup {
    /// Report this once, whatever else happens.
    #[must_use]
    pub fn once(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            scope: DedupScope::Once,
            window: None,
        }
    }

    /// Report this once per whatever `scope` names.
    #[must_use]
    pub fn per(key: impl Into<String>, scope: DedupScope) -> Self {
        Self {
            key: key.into(),
            scope,
            window: None,
        }
    }
}

/// How widely a finding is collapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DedupScope {
    /// Once for the whole run.
    Once,
    /// Once per connection.
    PerConnection,
    /// Once per unit identifier.
    PerUnit,
    /// Once per range of registers.
    PerRegisterRange,
    /// Once per exchange.
    PerTransaction,
}

/// One claim about decoded bytes.
///
/// **The fields are private and the constructors are the only way to build
/// one.** That is what makes the model's two rules structural rather than
/// conventional: only [`Finding::fail`] takes a severity, and every
/// constructor for a kind that is not an assertion of fault requires a
/// [`Justification`].
///
/// They were public once, and review pointed out that the documentation's
/// claim — that a `Pass` carrying a severity of `Error` "cannot be written
/// down" — was simply untrue while a struct literal could write it down. A
/// rule a caller can step around is a convention, and this file called it
/// something stronger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    id: &'static str,
    kind: Kind,
    severity: Option<Severity>,
    group: Group,
    confidence: Confidence,
    justification: Option<Justification>,
    evidence: Evidence,
    next_check: Option<NextCheck>,
    dedup: Dedup,
    source: &'static str,
    message: String,
}

impl Finding {
    /// Asserts a fault.
    ///
    /// The only constructor that takes a severity, which is what makes a
    /// `Pass` with a severity impossible rather than merely forbidden.
    #[must_use]
    pub fn fail(
        id: &'static str,
        source: &'static str,
        group: Group,
        severity: Severity,
        confidence: Confidence,
        message: impl Into<String>,
        evidence: Evidence,
    ) -> Self {
        Self {
            id,
            kind: Kind::Fail,
            severity: Some(severity),
            group,
            confidence,
            justification: None,
            evidence,
            next_check: None,
            dedup: Dedup::once(id),
            source,
            message: message.into(),
        }
    }

    /// Records that a check ran and the traffic conformed.
    ///
    /// Worth emitting: it is the difference between "nothing is wrong here"
    /// and "salman did not look", and a report of only failures cannot tell
    /// the two apart.
    #[must_use]
    pub fn pass(
        id: &'static str,
        source: &'static str,
        group: Group,
        message: impl Into<String>,
        evidence: Evidence,
    ) -> Self {
        Self {
            id,
            kind: Kind::Pass,
            severity: None,
            group,
            confidence: Confidence::Certain,
            justification: None,
            evidence,
            next_check: None,
            dedup: Dedup::once(id),
            source,
            message: message.into(),
        }
    }

    /// Records something found that salman will not decide about.
    #[must_use]
    pub fn open(
        id: &'static str,
        source: &'static str,
        group: Group,
        confidence: Confidence,
        justification: Justification,
        message: impl Into<String>,
        evidence: Evidence,
    ) -> Self {
        Self {
            id,
            kind: Kind::Open,
            severity: None,
            group,
            confidence,
            justification: Some(justification),
            evidence,
            next_check: None,
            dedup: Dedup::once(id),
            source,
            message: message.into(),
        }
    }

    /// Records that salman could not check, and why.
    #[must_use]
    pub fn cannot_determine(
        id: &'static str,
        source: &'static str,
        group: Group,
        justification: Justification,
        message: impl Into<String>,
        evidence: Evidence,
    ) -> Self {
        Self {
            id,
            kind: Kind::CannotDetermine,
            severity: None,
            group,
            confidence: Confidence::Certain,
            justification: Some(justification),
            evidence,
            next_check: None,
            dedup: Dedup::once(id),
            source,
            message: message.into(),
        }
    }

    /// Records that a check does not apply here, and why.
    #[must_use]
    pub fn not_applicable(
        id: &'static str,
        source: &'static str,
        group: Group,
        justification: Justification,
        message: impl Into<String>,
        evidence: Evidence,
    ) -> Self {
        Self {
            id,
            kind: Kind::NotApplicable,
            severity: None,
            group,
            confidence: Confidence::Certain,
            justification: Some(justification),
            evidence,
            next_check: None,
            dedup: Dedup::once(id),
            source,
            message: message.into(),
        }
    }

    /// Records something worth knowing that is not a judgement.
    #[must_use]
    pub fn informational(
        id: &'static str,
        source: &'static str,
        group: Group,
        message: impl Into<String>,
        evidence: Evidence,
    ) -> Self {
        Self {
            id,
            kind: Kind::Informational,
            severity: None,
            group,
            confidence: Confidence::Certain,
            justification: None,
            evidence,
            next_check: None,
            dedup: Dedup::once(id),
            source,
            message: message.into(),
        }
    }

    /// Its stable dotted identifier, such as `mbtcp.length.disagrees_with_pdu`.
    ///
    /// This is API. Filters in someone's build server and suppressions in
    /// someone's configuration file depend on it, so renaming one is a
    /// breaking change.
    #[must_use]
    pub const fn id(&self) -> &'static str {
        self.id
    }

    /// What kind of claim it is.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// How bad, when it is an assertion of fault, and `None` otherwise.
    #[must_use]
    pub const fn severity(&self) -> Option<Severity> {
        self.severity
    }

    /// What sort of thing was observed.
    #[must_use]
    pub const fn group(&self) -> Group {
        self.group
    }

    /// How sure salman is.
    #[must_use]
    pub const fn confidence(&self) -> Confidence {
        self.confidence
    }

    /// Why this is not an assertion of fault, when it is not.
    #[must_use]
    pub const fn justification(&self) -> Option<Justification> {
        self.justification
    }

    /// What a person needs to check the claim.
    #[must_use]
    pub const fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    /// What to do next, if salman has a suggestion.
    #[must_use]
    pub const fn next_check(&self) -> Option<&NextCheck> {
        self.next_check.as_ref()
    }

    /// How repeats of this finding are collapsed.
    #[must_use]
    pub const fn dedup(&self) -> &Dedup {
        &self.dedup
    }

    /// Which part of salman made the claim.
    ///
    /// It matters when the decoder is the thing that is wrong.
    #[must_use]
    pub const fn source(&self) -> &'static str {
        self.source
    }

    /// One sentence, for a person.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Says what to do next.
    #[must_use]
    pub fn suggesting(mut self, next: NextCheck) -> Self {
        self.next_check = Some(next);
        self
    }

    /// Sets how repeats are collapsed.
    #[must_use]
    pub fn deduplicated(mut self, dedup: Dedup) -> Self {
        self.dedup = dedup;
        self
    }

    /// Whether the two structural rules hold.
    ///
    /// They hold by construction now that the fields are private, and this
    /// remains so a test can say so rather than a reader having to take the
    /// constructors on trust.
    #[must_use]
    pub const fn is_well_formed(&self) -> bool {
        let severity_ok = if self.kind.asserts_a_fault() {
            self.severity.is_some()
        } else {
            self.severity.is_none()
        };
        let justification_ok = match self.kind {
            Kind::Open | Kind::CannotDetermine | Kind::NotApplicable => {
                self.justification.is_some()
            }
            Kind::Fail | Kind::Pass | Kind::Informational => true,
        };
        severity_ok && justification_ok
    }
}

impl fmt::Display for Finding {
    /// One line, then what supports it.
    ///
    /// Plain text, no colour, in the same style as salman's compiler
    /// diagnostics — so that a golden test can compare bytes and meaning never
    /// depends on a colour a reader may not see.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.severity {
            Some(severity) => write!(f, "{severity}[{}]", self.id)?,
            None => write!(f, "{}[{}]", self.kind, self.id)?,
        }
        write!(f, ": {}", self.message)?;

        writeln!(f)?;
        write!(f, "  in {}", self.evidence.artifact)?;
        if let Some(frame) = self.evidence.frame {
            write!(f, ", frame {frame}")?;
        }
        if let Some(offset) = self.evidence.offset {
            write!(f, ", byte {offset}")?;
        }
        if let Some(transaction) = &self.evidence.transaction {
            write!(
                f,
                ", transaction {} unit {}",
                transaction.transaction, transaction.unit
            )?;
        }

        if let Some(observed) = &self.evidence.observed {
            write!(f, "\n  {observed}")?;
        }
        if !self.evidence.bytes.is_empty() {
            write!(f, "\n  bytes: {:02X?}", self.evidence.bytes)?;
        }
        if let Some(justification) = self.justification {
            write!(f, "\n  because {justification}")?;
        }
        write!(f, "\n  confidence: {}", self.confidence)?;
        if self.group.is_about_salman() {
            write!(f, " ({} — this is about salman, not the wire)", self.group)?;
        } else {
            write!(f, " ({})", self.group)?;
        }
        if let Some(next) = &self.next_check {
            write!(f, "\n  next: {next}")?;
        }
        Ok(())
    }
}
