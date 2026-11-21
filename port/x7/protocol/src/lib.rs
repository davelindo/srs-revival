//! Sony SRS-X7 wire format recovered from Armada 2.13 and Laphroaig 2.09.
//! Firmware anchors: MC 0x58154dd0/0x58154eee, module 0x6026f814/0x6026f5bc.
//! This is Sony's five-byte header protocol. All storage belongs to the caller.
#![no_std]
#![forbid(unsafe_code)]

pub const IDLE: u8 = 0xff;
pub const ESCAPE: u8 = 0x27;

const fn crc_table() -> [u8; 256] {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut value = i as u8;
        let mut bit = 0;
        while bit < 8 {
            value = (value << 1) ^ if value & 0x80 != 0 { 0x85 } else { 0 };
            bit += 1;
        }
        table[i] = value;
        i += 1;
    }
    table
}

// 256 read-only bytes replace the eight branch/shift steps per received byte.
static CRC: [u8; 256] = crc_table();

pub fn crc8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |crc, &byte| CRC[(crc ^ byte) as usize])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    WriteFixed = 0,
    ReadFixed = 1,
    WriteVariable = 2,
    ReadVariable = 3,
    Retry = 4,
}

impl Kind {
    fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0 => Self::WriteFixed,
            1 => Self::ReadFixed,
            2 => Self::WriteVariable,
            3 => Self::ReadVariable,
            4 => Self::Retry,
            _ => return None,
        })
    }

    pub fn is_write(self) -> bool {
        matches!(self, Self::WriteFixed | Self::WriteVariable)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub register: u8,
    /// Little-endian on the wire: write length or read maximum, zero for fixed.
    pub length: u16,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Request<'a> {
    pub header: Header,
    /// Fixed values retain byte order; they are not automatically a native u16.
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    HeaderCrc,
    DataCrc,
    Escape,
    Operation,
    Length,
    OutputFull,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self, f)
    }
}

impl core::error::Error for Error {}

/// Incremental receiver. A packet becomes visible only after all required CRCs
/// pass. An oversized body is drained, never written beyond the caller's buffer.
pub struct Parser<const N: usize> {
    header: [u8; 5],
    header_used: usize,
    data: [u8; N],
    data_used: usize,
    needed: usize,
    drain: usize,
    crc: u8,
    escaped: bool,
}

impl<const N: usize> Default for Parser<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Parser<N> {
    pub const fn new() -> Self {
        Self { header: [0; 5], header_used: 0, data: [0; N], data_used: 0, needed: 0, drain: 0, crc: 0, escaped: false }
    }

    /// Discard an interrupted transaction at an independently observed boundary.
    pub fn reset(&mut self) {
        self.header_used = 0;
        self.data_used = 0;
        self.needed = 0;
        self.drain = 0;
        self.crc = 0;
        self.escaped = false;
    }

    fn decoded_header(&self) -> Header {
        Header {
            kind: Kind::from_byte(self.header[0]).expect("validated operation"),
            register: self.header[1],
            length: u16::from_le_bytes([self.header[2], self.header[3]]),
        }
    }

    pub fn push(&mut self, raw: u8) -> Option<Result<Request<'_>, Error>> {
        // The physical transport may clock idle between escaped bytes too.
        if raw == IDLE {
            return None;
        }
        let byte = if self.escaped {
            self.escaped = false;
            match raw {
                1 => IDLE,
                ESCAPE => ESCAPE,
                _ => {
                    self.reset();
                    return Some(Err(Error::Escape));
                }
            }
        } else if raw == ESCAPE {
            self.escaped = true;
            return None;
        } else {
            raw
        };
        if self.drain != 0 {
            self.drain -= 1;
            return None;
        }
        if self.header_used < 5 {
            self.header[self.header_used] = byte;
            self.header_used += 1;
            if self.header_used < 5 {
                return None;
            }
            if crc8(&self.header[..4]) != self.header[4] {
                self.reset();
                return Some(Err(Error::HeaderCrc));
            }
            let Some(kind) = Kind::from_byte(self.header[0]) else {
                self.reset();
                return Some(Err(Error::Operation));
            };
            if !kind.is_write() {
                let header = self.decoded_header();
                self.reset();
                return Some(Ok(Request { header, payload: &[] }));
            }
            self.needed = if kind == Kind::WriteFixed {
                2
            } else {
                u16::from_le_bytes([self.header[2], self.header[3]]) as usize
            };
            if self.needed > N {
                let drain = self.needed + 1;
                self.reset();
                self.drain = drain;
                return Some(Err(Error::Length));
            }
            return None;
        }
        if self.data_used < self.needed {
            self.data[self.data_used] = byte;
            self.data_used += 1;
            self.crc = CRC[(self.crc ^ byte) as usize];
            return None;
        }
        if self.crc != byte {
            self.reset();
            return Some(Err(Error::DataCrc));
        }
        let header = self.decoded_header();
        let count = self.needed;
        self.reset();
        Some(Ok(Request { header, payload: &self.data[..count] }))
    }
}

pub trait Sink {
    fn put(&mut self, byte: u8) -> Result<(), Error>;
}

// The controller escapes requests. Its reply reader clocks exactly five header
// bytes and, for reads, the declared data length plus one CRC byte.
struct ControllerRequest<'a, S>(&'a mut S);

impl<S: Sink> Sink for ControllerRequest<'_, S> {
    fn put(&mut self, byte: u8) -> Result<(), Error> {
        match byte {
            IDLE | ESCAPE => {
                self.0.put(ESCAPE)?;
                self.0.put(if byte == IDLE { 1 } else { ESCAPE })
            }
            _ => self.0.put(byte),
        }
    }
}

fn header(kind: u8, register: u8, length: u16, out: &mut impl Sink) -> Result<(), Error> {
    let [lo, hi] = length.to_le_bytes();
    body(&[kind, register, lo, hi], out)
}

fn body(bytes: &[u8], out: &mut impl Sink) -> Result<(), Error> {
    for &byte in bytes {
        out.put(byte)?;
    }
    out.put(crc8(bytes))
}

pub fn encode_request(h: Header, payload: &[u8], out: &mut impl Sink) -> Result<(), Error> {
    let valid = match h.kind {
        Kind::WriteFixed => payload.len() == 2 && h.length == 0,
        Kind::WriteVariable => payload.len() == h.length as usize,
        _ => payload.is_empty(),
    };
    if !valid {
        return Err(Error::Length);
    }
    let mut wire = ControllerRequest(out);
    header(h.kind as u8, h.register, h.length, &mut wire)?;
    if h.kind.is_write() {
        body(payload, &mut wire)?;
    }
    Ok(())
}

pub fn write_ack(h: Header, out: &mut impl Sink) -> Result<(), Error> {
    if !h.kind.is_write() {
        return Err(Error::Operation);
    }
    header(h.kind as u8, h.register, h.length, out)
}

pub fn read_reply(kind: Kind, register: u8, payload: &[u8], out: &mut impl Sink) -> Result<(), Error> {
    let length = match kind {
        Kind::ReadFixed if payload.len() == 2 => 0,
        Kind::ReadVariable => u16::try_from(payload.len()).map_err(|_| Error::Length)?,
        _ => return Err(Error::Operation),
    };
    header(kind as u8, register, length, out)?;
    body(payload, out)
}

/// Firmware error class 0xd: header/length rejection e0, data rejection e1.
pub fn error_reply(error: Error, out: &mut impl Sink) -> Result<(), Error> {
    header(0x0d, if error == Error::DataCrc { 0xe1 } else { 0xe0 }, 0, out)
}

/// Fixed storage for an encoded packet. No allocation and no silent truncation.
pub struct Buffer<const N: usize> {
    bytes: [u8; N],
    used: usize,
}

impl<const N: usize> Default for Buffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Buffer<N> {
    pub const fn new() -> Self {
        Self { bytes: [0; N], used: 0 }
    }
    pub fn clear(&mut self) {
        self.used = 0;
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.used]
    }
}

impl<const N: usize> Sink for Buffer<N> {
    fn put(&mut self, byte: u8) -> Result<(), Error> {
        let slot = self.bytes.get_mut(self.used).ok_or(Error::OutputFull)?;
        *slot = byte;
        self.used += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
