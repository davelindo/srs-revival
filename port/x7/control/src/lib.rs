//! A fixed, versioned Linux ioctl boundary. Bytes have the same layout on the
//! ARM kernel, ARM userspace and native diagnostic tools. No native pointers or
//! native-endian integers are embedded in a message.
#![no_std]
#![forbid(unsafe_code)]

pub const BODY_MAX: usize = 512;
pub const PACKET_SIZE: usize = 8 + BODY_MAX;
pub const IOCTL: u32 = 0xc208_7801; // _IOWR('x', 1, [u8; 520]) on ARM Linux
pub const SET_FIXED: u8 = 0;
pub const SET_VARIABLE: u8 = 1;
pub const NOTIFY: u8 = 2;
pub const POP_WRITE: u8 = 3;
pub const STATUS: u8 = 4;
pub const ABI: u8 = 1;
/// Exported by the kernel module in sysfs so an older rescue can hand off to a newer slot.
pub const DRIVER_MODINFO: &[u8; 10] = b"version=5\0";

#[derive(Clone)]
#[repr(C)]
pub struct Packet {
    pub operation: u8,
    pub kind: u8,
    pub register: u8,
    pub abi: u8,
    pub length_le: [u8; 2],
    pub reserved: [u8; 2],
    pub data: [u8; BODY_MAX],
}

impl Packet {
    pub const fn new(operation: u8) -> Self {
        Self { operation, kind: 0, register: 0, abi: ABI, length_le: [0; 2], reserved: [0; 2], data: [0; BODY_MAX] }
    }

    pub fn length(&self) -> usize {
        u16::from_le_bytes(self.length_le) as usize
    }

    pub fn set_data(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > BODY_MAX {
            return false;
        }
        self.data[..bytes.len()].copy_from_slice(bytes);
        self.length_le = (bytes.len() as u16).to_le_bytes();
        true
    }

    pub fn payload(&self) -> Option<&[u8]> {
        self.data.get(..self.length())
    }
}

const _: () = assert!(core::mem::size_of::<Packet>() == PACKET_SIZE);
