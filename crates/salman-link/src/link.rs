// SPDX-License-Identifier: Apache-2.0
//! The link itself.

use salman_core::posture::{DenialReason, Effect, Permit, PostureState};
use salman_core::value::Value;
use salman_lang::address::{AddressSize, DirectAddress};
use salman_modbus::device::Table;
use salman_modbus::pdu::{Bits, Request, Response, Words};
use salman_modbus_net::client::{Client, ClientError};
use salman_project::map::{Direction, Mapping, MappingError};
use salman_vm::memory::Memory;

/// What is on the other end of a link.
///
/// Not a detail: it decides whether outputs may be written at all. See the
/// crate documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peer {
    /// A device salman is simulating, in this process or another.
    Simulated,
    /// Real equipment. A link may read it and will not drive it.
    Live,
}

/// One device's mappings, bound to a connection.
#[derive(Debug)]
pub struct Link {
    client: Client,
    unit: u8,
    peer: Peer,
    mappings: Vec<Mapping>,
    name: String,
}

impl Link {
    /// Binds a device's mappings to a connection.
    ///
    /// # Errors
    ///
    /// Returns [`LinkError::Refused`] if the posture does not permit simulated
    /// writes — a link is I/O, and salman is read-only until told otherwise —
    /// and [`LinkError::WouldDriveALiveDevice`] if the mappings would write to
    /// a live device.
    pub fn new(
        name: impl Into<String>,
        client: Client,
        unit: u8,
        peer: Peer,
        mappings: Vec<Mapping>,
        posture: &PostureState,
        now_ms: u64,
    ) -> Result<Self, LinkError> {
        match posture.permits(Effect::WriteSimulated, now_ms) {
            Permit::Allowed | Permit::RequiresConfirmation => {}
            Permit::Denied(reason) => return Err(LinkError::Refused { reason }),
        }
        let name = name.into();
        if peer == Peer::Live {
            for mapping in &mappings {
                if mapping.direction()? == Direction::Output {
                    return Err(LinkError::WouldDriveALiveDevice {
                        device: name,
                        image: mapping.image.to_string(),
                    });
                }
            }
        }
        Ok(Self {
            client,
            unit,
            peer,
            mappings,
            name,
        })
    }

    /// What the project calls this device.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What is on the other end.
    #[must_use]
    pub const fn peer(&self) -> Peer {
        self.peer
    }

    /// Reads every input mapping and drives the physical inputs.
    ///
    /// Called **before** the scan latches, so the whole of what the program
    /// sees this scan arrived at the same moment.
    ///
    /// # Errors
    ///
    /// Returns [`LinkError`] naming the mapping that failed. One failed
    /// mapping stops the poll: a scan running on inputs that are half fresh
    /// and half stale is worse than a scan that did not run.
    pub fn poll_inputs(&mut self, memory: &mut Memory) -> Result<(), LinkError> {
        for index in 0..self.mappings.len() {
            let Some(mapping) = self.mappings.get(index).cloned() else {
                continue;
            };
            if mapping.direction()? != Direction::Input {
                continue;
            }
            let response = self
                .client
                .read_from(self.unit, &read_request(&mapping))
                .map_err(|error| LinkError::Transport {
                    device: self.name.clone(),
                    image: mapping.image.to_string(),
                    error,
                })?;
            self.store(&mapping, &response, memory)?;
        }
        Ok(())
    }

    /// Writes every output mapping from the published outputs.
    ///
    /// Called **after** the scan publishes, so what goes to the device is what
    /// the program finished with rather than a value it was part way through
    /// computing.
    ///
    /// # Errors
    ///
    /// Returns [`LinkError::WouldDriveALiveDevice`] if the peer is live —
    /// which [`Link::new`] already refused, and which is checked again here
    /// because this is the function that would do it.
    pub fn publish_outputs(&mut self, memory: &Memory) -> Result<(), LinkError> {
        for index in 0..self.mappings.len() {
            let Some(mapping) = self.mappings.get(index).cloned() else {
                continue;
            };
            if mapping.direction()? != Direction::Output {
                continue;
            }
            if self.peer == Peer::Live {
                return Err(LinkError::WouldDriveALiveDevice {
                    device: self.name.clone(),
                    image: mapping.image.to_string(),
                });
            }
            let request = self.gather(&mapping, memory)?;
            // A simulated device is `Effect::WriteSimulated`, which the
            // posture already permitted when the link was built. It does not
            // go through `Client::write`, which is for engineering writes to
            // real equipment and demands a confirmation for each one.
            self.client
                .write_simulated(self.unit, &request)
                .map_err(|error| LinkError::Transport {
                    device: self.name.clone(),
                    image: mapping.image.to_string(),
                    error,
                })?;
        }
        Ok(())
    }

    /// Puts a response's values into the physical input area.
    fn store(
        &self,
        mapping: &Mapping,
        response: &Response,
        memory: &mut Memory,
    ) -> Result<(), LinkError> {
        for item in 0..mapping.count {
            let address = nth(&mapping.image, item).ok_or_else(|| LinkError::AddressRun {
                device: self.name.clone(),
                image: mapping.image.to_string(),
                item,
            })?;
            let value = match response {
                Response::ReadCoils(bits) | Response::ReadDiscreteInputs(bits) => {
                    bits.get(item).map(Value::Bool)
                }
                Response::ReadHoldingRegisters(words) | Response::ReadInputRegisters(words) => {
                    words.get(item).map(Value::Word)
                }
                _ => None,
            };
            let Some(value) = value else {
                return Err(LinkError::ShortAnswer {
                    device: self.name.clone(),
                    image: mapping.image.to_string(),
                    expected: mapping.count,
                    item,
                });
            };
            memory
                .drive_input(&address, &value)
                .map_err(|_| LinkError::AddressRun {
                    device: self.name.clone(),
                    image: mapping.image.to_string(),
                    item,
                })?;
        }
        Ok(())
    }

    /// Collects the published outputs into a write request.
    fn gather(&self, mapping: &Mapping, memory: &Memory) -> Result<Request, LinkError> {
        let outputs = memory.physical_outputs();
        let mut bits = Vec::with_capacity(usize::from(mapping.count));
        let mut words = Vec::with_capacity(usize::from(mapping.count));
        for item in 0..mapping.count {
            let address = nth(&mapping.image, item).ok_or_else(|| LinkError::AddressRun {
                device: self.name.clone(),
                image: mapping.image.to_string(),
                item,
            })?;
            let position = outputs
                .resolve(&address)
                .map_err(|_| LinkError::AddressRun {
                    device: self.name.clone(),
                    image: mapping.image.to_string(),
                    item,
                })?;
            let value = outputs
                .read(position)
                .ok_or_else(|| LinkError::AddressRun {
                    device: self.name.clone(),
                    image: mapping.image.to_string(),
                    item,
                })?;
            match value {
                Value::Bool(bit) => bits.push(bit),
                // The image is typed by width rather than by declaration, so a
                // word address always reads back as a WORD. Anything else here
                // would mean the image resolved an address to a size the
                // mapping checks already refused.
                Value::Word(word) => words.push(word),
                other => {
                    return Err(LinkError::UnexpectedWidth {
                        device: self.name.clone(),
                        image: mapping.image.to_string(),
                        found: other.type_of().to_string(),
                    });
                }
            }
        }
        match mapping.table {
            Table::Coils => Ok(Request::WriteMultipleCoils {
                start: mapping.device_start,
                values: Bits::from_iter_of(bits).ok_or(LinkError::TooManyItems {
                    count: mapping.count,
                })?,
            }),
            Table::HoldingRegisters => Ok(Request::WriteMultipleRegisters {
                start: mapping.device_start,
                values: Words::new(&words).ok_or(LinkError::TooManyItems {
                    count: mapping.count,
                })?,
            }),
            // Refused when the project file was read; repeated here so that
            // this function is total rather than relying on it.
            Table::DiscreteInputs | Table::InputRegisters => {
                Err(LinkError::Mapping(MappingError::TableNotWritable {
                    table: mapping.table,
                }))
            }
        }
    }
}

/// The read a mapping asks for.
fn read_request(mapping: &Mapping) -> Request {
    match mapping.table {
        Table::Coils => Request::ReadCoils {
            start: mapping.device_start,
            quantity: mapping.count,
        },
        Table::DiscreteInputs => Request::ReadDiscreteInputs {
            start: mapping.device_start,
            quantity: mapping.count,
        },
        Table::HoldingRegisters => Request::ReadHoldingRegisters {
            start: mapping.device_start,
            quantity: mapping.count,
        },
        Table::InputRegisters => Request::ReadInputRegisters {
            start: mapping.device_start,
            quantity: mapping.count,
        },
    }
}

/// The address `n` items after `first`.
///
/// Bit addresses advance a bit at a time and carry into the next byte; word
/// addresses advance a word at a time. `None` if the run would pass the end of
/// the address space.
#[must_use]
pub fn nth(first: &DirectAddress, n: u16) -> Option<DirectAddress> {
    let path = first.path.as_ref()?;
    let mut moved = first.clone();
    match first.size {
        AddressSize::Bit => {
            let (byte, bit) = match path.as_slice() {
                [byte] => (*byte, 0),
                [byte, bit] => (*byte, *bit),
                _ => return None,
            };
            let index = u64::from(byte) * 8 + u64::from(bit) + u64::from(n);
            moved.path = Some(vec![u32::try_from(index / 8).ok()?, (index % 8) as u32]);
            Some(moved)
        }
        AddressSize::Word => {
            let [index] = path.as_slice() else {
                return None;
            };
            moved.path = Some(vec![index.checked_add(u32::from(n))?]);
            Some(moved)
        }
        // Refused when the project file is read: these span more than one
        // register and the word order is undefined.
        AddressSize::Byte | AddressSize::DoubleWord | AddressSize::LongWord => None,
    }
}

/// Why a link could not do what a mapping asked.
#[derive(Debug)]
pub enum LinkError {
    /// The posture does not permit input and output at all.
    Refused {
        /// Why, in a form fit to show a user.
        reason: DenialReason,
    },
    /// The mappings would write to real equipment. Categorically refused; see
    /// the crate documentation.
    WouldDriveALiveDevice {
        /// What the project calls the device.
        device: String,
        /// The image address the mapping named.
        image: String,
    },
    /// The device did not answer, or answered with an exception.
    Transport {
        /// What the project calls the device.
        device: String,
        /// The image address of the mapping being run.
        image: String,
        /// What went wrong.
        error: ClientError,
    },
    /// The device answered with fewer items than were asked for.
    ShortAnswer {
        /// What the project calls the device.
        device: String,
        /// The image address of the mapping.
        image: String,
        /// How many were asked for.
        expected: u16,
        /// The first item that was missing.
        item: u16,
    },
    /// A mapping's run of image addresses does not exist.
    AddressRun {
        /// What the project calls the device.
        device: String,
        /// The image address the mapping started at.
        image: String,
        /// Which item could not be placed.
        item: u16,
    },
    /// The image gave back a value of a width the mapping cannot carry.
    UnexpectedWidth {
        /// What the project calls the device.
        device: String,
        /// The image address of the mapping.
        image: String,
        /// What came back.
        found: String,
    },
    /// More items than one Modbus frame carries.
    TooManyItems {
        /// How many the mapping asked for.
        count: u16,
    },
    /// The mapping itself does not mean anything.
    Mapping(MappingError),
}

impl From<MappingError> for LinkError {
    fn from(error: MappingError) -> Self {
        Self::Mapping(error)
    }
}

impl core::fmt::Display for LinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused { reason } => {
                write!(f, "salman will not run input and output here: {reason:?}")
            }
            Self::WouldDriveALiveDevice { device, image } => write!(
                f,
                "the mapping at {image} would drive outputs on {device}, which is real \
                 equipment. salman reads live devices and does not drive them: a tool that \
                 writes a plant's outputs every scan is a controller, and salman has no \
                 watchdog, no failsafe state and no safety assessment. There is no setting \
                 that enables this"
            ),
            Self::Transport {
                device,
                image,
                error,
            } => write!(f, "{device}, running the mapping at {image}: {error}"),
            Self::ShortAnswer {
                device,
                image,
                expected,
                item,
            } => write!(
                f,
                "{device} answered the mapping at {image} with fewer than the {expected} \
                 items asked for; item {item} is missing"
            ),
            Self::AddressRun {
                device,
                image,
                item,
            } => write!(
                f,
                "{device}: the mapping at {image} has no place for item {item}"
            ),
            Self::UnexpectedWidth {
                device,
                image,
                found,
            } => write!(
                f,
                "{device}: the mapping at {image} read a {found} from the process image,                  and a Modbus register is a WORD"
            ),
            Self::TooManyItems { count } => {
                write!(f, "{count} items is more than one Modbus frame carries")
            }
            Self::Mapping(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for LinkError {}
