use crate::hardware::{
    bit_ops::BitOps,
    constants::cpu::flags::*,
    cpu::{
        Cpu,
        addressing_modes::{AddressingMode, implementations::MemoryAddress},
    },
    cpu_bus::CpuBus,
};

/// # Returns:
/// The ammount of extra cycles that operation required
pub(super) type Operation<T> = fn(&mut Cpu, &mut CpuBus, &mut Box<dyn AddressingMode<T>>);

pub(super) const ADC: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result: u16 =
        cpu.accumulator as u16 + argument as u16 + cpu.status.get_flag_enabled(CARRY) as u16;

    cpu.status.set_flag_enabled(CARRY, result > 0xFF);
    cpu.status.set_flag_enabled(ZERO, (result as u8) == 0);
    // If the result's sign is different from both A's and memory's,
    // signed overflow (or underflow) occurred.
    // https://www.nesdev.org/wiki/Instruction_reference#ADC
    cpu.status.set_flag_enabled(
        OVERFLOW,
        (result as u8 ^ cpu.accumulator) & (result as u8 ^ argument) & 0x80 > 0,
    );
    cpu.status
        .set_flag_enabled(NEGATIVE, result as u8 & 0x80 > 0);

    cpu.accumulator = result as u8;
};

pub(super) const ALR: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let arguemnt_and = cpu.accumulator & argument;
    let result = arguemnt_and >> 1;

    cpu.status.set_flag_enabled(CARRY, arguemnt_and & 0x1 > 0);
    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.accumulator = result;
};

pub(super) const ANC: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = cpu.accumulator & argument;

    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);
    cpu.status.set_flag_enabled(CARRY, result & 0x80 > 0);

    cpu.accumulator = result;
};

pub(super) const AND: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = cpu.accumulator & argument;

    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.accumulator = result;
};

pub(super) const ANE: Operation<u8> = |_, _, _| {
    // TODO: implement this bullshit https://www.nesdev.org/wiki/Visual6502wiki/6502_Opcode_8B_(XAA,_ANE)
};

pub(super) const ARR: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let anded = cpu.accumulator & argument;
    let mut result = anded >> 1;

    if cpu.status.get_flag_enabled(CARRY) {
        result |= 0x80;
    }

    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);
    cpu.status.set_flag_enabled(CARRY, result & 0x40 > 0);
    cpu.status
        .set_flag_enabled(OVERFLOW, ((result >> 6) & 1) ^ ((result >> 5) & 1) > 0);

    cpu.accumulator = result;
};

pub(super) const ASL: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus) as u16;
    let result = argument << 1;
    cpu.status.set_flag_enabled(CARRY, result > 0xFF);
    cpu.status.set_flag_enabled(ZERO, result & 0xFF == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    addressing_mode.write(result as u8, cpu, bus);
};

fn branch(cpu: &mut Cpu, addressing_mode: &mut Box<dyn AddressingMode<i8>>, address: i8) {
    addressing_mode.cpu_add_another_required_cycle();
    let new_address = (cpu.program_counter as i32 + address as i32) as u16;
    if new_address & 0xFF00 != cpu.program_counter & 0xFF00 {
        addressing_mode.cpu_add_another_required_cycle();
    }
    cpu.program_counter = new_address;
}

pub(super) const BCC: Operation<i8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    if !cpu.status.get_flag_enabled(CARRY) {
        branch(cpu, addressing_mode, argument);
    }
};

pub(super) const BCS: Operation<i8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    if cpu.status.get_flag_enabled(CARRY) {
        branch(cpu, addressing_mode, argument);
    }
};

pub(super) const BEQ: Operation<i8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    if cpu.status.get_flag_enabled(ZERO) {
        branch(cpu, addressing_mode, argument);
    }
};

pub(super) const BIT: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    cpu.status
        .set_flag_enabled(ZERO, cpu.accumulator & argument == 0);
    cpu.status.set_flag_enabled(NEGATIVE, argument & 0x80 > 0);
    cpu.status.set_flag_enabled(OVERFLOW, argument & 0x40 > 0);
};

pub(super) const BMI: Operation<i8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    if cpu.status.get_flag_enabled(NEGATIVE) {
        branch(cpu, addressing_mode, argument);
    }
};

pub(super) const BNE: Operation<i8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    if !cpu.status.get_flag_enabled(ZERO) {
        branch(cpu, addressing_mode, argument);
    }
};

pub(super) const BPL: Operation<i8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    if !cpu.status.get_flag_enabled(NEGATIVE) {
        branch(cpu, addressing_mode, argument);
    }
};

pub(super) const BRK: Operation<()> = |cpu, bus, _| {
    cpu.is_resetting = true;
    cpu.program_counter += 1;

    cpu.push_stack_u16(cpu.program_counter, bus);
    cpu.push_stack(cpu.status | BREAK, bus);

    cpu.status.set_flag_enabled(INTERRUPT_DISABLE, true);
    cpu.program_counter = bus.read_u16(0xFFFE);
};

pub(super) const BVC: Operation<i8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    if !cpu.status.get_flag_enabled(OVERFLOW) {
        branch(cpu, addressing_mode, argument);
    }
};

pub(super) const BVS: Operation<i8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    if cpu.status.get_flag_enabled(OVERFLOW) {
        branch(cpu, addressing_mode, argument);
    }
};

pub(super) const CLC: Operation<()> = |cpu, _, _| {
    cpu.status.set_flag_enabled(CARRY, false);
};

pub(super) const CLD: Operation<()> = |cpu, _, _| {
    cpu.status.set_flag_enabled(DECIMAL_MODE, false);
};

pub(super) const CLI: Operation<()> = |cpu, _, _| {
    // TODO: delay by 1 instruciton
    cpu.status.set_flag_enabled(INTERRUPT_DISABLE, false);
};

pub(super) const CLV: Operation<()> = |cpu, _, _| {
    cpu.status.set_flag_enabled(OVERFLOW, false);
};

pub(super) const CMP: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = cpu.accumulator.wrapping_sub(argument);

    cpu.status
        .set_flag_enabled(CARRY, cpu.accumulator >= argument);
    cpu.status
        .set_flag_enabled(ZERO, cpu.accumulator == argument);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);
};

pub(super) const CPX: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = cpu.x.wrapping_sub(argument);

    cpu.status.set_flag_enabled(CARRY, cpu.x >= argument);
    cpu.status.set_flag_enabled(ZERO, cpu.x == argument);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);
};

pub(super) const CPY: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = cpu.y.wrapping_sub(argument);

    cpu.status.set_flag_enabled(CARRY, cpu.y >= argument);
    cpu.status.set_flag_enabled(ZERO, cpu.y == argument);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);
};

pub(super) const DCP: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument: u8 = addressing_mode.read(cpu, bus);
    let result = argument.wrapping_sub(1);

    addressing_mode.write(result, cpu, bus);
    CMP(cpu, bus, addressing_mode);
};

pub(super) const DEC: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = argument.wrapping_sub(1);
    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    addressing_mode.write(result, cpu, bus);
};

pub(super) const DEX: Operation<()> = |cpu, _, _| {
    let result = cpu.x.wrapping_sub(1);
    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.x = result;
};

pub(super) const DEY: Operation<()> = |cpu, _, _| {
    let result = cpu.y.wrapping_sub(1);
    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.y = result;
};

pub(super) const EOR: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = cpu.accumulator ^ argument;
    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.accumulator = result;
};

pub(super) const INC: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = argument.wrapping_add(1);
    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    addressing_mode.write(result, cpu, bus);
};

pub(super) const INX: Operation<()> = |cpu, _, _| {
    let result = cpu.x.wrapping_add(1);
    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.x = result;
};

pub(super) const INY: Operation<()> = |cpu, _, _| {
    let result = cpu.y.wrapping_add(1);
    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.y = result;
};

pub(super) const ISB: Operation<u8> = |cpu, bus, addressing_mode| {
    INC(cpu, bus, addressing_mode);
    SBC(cpu, bus, addressing_mode);
};

pub(super) const JAM: Operation<()> = |cpu, _, _| {
    cpu.is_jammed = true;
};

pub(super) const JMP: Operation<MemoryAddress> = |cpu, bus, addressing_mode| {
    let argument: MemoryAddress = addressing_mode.read(cpu, bus);

    cpu.program_counter = argument.get_address();
};

pub(super) const JSR: Operation<MemoryAddress> = |cpu, bus, addressing_mode| {
    let argument: MemoryAddress = addressing_mode.read(cpu, bus);
    let result = cpu.program_counter.wrapping_sub(1);

    cpu.push_stack_u16(result, bus);

    cpu.program_counter = argument.get_address();
};

pub(super) const LAS: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = argument & cpu.stack_pointer;
    cpu.accumulator = result;
    cpu.x = result;
    cpu.stack_pointer = result;

    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);
};

pub(super) const LAX: Operation<u8> = |cpu, bus, addressing_mode| {
    LDA(cpu, bus, addressing_mode);
    LDX(cpu, bus, addressing_mode);
};

pub(super) const LDA: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    cpu.status.set_flag_enabled(ZERO, argument == 0);
    cpu.status.set_flag_enabled(NEGATIVE, argument & 0x80 > 0);

    cpu.accumulator = argument;
};

pub(super) const LDX: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    cpu.status.set_flag_enabled(ZERO, argument == 0);
    cpu.status.set_flag_enabled(NEGATIVE, argument & 0x80 > 0);

    cpu.x = argument;
};

pub(super) const LDY: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);

    cpu.status.set_flag_enabled(ZERO, argument == 0);
    cpu.status.set_flag_enabled(NEGATIVE, argument & 0x80 > 0);

    cpu.y = argument;
};

pub(super) const LSR: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = argument >> 1;

    cpu.status.set_flag_enabled(CARRY, argument & 0x1 > 0);
    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, false);

    addressing_mode.write(result, cpu, bus);
};

pub(super) const LXA: Operation<u8> = |_, _, _| {
    //TODO: impl this
};
pub(super) fn make_nop<T>() -> Operation<T> {
    |_, _, _| {}
}

pub(super) const ORA: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = cpu.accumulator | argument;

    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.accumulator = result;
};

pub(super) const PHA: Operation<()> = |cpu, bus, _| {
    cpu.push_stack(cpu.accumulator, bus);
};

pub(super) const PHP: Operation<()> = |cpu, bus, _| {
    cpu.push_stack(cpu.status | BREAK | UNUSED, bus);
};

pub(super) const PLA: Operation<()> = |cpu, bus, _| {
    let result = cpu.pop_stack(bus);

    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.accumulator = result;
};

pub(super) const PLP: Operation<()> = |cpu, bus, _| {
    let argument = cpu.pop_stack(bus);
    let result = (argument & !BREAK) | UNUSED;

    cpu.status = result;
};

pub(super) const RLA: Operation<u8> = |cpu, bus, addressing_mode| {
    ROL(cpu, bus, addressing_mode);
    AND(cpu, bus, addressing_mode);
};

pub(super) const ROL: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus) as u16;
    let mut result = argument << 1;

    if cpu.status.get_flag_enabled(CARRY) {
        result |= 0x1;
    }

    cpu.status.set_flag_enabled(CARRY, result > 0xFF);
    cpu.status.set_flag_enabled(ZERO, result & 0xFF == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    addressing_mode.write(result as u8, cpu, bus);
};

pub(super) const ROR: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let mut result = argument >> 1;

    if cpu.status.get_flag_enabled(CARRY) {
        result |= 0x80;
    }

    cpu.status.set_flag_enabled(CARRY, argument & 0x1 > 0);
    cpu.status.set_flag_enabled(ZERO, result & 0xFF == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    addressing_mode.write(result, cpu, bus);
};

pub(super) const RRA: Operation<u8> = |cpu, bus, addressing_mode| {
    ROR(cpu, bus, addressing_mode);
    ADC(cpu, bus, addressing_mode);
};

pub(super) const RTI: Operation<()> = |cpu, bus, _| {
    let flags = cpu.pop_stack(bus);
    cpu.status = (flags & !BREAK) | UNUSED;

    cpu.program_counter = cpu.pop_stack_u16(bus);
};

pub(super) const RTS: Operation<()> = |cpu, bus, _| {
    cpu.program_counter = cpu.pop_stack_u16(bus);
    cpu.program_counter += 1;
};

pub(super) const SAX: Operation<u8> = |cpu, bus, addressing_mode| {
    addressing_mode.write(cpu.accumulator & cpu.x, cpu, bus);
};

pub(super) const SBC: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    // Math best explain here:
    // https://www.nesdev.org/wiki/Instruction_reference#SBC
    // and the comment here (line 688):
    // https://github.com/OneLoneCoder/olcNES/blob/master/Part%232%20-%20CPU/olc6502.cpp#L688
    let result =
        cpu.accumulator as u16 + (!argument) as u16 + cpu.status.get_flag_enabled(CARRY) as u16;

    cpu.status.set_flag_enabled(CARRY, result > 0xFF);
    cpu.status.set_flag_enabled(ZERO, result & 0xFF == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);
    cpu.status.set_flag_enabled(
        OVERFLOW,
        ((cpu.accumulator ^ (result as u8)) & (cpu.accumulator ^ argument) & 0x80) > 0,
    );

    cpu.accumulator = result as u8;
};

pub(super) const SBX: Operation<u8> = |cpu, bus, addressing_mode| {
    let argument = addressing_mode.read(cpu, bus);
    let result = (cpu.accumulator & cpu.x).wrapping_sub(argument);
    cpu.x = result;

    cpu.status
        .set_flag_enabled(CARRY, cpu.accumulator >= argument);
    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);
};

pub(super) const SEC: Operation<()> = |cpu, _, _| {
    cpu.status.set_flag_enabled(CARRY, true);
};

pub(super) const SED: Operation<()> = |cpu, _, _| {
    cpu.status.set_flag_enabled(DECIMAL_MODE, true);
};

pub(super) const SEI: Operation<()> = |cpu, _, _| {
    cpu.status.set_flag_enabled(INTERRUPT_DISABLE, true);
};

pub(super) const SHA: Operation<MemoryAddress> = |cpu, bus, addressing_mode| {
    let mut argument: MemoryAddress = addressing_mode.read(cpu, bus);
    let value = cpu.accumulator & cpu.x & ((argument.get_address() >> 8) as u8).wrapping_add(1);
    argument.set_value(value);
    addressing_mode.write(argument, cpu, bus);
};

pub(super) const SHY: Operation<MemoryAddress> = |cpu, bus, addressing_mode| {
    let mut argument: MemoryAddress = addressing_mode.read(cpu, bus);
    let value = cpu.accumulator & cpu.y & ((argument.get_address() >> 8) as u8).wrapping_add(1);
    argument.set_value(value);
    addressing_mode.write(argument, cpu, bus);
};

pub(super) const SHX: Operation<MemoryAddress> = |cpu, bus, addressing_mode| {
    let mut argument: MemoryAddress = addressing_mode.read(cpu, bus);
    let value = cpu.accumulator & cpu.x & ((argument.get_address() >> 8) as u8).wrapping_add(1);
    argument.set_value(value);
    addressing_mode.write(argument, cpu, bus);
};

pub(super) const SLO: Operation<u8> = |cpu, bus, addressing_mode| {
    ASL(cpu, bus, addressing_mode);
    ORA(cpu, bus, addressing_mode);
};

pub(super) const SRE: Operation<u8> = |cpu, bus, addressing_mode| {
    LSR(cpu, bus, addressing_mode);
    EOR(cpu, bus, addressing_mode);
};

pub(super) const STA: Operation<u8> = |cpu, bus, addressing_mode| {
    addressing_mode.write(cpu.accumulator, cpu, bus);
};

pub(super) const STX: Operation<u8> = |cpu, bus, addressing_mode| {
    addressing_mode.write(cpu.x, cpu, bus);
};

pub(super) const STY: Operation<u8> = |cpu, bus, addressing_mode| {
    addressing_mode.write(cpu.y, cpu, bus);
};

pub(super) const TAS: Operation<MemoryAddress> = |cpu, bus, addressing_mode| {
    cpu.stack_pointer = cpu.accumulator & cpu.x;
    SHA(cpu, bus, addressing_mode);
};

pub(super) const TAX: Operation<()> = |cpu, _, _| {
    let result = cpu.accumulator;

    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.x = result;
};

pub(super) const TAY: Operation<()> = |cpu, _, _| {
    let result = cpu.accumulator;

    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.y = result;
};

pub(super) const TSX: Operation<()> = |cpu, _, _| {
    let result = cpu.stack_pointer;

    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.x = result;
};

pub(super) const TXA: Operation<()> = |cpu, _, _| {
    let result = cpu.x;

    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.accumulator = result;
};

pub(super) const TXS: Operation<()> = |cpu, _, _| {
    let result = cpu.x;

    cpu.stack_pointer = result;
};

pub(super) const TYA: Operation<()> = |cpu, _, _| {
    let result = cpu.y;

    cpu.status.set_flag_enabled(ZERO, result == 0);
    cpu.status.set_flag_enabled(NEGATIVE, result & 0x80 > 0);

    cpu.accumulator = result;
};
