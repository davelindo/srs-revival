use core::ptr::{read_volatile, write_volatile};
use dm870::Mmio;

/// Two ioremap regions, computed once. No HashMap, page lookup or allocation in
/// the interrupt handler. The transport touches only SPI and GPIO here.
pub struct Bus {
    pub spi: *mut u8,
    pub gpio: *mut u8,
    pub pads: *mut u8,
    pub clocks: *mut u8,
}

impl Bus {
    pub fn mappings(&self) -> [*mut u8; 4] {
        [self.spi, self.gpio, self.pads, self.clocks]
    }

    fn address(&self, physical: u32) -> *mut u8 {
        let (base, offset) = if physical & !0xfff == dm870::spi_slave::BASE {
            (self.spi, physical & 0xfff)
        } else if physical & !0xfff == 0xe001_3000 {
            (self.pads, physical & 0xfff)
        } else if physical & !0xfff == 0xe000_7000 {
            (self.clocks, physical & 0xfff)
        } else {
            debug_assert_eq!(physical & !0xfff, 0xffff_0000);
            (self.gpio, physical & 0xfff)
        };
        // Both regions cover a page; Mmio callers use the retained DM870 PAC.
        unsafe { base.add(offset as usize) }
    }
}

impl Mmio for Bus {
    fn r32(&mut self, pa: u32) -> u32 {
        unsafe { read_volatile(self.address(pa).cast()) }
    }
    fn w32(&mut self, pa: u32, value: u32) {
        unsafe { write_volatile(self.address(pa).cast(), value) }
    }
    fn r8(&mut self, pa: u32) -> u8 {
        unsafe { read_volatile(self.address(pa)) }
    }
    fn w8(&mut self, pa: u32, value: u8) {
        unsafe { write_volatile(self.address(pa), value) }
    }
}

/// This module targets the single-core, non-preemptible Nokia kernel. Preserve
/// CPSR around shared state; no sleeping or user-memory access inside the guard.
pub struct IrqGuard(u32);

impl IrqGuard {
    pub fn new() -> Self {
        let saved: u32;
        let masked: u32;
        unsafe {
            core::arch::asm!(
                "mrs {saved}, cpsr", "orr {masked}, {saved}, #0x80", "msr cpsr_c, {masked}",
                saved = out(reg) saved, masked = out(reg) masked,
                options(nostack, preserves_flags),
            );
        }
        let _ = masked;
        Self(saved)
    }
}

impl Drop for IrqGuard {
    fn drop(&mut self) {
        unsafe {
            core::arch::asm!("msr cpsr_c, {saved}", saved = in(reg) self.0, options(nostack, preserves_flags));
        }
    }
}
