//! ABI of the retained Nokia 2.6.24.7 ARMv5 UP kernel. Function signatures and
//! file_operations/miscdevice come from Linux v2.6.24 include/linux/{fs,miscdevice,
//! interrupt}. The module metadata offsets come from that kernel's own
//! warp_debugger.ko: size 0x160, init 0xc8, exit 0x12c, name 0x0c.
use core::ffi::{c_char, c_void};

#[repr(C, align(32))]
pub struct Module {
    pub state: u32,
    pub list: [usize; 2],
    pub name: [u8; 60],
    pub private_before_init: [u8; 128],
    pub init: extern "C" fn() -> i32,
    pub private_before_exit: [u8; 96],
    pub exit: extern "C" fn(),
    pub private_tail: [u8; 48],
}

pub type Read = unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut i64) -> isize;
pub type Ioctl = unsafe extern "C" fn(*mut c_void, u32, usize) -> isize;
pub type Poll = unsafe extern "C" fn(*mut c_void, *mut PollTable) -> u32;

#[repr(C)]
pub struct FileOperations {
    pub owner: *mut Module,
    pub llseek: usize,
    pub read: Option<Read>,
    pub write_aio_readdir: [usize; 4],
    pub poll: Option<Poll>,
    pub ioctl: usize,
    pub unlocked_ioctl: Option<Ioctl>,
    pub unused: [usize; 17],
}

#[repr(C)]
pub struct MiscDevice {
    pub minor: i32,
    pub name: *const c_char,
    pub fops: *const FileOperations,
    pub list: [usize; 2],
    pub parent: *mut c_void,
    pub device: *mut c_void,
}

/// UP kernel without spinlock debugging: init_waitqueue_head at c004de74
/// initializes list pointers at offsets 0 and 4 (the spinlock has size zero).
#[repr(C)]
pub struct WaitHead {
    pub list: [usize; 2],
}

#[repr(C)]
pub struct PollTable {
    pub queue: Option<unsafe extern "C" fn(*mut c_void, *mut WaitHead, *mut PollTable)>,
    pub key: usize,
}

/// init_timer at c0042ce4 writes its base at +0x14, matching Linux timer.h.
#[repr(C)]
pub struct Timer {
    pub entry: [usize; 2],
    pub expires: usize,
    pub function: extern "C" fn(usize),
    pub data: usize,
    pub base: usize,
}

unsafe extern "C" {
    pub fn misc_register(device: *mut MiscDevice) -> i32;
    pub fn misc_deregister(device: *mut MiscDevice) -> i32;
    pub fn __copy_to_user(to: *mut u8, from: *const u8, count: usize) -> usize;
    pub fn __copy_from_user(to: *mut u8, from: *const u8, count: usize) -> usize;
    pub fn printk(format: *const c_char, ...) -> i32;
    pub fn __arm_ioremap(physical: usize, length: usize, kind: u32) -> *mut u8;
    pub fn __iounmap(address: *mut u8);
    pub fn request_irq(
        number: u32,
        handler: extern "C" fn(i32, *mut c_void) -> i32,
        flags: usize,
        name: *const c_char,
        identity: *mut c_void,
    ) -> i32;
    pub fn free_irq(number: u32, identity: *mut c_void);
    pub fn sulu_irq_set_trig(number: u32, mode: u32) -> i32;
    pub fn init_timer(timer: *mut Timer);
    pub fn mod_timer(timer: *mut Timer, expires: usize) -> i32;
    pub fn del_timer(timer: *mut Timer) -> i32;
    pub fn init_waitqueue_head(head: *mut WaitHead);
    pub fn __wake_up(head: *mut WaitHead, mode: u32, exclusive: i32, key: *mut c_void);
    pub static jiffies: usize;
}

// Fail at compile time if the compiler's target changes these kernel layouts.
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(core::mem::size_of::<Module>() == 0x160);
    assert!(core::mem::offset_of!(Module, name) == 0x0c);
    assert!(core::mem::offset_of!(Module, init) == 0xc8);
    assert!(core::mem::offset_of!(Module, exit) == 0x12c);
    assert!(core::mem::size_of::<FileOperations>() == 27 * 4);
    assert!(core::mem::size_of::<MiscDevice>() == 7 * 4);
    assert!(core::mem::size_of::<WaitHead>() == 8);
    assert!(core::mem::offset_of!(Timer, base) == 0x14);
};
