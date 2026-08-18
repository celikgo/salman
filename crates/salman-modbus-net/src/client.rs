// SPDX-License-Identifier: Apache-2.0
//! A Modbus TCP client, over a blocking socket.
//!
//! # Writes are gated, and the gate cannot be walked around
//!
//! [`Client::read_from`] issues a read and asks nobody. [`Client::write`]
//! issues a write and requires two separate things: an ARMED
//! [`PostureState`](salman_core::posture::PostureState), and a
//! [`UserConfirmation`] taken **by value**.
//!
//! Taking it by value is the mechanism, not a style choice. A confirmation is
//! consumed by the call it authorises, so a caller that obtained one from a
//! human cannot hold it and write ten more times. The type has no public
//! constructor and can only come from
//! [`ConfirmationRequest::ask`](salman_core::posture::ConfirmationRequest::ask),
//! which needs something able to put the question to a person — so an
//! automated caller cannot manufacture consent.
//!
//! The posture is checked here as well as by whatever called in. That is
//! deliberate duplication: a check a caller can forget is not a boundary.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use salman_core::posture::{DenialReason, Effect, Permit, PostureState, UserConfirmation};
use salman_modbus::function::ExceptionCode;
use salman_modbus::limits::MAX_TCP_ADU;
use salman_modbus::pdu::{DecodeError, Request, Response};
use salman_modbus::tcp::{FrameError, Framer, TcpAdu};

/// How long to wait for a response when nothing else is said.
///
/// MG §4.4.1.4 is explicit that the specification deliberately gives no
/// required response time, and field defaults vary by an order of magnitude —
/// libmodbus uses 500 ms, pymodbus 3 s. salman picks one, states it, and lets
/// it be changed; there is no standard value to defer to.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(1000);

/// A connection to one Modbus TCP server.
#[derive(Debug)]
pub struct Client {
    stream: TcpStream,
    framer: Framer,
    next_transaction: u16,
    /// Responses whose transaction identifier matched no outstanding request.
    ///
    /// Counted rather than hidden: a stale response is evidence about the
    /// device — usually that an earlier request timed out and was answered
    /// after salman stopped waiting.
    unmatched_responses: u64,
    /// Where this connection goes, for the confirmation prompt and for
    /// diagnostics. A confirmation that cannot say which device it is about is
    /// not one a person can act on.
    peer: String,
}

impl Client {
    /// Opens a connection.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Io`] if the address cannot be resolved or the
    /// connection cannot be made.
    pub fn connect(address: impl ToSocketAddrs, timeout: Duration) -> Result<Self, ClientError> {
        let mut last = io::Error::new(io::ErrorKind::InvalidInput, "no address to connect to");
        for socket in address.to_socket_addrs()? {
            match TcpStream::connect_timeout(&socket, timeout) {
                Ok(stream) => return Self::from_stream(stream, timeout),
                Err(error) => last = error,
            }
        }
        Err(ClientError::Io(last))
    }

    /// Takes a connection somebody else opened.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Io`] if the socket's timeouts cannot be set.
    pub fn from_stream(stream: TcpStream, timeout: Duration) -> Result<Self, ClientError> {
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        // Modbus frames are small and latency matters more than packing, so a
        // request is not held back waiting for company.
        let _ = stream.set_nodelay(true);
        let peer = stream
            .peer_addr()
            .map_or_else(|_| "unknown".to_string(), |a: SocketAddr| a.to_string());
        Ok(Self {
            stream,
            framer: Framer::new(),
            next_transaction: 1,
            unmatched_responses: 0,
            peer,
        })
    }

    /// How the peer is identified in confirmations and diagnostics.
    #[must_use]
    pub fn peer(&self) -> &str {
        &self.peer
    }

    /// How many responses arrived that matched no outstanding request.
    #[must_use]
    pub const fn unmatched_responses(&self) -> u64 {
        self.unmatched_responses
    }

    /// Issues a read.
    ///
    /// Reading is [`Effect::ReadDevice`], which every posture permits: salman
    /// is read-only by default, and this is what "read-only" means.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::NotARead`] if given a request that would change
    /// the device — use [`Client::write`] for those, which is where the
    /// permission checks are. Otherwise the transport and decoding errors.
    pub fn read_from(&mut self, unit: u8, request: &Request) -> Result<Response, ClientError> {
        if request.is_write() {
            return Err(ClientError::NotARead {
                function: request.function().0,
            });
        }
        self.exchange(unit, request)
    }

    /// Issues a write to a real device.
    ///
    /// `confirmation` is taken by value and consumed: it authorises **this**
    /// call and cannot be reused for the next one. See the module
    /// documentation for why that is the mechanism rather than a convention.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::NotAWrite`] if given a read;
    /// [`ClientError::Refused`] if the posture does not permit a live write —
    /// which at anything below ARMED it does not. Otherwise the transport and
    /// decoding errors.
    pub fn write(
        &mut self,
        unit: u8,
        request: &Request,
        posture: &PostureState,
        now_ms: u64,
        _confirmation: UserConfirmation,
    ) -> Result<Response, ClientError> {
        // Nothing is read from the confirmation, and nothing needs to be: it
        // carries no data, and its whole content is that it exists. Taking it
        // **by value** is the enforcement — the caller gives up ownership, so
        // one confirmation authorises one write and the next call needs a new
        // one. The body could not weaken that if it tried.
        if !request.is_write() {
            return Err(ClientError::NotAWrite {
                function: request.function().0,
            });
        }
        match posture.permits(Effect::WriteLiveDevice, now_ms) {
            // A confirmation was supplied, which is what this asks for.
            Permit::RequiresConfirmation => {}
            Permit::Allowed => {}
            Permit::Denied(reason) => return Err(ClientError::Refused { reason }),
        }
        self.exchange(unit, request)
    }

    /// Issues a write to a device salman is simulating.
    ///
    /// This is `Effect::WriteSimulated`, not `Effect::WriteLiveDevice`, and so
    /// it needs no confirmation: nothing on the other end is real. It exists
    /// for the IO-mapping link, which writes a program's outputs every scan
    /// and could not ask a person about each one.
    ///
    /// **The caller is responsible for knowing that the peer is simulated.**
    /// There is nothing in a socket that says so, which is exactly why
    /// `salman_link::Link` takes it as an explicit `Peer` and refuses to run
    /// output mappings against a live one. Nothing here can check it.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::NotAWrite`] if given a read, and the transport
    /// and decoding errors otherwise. There is no posture check here because
    /// there is no posture that forbids simulation once salman is doing any:
    /// the check belongs where the peer is known to be simulated.
    pub fn write_simulated(
        &mut self,
        unit: u8,
        request: &Request,
    ) -> Result<Response, ClientError> {
        if !request.is_write() {
            return Err(ClientError::NotAWrite {
                function: request.function().0,
            });
        }
        self.exchange(unit, request)
    }

    /// Sends one request and waits for the response that answers it.
    fn exchange(&mut self, unit: u8, request: &Request) -> Result<Response, ClientError> {
        let transaction = self.allocate_transaction();
        let adu = TcpAdu::new(transaction, unit, request.encode());
        self.stream.write_all(&adu.to_vec())?;
        self.stream.flush()?;

        let mut scratch = [0_u8; MAX_TCP_ADU];
        loop {
            let read = self.stream.read(&mut scratch)?;
            if read == 0 {
                return Err(ClientError::ConnectionClosed);
            }
            let mut rest = scratch.get(..read).unwrap_or(&[]);
            loop {
                let (used, outcome) = self.framer.advance(rest);
                rest = rest.get(used..).unwrap_or(&[]);
                match outcome {
                    Ok(Some(frame)) => {
                        if frame.header.transaction != transaction {
                            // MG mandates matching on the transaction
                            // identifier and says nothing about ordering, so a
                            // response that answers something else is not a
                            // fault — it is almost always an answer to a
                            // request salman already gave up on.
                            self.unmatched_responses += 1;
                            continue;
                        }
                        let response = Response::decode(frame.pdu.as_bytes(), request)?;
                        if let Response::Exception { code, .. } = response {
                            return Err(ClientError::Exception { code, unit });
                        }
                        return Ok(response);
                    }
                    Ok(None) => break,
                    Err(error) => return Err(ClientError::Framing(error)),
                }
            }
        }
    }

    /// The next transaction identifier.
    ///
    /// Wraps 65535 to 1 rather than to 0. Zero is a perfectly legal
    /// identifier, and skipping it keeps "transaction 0" meaning "this client
    /// has just connected", which is a genuinely useful thing to see in a
    /// capture. Implementations differ here — libmodbus wraps to 0, NModbus
    /// to 1 — and no specification says.
    fn allocate_transaction(&mut self) -> u16 {
        let transaction = self.next_transaction;
        self.next_transaction = self.next_transaction.checked_add(1).unwrap_or(1);
        transaction
    }
}

/// Why a client call did not produce a response.
#[derive(Debug)]
pub enum ClientError {
    /// The socket failed, or the read timed out.
    Io(io::Error),
    /// The server closed the connection.
    ConnectionClosed,
    /// The stream could not be framed, so the connection is unusable.
    Framing(FrameError),
    /// The response could not be decoded.
    Decode(DecodeError),
    /// The server refused, and said why.
    Exception {
        /// What the server said.
        code: ExceptionCode,
        /// Which unit was addressed.
        unit: u8,
    },
    /// A write was passed to the read path, which does not check permissions.
    NotARead {
        /// The function code offered.
        function: u8,
    },
    /// A read was passed to the write path.
    NotAWrite {
        /// The function code offered.
        function: u8,
    },
    /// The posture does not permit writing to a real device.
    Refused {
        /// Why, in a form fit to show a user.
        reason: DenialReason,
    },
}

impl From<io::Error> for ClientError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<DecodeError> for ClientError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::ConnectionClosed => f.write_str("the server closed the connection"),
            Self::Framing(error) => write!(f, "{error}"),
            Self::Decode(error) => write!(f, "{error}"),
            Self::Exception { code, unit } => {
                write!(f, "unit {unit} refused the request: {code}")
            }
            Self::NotARead { function } => write!(
                f,
                "function 0x{function:02X} changes the device, and reads do not go through \
                 the permission checks; use Client::write"
            ),
            Self::NotAWrite { function } => write!(
                f,
                "function 0x{function:02X} does not change the device; use Client::read_from"
            ),
            Self::Refused { reason } => write!(f, "salman will not write to a device: {reason:?}"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Framing(error) => Some(error),
            Self::Decode(error) => Some(error),
            _ => None,
        }
    }
}
