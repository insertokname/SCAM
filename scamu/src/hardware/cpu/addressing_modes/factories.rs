//! # Factories
//!
//! This module is responsible for creating simple helpers that return
//! an [super::AddressingMode]. They are used in the
//! [lookup table](crate::hardware::cpu::instructions::INSTRUCTIONS_LOOKUP).

use crate::hardware::{cpu::Cpu, cpu_bus::CpuBus};

use super::implementations::*;

fn format_hex_u8(value: u8) -> String {
    format!("${value:02X}")
}

// fn format_hex_u8(value: i8) -> String {
//     if value < 0 {
//         format!("-${:02X}", (-value) as u8)
//     } else {
//         format!("${:02X}", value as u8)
//     }
// }

fn format_hex_u16(value: u16) -> String {
    format!("${value:04X}")
}

pub(crate) type AddressingModeFactory<AM> = fn(cpu: &Cpu, bus: &CpuBus) -> Box<AM>;

// /// Implicit addressing mode
// ///
// /// Instructions using implicit mode do not require a parameter (ex: CLC)
pub(crate) const IMPLICIT: fn(cpu: &Cpu, bus: &CpuBus) -> Box<ImplicitAddressingMode> =
    |_: &Cpu, _: &CpuBus| {
        Box::new(ImplicitAddressingMode {
            cpu_program_counter_offset: 0,
            cpu_additional_cycles_required: 0,
        })
    };

/// Accumulator addressing mode
///
/// Gets the acculumator as the argument
pub(crate) const ACCUMULATOR: fn(cpu: &Cpu, bus: &CpuBus) -> Box<AccumulatorAddressingMode> =
    |_: &Cpu, _: &CpuBus| {
        Box::new(AccumulatorAddressingMode {
            cpu_program_counter_offset: 0,
            cpu_additional_cycles_required: 0,
            display: format!("A"),
        })
    };

/// Immediate addressing mode
///
/// Gets the next byte as the argument
pub(crate) const IMMEDIATE: fn(cpu: &Cpu, bus: &CpuBus) -> Box<MemoryAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let address = cpu.program_counter;

        let value = bus.peek(address);

        Box::new(MemoryAddressingMode {
            address,
            cpu_program_counter_offset: 1,
            cpu_additional_cycles_required: 0,
            display: format!("#{}", format_hex_u8(value)),
        })
    };

/// Zero page addressing mode
///
/// Uses the next byte as a zero-page address (0x0000–0x00FF).
/// The CPU treats the operand as the low byte of the address and
/// assumes the high byte is 0x00.
///
/// # Example
///
/// LDA $42
///
/// Loads the value from memory at address 0x0042 into the accumulator
/// register.
pub(crate) const ZERO_PAGE: fn(cpu: &Cpu, bus: &CpuBus) -> Box<MemoryAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let address = bus.peek(cpu.program_counter) as u16;

        let value = bus.peek(address);

        Box::new(MemoryAddressingMode {
            address,
            cpu_program_counter_offset: 1,
            cpu_additional_cycles_required: 0,
            display: format!("{} = {value:02X}", format_hex_u8(address as u8),),
        })
    };

/// Zero page with x offset addressing mode
///
/// Uses the next byte + the x register as a zero-page address
/// (0x0000–0x00FF). The CPU treats the operand as the low byte of the
/// address and assumes the high byte is 0x00. The addition wraps around
/// within the zero page (i.e., (operand + X) & 0xFF).
///
/// # Example
///
/// LDA $42, X
///
/// Loads the value from memory at address 0x0042 + X into the accumulator
/// register.
pub(crate) const ZERO_PAGE_X_OFFSET: fn(cpu: &Cpu, bus: &CpuBus) -> Box<MemoryAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let argument = cpu.program_counter;
        let address = bus.peek(argument);
        let offset_address = address.wrapping_add(cpu.x) as u16;
        let value = bus.peek(offset_address);

        Box::new(MemoryAddressingMode {
            address: offset_address,
            cpu_program_counter_offset: 1,
            cpu_additional_cycles_required: 0,
            display: format!(
                "{},X @ {offset_address:02X} = {value:02X}",
                format_hex_u8(address as u8)
            ),
        })
    };

/// Zero page with y offset addressing mode
///
/// Uses the next byte + the y register as a zero-page address
/// (0x0000–0x00FF). The CPU treats the operand as the low byte of the
/// address and assumes the high byte is 0x00. The addition wraps around
/// within the zero page (i.e., (operand + y) & 0xFF).
///
/// # Example
///
/// LDA $42, Y
///
/// Loads the value from memory at address 0x0042 + Y into the accumulator
/// register.
pub(crate) const ZERO_PAGE_Y_OFFSET: fn(cpu: &Cpu, bus: &CpuBus) -> Box<MemoryAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let argument = cpu.program_counter;
        let address = bus.peek(argument);
        let offset_address = address.wrapping_add(cpu.y) as u16;
        let value = bus.peek(offset_address);

        Box::new(MemoryAddressingMode {
            address: offset_address,
            cpu_program_counter_offset: 1,
            cpu_additional_cycles_required: 0,
            display: format!(
                "{},Y @ {offset_address:02X} = {value:02X}",
                format_hex_u8(address as u8)
            ),
        })
    };

/// Absolute addressing mode
///
/// Uses the next two bytes as the low and high parts of the target address,
/// allowing access to any location in memory.
///
/// # Example
///
/// LDA $1234
///
/// Loads the value from memory at address 0x1234 into the accumulator register.
pub(crate) const ABSOLUTE: fn(cpu: &Cpu, bus: &CpuBus) -> Box<MemoryAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let address = bus.peek_u16(cpu.program_counter);

        let value = bus.peek(address);

        Box::new(MemoryAddressingMode {
            address,
            cpu_program_counter_offset: 2,
            cpu_additional_cycles_required: 0,
            display: format!("{} = {value:02X}", format_hex_u16(address)),
        })
    };

/// [ABSOLUTE] but displays differently
pub(crate) const ABSOLUTE_JMP: fn(cpu: &Cpu, bus: &CpuBus) -> Box<MemoryAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let address = bus.peek_u16(cpu.program_counter);

        Box::new(MemoryAddressingMode {
            address,
            cpu_program_counter_offset: 2,
            cpu_additional_cycles_required: 0,
            display: format!("{}", format_hex_u16(address)),
        })
    };

// /// Absolute addressing mode
// ///
// ///
// /// Used for jump instructions to allow them to also access the memory location
// pub(crate) const ABSOLUTE_JUMPING: fn(cpu: &Cpu, bus: &Bus) -> Box<MemoryAddressingMode> = |cpu: &Cpu, bus: &Bus| {
//     let address = bus.peek_u16(cpu.program_counter);

//     Box::new(JumpingAddressingMode {
//         address,
//         cpu_program_counter_offset: 2,
//         cpu_additional_cycles_required: 0,
//         display: format!("{}", format_hex_u16(address)),
//     })
// };

/// Absolute with x offset addressing mode
///
/// Uses the next two bytes as the low and high parts of the target
/// address then adds x to it.
///
/// # Example
///
/// LDA $1234, X
///
/// Loads the value from memory at address 0x1234 + X into the accumulator register.
pub(crate) const ABSOLUTE_X_OFFSET: fn(cpu: &Cpu, bus: &CpuBus) -> Box<MemoryAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let address = bus.peek_u16(cpu.program_counter);
        let offset_address = address + cpu.x as u16;
        let value = bus.peek(offset_address);

        let add_cycle = offset_address & 0xFF00 != address & 0xFF00;

        Box::new(MemoryAddressingMode {
            address: offset_address,
            cpu_program_counter_offset: 2,
            cpu_additional_cycles_required: add_cycle as u8,
            display: format!(
                "{},X @ {offset_address:04X} = {value:02X}",
                format_hex_u16(address)
            ),
        })
    };

/// Absolute with y offset addressing mode
///
/// Uses the next two bytes as the low and high parts of the target
/// address then adds y to it.
///
/// # Example
///
/// LDA $1234, Y
///
/// Loads the value from memory at address 0x1234 + Y into the accumulator register.
pub(crate) const ABSOLUTE_Y_OFFSET: fn(cpu: &Cpu, bus: &CpuBus) -> Box<MemoryAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let address = bus.peek_u16(cpu.program_counter);
        let offset_address = address.wrapping_add(cpu.y as u16);
        let value = bus.peek(offset_address);

        let add_cycle = offset_address & 0xFF00 != address & 0xFF00;

        Box::new(MemoryAddressingMode {
            address: offset_address,
            cpu_program_counter_offset: 2,
            cpu_additional_cycles_required: add_cycle as u8,
            display: format!(
                "{},Y @ {offset_address:04X} = {value:02X}",
                format_hex_u16(address)
            ),
        })
    };

// /// Absolute with y offset addressing mode
// ///
// /// Uses the next two bytes as the low and high parts of the target
// /// address then adds y to it.
// ///
// /// # Example
// ///
// /// LDA $1234, Y
// ///
// /// Loads the value from memory at address 0x1234 + Y into the accumulator register.
// pub(crate) const ABSOLUTE_Y_OFFSET_JUMPING: AddressingModeFactory<MemoryAddress> =
//     |cpu: &Cpu, bus: &Bus| {
//         let address = bus.peek_u16(cpu.program_counter);
//         let offset_address = address.wrapping_add(cpu.y as u16);
//         let value = bus.peek(offset_address);

//         let add_cycle = offset_address & 0xFF00 != address & 0xFF00;

//         Box::new(JumpingAddressingMode {
//             address: offset_address,
//             cpu_program_counter_offset: 2,
//             cpu_additional_cycles_required: add_cycle as u8,
//             display: format!(
//                 "{},Y @ {offset_address:04X} = {value:02X}",
//                 format_hex_u16(address)
//             ),
//         })
//     };

// apparently this is only used for jumping and the INDIRECT_JUMPING already exists so wtv
// /// Indirect addressing mode
// ///
// /// Reads a 16-bit pointer from the next two bytes and returns the
// /// pointed-to address.
// pub(crate) const INDIRECT: AddressingModeFactory<u8> = |cpu: & Cpu, bus: &Bus| {
//     let pointer_address = bus.peek_u16(cpu.program_counter);
//     cpu.program_counter += 2;

//     let low = bus.peek(pointer_address) as u16;

//     // bug in 6502 wrapping page https://www.nesdev.org/6502bugs.txt
//     // An indirect JMP (xxFF) will fail because the MSB will be fetched
//     // from address xx00 instead of page xx+1
//     let high_address = (pointer_address & 0xFF00) | ((pointer_address + 1) & 0x00FF);
//     let high = bus.peek(high_address) as u16;
//     let address = (high << 8) | low;

//     Box::new(MemoryAddressingMode {
//         address,
//         additional_cycles_required: 0,
//     })
// };

/// Indirect addressing mode
///
/// Used for jump instructions to allow them to also access the memory location
pub(crate) const INDIRECT: fn(cpu: &Cpu, bus: &CpuBus) -> Box<MemoryAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let pointer_address = bus.peek_u16(cpu.program_counter);

        let low = bus.peek(pointer_address) as u16;

        // bug in 6502 wrapping page https://www.nesdev.org/6502bugs.txt
        // An indirect JMP (xxFF) will fail because the MSB will be fetched
        // from address xx00 instead of page xx+1
        let high_address = (pointer_address & 0xFF00) | ((pointer_address + 1) & 0x00FF);
        let high = bus.peek(high_address) as u16;
        let address = (high << 8) | low;

        Box::new(MemoryAddressingMode {
            address,
            cpu_program_counter_offset: 2,
            cpu_additional_cycles_required: 0,
            display: format!("({}) = {address:04X}", format_hex_u16(pointer_address)),
        })
    };

/// Indirect with x offset addressing mode
///
/// Reads an 8-bit pointer to a zero page location from the next byte + x
/// and then uses that as the actual address.
pub(crate) const INDIRECT_X_OFFSET: fn(cpu: &Cpu, bus: &CpuBus) -> Box<MemoryAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let argument = bus.peek(cpu.program_counter);

        let pointer = argument.wrapping_add(cpu.x);
        let pointer_address = pointer as u16;

        let low = bus.peek(pointer_address) as u16;
        let high_address = (pointer_address & 0xFF00) | ((pointer_address + 1) & 0x00FF);
        let high = bus.peek(high_address) as u16;
        let address = (high << 8) | low;

        let value = bus.peek(address);

        Box::new(MemoryAddressingMode {
            address,
            cpu_program_counter_offset: 1,
            cpu_additional_cycles_required: 0,
            display: format!(
                "({},X) @ {pointer_address:02X} = {address:04X} = {value:02X}",
                format_hex_u8(argument)
            ),
        })
    };

/// Indirect with y offset addressing mode
///
/// Reads an 8-bit pointer to a zero page location from the next byte
/// and then adds y to that loccation and returns that new address.
pub(crate) const INDIRECT_Y_OFFSET: fn(cpu: &Cpu, bus: &CpuBus) -> Box<MemoryAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let argument = bus.peek(cpu.program_counter) as u16;

        let low = bus.peek(argument);
        let high_addr = (argument & 0xFF00) | ((argument + 1) & 0x00FF);
        let high = bus.peek(high_addr);
        let address = (high as u16) << 8 | low as u16;
        let offset_address = address.wrapping_add(cpu.y as u16);
        let add_cycle = offset_address & 0xFF00 != address & 0xFF00;

        let value = bus.peek(offset_address);

        Box::new(MemoryAddressingMode {
            address: offset_address,
            cpu_program_counter_offset: 1,
            cpu_additional_cycles_required: add_cycle as u8,
            // display: format!("({}),y", format_hex_u16(address)),
            display: format!(
                "({}),Y = {address:04X} @ {offset_address:04X} = {value:02X}",
                format_hex_u8(argument as u8)
            ),
        })
    };

// /// Indirect with y offset addressing mode
// ///
// /// Reads an 8-bit pointer to a zero page location from the next byte
// /// and then adds y to that loccation and returns that new address.
// pub(crate) const INDIRECT_Y_OFFSET_JUMPING: AddressingModeFactory<MemoryAddress> =
//     |cpu: &Cpu, bus: &Bus| {
//         let argument = bus.peek(cpu.program_counter) as u16;

//         let low = bus.peek(argument);
//         let high_addr = (argument & 0xFF00) | ((argument + 1) & 0x00FF);
//         let high = bus.peek(high_addr);
//         let address = (high as u16) << 8 | low as u16;
//         let offset_address = address.wrapping_add(cpu.y as u16);
//         let add_cycle = offset_address & 0xFF00 != address & 0xFF00;

//         let value = bus.peek(offset_address);

//         Box::new(JumpingAddressingMode {
//             address: offset_address,
//             cpu_program_counter_offset: 1,
//             cpu_additional_cycles_required: add_cycle as u8,
//             // display: format!("({}),y", format_hex_u16(address)),
//             display: format!(
//                 "({}),Y = {address:04X} @ {offset_address:04X} = {value:02X}",
//                 format_hex_u8(argument as u8)
//             ),
//         })
//     };

/// Relative addressing mode
///
/// Only branch instructions use this.
pub(crate) const RELATIVE: fn(cpu: &Cpu, bus: &CpuBus) -> Box<RelativeAddressingMode> =
    |cpu: &Cpu, bus: &CpuBus| {
        let address = cpu.program_counter;

        let value = bus.peek(address) as i8;

        Box::new(RelativeAddressingMode {
            address,
            cpu_program_counter_offset: 1,
            cpu_additional_cycles_required: 0,
            display: format!(
                "{}",
                format_hex_u16(((address as i32) + (value as i32) + 1) as u16)
            ),
        })
    };
