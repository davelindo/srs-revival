use super::*;
use x7_protocol::{Buffer, Header};
use x7_transport::WIRE_MAX;

fn request(kind: Kind, register: u8, length: u16) -> Request<'static> {
    Request { header: Header { kind, register, length }, payload: &[] }
}

fn expected(kind: Kind, register: u8, payload: &[u8]) -> Buffer<WIRE_MAX> {
    let mut out = Buffer::new();
    read_reply(kind, register, payload, &mut out).unwrap();
    out
}

#[test]
fn first_direct_status_poll_keeps_the_armada_boot_callback_armed() {
    let mut bank = Registers::new();
    let mut reply = Buffer::<WIRE_MAX>::new();
    bank.serve(request(Kind::ReadFixed, 0x11, 0), &mut reply).unwrap();
    assert_eq!(reply.bytes(), expected(Kind::ReadFixed, 0x11, &[0, 0]).bytes());
    assert!(!bank.pending());
}

#[test]
fn immediate_named_read_of_configuration_returns_the_provisioned_value() {
    let mut bank = Registers::new();
    for (register, configured) in [(0x81, 1), (0x87, 2)] {
        bank.set_fixed(register, [configured, 0]);
        let write = Request { header: Header { kind: Kind::WriteFixed, register, length: 0 }, payload: &[3, 0] };
        let mut out = Buffer::<WIRE_MAX>::new();
        bank.serve(write, &mut out).unwrap();
        out.clear();
        bank.serve(request(Kind::ReadFixed, register, 0), &mut out).unwrap();
        assert_eq!(out.bytes(), expected(Kind::ReadFixed, register, &[configured, 0]).bytes());
        let queued = bank.pop_write().unwrap();
        assert_eq!((queued.register, queued.payload()), (register, &[3, 0][..]));
    }
}

#[test]
fn pending_peek_and_drain_preserve_successive_boot_statuses() {
    let mut bank = Registers::new();
    bank.notify(Kind::ReadFixed, 0x11, &[0x20, 0]).unwrap();
    bank.notify(Kind::ReadVariable, 0x25, b"s9602.21.2.09\0").unwrap();
    bank.notify(Kind::ReadFixed, 0x11, &[0x21, 0]).unwrap();
    let mut out = Buffer::<WIRE_MAX>::new();
    bank.serve(request(Kind::ReadFixed, 0x0c, 0), &mut out).unwrap();
    assert_eq!(out.bytes(), expected(Kind::ReadFixed, 0x0c, &[0x11, 0]).bytes());
    assert_eq!(bank.pending_count(), 3);
    for (kind, register, payload) in [
        (Kind::ReadFixed, 0x11, &[0x20, 0][..]),
        (Kind::ReadVariable, 0x25, &b"s9602.21.2.09\0"[..]),
        (Kind::ReadFixed, 0x11, &[0x21, 0][..]),
    ] {
        out.clear();
        bank.serve(request(Kind::ReadVariable, 0, 512), &mut out).unwrap();
        assert_eq!(out.bytes(), expected(kind, register, payload).bytes());
    }
    assert!(!bank.pending());
}

#[test]
fn named_read_removes_only_first_match_and_honors_nonzero_maximum() {
    let mut bank = Registers::new();
    bank.notify(Kind::ReadVariable, 0x25, b"first\0").unwrap();
    bank.notify(Kind::ReadFixed, 0x11, &[0x21, 0]).unwrap();
    bank.notify(Kind::ReadVariable, 0x25, b"second\0").unwrap();
    let mut out = Buffer::<WIRE_MAX>::new();
    bank.serve(request(Kind::ReadVariable, 0x25, 3), &mut out).unwrap();
    assert_eq!(out.bytes(), expected(Kind::ReadVariable, 0x25, b"fir").bytes());
    out.clear();
    bank.serve(request(Kind::ReadVariable, 0, 512), &mut out).unwrap();
    assert_eq!(out.bytes(), expected(Kind::ReadFixed, 0x11, &[0x21, 0]).bytes());
    out.clear();
    bank.serve(request(Kind::ReadVariable, 0x25, 0), &mut out).unwrap();
    assert_eq!(out.bytes(), expected(Kind::ReadVariable, 0x25, b"second\0").bytes());
    assert!(!bank.pending());
}

#[test]
fn full_output_or_queue_does_not_commit_a_write() {
    let mut bank = Registers::new();
    let write =
        || Request { header: Header { kind: Kind::WriteFixed, register: 0x41, length: 0 }, payload: &[0x21, 0] };
    assert_eq!(bank.serve(write(), &mut Buffer::<1>::new()), Err(Error::OutputFull));
    assert!(!bank.has_writes());
    assert_eq!(bank.fixed(0x41), None);
    for _ in 0..WRITES {
        bank.serve(write(), &mut Buffer::<WIRE_MAX>::new()).unwrap();
    }
    assert_eq!(bank.serve(write(), &mut Buffer::<WIRE_MAX>::new()), Err(Error::OutputFull));
    for _ in 0..WRITES {
        assert_eq!(bank.pop_write().unwrap().payload(), &[0x21, 0]);
    }
    assert!(bank.pop_write().is_none());
}

#[test]
fn notification_overflow_preserves_existing_order() {
    let mut bank = Registers::new();
    for n in 0..NOTES {
        bank.notify(Kind::ReadFixed, 0x11, &[n as u8, 0]).unwrap();
    }
    assert_eq!(bank.notify(Kind::ReadFixed, 0x11, &[99, 0]), Err(Error::OutputFull));
    assert_eq!(bank.serve(request(Kind::ReadVariable, 0, 512), &mut Buffer::<1>::new()), Err(Error::OutputFull));
    assert_eq!(bank.pending_count(), NOTES);
    for n in 0..NOTES {
        let mut out = Buffer::<WIRE_MAX>::new();
        bank.serve(request(Kind::ReadVariable, 0, 512), &mut out).unwrap();
        assert_eq!(out.bytes(), expected(Kind::ReadFixed, 0x11, &[n as u8, 0]).bytes());
    }
}

#[test]
fn cached_values_empty_queue_and_unknown_registers_are_distinct() {
    let mut bank = Registers::new();
    bank.set_fixed(0x11, [0x21, 0]);
    bank.set_variable(0x25, b"version\0").unwrap();
    let mut out = Buffer::<WIRE_MAX>::new();
    bank.serve(request(Kind::ReadVariable, 0, 0), &mut out).unwrap();
    assert_eq!(out.bytes(), expected(Kind::ReadVariable, 0, &[0, 0]).bytes());
    out.clear();
    bank.serve(request(Kind::ReadVariable, 0x25, 4), &mut out).unwrap();
    assert_eq!(out.bytes(), expected(Kind::ReadVariable, 0x25, b"vers").bytes());
    assert_eq!(bank.serve(request(Kind::ReadFixed, 0xfa, 0), &mut Buffer::<WIRE_MAX>::new()), Err(Error::Operation));
    assert!(!bank.pending());
    assert!(core::mem::size_of::<Registers>() <= 9 * 1024);
}

#[test]
fn all_boot_and_network_strings_can_remain_cached_together() {
    let mut bank = Registers::new();
    let registers = [0x25, 0x26, 0x27, 0x90, 0x33, 0x38, 0x39, 0x3a, 0x3e, 0x3f, 0x58, 0x5c, 0x5d, 0x5e, 0x65, 0x66];
    for register in registers {
        bank.set_variable(register, &[register, 0]).unwrap();
    }
    for register in registers {
        let mut out = Buffer::<WIRE_MAX>::new();
        bank.serve(request(Kind::ReadVariable, register, 0), &mut out).unwrap();
        assert_eq!(out.bytes(), expected(Kind::ReadVariable, register, &[register, 0]).bytes());
    }
    assert_eq!(bank.set_variable(0x80, b"overflow"), Err(Error::OutputFull));
}
