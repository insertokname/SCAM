//! # Implementations
//!
//! The Addressing mode has 2 purposes:
//!
//! 1. Different addressing modes write to different things.
//! Some of them write to the bus, others to registers while
//! others don't need writing at all. This is a common abstraction
//! over the logic of writing and reading addressing modes.
//!
//! 2. Addressing modes all get displayed in different ways depending
//! on their value. The addressing mode trait provides a common interface
//!  for them
pub(super) mod factories;
pub(super) mod implementations;

use std::fmt::Debug;

use crate::hardware::{cpu::Cpu, cpu_bus::CpuBus};

pub(super) trait AddressingMode<T: Debug> {
    fn cpu_program_counter_offset(&self) -> u16;
    fn cpu_additional_cycles_required(&self) -> u8;
    fn cpu_add_another_required_cycle(&mut self);
    fn read(&self, cpu: &Cpu, bus: &CpuBus) -> T;
    fn write(&mut self, new_value: T, cpu: &mut Cpu, bus: &mut CpuBus);
    fn display(&self) -> &str;
}
