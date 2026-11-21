use super::*;

fn encoded(kind: Kind, register: u8, payload: &[u8], length: u16) -> Buffer<1024> {
    let mut out = Buffer::new();
    encode_request(Header { kind, register, length }, payload, &mut out).unwrap();
    out
}

#[test]
fn read_requests_match_the_retained_sony_codec_vectors() {
    // firmware/x7_spi.py; its CRC and request construction were checked by
    // executing the two original firmware CRC routines and the MC builder.
    assert_eq!(encoded(Kind::ReadFixed, 0x11, &[], 0).bytes(), &[1, 0x11, 0, 0, 0x7f]);
    assert_eq!(encoded(Kind::ReadVariable, 0x25, &[], 20).bytes(), &[3, 0x25, 20, 0, 0x13]);
}

#[test]
fn crc_matches_the_vendor_bitwise_routine_for_every_byte_and_seed() {
    for seed in 0..=255u8 {
        for byte in 0..=255u8 {
            let mut expected = seed ^ byte;
            for _ in 0..8 {
                expected = (expected << 1) ^ if expected & 0x80 != 0 { 0x85 } else { 0 };
            }
            assert_eq!(CRC[(seed ^ byte) as usize], expected);
        }
    }
}

#[test]
fn all_operations_accept_only_a_complete_checked_packet() {
    for (kind, payload, length) in [
        (Kind::ReadFixed, &[][..], 0),
        (Kind::WriteFixed, &[0xff, 0x27][..], 0),
        (Kind::ReadVariable, &[][..], 0x1234),
        (Kind::WriteVariable, &[0xff, 0x27, 0x41, 0][..], 4),
        (Kind::WriteVariable, &[][..], 0),
        (Kind::Retry, &[][..], 0),
    ] {
        let bytes = encoded(kind, 0x27, payload, length);
        let mut parser = Parser::<256>::new();
        for (index, &byte) in bytes.bytes().iter().enumerate() {
            let result = parser.push(byte);
            if index + 1 == bytes.bytes().len() {
                let request = result.unwrap().unwrap();
                assert_eq!(request.header, Header { kind, register: 0x27, length });
                assert_eq!(request.payload, payload);
            } else {
                assert!(result.is_none());
            }
        }
    }
}

#[test]
fn corruption_never_publishes_a_write() {
    let original = encoded(Kind::WriteFixed, 0x76, &[5, 0], 0);
    assert_eq!(original.bytes().len(), 8);
    for position in 0..8 {
        let mut parser = Parser::<16>::new();
        let mut rejected = false;
        for (i, &byte) in original.bytes().iter().enumerate() {
            match parser.push(if i == position { byte ^ 1 } else { byte }) {
                Some(Ok(_)) => panic!("corrupted byte {position} accepted"),
                Some(Err(_)) => rejected = true,
                None => {}
            }
        }
        assert!(rejected);
    }
}

#[test]
fn oversized_writes_are_drained_and_the_next_frame_survives() {
    let too_long = encoded(Kind::WriteVariable, 0x40, &[0xff; 16], 16);
    let next = encoded(Kind::ReadFixed, 0x11, &[], 0);
    let mut parser = Parser::<8>::new();
    let mut failures = 0;
    let mut reads = 0;
    for &byte in too_long.bytes().iter().chain(next.bytes()) {
        match parser.push(byte) {
            Some(Err(Error::Length)) => failures += 1,
            Some(Ok(request)) => {
                assert_eq!(request.header.register, 0x11);
                reads += 1;
            }
            Some(Err(error)) => panic!("unexpected error {error:?}"),
            None => {}
        }
    }
    assert_eq!((failures, reads), (1, 1));
}

#[test]
fn interrupted_transfers_cannot_publish_stale_data() {
    let incomplete = encoded(Kind::WriteFixed, 0x40, &[5, 0], 0);
    let next = encoded(Kind::ReadFixed, 0x11, &[], 0);
    let mut parser = Parser::<16>::new();
    for &byte in &incomplete.bytes()[..6] {
        assert!(parser.push(byte).is_none());
    }
    parser.reset();
    let mut completed = 0;
    for &byte in next.bytes() {
        if let Some(request) = parser.push(byte) {
            assert_eq!(request.unwrap().header.kind, Kind::ReadFixed);
            completed += 1;
        }
    }
    assert_eq!(completed, 1);
}

#[test]
fn fixed_write_ack_has_no_body_and_preserves_the_register() {
    let h = Header { kind: Kind::WriteFixed, register: 0x76, length: 0 };
    let mut out = Buffer::<16>::new();
    write_ack(h, &mut out).unwrap();
    assert_eq!(out.bytes().len(), 5);
    assert_eq!(&out.bytes()[..4], &[0, 0x76, 0, 0]);
    assert_eq!(out.bytes()[4], crc8(&[0, 0x76, 0, 0]));
}

#[test]
fn fixed_read_has_separate_header_and_data_checksums() {
    let mut out = Buffer::<16>::new();
    read_reply(Kind::ReadFixed, 0x11, &[0x21, 0], &mut out).unwrap();
    assert_eq!(out.bytes().len(), 8);
    assert_eq!(&out.bytes()[..4], &[1, 0x11, 0, 0]);
    assert_eq!(out.bytes()[4], crc8(&[1, 0x11, 0, 0]));
    assert_eq!(&out.bytes()[5..7], &[0x21, 0]);
    assert_eq!(out.bytes()[7], crc8(&[0x21, 0]));
}

#[test]
fn the_controller_reads_raw_header_crc_and_payload_octets() {
    let mut out = Buffer::<32>::new();
    read_reply(Kind::ReadVariable, 0x38, b"example.test\0", &mut out).unwrap();
    assert_eq!(&out.bytes()[..5], &[3, 0x38, 13, 0, 0x27]);
    assert_eq!(out.bytes().len(), 19);

    out.clear();
    read_reply(Kind::ReadVariable, 0x40, &[0xff, 0x27], &mut out).unwrap();
    assert_eq!(&out.bytes()[5..], &[0xff, 0x27, crc8(&[0xff, 0x27])]);
}

#[test]
fn only_the_controller_to_module_direction_escapes_the_header_crc() {
    let h = Header { kind: Kind::WriteVariable, register: 0x40, length: 15 };
    let mut buffer = Buffer::<64>::new();
    encode_request(h, &[0xff; 15], &mut buffer).unwrap();
    assert_eq!(&buffer.bytes()[..8], &[2, 0x40, 15, 0, 0x27, 0x27, 0x27, 1]);
    buffer.clear();
    write_ack(h, &mut buffer).unwrap();
    assert_eq!(buffer.bytes(), &[2, 0x40, 15, 0, 0x27]);
}

#[test]
fn full_output_is_an_error_instead_of_a_silently_truncated_packet() {
    let mut out = Buffer::<4>::new();
    assert_eq!(read_reply(Kind::ReadFixed, 0x11, &[0x21, 0], &mut out), Err(Error::OutputFull));
}
