// SPDX-License-Identifier: Apache-2.0
//! A Modbus TCP server, so that a test can drive both ends.
//!
//! This is a **simulator**, and the word is meant: it answers for a device
//! salman is pretending to be. Running it is `Effect::WriteSimulated`, which
//! needs the SIMULATE posture — a simulator whose whole purpose is to accept
//! writes has no business running at a posture that forbids them.
//!
//! # What a conforming server does that a naive one does not
//!
//! Three behaviours here are easy to get wrong and are each asserted in the
//! tests:
//!
//! * the unit identifier is **echoed**, not replaced with what salman thinks
//!   it should be. MG §4.4.1.2 specifies `0xFF` for a device that is not a
//!   gateway and also accepts `0x00`, and a client is entitled to send either;
//! * a frame that cannot be framed at all **closes the connection** rather
//!   than provoking an answer. There is nothing in a Modbus TCP stream that
//!   says where the next frame starts, so answering and continuing would mean
//!   answering from the middle of something;
//! * a function code the server does not implement gets **exception 01**,
//!   which is a real answer. Silence is for frames whose integrity failed, and
//!   on TCP that is the transport's problem rather than Modbus's.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use salman_core::posture::{DenialReason, Effect, Permit, PostureState};
use salman_modbus::device::Device;
use salman_modbus::function::ExceptionCode;
use salman_modbus::limits::MAX_TCP_ADU;
use salman_modbus::pdu::{DecodeError, Request, Response};
use salman_modbus::tcp::{Framer, TcpAdu};

/// How long the accept loop waits between polls when nothing is arriving.
const ACCEPT_POLL: Duration = Duration::from_millis(2);

/// A listening Modbus TCP simulator.
#[derive(Debug)]
pub struct Server {
    listener: TcpListener,
    device: Arc<Mutex<Device>>,
}

impl Server {
    /// Binds a listener and takes ownership of the device it answers for.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Refused`] if the posture does not permit
    /// simulated writes, and [`ServerError::Io`] if the address cannot be
    /// bound.
    pub fn bind(
        address: impl ToSocketAddrs,
        device: Device,
        posture: &PostureState,
        now_ms: u64,
    ) -> Result<Self, ServerError> {
        match posture.permits(Effect::WriteSimulated, now_ms) {
            Permit::Allowed | Permit::RequiresConfirmation => {}
            Permit::Denied(reason) => return Err(ServerError::Refused { reason }),
        }
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            device: Arc::new(Mutex::new(device)),
        })
    }

    /// Where it is listening, which is how a test finds the port the operating
    /// system chose.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Io`] if the socket has no local address.
    pub fn local_addr(&self) -> Result<SocketAddr, ServerError> {
        Ok(self.listener.local_addr()?)
    }

    /// Runs the accept loop on its own thread.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Io`] if the listening address cannot be read.
    pub fn spawn(self) -> Result<ServerHandle, ServerError> {
        let address = self.local_addr()?;
        let running = Arc::new(AtomicBool::new(true));
        let served = Arc::new(AtomicU64::new(0));
        let device = Arc::clone(&self.device);

        let stop = Arc::clone(&running);
        let counter = Arc::clone(&served);
        let thread = thread::spawn(move || {
            let mut connections: Vec<JoinHandle<()>> = Vec::new();
            while stop.load(Ordering::Relaxed) {
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        let device = Arc::clone(&device);
                        let stop = Arc::clone(&stop);
                        let counter = Arc::clone(&counter);
                        connections.push(thread::spawn(move || {
                            serve(&stream, &device, &stop, &counter);
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(ACCEPT_POLL);
                    }
                    Err(_) => break,
                }
            }
            for connection in connections {
                let _ = connection.join();
            }
        });

        Ok(ServerHandle {
            address,
            running,
            served,
            device: self.device,
            thread: Some(thread),
        })
    }
}

/// A running simulator.
///
/// Dropping it stops the server and waits for its threads, so a test cannot
/// leave one listening behind it.
#[derive(Debug)]
pub struct ServerHandle {
    address: SocketAddr,
    running: Arc<AtomicBool>,
    served: Arc<AtomicU64>,
    device: Arc<Mutex<Device>>,
    thread: Option<JoinHandle<()>>,
}

impl ServerHandle {
    /// Where it is listening.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// How many requests it has answered.
    #[must_use]
    pub fn served(&self) -> u64 {
        self.served.load(Ordering::Relaxed)
    }

    /// Looks at the device the simulator is answering for.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::DeviceUnavailable`] if a connection thread
    /// panicked while holding the device, which would leave its contents
    /// unreliable.
    pub fn with_device<T>(&self, f: impl FnOnce(&mut Device) -> T) -> Result<T, ServerError> {
        let mut device = self
            .device
            .lock()
            .map_err(|_| ServerError::DeviceUnavailable)?;
        Ok(f(&mut device))
    }

    /// Stops the server and waits for its threads.
    pub fn shutdown(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Answers one connection until it closes or the server stops.
fn serve(
    mut stream: &TcpStream,
    device: &Arc<Mutex<Device>>,
    running: &Arc<AtomicBool>,
    served: &Arc<AtomicU64>,
) {
    // Short enough that a stopped server does not wait on an idle client, long
    // enough that a slow one is not cut off.
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let _ = stream.set_nodelay(true);
    let mut framer = Framer::new();
    let mut scratch = [0_u8; MAX_TCP_ADU];

    while running.load(Ordering::Relaxed) {
        let read = match stream.read(&mut scratch) {
            Ok(0) => return,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(_) => return,
        };
        let mut rest = scratch.get(..read).unwrap_or(&[]);
        loop {
            let (used, outcome) = framer.advance(rest);
            rest = rest.get(used..).unwrap_or(&[]);
            match outcome {
                Ok(Some(frame)) => {
                    let Some(reply) = answer(&frame, device) else {
                        return;
                    };
                    if stream.write_all(&reply.to_vec()).is_err() {
                        return;
                    }
                    let _ = stream.flush();
                    served.fetch_add(1, Ordering::Relaxed);
                }
                Ok(None) => break,
                // Framing is lost and nothing in the stream can recover it.
                // Closing is the only honest thing left.
                Err(_) => return,
            }
        }
    }
}

/// Works out what to send back, or `None` if the connection should close.
fn answer(frame: &TcpAdu, device: &Arc<Mutex<Device>>) -> Option<TcpAdu> {
    let function = frame.pdu.function();
    let response = match Request::decode(frame.pdu.as_bytes()) {
        Ok(request) => {
            let mut device = device.lock().ok()?;
            match device.apply(&request) {
                Ok(response) => response,
                Err(code) => Response::Exception { function, code },
            }
        }
        Err(error) => Response::Exception {
            function,
            code: exception_for(&error),
        },
    };
    // The unit identifier is echoed rather than replaced. A client may address
    // 0x00 or 0xFF and is entitled to see back what it sent.
    Some(TcpAdu::new(
        frame.header.transaction,
        frame.header.unit,
        response.encode(),
    ))
}

/// Which exception a decoding failure deserves.
///
/// The mapping follows APS §7's definitions rather than the shape of salman's
/// error type: 01 is "I do not implement this function", 03 is "this request
/// is structurally wrong". Nothing here answers 02, because an address is only
/// wrong relative to a device's map and a request that did not decode never
/// reached one.
const fn exception_for(error: &DecodeError) -> ExceptionCode {
    match error {
        DecodeError::FunctionUnknown { .. } | DecodeError::FunctionNotImplemented { .. } => {
            ExceptionCode::ILLEGAL_FUNCTION
        }
        DecodeError::QuantityOutOfRange { .. }
        | DecodeError::ByteCountDisagreesWithQuantity { .. }
        | DecodeError::CoilValueNotOnOrOff { .. }
        | DecodeError::Truncated { .. }
        | DecodeError::TrailingBytes { .. }
        | DecodeError::TooLong { .. }
        | DecodeError::Empty
        | DecodeError::FunctionDoesNotAnswerRequest { .. } => ExceptionCode::ILLEGAL_DATA_VALUE,
    }
}

/// Why a server could not start, or could not be inspected.
#[derive(Debug)]
pub enum ServerError {
    /// The socket failed.
    Io(std::io::Error),
    /// The posture does not permit running a simulator.
    Refused {
        /// Why, in a form fit to show a user.
        reason: DenialReason,
    },
    /// A connection thread panicked while holding the device.
    DeviceUnavailable,
}

impl From<std::io::Error> for ServerError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Refused { reason } => write!(
                f,
                "salman will not run a simulator at this posture: {reason:?}"
            ),
            Self::DeviceUnavailable => {
                f.write_str("the simulated device is unavailable after a thread panicked")
            }
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
