extern crate std;

use super::*;
use dm870::{gpio, spi_slave, uart::fifo};
use std::{collections::VecDeque, vec::Vec};
use x7_protocol::{Header, encode_request, read_reply, write_ack};

struct Bus {
    rx: VecDeque<u8>,
    tx: VecDeque<u8>,
    a: u32,
    b: u32,
    selected: bool,
    selected_tx_resets: usize,
}

impl Bus {
    fn new() -> Self {
        Self { rx: VecDeque::new(), tx: VecDeque::new(), a: !0, b: 0, selected: false, selected_tx_resets: 0 }
    }
    fn ready(&self) -> bool {
        self.b & (1 << 18) != 0
    }
    fn receive(&mut self, data: &[u8]) {
        for &byte in data {
            assert!(self.rx.len() < 63, "emulated hardware FIFO overrun");
            self.rx.push_back(byte);
        }
    }
}

impl Mmio for Bus {
    fn r32(&mut self, pa: u32) -> u32 {
        match pa {
            p if p == spi_slave::BASE + fifo::RX_FILL => self.rx.len() as u32,
            p if p == spi_slave::BASE + fifo::TX_FREE => 63 - self.tx.len() as u32,
            p if p == gpio::BASE + gpio::reg::HIGH => (self.b & !(1 << 4)) | if self.selected { 0 } else { 1 << 4 },
            _ => 0,
        }
    }
    fn r8(&mut self, pa: u32) -> u8 {
        assert_eq!(pa, spi_slave::BASE + fifo::RX_DATA);
        self.rx.pop_front().unwrap_or(0xff)
    }
    fn w32(&mut self, pa: u32, value: u32) {
        match pa {
            p if p == spi_slave::BASE + fifo::RX_RESET => self.rx.clear(),
            p if p == spi_slave::BASE + fifo::TX_RESET => {
                self.selected_tx_resets += usize::from(self.selected);
                self.tx.clear();
            }
            p if p == gpio::BASE + gpio::reg::HIGH => self.b |= value,
            p if p == gpio::BASE + gpio::reg::LOW => self.b &= !value,
            p if p == gpio::BASE + gpio::reg::DRIVE => self.a &= !value,
            p if p == gpio::BASE + gpio::reg::RELEASE => self.a |= value,
            _ => {}
        }
    }
    fn w8(&mut self, pa: u32, value: u8) {
        assert_eq!(pa, spi_slave::BASE + fifo::TX_DATA);
        assert!(self.tx.len() < 63);
        self.tx.push_back(value);
    }
}

struct Register {
    value: Vec<u8>,
    pending: bool,
    calls: usize,
    written: Vec<u8>,
}

impl Register {
    fn new(value: &[u8]) -> Self {
        Self { value: value.into(), pending: true, calls: 0, written: Vec::new() }
    }
}

impl Endpoint for Register {
    fn pending(&self) -> bool {
        self.pending
    }
    fn serve(&mut self, request: Request<'_>, reply: &mut impl Sink) -> Result<(), Error> {
        self.calls += 1;
        if request.header.kind.is_write() {
            self.written.extend_from_slice(request.payload);
            write_ack(request.header, reply)
        } else {
            self.pending = false;
            read_reply(request.header.kind, request.header.register, &self.value, reply)
        }
    }
}

fn request(kind: Kind, register: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Buffer::<WIRE_MAX>::new();
    let length = if kind == Kind::WriteVariable {
        payload.len() as u16
    } else if kind == Kind::ReadVariable {
        512
    } else {
        0
    };
    encode_request(Header { kind, register, length }, payload, &mut out).unwrap();
    out.bytes().into()
}

fn exchange(
    t: &mut Transport<Bus>,
    ep: &mut Register,
    input: &[u8],
    output_size: usize,
    service_in_request: bool,
) -> Vec<u8> {
    t.bus.selected = true;
    for block in input.chunks(32) {
        t.bus.receive(block);
        if service_in_request {
            t.service(ep);
        }
    }
    assert!(!t.bus.ready(), "response-ready must stay low through request selection");
    t.bus.selected = false;
    t.service(ep);
    assert!(t.bus.ready(), "release of request selection must expose the response");
    t.bus.selected = true;
    t.service(ep);
    let mut response = Vec::new();
    for index in 0..output_size {
        response.push(t.bus.tx.pop_front().expect("reply FIFO starved"));
        if index % 16 == 15 {
            t.service(ep);
        }
    }
    t.bus.selected = false;
    t.service(ep);
    assert!(!t.bus.ready());
    response
}

#[test]
fn pending_uses_the_sony_output_pin_and_drives_both_levels() {
    let mut t = Transport::new(Bus::new());
    let mut ep = Register::new(&[0x21, 0]);
    t.service(&mut ep);
    assert_eq!((t.bus.a & (1 << 6), t.bus.b & (1 << 6)), (0, 1 << 6));
    ep.pending = false;
    t.service(&mut ep);
    assert_eq!((t.bus.a & (1 << 6), t.bus.b & (1 << 6)), (0, 0));
}

#[test]
fn a_complete_sony_status_transaction_uses_the_ready_handshake() {
    let mut t = Transport::new(Bus::new());
    let mut ep = Register::new(&[0x21, 0]);
    let response = exchange(&mut t, &mut ep, &request(Kind::ReadFixed, 0x11, &[]), 8, true);
    assert_eq!(&response[..7], &[1, 0x11, 0, 0, 0x7f, 0x21, 0]);
    assert_eq!(response[7], x7_protocol::crc8(&[0x21, 0]));
    assert_eq!(ep.calls, 1);
    assert_eq!(t.counters.bad_frames, 0);
    assert!(!t.busy());
}

#[test]
fn a_request_clocked_between_service_calls_is_still_answered() {
    let mut t = Transport::new(Bus::new());
    let mut ep = Register::new(&[0x21, 0]);
    let response = exchange(&mut t, &mut ep, &request(Kind::ReadFixed, 0x11, &[]), 8, false);
    assert_eq!(&response[5..7], &[0x21, 0]);
    assert_eq!(ep.calls, 1);
}

#[test]
fn a_long_raw_reply_refills_the_hardware_fifo() {
    let value = [0xff; 256];
    let mut expected = Buffer::<WIRE_MAX>::new();
    read_reply(Kind::ReadVariable, 0x25, &value, &mut expected).unwrap();
    assert_eq!(expected.bytes().len(), value.len() + 6);
    let mut t = Transport::new(Bus::new());
    let mut ep = Register::new(&value);
    let actual = exchange(&mut t, &mut ep, &request(Kind::ReadVariable, 0x25, &[]), expected.bytes().len(), true);
    assert_eq!(actual, expected.bytes());
}

#[test]
fn a_long_write_is_delivered_once_and_acknowledged_without_echoing_the_body() {
    let mut t = Transport::new(Bus::new());
    let mut ep = Register::new(&[]);
    let value = [0x27; 130];
    let actual = exchange(&mut t, &mut ep, &request(Kind::WriteVariable, 0x40, &value), 5, true);
    assert_eq!(&actual[..4], &[2, 0x40, 130, 0]);
    assert_eq!(ep.written, value);
    assert_eq!(ep.calls, 1);
    assert_eq!(t.bus.selected_tx_resets, 0);
}

#[test]
fn an_abandoned_reply_clears_stale_tx_at_deselect_before_the_next_write() {
    let mut t = Transport::new(Bus::new());
    let mut ep = Register::new(&[0x21, 0]);
    let _ = exchange(&mut t, &mut ep, &request(Kind::ReadFixed, 0x11, &[]), 3, true);
    assert_eq!(t.first.drained, Some(false));
    assert!(t.bus.tx.is_empty());
    let ack = exchange(&mut t, &mut ep, &request(Kind::WriteFixed, 0x48, &[1, 0]), 5, true);
    assert_eq!(&ack[..2], &[0, 0x48]);
    assert_eq!(t.bus.selected_tx_resets, 0);
}

#[test]
fn corrupt_data_is_rejected_before_an_endpoint_can_act_on_it() {
    let mut t = Transport::new(Bus::new());
    let mut ep = Register::new(&[]);
    let mut bad = request(Kind::WriteFixed, 0x76, &[5, 0]);
    *bad.last_mut().unwrap() ^= 1;
    let response = exchange(&mut t, &mut ep, &bad, 5, true);
    assert_eq!(&response[..4], &[0x0d, 0xe1, 0, 0]);
    assert_eq!(ep.calls, 0);
    assert!(ep.written.is_empty());
}

#[test]
fn retry_replays_the_last_reply_without_repeating_application_side_effects() {
    let mut t = Transport::new(Bus::new());
    let mut ep = Register::new(&[0x21, 0]);
    let original = exchange(&mut t, &mut ep, &request(Kind::ReadFixed, 0x11, &[]), 8, true);
    let retried = exchange(&mut t, &mut ep, &request(Kind::Retry, 0, &[]), 8, true);
    assert_eq!(original, retried);
    assert_eq!(t.first.rx[..t.first.rx_len], request(Kind::ReadFixed, 0x11, &[]));
    assert_eq!(t.first.tx[..t.first.tx_len], original);
    assert_eq!(t.first.drained, Some(true));
    assert_eq!(ep.calls, 1);
    assert_eq!(t.counters.retries, 1);
}

#[test]
fn the_transport_fits_its_two_kibibyte_static_budget() {
    assert!(core::mem::size_of::<Transport<dm870::Physical>>() <= 2048);
}

#[test]
fn tx_space_interrupt_is_requested_only_until_the_reply_is_buffered() {
    let mut t = Transport::new(Bus::new());
    let mut ep = Register::new(&[0xff; 256]);
    assert!(!t.needs_tx());
    t.bus.selected = true;
    t.bus.receive(&request(Kind::ReadVariable, 0x25, &[]));
    t.service(&mut ep);
    assert!(!t.needs_tx());
    t.bus.selected = false;
    t.service(&mut ep);
    assert!(t.needs_tx());
    while t.needs_tx() {
        t.bus.tx.clear();
        t.service(&mut ep);
    }
    assert!(t.bus.ready());
    t.bus.tx.clear();
    t.service(&mut ep);
    assert!(!t.busy());
    assert!(!t.needs_tx());
}

#[test]
fn an_abandoned_reply_recovers_and_retains_the_retry_cache() {
    let mut t = Transport::new(Bus::new());
    let mut ep = Register::new(&[0x21, 0]);
    t.bus.selected = true;
    t.bus.receive(&request(Kind::ReadFixed, 0x11, &[]));
    t.service(&mut ep);
    t.bus.selected = false;
    t.service(&mut ep);
    let expected: Vec<_> = t.bus.tx.iter().copied().collect();
    assert!(t.bus.ready());
    t.abort();
    assert!(!t.bus.ready());
    assert!(t.bus.tx.is_empty());
    assert!(!t.busy());
    let retried = exchange(&mut t, &mut ep, &request(Kind::Retry, 0, &[]), expected.len(), true);
    assert_eq!(retried, expected);
    assert_eq!(ep.calls, 1);
}
