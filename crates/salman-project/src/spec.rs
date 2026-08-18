// SPDX-License-Identifier: Apache-2.0
//! The project file format.
//!
//! YAML, read with the same reader as the declarative test format and under
//! the same rule: **unknown keys are refused**. A misspelt key that was
//! ignored would leave a mapping absent, and a program reading zeros from an
//! input it believed was configured is the exact failure this whole layer
//! exists to prevent.
//!
//! Addresses in this file are written as they are written in Structured Text —
//! `%IW0`, `%QX0.0` — and are lexed with salman's own lexer, so they mean here
//! what they mean in source.

use std::collections::BTreeSet;

use salman_lang::address::DirectAddress;
use salman_modbus::device::Table;
use serde::Deserialize;

use crate::map::{Mapping, check_all};

/// How salman reaches a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Protocol {
    /// Modbus TCP, as specified in MODBUS Messaging on TCP/IP V1.0b.
    #[serde(rename = "modbus-tcp")]
    ModbusTcp,
}

/// Which of a device's four tables a mapping reads or writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum TableName {
    #[serde(rename = "discrete-inputs")]
    DiscreteInputs,
    #[serde(rename = "coils")]
    Coils,
    #[serde(rename = "input-registers")]
    InputRegisters,
    #[serde(rename = "holding-registers")]
    HoldingRegisters,
}

impl TableName {
    const fn table(self) -> Table {
        match self {
            Self::DiscreteInputs => Table::DiscreteInputs,
            Self::Coils => Table::Coils,
            Self::InputRegisters => Table::InputRegisters,
            Self::HoldingRegisters => Table::HoldingRegisters,
        }
    }
}

/// One mapping, as written in the file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingSpec {
    /// Which table on the device.
    table: TableName,
    /// The first PDU address. Zero-based, exactly as it goes on the wire.
    from: u16,
    /// How many items.
    count: u16,
    /// Where it appears in the process image, as `%IW0` or `%QX0.0`.
    to: String,
}

/// One device, as written in the file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceSpec {
    name: String,
    protocol: Protocol,
    address: String,
    #[serde(default = "default_unit")]
    unit: u8,
    #[serde(default)]
    map: Vec<MappingSpec>,
}

/// The unit identifier used when a file does not say.
///
/// MG §4.4.1.2 specifies `0xFF` for a server that is the end device itself,
/// and notes that `0x00` is also accepted. A project talking to a device
/// rather than through a gateway wants `0xFF`, so that is the default; a
/// gateway needs the serial address and has to say so.
const fn default_unit() -> u8 {
    0xFF
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectSpec {
    #[serde(default = "default_dialect")]
    dialect: String,
    sources: Vec<String>,
    #[serde(default)]
    devices: Vec<DeviceSpec>,
}

fn default_dialect() -> String {
    "generic".to_string()
}

/// A device and everything mapped from it.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// What the file calls it, used in diagnostics and confirmations.
    pub name: String,
    /// How salman reaches it.
    pub protocol: Protocol,
    /// Where it is, as written — `host:port` for Modbus TCP.
    pub address: String,
    /// The unit identifier to send.
    pub unit: u8,
    /// What is mapped from it, checked.
    pub mappings: Vec<Mapping>,
}

/// A checked project.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// Which dialect the sources are read with.
    pub dialect: String,
    /// The source files, in the order given.
    pub sources: Vec<String>,
    /// The devices, in the order given.
    pub devices: Vec<Device>,
}

impl Project {
    /// Reads and checks a project file.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectError`] naming everything wrong with the file, not
    /// only the first thing: a file with three bad mappings should take one
    /// run to find out about, not three.
    pub fn parse(text: &str, image_bytes: usize) -> Result<Self, ProjectError> {
        let spec: ProjectSpec =
            serde_saphyr::from_str(text).map_err(|e| ProjectError::Syntax(e.to_string()))?;

        let mut problems = Vec::new();
        if spec.sources.is_empty() {
            problems.push("a project needs at least one source file".to_string());
        }

        let mut names = BTreeSet::new();
        let mut devices = Vec::with_capacity(spec.devices.len());
        for device in spec.devices {
            if !names.insert(device.name.clone()) {
                problems.push(format!(
                    "two devices are called {:?}, so a diagnostic could not say which",
                    device.name
                ));
            }

            let mut mappings = Vec::with_capacity(device.map.len());
            for entry in device.map {
                match parse_address(&entry.to) {
                    Ok(image) => mappings.push(Mapping {
                        table: entry.table.table(),
                        device_start: entry.from,
                        count: entry.count,
                        image,
                    }),
                    Err(message) => problems.push(format!(
                        "device {:?}: {:?} is not a process image address: {message}",
                        device.name, entry.to
                    )),
                }
            }

            for problem in check_all(&mappings, image_bytes) {
                problems.push(format!("device {:?}: {problem}", device.name));
            }

            devices.push(Device {
                name: device.name,
                protocol: device.protocol,
                address: device.address,
                unit: device.unit,
                mappings,
            });
        }

        // Two devices writing the same image bits is the same fault as one
        // device doing it, and is easier to miss because each file section
        // looks right on its own.
        for (index, first) in devices.iter().enumerate() {
            for second in devices.iter().skip(index + 1) {
                for a in &first.mappings {
                    for b in &second.mappings {
                        if a.overlaps(b).unwrap_or(false) {
                            problems.push(format!(
                                "devices {:?} and {:?} both claim {}",
                                first.name, second.name, a.image
                            ));
                        }
                    }
                }
            }
        }

        if !problems.is_empty() {
            return Err(ProjectError::Invalid(problems));
        }
        Ok(Self {
            dialect: spec.dialect,
            sources: spec.sources,
            devices,
        })
    }

    /// Every mapping in the project, with the device it belongs to.
    pub fn mappings(&self) -> impl Iterator<Item = (&Device, &Mapping)> {
        self.devices
            .iter()
            .flat_map(|device| device.mappings.iter().map(move |m| (device, m)))
    }
}

/// Reads a process image address written as it would be in source.
fn parse_address(written: &str) -> Result<DirectAddress, String> {
    use salman_core::span::{FileId, SourceMap};
    use salman_lang::dialect::Dialect;
    use salman_lang::token::TokenKind;

    let mut sources = SourceMap::new();
    let file: FileId = sources
        .add("project", written)
        .map_err(|_| "the address is too long".to_string())?;
    let (stream, diagnostics) = salman_lang::lexer::lex(file, written, &Dialect::generic());
    if diagnostics.has_errors() {
        return Err("it does not lex as an address".to_string());
    }
    let tokens = stream.tokens();
    match tokens {
        [token, end] if matches!(end.kind, TokenKind::Eof) => match token.kind {
            TokenKind::DirectAddress(index) => stream
                .address(index)
                .cloned()
                .ok_or_else(|| "the address did not resolve".to_string()),
            _ => Err("it is not a %-address".to_string()),
        },
        _ => Err("it is not a single %-address".to_string()),
    }
}

/// Why a project file could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectError {
    /// The file is not the shape a project file has.
    Syntax(String),
    /// The file parsed and says something that cannot be done.
    Invalid(Vec<String>),
}

impl core::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Syntax(message) => write!(f, "{message}"),
            Self::Invalid(problems) => {
                for (index, problem) in problems.iter().enumerate() {
                    if index > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{problem}")?;
                }
                Ok(())
            }
        }
    }
}

impl core::error::Error for ProjectError {}

/// Everything wrong with the project, one per line.
impl ProjectError {
    /// The problems, as a list.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        match self {
            Self::Syntax(message) => vec![message.clone()],
            Self::Invalid(problems) => problems.clone(),
        }
    }
}
