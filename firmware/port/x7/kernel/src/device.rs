//! Faultable userspace copies stay outside the driver's IRQ-disabled scopes.
use crate::{abi, runtime};
use core::{ffi::c_void, fmt::Write};
use x7_control::{PACKET_SIZE, Packet};

struct Text {
    bytes: [u8; 320],
    used: usize,
}
impl core::fmt::Write for Text {
    fn write_str(&mut self, value: &str) -> core::fmt::Result {
        let end = self.used.checked_add(value.len()).ok_or(core::fmt::Error)?;
        let destination = self.bytes.get_mut(self.used..end).ok_or(core::fmt::Error)?;
        destination.copy_from_slice(value.as_bytes());
        self.used = end;
        Ok(())
    }
}

pub unsafe extern "C" fn read(_file: *mut c_void, out: *mut u8, count: usize, position: *mut i64) -> isize {
    let c = runtime::snapshot();
    let mut text = Text { bytes: [0; 320], used: 0 };
    let _ = writeln!(
        text,
        "sony-x7-spi abi=1 backend=irq irq={} timers={} requests={} bad={} retries={} aborts={} pending={} busy={}",
        c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]
    );
    let first = runtime::first_transfer();
    let drained = match first.drained {
        None => "pending",
        Some(true) => "yes",
        Some(false) => "no",
    };
    let _ = writeln!(
        text,
        "first_rx={:02x?} first_tx={:02x?} first_fifo_drained={drained}",
        &first.rx[..first.rx_len],
        &first.tx[..first.tx_len]
    );
    // VFS validated the destination. Its loff_t pointer is a kernel pointer.
    let offset = unsafe { *position };
    if offset < 0 {
        return -22;
    }
    let offset = offset as usize;
    if offset >= text.used {
        return 0;
    }
    let n = count.min(text.used - offset);
    let missing = unsafe { abi::__copy_to_user(out, text.bytes.as_ptr().add(offset), n) };
    if missing != 0 {
        return -14;
    }
    unsafe { *position += n as i64 };
    n as isize
}

pub unsafe extern "C" fn ioctl(_file: *mut c_void, operation: u32, argument: usize) -> isize {
    if operation != x7_control::IOCTL {
        return -25;
    }
    // unlocked_ioctl does not validate a user pointer for us. ARM TASK_SIZE is
    // 0xc0000000 in this kernel; reject wraparound and kernel destinations.
    if argument == 0 || argument.checked_add(PACKET_SIZE).is_none_or(|end| end > 0xc000_0000) {
        return -14;
    }
    let user = argument as *mut u8;
    let mut packet = Packet::new(0);
    if unsafe { abi::__copy_from_user((&raw mut packet).cast(), user, PACKET_SIZE) } != 0 {
        return -14;
    }
    if packet.abi != x7_control::ABI || packet.reserved != [0; 2] || packet.length() > x7_control::BODY_MAX {
        return -22;
    }
    match packet.operation {
        x7_control::POP_WRITE => {
            if let Err(e) = runtime::begin_pop(&mut packet) {
                return e;
            }
            let missing = unsafe { abi::__copy_to_user(user, (&raw const packet).cast(), PACKET_SIZE) };
            runtime::end_pop(missing == 0);
            if missing == 0 { 0 } else { -14 }
        }
        x7_control::STATUS => {
            let values = runtime::snapshot();
            for (n, value) in values.iter().enumerate() {
                packet.data[n * 4..n * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            packet.length_le = (values.len() as u16 * 4).to_le_bytes();
            if unsafe { abi::__copy_to_user(user, (&raw const packet).cast(), PACKET_SIZE) } == 0 { 0 } else { -14 }
        }
        _ => runtime::update(&packet).map_or_else(|e| e, |()| 0),
    }
}
