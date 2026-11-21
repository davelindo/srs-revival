//! Sony register values and ordered notifications. This is shared by boot and
//! the Linux IRQ backend. The stock queue behavior is visible at 0x6026ffa8:
//! register 0 drains, 0x0c peeks, a named read removes its first queued value.
#![no_std]
#![forbid(unsafe_code)]

use x7_protocol::{Error, Kind, Request, Sink, read_reply, write_ack};
use x7_transport::{BODY_MAX, Endpoint};

const NOTES: usize = 8;
const WRITES: usize = 4;
// Boot identities plus MAC, IP, mask, gateway, DNS and name need more than
// eight cached strings. Keep the storage fixed and allocation-free.
const VARIABLES: usize = 16;
const VALUE_MAX: usize = 128;

#[derive(Clone, Copy)]
pub struct Message {
    pub kind: Kind,
    pub register: u8,
    pub length: u16,
    pub bytes: [u8; BODY_MAX],
}

impl Message {
    const EMPTY: Self = Self { kind: Kind::ReadFixed, register: 0, length: 0, bytes: [0; BODY_MAX] };

    pub fn payload(&self) -> &[u8] {
        &self.bytes[..self.length as usize]
    }
}

struct Queue<const N: usize> {
    slots: [Message; N],
    order: [u8; N],
    count: usize,
}

impl<const N: usize> Queue<N> {
    const fn new() -> Self {
        Self { slots: [Message::EMPTY; N], order: [0; N], count: 0 }
    }

    fn push(&mut self, kind: Kind, register: u8, payload: &[u8]) -> Result<(), Error> {
        if payload.len() > BODY_MAX {
            return Err(Error::Length);
        }
        if self.count == N {
            return Err(Error::OutputFull);
        }
        let slot = (0..N).find(|n| !self.order[..self.count].contains(&(*n as u8))).ok_or(Error::OutputFull)?;
        let item = &mut self.slots[slot];
        item.kind = kind;
        item.register = register;
        item.length = payload.len() as u16;
        item.bytes[..payload.len()].copy_from_slice(payload);
        self.order[self.count] = slot as u8;
        self.count += 1;
        Ok(())
    }

    fn get(&self, index: usize) -> Option<&Message> {
        (index < self.count).then(|| &self.slots[self.order[index] as usize])
    }

    fn remove(&mut self, index: usize) {
        self.order.copy_within(index + 1..self.count, index);
        self.count -= 1;
    }

    fn matching(&self, register: u8) -> Option<usize> {
        (0..self.count).find(|&n| self.get(n).is_some_and(|m| m.register == register))
    }
}

#[derive(Clone, Copy)]
struct Variable {
    register: u8,
    length: u8,
    valid: bool,
    bytes: [u8; VALUE_MAX],
}

pub struct Registers {
    fixed: [[u8; 2]; 256],
    fixed_valid: [u8; 32],
    variables: [Variable; VARIABLES],
    notes: Queue<NOTES>,
    writes: Queue<WRITES>,
}

impl Default for Registers {
    fn default() -> Self {
        Self::new()
    }
}

impl Registers {
    pub const fn new() -> Self {
        let mut bank = Self {
            fixed: [[0; 2]; 256],
            fixed_valid: [0; 32],
            variables: [Variable { register: 0, length: 0, valid: false, bytes: [0; VALUE_MAX] }; VARIABLES],
            notes: Queue::new(),
            writes: Queue::new(),
        };
        // A direct 0x11 poll can arrive before userspace. Zero leaves the
        // Armada startup callback armed until Linux publishes its ready reply.
        bank.set_fixed(0x11, [0, 0]);
        bank
    }

    pub const fn set_fixed(&mut self, register: u8, value: [u8; 2]) {
        self.fixed[register as usize] = value;
        self.fixed_valid[register as usize / 8] |= 1 << (register % 8);
    }

    pub fn fixed(&self, register: u8) -> Option<[u8; 2]> {
        (self.fixed_valid[register as usize / 8] & (1 << (register % 8)) != 0).then(|| self.fixed[register as usize])
    }

    pub fn set_variable(&mut self, register: u8, value: &[u8]) -> Result<(), Error> {
        if value.len() > VALUE_MAX {
            return Err(Error::Length);
        }
        let index = self
            .variables
            .iter()
            .position(|v| v.valid && v.register == register)
            .or_else(|| self.variables.iter().position(|v| !v.valid))
            .ok_or(Error::OutputFull)?;
        let item = &mut self.variables[index];
        item.register = register;
        item.length = value.len() as u8;
        item.bytes[..value.len()].copy_from_slice(value);
        item.valid = true;
        Ok(())
    }

    /// Each notification is retained, including successive values of one
    /// register. Power/boot events must not be silently replaced by later ones.
    pub fn notify(&mut self, kind: Kind, register: u8, value: &[u8]) -> Result<(), Error> {
        match kind {
            Kind::ReadFixed if value.len() == 2 => {}
            Kind::ReadVariable => {}
            _ => return Err(Error::Operation),
        }
        self.notes.push(kind, register, value)
    }

    pub fn pop_write(&mut self) -> Option<Message> {
        let item = *self.writes.get(0)?;
        self.writes.remove(0);
        Some(item)
    }

    pub fn peek_write(&self) -> Option<&Message> {
        self.writes.get(0)
    }

    pub fn has_writes(&self) -> bool {
        self.writes.count != 0
    }

    pub fn pending_count(&self) -> usize {
        self.notes.count
    }

    pub fn notification_room(&self) -> bool {
        self.notes.count < NOTES
    }
}

fn limit(bytes: &[u8], maximum: u16) -> &[u8] {
    // The stock named-read path treats zero as no maximum, not an empty read.
    if maximum == 0 { bytes } else { &bytes[..bytes.len().min(maximum as usize)] }
}

impl Endpoint for Registers {
    fn pending(&self) -> bool {
        self.notes.count != 0
    }

    fn serve(&mut self, request: Request<'_>, reply: &mut impl Sink) -> Result<(), Error> {
        let h = request.header;
        if h.kind.is_write() {
            if request.payload.len() > BODY_MAX || (h.kind == Kind::WriteFixed && request.payload.len() != 2) {
                return Err(Error::Length);
            }
            if self.writes.count == WRITES {
                return Err(Error::OutputFull);
            }
            // Encode before committing: an insufficient output buffer must
            // neither acknowledge nor apply a write that the host will retry.
            write_ack(h, reply)?;
            self.writes.push(h.kind, h.register, request.payload)?;
            // On these two registers 03 00 asks for the configured value.
            // Armada can issue a named read before userspace pops the query.
            let query = matches!(h.register, 0x81 | 0x87) && request.payload == [3, 0];
            if h.kind == Kind::WriteFixed && !query {
                self.set_fixed(h.register, [request.payload[0], request.payload[1]]);
            }
            return Ok(());
        }
        if !matches!(h.kind, Kind::ReadFixed | Kind::ReadVariable) {
            return Err(Error::Operation);
        }
        if h.register == 0x0c {
            let next = self.notes.get(0).map_or(0, |m| m.register);
            return read_reply(h.kind, h.register, &[next, 0], reply);
        }
        let queued =
            if h.register == 0 { (self.notes.count != 0).then_some(0) } else { self.notes.matching(h.register) };
        if let Some(index) = queued {
            let item = self.notes.get(index).ok_or(Error::Operation)?;
            // A queue-drain returns the queued register and kind, even when
            // the host asked for variable register 0 to discover its contents.
            let kind = if h.register == 0 { item.kind } else { h.kind };
            let bytes = if kind == Kind::ReadVariable && h.register != 0 {
                limit(item.payload(), h.length)
            } else {
                item.payload()
            };
            read_reply(kind, item.register, bytes, reply)?;
            self.notes.remove(index);
            return Ok(());
        }
        if h.register == 0 {
            return read_reply(h.kind, 0, &[0, 0], reply);
        }
        match h.kind {
            Kind::ReadFixed => read_reply(h.kind, h.register, &self.fixed(h.register).ok_or(Error::Operation)?, reply),
            Kind::ReadVariable => {
                let item =
                    self.variables.iter().find(|v| v.valid && v.register == h.register).ok_or(Error::Operation)?;
                read_reply(h.kind, h.register, limit(&item.bytes[..item.length as usize], h.length), reply)
            }
            _ => Err(Error::Operation),
        }
    }
}

#[cfg(test)]
mod tests;
