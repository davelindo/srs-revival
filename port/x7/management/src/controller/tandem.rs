//! The native SongPal stream carried in Sony register 0x40.
//! Network 0x60254bc4 emits 01 20 LEN SOCKET FRAGMENT FRAME.
//! MC 0x58116d72 emits 01 21 LEN SOCKET 00 FRAME; it rejects LEN >= 254.
//! MC 0x58158b18 accepts complete incoming fragments (flag zero).

#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: u8,
    pub sequence: u8,
    pub payload: Vec<u8>,
}

fn wire(kind: u8, sequence: u8, payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > 244 {
        return None;
    }
    let mut body = vec![kind, sequence];
    body.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    body.extend_from_slice(payload);
    body.push(body.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
    let mut encoded = vec![0x3e];
    for byte in body {
        match byte {
            0x3c..=0x3e => encoded.extend_from_slice(&[0x3d, byte & !0x10]),
            _ => encoded.push(byte),
        }
    }
    encoded.push(0x3c);
    (encoded.len() <= 253).then_some(encoded)
}

pub fn envelope(kind: u8, sequence: u8, payload: &[u8]) -> Option<Vec<u8>> {
    let wire = wire(kind, sequence, payload)?;
    let mut envelope = vec![1, 0x20, wire.len() as u8, 0, 0];
    envelope.extend_from_slice(&wire);
    Some(envelope)
}

pub fn receive(envelope: &[u8]) -> Option<Frame> {
    if envelope.len() < 14
        || envelope[..2] != [1, 0x21]
        || envelope[3..5] != [0, 0]
        || envelope[2] as usize != envelope.len() - 5
    {
        return None;
    }
    let wire = &envelope[5..];
    if wire.first() != Some(&0x3e) || wire.last() != Some(&0x3c) {
        return None;
    }
    let mut decoded = Vec::with_capacity(wire.len());
    let mut escaped = false;
    for &byte in &wire[1..wire.len() - 1] {
        if escaped {
            if !(0x2c..=0x2e).contains(&byte) {
                return None;
            }
            decoded.push(byte | 0x10);
            escaped = false;
        } else if byte == 0x3d {
            escaped = true;
        } else if matches!(byte, 0x3c | 0x3e) {
            return None;
        } else {
            decoded.push(byte);
        }
    }
    if escaped || decoded.len() < 7 {
        return None;
    }
    let length = u32::from_be_bytes(decoded[2..6].try_into().ok()?) as usize;
    if length != decoded.len() - 7 {
        return None;
    }
    let checksum = decoded[..decoded.len() - 1].iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    if decoded.last() != Some(&checksum) {
        return None;
    }
    Some(Frame { kind: decoded[0], sequence: decoded[1], payload: decoded[6..6 + length].to_vec() })
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Power {
    pub network_standby: Option<bool>,
    pub auto_standby: Option<bool>,
    pub battery: Option<u16>,
}

impl Power {
    pub fn incoming(&mut self, payload: &[u8]) -> bool {
        if payload.len() < 8 || !matches!(payload[0], 0xf3 | 0xf5) || payload[1] != 0x12 || payload[5] != 0 {
            return false;
        }
        let category = u16::from_be_bytes([payload[2], payload[3]]);
        match category {
            0x0fff | 0x1fff if payload.len() == 9 && payload[6..8] == [1, 1] && payload[8] <= 1 => {
                let value = Some(payload[8] == 1);
                if category == 0x0fff {
                    self.network_standby = value;
                } else {
                    self.auto_standby = value;
                }
                true
            }
            0x20ff..=0x27ff if category & 0xff == 0xff => {
                self.battery = Some(category);
                true
            }
            _ => false,
        }
    }
}

pub fn query(category: u16) -> [u8; 4] {
    let [a, b] = category.to_be_bytes();
    [0xf2, 0x12, a, b]
}
pub fn set(category: u16, enabled: bool) -> [u8; 7] {
    let [a, b] = category.to_be_bytes();
    [0xf4, 0x12, a, b, 1, 1, u8::from(enabled)]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn battery_request_matches_the_retained_client_and_controller_capture() {
        assert_eq!(
            envelope(0, 0, &[0xf2, 0x12, 0x2f, 0xff]).unwrap(),
            vec![1, 0x20, 13, 0, 0, 0x3e, 0, 0, 0, 0, 0, 4, 0xf2, 0x12, 0x2f, 0xff, 0x36, 0x3c]
        );
        let ack = [1, 0x21, 9, 0, 0, 0x3e, 1, 1, 0, 0, 0, 0, 2, 0x3c];
        assert_eq!(receive(&ack), Some(Frame { kind: 1, sequence: 1, payload: vec![] }));
    }
    #[test]
    fn codec_preserves_escaped_bytes_and_rejects_corruption_and_other_sockets() {
        let mut data = envelope(0, 1, &[0x3c, 0x3d, 0x3e]).unwrap();
        data[1] = 0x21;
        assert_eq!(receive(&data), Some(Frame { kind: 0, sequence: 1, payload: vec![0x3c, 0x3d, 0x3e] }));
        data[3] = 1;
        assert_eq!(receive(&data), None);
        data[3] = 0;
        data[10] ^= 1;
        assert_eq!(receive(&data), None);
    }
    #[test]
    fn captured_battery_and_reconstructed_boolean_readback_layouts() {
        let mut power = Power::default();
        assert!(power.incoming(&[0xf3, 0x12, 0x21, 0xff, 0, 0, 0, 0]));
        assert!(!power.incoming(&[0xf3, 0x12, 0x22, 0xff, 0, 1, 0, 0]));
        assert_eq!(power.battery, Some(0x21ff));
        assert!(power.incoming(&[0xf3, 0x12, 0x0f, 0xff, 0, 0, 1, 1, 1]));
        assert_eq!(power.network_standby, Some(true));
        assert_eq!(set(0x0fff, true), [0xf4, 0x12, 0x0f, 0xff, 1, 1, 1]);
    }
}
