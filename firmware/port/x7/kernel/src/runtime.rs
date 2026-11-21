//! Driver state and IRQ serialization for the retained UP Nokia kernel.
use crate::{
    abi,
    bus::{Bus, IrqGuard},
};
use core::{
    ffi::c_void,
    mem::MaybeUninit,
    ptr::{addr_of, addr_of_mut, null_mut, read_volatile},
};
use dm870::{
    Mmio,
    clocks::{Gates, tgen},
    pads::pads,
    spi_slave::{BASE, Config, NOP_CHARACTER, STOCK_DIVISOR, reg},
    uart::fifo,
};
use x7_control::Packet;
use x7_protocol::{Error, Kind};
use x7_registers::Registers;
use x7_transport::{SONY_PADS, Transport};

const IRQ: u32 = 20;
const SELECT_IRQ: u32 = 31; // GPIO 4 in the X7's KnOS GPIO-to-IRQ table
const SELECT_TRIGGER: u32 = 2; // KnOS 0x601900c0 translates GPIO mode 3 to Sulu mode 2.
const TRIGGER_CONFIG: u32 = 0xffff_0160 + 4 * (SELECT_IRQ / 16);
const TRIGGER_SHIFT: u32 = 2 * (SELECT_IRQ % 16);
const RX_IRQ: u32 = 0x300;
const TX_IRQ: u32 = 0x400;
const ABANDON_TICKS: u32 = 25; // 250 ms at HZ=100, below Sony's 500 ms wait

static mut LINK: MaybeUninit<Transport<Bus>> = MaybeUninit::uninit();
static mut BANK: Registers = Registers::new();
static mut WAIT: abi::WaitHead = abi::WaitHead { list: [0; 2] };
static mut TIMER: abi::Timer = abi::Timer { entry: [0; 2], expires: 0, function: tick, data: 0, base: 0 };
static mut TICKS: u32 = 0;
static mut IRQS: u32 = 0;
static mut PREVIOUS_TRIGGER: u32 = 0;
static mut TIMER_CALLS: u32 = 0;
static mut POP_BUSY: bool = false;

// No shared reference escapes this IRQ-disabled scope. The closure may neither
// sleep nor access userspace memory. The kernel has one CPU and no preemption.
fn with<R>(f: impl FnOnce(&mut Transport<Bus>, &mut Registers) -> R) -> R {
    let _guard = IrqGuard::new();
    unsafe { f(&mut *addr_of_mut!(LINK).cast::<Transport<Bus>>(), &mut *addr_of_mut!(BANK)) }
}

pub fn snapshot() -> [u32; 8] {
    with(|link, bank| {
        [
            unsafe { IRQS },
            unsafe { TIMER_CALLS },
            link.counters.requests,
            link.counters.bad_frames,
            link.counters.retries,
            link.counters.reply_aborts,
            bank.pending_count() as u32,
            link.busy() as u32,
        ]
    })
}

pub fn first_transfer() -> x7_transport::FirstTransfer {
    with(|link, _| link.first)
}

pub unsafe extern "C" fn poll(file: *mut c_void, table: *mut abi::PollTable) -> u32 {
    // Register first, then check under the lock: no lost arrival between the
    // empty check and the caller going to sleep. __pollwait may allocate.
    if !table.is_null() {
        if let Some(queue) = unsafe { (*table).queue } {
            unsafe { queue(file, addr_of_mut!(WAIT), table) };
        }
    }
    with(|_, bank| (if bank.has_writes() { 0x41 } else { 0 }) | (if bank.notification_room() { 0x104 } else { 0 }))
}

pub fn begin_pop(packet: &mut Packet) -> Result<(), isize> {
    with(|_, bank| {
        if unsafe { POP_BUSY } {
            return Err(-16);
        }
        let item = bank.peek_write().ok_or(-11isize)?;
        packet.kind = item.kind as u8;
        packet.register = item.register;
        packet.set_data(item.payload());
        unsafe { POP_BUSY = true };
        Ok(())
    })
}

pub fn end_pop(copied: bool) {
    with(|_, bank| {
        if copied {
            let _ = bank.pop_write();
        }
        unsafe { POP_BUSY = false };
    });
}

fn errno(error: Error) -> isize {
    match error {
        Error::OutputFull => -11,
        _ => -22,
    }
}

pub fn update(packet: &Packet) -> Result<(), isize> {
    with(|link, bank| {
        let payload = packet.payload().ok_or(-22isize)?;
        let result = match packet.operation {
            x7_control::SET_FIXED if payload.len() == 2 => {
                bank.set_fixed(packet.register, [payload[0], payload[1]]);
                Ok(())
            }
            x7_control::SET_VARIABLE => bank.set_variable(packet.register, payload),
            x7_control::NOTIFY => match packet.kind {
                1 => bank.notify(Kind::ReadFixed, packet.register, payload),
                3 => bank.notify(Kind::ReadVariable, packet.register, payload),
                _ => Err(Error::Operation),
            },
            _ => Err(Error::Operation),
        };
        // Publish request GPIO now, without a periodic idle wakeup.
        service(link, bank);
        result.map_err(errno)
    })
}

fn service(link: &mut Transport<Bus>, bank: &mut Registers) {
    let had_writes = bank.has_writes();
    let had_room = bank.notification_room();
    link.service(bank);
    let refill = link.needs_tx();
    link.bus().w32(BASE + if refill { fifo::IRQ_MASK_SET } else { fifo::IRQ_MASK_CLEAR }, TX_IRQ);
    if link.busy() {
        let now = unsafe { read_volatile(addr_of!(abi::jiffies)) };
        unsafe { abi::mod_timer(addr_of_mut!(TIMER), now.wrapping_add(1)) };
    }
    if (!had_writes && bank.has_writes()) || (!had_room && bank.notification_room()) {
        unsafe { abi::__wake_up(addr_of_mut!(WAIT), 3, 1, null_mut()) };
    }
}

extern "C" fn interrupt(number: i32, _identity: *mut c_void) -> i32 {
    with(|link, bank| {
        if number == SELECT_IRQ as i32 {
            service(link, bank);
            return 1;
        }
        let pending = link.bus().r32(BASE + fifo::STATUS) & link.bus().r32(BASE + fifo::IRQ_MASK_CLEAR);
        if pending & (RX_IRQ | TX_IRQ) == 0 {
            return 0;
        }
        unsafe {
            IRQS = IRQS.wrapping_add(1);
            TICKS = 0;
        }
        service(link, bank);
        // Sony's handler at 0x601e8618 acknowledges the latched receive
        // interrupt after reading the FIFO, including a short final fragment.
        if pending & RX_IRQ != 0 {
            link.bus().w32(BASE + fifo::STATUS, pending & RX_IRQ);
        }
        1
    })
}

extern "C" fn tick(_data: usize) {
    with(|link, bank| {
        unsafe {
            TIMER_CALLS = TIMER_CALLS.wrapping_add(1);
            TICKS = TICKS.wrapping_add(1);
            if TICKS >= ABANDON_TICKS {
                link.abort();
                TICKS = 0;
            }
        }
        service(link, bank);
    });
}

fn unmap(mappings: [*mut u8; 4]) {
    for pointer in mappings {
        if !pointer.is_null() {
            unsafe { abi::__iounmap(pointer) };
        }
    }
}

pub fn initialize() -> i32 {
    let mut bus = Bus {
        spi: unsafe { abi::__arm_ioremap(BASE as usize, 4096, 0) },
        gpio: unsafe { abi::__arm_ioremap(0xffff_0000, 4096, 0) },
        pads: unsafe { abi::__arm_ioremap(0xe001_3000, 4096, 0) },
        clocks: unsafe { abi::__arm_ioremap(0xe000_7000, 4096, 0) },
    };
    if [bus.spi, bus.gpio, bus.pads, bus.clocks].iter().any(|p| p.is_null()) {
        unmap(bus.mappings());
        return -12;
    }
    pads(&mut bus).route(&SONY_PADS);
    tgen(&mut bus).enable(Gates::STOCK_APP.and(Gates::SPI_SLAVE));
    bus.w32(BASE + fifo::IRQ_MASK_CLEAR, !0);
    let mut link = Transport::new(bus);
    let config = link.bus().r32(BASE + reg::CFG_SET);
    let divisor = link.bus().r32(BASE + reg::DIVISOR);
    let nop = link.bus().r32(BASE + reg::NOP);
    if config != (Config::LINK.word() | 1) || divisor != STOCK_DIVISOR || nop != u32::from(NOP_CHARACTER) {
        unsafe {
            abi::printk(c"<3>x7-spi: setup readback cfg=%08x div=%08x nop=%08x\n".as_ptr(), config, divisor, nop)
        };
        unmap(link.bus().mappings());
        return -5;
    }
    let trigger = (link.bus().r32(TRIGGER_CONFIG) >> TRIGGER_SHIFT) & 3;
    unsafe {
        PREVIOUS_TRIGGER = trigger;
        addr_of_mut!(LINK).write(MaybeUninit::new(link));
        abi::init_waitqueue_head(addr_of_mut!(WAIT));
        abi::init_timer(addr_of_mut!(TIMER));
    }
    let result = unsafe { abi::request_irq(IRQ, interrupt, 0x20, c"x7-spi".as_ptr(), addr_of_mut!(LINK).cast()) };
    if result != 0 {
        unmap(with(|link, _| link.bus().mappings()));
        return result;
    }
    let result = unsafe { abi::sulu_irq_set_trig(SELECT_IRQ, SELECT_TRIGGER) };
    let result = if result == 0 {
        unsafe { abi::request_irq(SELECT_IRQ, interrupt, 0x20, c"x7-spi-select".as_ptr(), addr_of_mut!(LINK).cast()) }
    } else {
        result
    };
    if result != 0 {
        unsafe {
            abi::sulu_irq_set_trig(SELECT_IRQ, trigger);
            abi::free_irq(IRQ, addr_of_mut!(LINK).cast());
        }
        unmap(with(|link, _| link.bus().mappings()));
    }
    result
}

pub fn activate() {
    with(|link, _| {
        // Sony uses threshold 4 plus the receive timer for a shorter tail.
        link.bus().w32(BASE + fifo::IRQ_MASK_SET, RX_IRQ);
    });
}

pub fn shutdown() {
    unsafe { abi::free_irq(SELECT_IRQ, addr_of_mut!(LINK).cast()) };
    with(|link, _| {
        unsafe { abi::sulu_irq_set_trig(SELECT_IRQ, PREVIOUS_TRIGGER) };
        link.bus().w32(BASE + fifo::IRQ_MASK_CLEAR, !0);
        link.abort();
        // UP kernel: del_timer_sync is the del_timer macro (Linux timer.h).
        unsafe { abi::del_timer(addr_of_mut!(TIMER)) };
    });
    unsafe { abi::free_irq(IRQ, addr_of_mut!(LINK).cast()) };
    unmap(with(|link, _| link.bus().mappings()));
}
