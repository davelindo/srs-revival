//! Sony's GPIO handshakes around the DM870 SPI slave. No allocation, scheduler
//! or Linux dependency: the boot stub can poll it; an IRQ backend can service it.
#![no_std]

use dm870::{
    Mmio,
    gpio::{Level, Pin, gpio},
    pads::{Mtx, Routing},
    spi_slave::{TX_DEPTH, spi_slave},
};
use x7_protocol::{Buffer, Error, Kind, Parser, Request, Sink, error_reply};

/// Sony runtime GPIO object indexes, not the Zeppelin carrier's signal names.
pub const REQUEST: Pin = Pin::HostNireq; // GPIO 6
pub const READY: Pin = Pin::IrqTunerMod; // GPIO 18
pub const SELECT: Pin = Pin::FactReset; // GPIO 4, active low on Sony

/// Read from this X7, all sixteen pad-matrix copies. WILD is owned by its driver.
pub const SONY_PADS: Routing = Routing(&[
    (Mtx::GpioGroupA, 0xf02),
    (Mtx::GpioGroupB, 0xf03),
    (Mtx::GpioGroupC, 0xf01),
    (Mtx::GpioGroupD, 0xf06),
    (Mtx::GpioGroupE, 0x303),
    (Mtx::GpioGroupF, 0x307),
    (Mtx::Usb, 0x20),
    (Mtx::Ethernet, 1),
    (Mtx::AllIns, 0),
]);

// The original Sony receiver clamps its body reads to 0x200 bytes.
pub const BODY_MAX: usize = 512;
pub const WIRE_MAX: usize = 2 * (BODY_MAX + 6);

pub trait Endpoint {
    fn serve(&mut self, request: Request<'_>, reply: &mut impl Sink) -> Result<(), Error>;
    fn pending(&self) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Receive,
    RequestEnd,
    Reply,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Counters {
    pub requests: u32,
    pub bad_frames: u32,
    pub retries: u32,
    pub reply_aborts: u32,
}

/// Bounded hardware evidence from driver start. RX holds the first header bytes seen in
/// the FIFO. TX holds the prefix offered on the first reply, even if the master abandons it.
#[derive(Default, Clone, Copy)]
pub struct FirstTransfer {
    pub rx: [u8; 5],
    pub rx_len: usize,
    pub tx: [u8; 8],
    pub tx_len: usize,
    pub drained: Option<bool>,
}

pub struct Transport<M: Mmio> {
    bus: M,
    parser: Parser<BODY_MAX>,
    reply: Buffer<WIRE_MAX>,
    sent: usize,
    phase: Phase,
    reply_selected: bool,
    received: bool,
    request_level: bool,
    pub counters: Counters,
    pub first: FirstTransfer,
}

impl<M: Mmio> Transport<M> {
    pub fn new(mut bus: M) -> Self {
        // Idle outputs are driven low on the captured Sony board, not released.
        gpio(&mut bus).drive(REQUEST, Level::Low);
        gpio(&mut bus).drive(READY, Level::Low);
        // Sony leaves GPIO 4 as an input with its pull-up latch clear.
        gpio(&mut bus).release(SELECT, false);
        spi_slave(&mut bus).init();
        Self {
            bus,
            parser: Parser::new(),
            reply: Buffer::new(),
            sent: 0,
            phase: Phase::Receive,
            reply_selected: false,
            received: false,
            request_level: false,
            counters: Counters::default(),
            first: FirstTransfer::default(),
        }
    }

    pub fn bus(&mut self) -> &mut M {
        &mut self.bus
    }

    pub fn busy(&self) -> bool {
        self.phase != Phase::Receive || self.received
    }

    /// Enable the TX-space interrupt only while an encoded reply needs refill.
    /// Leaving it enabled on an empty FIFO would create a permanent IRQ storm.
    pub fn needs_tx(&self) -> bool {
        self.phase == Phase::Reply && self.sent < self.reply.bytes().len()
    }

    /// Recover an abandoned physical transaction. Keep the last encoded reply
    /// so the controller can still request its CRC-protected retry.
    pub fn abort(&mut self) {
        if self.first.tx_len != 0 && self.first.drained.is_none() {
            self.first.drained = Some(false);
        }
        self.parser.reset();
        self.bus.w32(dm870::spi_slave::BASE + dm870::uart::fifo::RX_RESET, 0);
        self.bus.w32(dm870::spi_slave::BASE + dm870::uart::fifo::TX_RESET, 0);
        gpio(&mut self.bus).drive(READY, Level::Low);
        self.phase = Phase::Receive;
        self.received = false;
        self.reply_selected = false;
        self.counters.reply_aborts = self.counters.reply_aborts.wrapping_add(1);
    }

    fn prepare_error(&mut self, error: Error) {
        self.reply.clear();
        // A five-byte error frame always fits WIRE_MAX.
        let _ = error_reply(error, &mut self.reply);
        self.counters.bad_frames = self.counters.bad_frames.wrapping_add(1);
        self.phase = Phase::RequestEnd;
        self.sent = 0;
    }

    /// Bounded work per call. Runtime scheduling/IRQ delivery is the caller's;
    /// this function never sleeps, spins waiting for hardware, or allocates.
    pub fn service(&mut self, endpoint: &mut impl Endpoint) {
        let selected = gpio(&mut self.bus).level(SELECT) == Level::Low;
        if self.phase == Phase::Reply && selected {
            self.reply_selected = true;
        }
        // Drain at most one FIFO's worth, including bytes arriving concurrently.
        for _ in 0..64 {
            let Some(byte) = spi_slave(&mut self.bus).rx() else {
                break;
            };
            if self.phase != Phase::Receive {
                continue;
            }
            if byte != x7_protocol::IDLE {
                if self.first.tx_len == 0 && self.first.rx_len < self.first.rx.len() {
                    self.first.rx[self.first.rx_len] = byte;
                    self.first.rx_len += 1;
                }
                self.received = true;
            }
            let Some(result) = self.parser.push(byte) else {
                continue;
            };
            match result {
                Ok(request) if request.header.kind == Kind::Retry => {
                    self.counters.retries = self.counters.retries.wrapping_add(1);
                    if self.reply.bytes().is_empty() {
                        self.prepare_error(Error::Operation);
                    } else {
                        self.phase = Phase::RequestEnd;
                        self.sent = 0;
                    }
                }
                Ok(request) => {
                    self.reply.clear();
                    let result = endpoint.serve(request, &mut self.reply);
                    if let Err(error) = result {
                        self.prepare_error(error);
                    } else {
                        self.counters.requests = self.counters.requests.wrapping_add(1);
                        self.phase = Phase::RequestEnd;
                        self.sent = 0;
                    }
                }
                Err(error) => self.prepare_error(error),
            }
        }
        let selected = gpio(&mut self.bus).level(SELECT) == Level::Low;
        if self.phase == Phase::Receive && self.received && !selected {
            // The select ended before a complete valid packet arrived.
            self.parser.reset();
            self.prepare_error(Error::Length);
        }
        if self.phase == Phase::RequestEnd && !selected {
            self.parser.reset();
            self.received = false;
            self.phase = Phase::Reply;
            self.reply_selected = false;
        }
        if self.phase == Phase::Reply {
            if self.first.tx_len == 0 {
                let len = self.first.tx.len().min(self.reply.bytes().len());
                self.first.tx[..len].copy_from_slice(&self.reply.bytes()[..len]);
                self.first.tx_len = len;
            }
            while self.sent < self.reply.bytes().len() {
                let byte = self.reply.bytes()[self.sent];
                if !spi_slave(&mut self.bus).tx(byte) {
                    break;
                }
                self.sent += 1;
            }
            let complete = self.sent == self.reply.bytes().len() && spi_slave(&mut self.bus).tx_free() == TX_DEPTH;
            if !selected && (complete || self.reply_selected) {
                self.first.drained.get_or_insert(complete);
                if !complete {
                    // Drop unclocked TX bytes while select is released. Resetting the FIFO
                    // in the next command can disturb the shifter in the middle of its body.
                    self.bus.w32(dm870::spi_slave::BASE + dm870::uart::fifo::TX_RESET, 0);
                    self.counters.reply_aborts = self.counters.reply_aborts.wrapping_add(1);
                }
                gpio(&mut self.bus).drive(READY, Level::Low);
                self.phase = Phase::Receive;
                self.reply_selected = false;
            } else {
                gpio(&mut self.bus).drive(READY, Level::High);
            }
        }
        let pending = endpoint.pending();
        if pending != self.request_level {
            gpio(&mut self.bus).drive(REQUEST, if pending { Level::High } else { Level::Low });
            self.request_level = pending;
        }
    }
}

#[cfg(test)]
mod tests;
