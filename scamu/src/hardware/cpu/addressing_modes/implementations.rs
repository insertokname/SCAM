//! # Implementations
//!
//! The implementations of different types of
//! [Addressing modes](super::AddressingMode).
use std::fmt::Debug;

use crate::hardware::{cpu::Cpu, cpu_bus::CpuBus};

use super::AddressingMode;

pub(crate) struct ImplicitAddressingMode {
    pub(crate) cpu_program_counter_offset: u16,
    pub(crate) cpu_additional_cycles_required: u8,
}

impl AddressingMode<()> for ImplicitAddressingMode {
    fn cpu_additional_cycles_required(&self) -> u8 {
        self.cpu_additional_cycles_required
    }

    fn cpu_program_counter_offset(&self) -> u16 {
        self.cpu_program_counter_offset
    }

    fn cpu_add_another_required_cycle(&mut self) {
        self.cpu_additional_cycles_required += 1
    }

    fn read(&self, _: &Cpu, _: &CpuBus) -> () {
        ()
    }

    fn write(&mut self, _: (), _: &mut Cpu, _: &mut CpuBus) {}

    fn display(&self) -> &str {
        ""
    }
}

pub(crate) struct AccumulatorAddressingMode {
    pub(crate) cpu_program_counter_offset: u16,
    pub(crate) cpu_additional_cycles_required: u8,
    pub(crate) display: String,
}

impl AddressingMode<u8> for AccumulatorAddressingMode {
    fn cpu_additional_cycles_required(&self) -> u8 {
        self.cpu_additional_cycles_required
    }

    fn cpu_program_counter_offset(&self) -> u16 {
        self.cpu_program_counter_offset
    }

    fn cpu_add_another_required_cycle(&mut self) {
        self.cpu_additional_cycles_required += 1
    }

    fn read(&self, cpu: &Cpu, _: &CpuBus) -> u8 {
        cpu.accumulator
    }

    fn write(&mut self, new_value: u8, cpu: &mut Cpu, _: &mut CpuBus) {
        cpu.accumulator = new_value;
    }

    fn display(&self) -> &str {
        &self.display
    }
}

pub(crate) struct MemoryAddressingMode {
    pub(crate) address: u16,
    pub(crate) cpu_program_counter_offset: u16,
    pub(crate) cpu_additional_cycles_required: u8,
    pub(crate) display: String,
}

impl AddressingMode<u8> for MemoryAddressingMode {
    fn cpu_additional_cycles_required(&self) -> u8 {
        self.cpu_additional_cycles_required
    }

    fn cpu_program_counter_offset(&self) -> u16 {
        self.cpu_program_counter_offset
    }

    fn cpu_add_another_required_cycle(&mut self) {
        self.cpu_additional_cycles_required += 1
    }

    fn read(&self, _: &Cpu, bus: &CpuBus) -> u8 {
        bus.read(self.address)
    }

    fn write(&mut self, new_value: u8, _: &mut Cpu, bus: &mut CpuBus) {
        bus.write(self.address, new_value);
    }

    fn display(&self) -> &str {
        &self.display
    }
}

impl AddressingMode<MemoryAddress> for MemoryAddressingMode {
    fn cpu_additional_cycles_required(&self) -> u8 {
        self.cpu_additional_cycles_required
    }

    fn cpu_program_counter_offset(&self) -> u16 {
        self.cpu_program_counter_offset
    }

    fn cpu_add_another_required_cycle(&mut self) {
        self.cpu_additional_cycles_required += 1
    }

    fn read(&self, _: &Cpu, bus: &CpuBus) -> MemoryAddress {
        MemoryAddress {
            value: bus.read(self.address),
            address: self.address,
        }
    }

    fn write(&mut self, new_value: MemoryAddress, _: &mut Cpu, bus: &mut CpuBus) {
        bus.write(self.address, new_value.value);
    }

    fn display(&self) -> &str {
        &self.display
    }
}

pub(crate) struct RelativeAddressingMode {
    pub(crate) address: u16,
    pub(crate) cpu_program_counter_offset: u16,
    pub(crate) cpu_additional_cycles_required: u8,
    pub(crate) display: String,
}

impl AddressingMode<i8> for RelativeAddressingMode {
    fn cpu_additional_cycles_required(&self) -> u8 {
        self.cpu_additional_cycles_required
    }

    fn cpu_program_counter_offset(&self) -> u16 {
        self.cpu_program_counter_offset
    }

    fn cpu_add_another_required_cycle(&mut self) {
        self.cpu_additional_cycles_required += 1
    }

    fn read(&self, _: &Cpu, bus: &CpuBus) -> i8 {
        bus.read(self.address) as i8
    }

    fn write(&mut self, new_value: i8, _: &mut Cpu, bus: &mut CpuBus) {
        bus.write(self.address, new_value as u8);
    }

    fn display(&self) -> &str {
        &self.display
    }
}

/// Gives the user access to both the address and the value at the address
#[derive(Clone, Copy)]
pub(crate) struct MemoryAddress {
    pub(crate) value: u8,
    pub(crate) address: u16,
}

impl Debug for MemoryAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value)
    }
}

#[allow(dead_code)]
impl MemoryAddress {
    pub fn get_value(&self) -> &u8 {
        &self.value
    }

    pub fn set_value(&mut self, new_value: u8) {
        self.value = new_value
    }

    pub fn get_address(&self) -> u16 {
        self.address
    }
}

// pub(crate) struct JumpingAddressingMode {
//     pub(crate) address: u16,
//     pub(crate) cpu_program_counter_offset: u16,
//     pub(crate) cpu_additional_cycles_required: u8,
//     pub(crate) display: String,
// }

// impl AddressingMode<MemoryAddress> for JumpingAddressingMode {
//     fn cpu_additional_cycles_required(&self) -> u8 {
//         self.cpu_additional_cycles_required
//     }

//     fn cpu_program_counter_offset(&self) -> u16 {
//         self.cpu_program_counter_offset
//     }

//     fn cpu_add_another_required_cycle(&mut self) {
//         self.cpu_additional_cycles_required += 1
//     }
//     fn read(&self, _: &Cpu, bus: &Bus) -> MemoryAddress {
//         MemoryAddress {
//             value: bus.read(self.address),
//             address: self.address,
//         }
//     }

//     fn write(&mut self, new_value: MemoryAddress, _: &mut Cpu, bus: &mut Bus) {
//         bus.write(self.address, new_value.value);
//     }

//     fn display(&self) -> &str {
//         &self.display
//     }
// }
