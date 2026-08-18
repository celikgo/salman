// SPDX-License-Identifier: Apache-2.0
//! What a finding points at.
//!
//! Evidence here means what it means in ordinary use: the actual bytes, where
//! they were, and what salman expected instead. A finding that says "the
//! length field is wrong" without saying which frame, at which byte offset,
//! with what in it, is not a finding — it is an opinion, and the reader has no
//! way to check it.

use core::fmt;

/// The thing a finding's bytes came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// How it is named — a file path, or a description of a live connection.
    pub name: String,
    /// The SHA-256 of the file, if it was a file.
    ///
    /// A finding about a capture that cannot say *which* capture is a finding
    /// nobody can reproduce, and a path is not enough: files get edited.
    pub sha256: Option<[u8; 32]>,
    /// How large it was, in bytes, if that is known.
    pub length: Option<u64>,
}

impl Artifact {
    /// An artefact known only by name — a live connection, say.
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sha256: None,
            length: None,
        }
    }

    /// A file, identified by its contents as well as its name.
    #[must_use]
    pub fn file(name: impl Into<String>, bytes: &[u8]) -> Self {
        Self {
            name: name.into(),
            sha256: Some(salman_core::hash::sha256(bytes)),
            length: Some(bytes.len() as u64),
        }
    }

    /// The hash in the form a person can compare, if there is one.
    #[must_use]
    pub fn sha256_hex(&self) -> Option<String> {
        self.sha256.map(|digest| {
            let mut out = String::with_capacity(64);
            for byte in digest {
                use core::fmt::Write;
                let _ = write!(out, "{byte:02x}");
            }
            out
        })
    }
}

impl fmt::Display for Artifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        if let Some(hash) = self.sha256_hex() {
            // The first eight characters, which is enough to tell two captures
            // apart and short enough to read.
            write!(f, " ({})", hash.get(..8).unwrap_or(&hash))?;
        }
        Ok(())
    }
}

/// Which Modbus exchange a finding is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionRef {
    /// The transaction identifier from the MBAP header.
    pub transaction: u16,
    /// The unit identifier.
    pub unit: u8,
    /// The frame the request was in, if it was captured.
    pub request_frame: Option<u64>,
    /// The frame the response was in, if it was captured.
    pub response_frame: Option<u64>,
}

/// What salman expected and what it found.
///
/// Two values rendered for a person, because a finding that says only "the
/// length is wrong" makes the reader go and look it up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    /// What the rule required.
    pub expected: String,
    /// What was there.
    pub actual: String,
}

impl Observed {
    /// An expectation and what contradicted it.
    #[must_use]
    pub fn new(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Self {
            expected: expected.into(),
            actual: actual.into(),
        }
    }
}

impl fmt::Display for Observed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expected {}, found {}", self.expected, self.actual)
    }
}

/// Everything a reader needs to check a finding for themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// Where the bytes came from.
    pub artifact: Artifact,
    /// Which frame of it, counting from zero.
    pub frame: Option<u64>,
    /// Where in the frame, in bytes.
    pub offset: Option<u64>,
    /// The exact bytes the claim is about.
    ///
    /// Bounded when it is recorded: a finding that carried a whole frame would
    /// make a report of a busy capture unreadable, and the frame index is
    /// there for anyone who wants the rest.
    pub bytes: Vec<u8>,
    /// Which exchange, if the finding is about one.
    pub transaction: Option<TransactionRef>,
    /// What was expected against what was there.
    pub observed: Option<Observed>,
    /// When it happened, in nanoseconds since the Unix epoch.
    pub timestamp: Option<u64>,
}

/// The most bytes a finding carries.
///
/// Enough for any Modbus frame. A finding is a pointer into evidence, not a
/// copy of it.
pub const MAX_EVIDENCE_BYTES: usize = 260;

impl Evidence {
    /// Evidence that names only where it came from.
    #[must_use]
    pub fn from(artifact: Artifact) -> Self {
        Self {
            artifact,
            frame: None,
            offset: None,
            bytes: Vec::new(),
            transaction: None,
            observed: None,
            timestamp: None,
        }
    }

    /// Names the frame this is about.
    #[must_use]
    pub const fn at_frame(mut self, frame: u64) -> Self {
        self.frame = Some(frame);
        self
    }

    /// Names the byte offset within the artefact.
    #[must_use]
    pub const fn at_offset(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Carries the bytes themselves, truncated to [`MAX_EVIDENCE_BYTES`].
    #[must_use]
    pub fn with_bytes(mut self, bytes: &[u8]) -> Self {
        let take = bytes.len().min(MAX_EVIDENCE_BYTES);
        self.bytes = bytes.get(..take).unwrap_or(&[]).to_vec();
        self
    }

    /// Names the exchange.
    #[must_use]
    pub const fn about(mut self, transaction: TransactionRef) -> Self {
        self.transaction = Some(transaction);
        self
    }

    /// Records what was expected against what was there.
    #[must_use]
    pub fn observing(mut self, observed: Observed) -> Self {
        self.observed = Some(observed);
        self
    }

    /// Records when.
    #[must_use]
    pub const fn at_time(mut self, nanos: u64) -> Self {
        self.timestamp = Some(nanos);
        self
    }
}
