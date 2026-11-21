//! Small Rust kernel component for the exact retained Nokia Linux kernel.
//! SPI work runs in IRQ context without allocation. Userspace is woken only for
//! controller writes. Load/unload and char-device ABI verified in Renode first.
#![no_std]

mod abi;
mod bus;
mod device;
mod runtime;

use abi::{FileOperations, MiscDevice, Module};
use core::ptr::{addr_of_mut, null_mut};

#[used]
#[unsafe(link_section = ".modinfo")]
static LICENSE: [u8; 12] = *b"license=GPL\0";
#[used]
#[unsafe(link_section = ".modinfo")]
static VERMAGIC: [u8; 36] = *b"vermagic=2.6.24.7 mod_unload ARMv5 \0";
#[used]
#[unsafe(link_section = ".modinfo")]
static VERSION: [u8; x7_control::DRIVER_MODINFO.len()] = *x7_control::DRIVER_MODINFO;

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".gnu.linkonce.this_module")]
static mut __this_module: Module = Module {
    state: 0,
    list: [0; 2],
    name: {
        let mut name = [0; 60];
        name[0] = b'x';
        name[1] = b'7';
        name[2] = b'_';
        name[3] = b's';
        name[4] = b'p';
        name[5] = b'i';
        name
    },
    private_before_init: [0; 128],
    init: init_module,
    private_before_exit: [0; 96],
    exit: cleanup_module,
    private_tail: [0; 48],
};

static mut FOPS: FileOperations = FileOperations {
    owner: addr_of_mut!(__this_module),
    llseek: 0,
    read: Some(device::read),
    write_aio_readdir: [0; 4],
    poll: Some(runtime::poll),
    ioctl: 0,
    unlocked_ioctl: Some(device::ioctl),
    unused: [0; 17],
};
static mut DEVICE: MiscDevice = MiscDevice {
    minor: 255, // kernel allocates a free minor; inspect /proc/misc, never guess
    name: c"x7-spi".as_ptr(),
    fops: &raw const FOPS,
    list: [0; 2],
    parent: null_mut(),
    device: null_mut(),
};

#[unsafe(no_mangle)]
pub extern "C" fn init_module() -> i32 {
    let result = runtime::initialize();
    if result != 0 {
        return result;
    }
    let result = unsafe { abi::misc_register(addr_of_mut!(DEVICE)) };
    if result == 0 {
        runtime::activate();
        unsafe { abi::printk(c"<6>x7-spi: Rust IRQ backend loaded, irq=20\n".as_ptr()) };
    } else {
        runtime::shutdown();
    }
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn cleanup_module() {
    unsafe { abi::misc_deregister(addr_of_mut!(DEVICE)) };
    runtime::shutdown();
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { abi::printk(c"<0>x7-spi: invariant failed\n".as_ptr()) };
    loop {
        core::hint::spin_loop();
    }
}
