//! Ok so this module is a bit wierd. We are basically defining 2 important
//! traits here: [InstructionTrait], [InstructionFactoryTrait] and their
//! sole implementations ([Instruction], [InstructionFactory]).
//! The [Instruction] holds information about what a specific opcode does. For
//! example the cyles required or the addressing mode. The [InstructionFactory]
//! can create an instruction each time one is needed in program execution.
//! The [InstructionFactory] is used to populate the [INSTRUCTIONS_LOOKUP]
//! and allow any user to get an instruciton by it's opcode from the table.
//!
//! So basically the [INSTRUCTIONS_LOOKUP] holds [InstructionFactory]s
//! that when instantiated return [Instruction]s that can be executed.

use std::{fmt::Debug, sync::LazyLock};

use crate::hardware::{
    cpu::{
        Cpu,
        addressing_modes::{AddressingMode, factories::*},
        operations::{Operation, *},
    },
    cpu_bus::CpuBus,
};

pub(super) struct Instruction<T> {
    operation: Operation<T>,
    operation_name: &'static str,
    addressing_mode: Box<dyn AddressingMode<T>>,
    cycles: u8,
    can_require_extra_cycles: bool,
    is_illegal: bool,
}

pub trait InstructionTrait {
    /// # Returns:
    /// The ammount of cycles required for that instruction to be executed
    fn execute(&mut self, cpu: &mut Cpu, bus: &mut CpuBus) -> u8;
    /// # Returns:
    /// The disassembled version of the instruction in string slice
    fn disassemble_instruction(&self) -> String;
    /// # Returns:
    /// The number you have to add to the program counter to go to the
    /// next instruction
    fn next_instruction_offset(&self) -> u16;
}

impl<T: Debug> InstructionTrait for Instruction<T> {
    fn execute(&mut self, cpu: &mut Cpu, bus: &mut CpuBus) -> u8 {
        (self.operation)(cpu, bus, &mut self.addressing_mode);
        let extra_cycles = if self.can_require_extra_cycles {
            self.addressing_mode.cpu_additional_cycles_required()
        } else {
            0
        };
        self.cycles + extra_cycles
    }
    fn disassemble_instruction(&self) -> String {
        format!(
            "{}{} {}",
            if self.is_illegal { "*" } else { " " },
            self.operation_name,
            self.addressing_mode.display()
        )
    }

    fn next_instruction_offset(&self) -> u16 {
        self.addressing_mode.cpu_program_counter_offset()
    }
}

pub(super) struct InstructionFactory<T, AM> {
    operation: Operation<T>,
    operation_name: &'static str,
    addressing_mode_factory: AddressingModeFactory<AM>,
    cycles: u8,
    can_require_extra_cycles: bool,
    is_illegal: bool,
}

pub(super) trait InstructionFactoryTrait: Send + Sync {
    /// # Returns:
    /// An executable and dissassemblable instruction
    fn create(&self, cpu: &Cpu, bus: &CpuBus) -> Box<dyn InstructionTrait>;
}

impl<T: 'static + Debug, AM: AddressingMode<T> + 'static> InstructionFactoryTrait
    for InstructionFactory<T, AM>
{
    fn create(&self, cpu: &Cpu, bus: &CpuBus) -> Box<dyn InstructionTrait> {
        Box::new(Instruction {
            operation: self.operation,
            addressing_mode: (self.addressing_mode_factory)(cpu, bus),
            cycles: self.cycles,
            operation_name: self.operation_name,
            can_require_extra_cycles: self.can_require_extra_cycles,
            is_illegal: self.is_illegal,
        })
    }
}

fn instruction_factory<T, AM>(
    operation: Operation<T>,
    mode: AddressingModeFactory<AM>,
    cycles: u8,
    name: &'static str,
    can_require_extra_cycles: bool,
    is_illegal: bool,
) -> Box<dyn InstructionFactoryTrait>
where
    T: 'static + Debug,
    AM: AddressingMode<T> + 'static,
{
    Box::new(InstructionFactory::<T, AM> {
        operation,
        addressing_mode_factory: mode,
        cycles,
        operation_name: name,
        can_require_extra_cycles,
        is_illegal,
    })
}

macro_rules! instruction {
    ($operation:expr, $mode:ident, $cycles:literal, $name:expr, $extra:expr, $illegal:expr) => {{ instruction_factory($operation, $mode, $cycles, $name, $extra, $illegal) }};
}

macro_rules! instruction_entry_set_name {
    (NOP, IMPLICIT, $cycles:literal, $extra:expr, $illegal:expr) => {{ instruction!(make_nop::<()>(), IMPLICIT, $cycles, "NOP", $extra, $illegal) }};
    (NOP, $mode:ident, $cycles:literal, $extra:expr, $illegal:expr) => {{ instruction!(make_nop::<u8>(), $mode, $cycles, "NOP", $extra, $illegal) }};
    ($instruction:ident, $mode:ident, $cycles:literal, $extra:expr, $illegal:expr) => {{
        instruction!(
            $instruction,
            $mode,
            $cycles,
            stringify!($instruction),
            $extra,
            $illegal
        )
    }};
}

macro_rules! instruction_entry {
    ({ * NOP, $mode:ident *, $cycles:literal }) => {
        instruction_entry_set_name!(NOP, $mode, $cycles, true, true)
    };
    ({ * NOP, $mode:ident, $cycles:literal }) => {
        instruction_entry_set_name!(NOP, $mode, $cycles, false, true)
    };
    ({ NOP, $mode:ident *, $cycles:literal }) => {
        instruction_entry_set_name!(NOP, $mode, $cycles, true, false)
    };
    ({ NOP, $mode:ident, $cycles:literal }) => {
        instruction_entry_set_name!(NOP, $mode, $cycles, false, false)
    };
    ({ * $instruction:ident, $mode:ident *, $cycles:literal }) => {
        instruction_entry_set_name!($instruction, $mode, $cycles, true, true)
    };
    ({ * $instruction:ident, $mode:ident, $cycles:literal }) => {
        instruction_entry_set_name!($instruction, $mode, $cycles, false, true)
    };
    ({ $instruction:ident, $mode:ident *, $cycles:literal }) => {
        instruction_entry_set_name!($instruction, $mode, $cycles, true, false)
    };
    ({ $instruction:ident, $mode:ident, $cycles:literal }) => {
        instruction_entry_set_name!($instruction, $mode, $cycles, false, false)
    };
}

macro_rules! instruction_factories {
    ($($entry:tt),* $(,)?) => {
        vec![$(instruction_entry!($entry)),*]
    };
}

pub(super) static INSTRUCTIONS_LOOKUP: LazyLock<&'static [Box<dyn InstructionFactoryTrait>]> =
    LazyLock::new(|| {
        let ops_slice = get_instructions().into_boxed_slice();
        Box::leak(ops_slice)
    });

#[rustfmt::skip]
fn get_instructions() -> Vec<Box<dyn InstructionFactoryTrait>> {
    // illegal ops from here https://www.masswerk.at/6502/6502_instruction_set.html
    // recreation of this table (page 10): http://archive.6502.org/datasheets/rockwell_r650x_r651x.pdf
    instruction_factories![
        { BRK, IMPLICIT    , 7 }, { ORA, INDIRECT_X_OFFSET , 6 }, {*JAM, IMPLICIT , 1 }, {*SLO, INDIRECT_X_OFFSET ,8 }, {*NOP, ZERO_PAGE         , 3 }, { ORA, ZERO_PAGE         , 3 },{ ASL, ZERO_PAGE         , 5 }, {*SLO, ZERO_PAGE         , 5 }, { PHP, IMPLICIT, 3 }, { ORA, IMMEDIATE         , 2 }, { ASL, ACCUMULATOR, 2 }, {*ANC, IMMEDIATE         , 2 }, {*NOP, ABSOLUTE          , 4 }, { ORA, ABSOLUTE          , 4 }, { ASL, ABSOLUTE          , 6 }, {*SLO, ABSOLUTE          , 6 }, 
        { BPL, RELATIVE*   , 2 }, { ORA, INDIRECT_Y_OFFSET*, 5 }, {*JAM, IMPLICIT , 1 }, {*SLO, INDIRECT_Y_OFFSET ,8 }, {*NOP, ZERO_PAGE_X_OFFSET, 4 }, { ORA, ZERO_PAGE_X_OFFSET, 4 },{ ASL, ZERO_PAGE_X_OFFSET, 6 }, {*SLO, ZERO_PAGE_X_OFFSET, 6 }, { CLC, IMPLICIT, 2 }, { ORA, ABSOLUTE_Y_OFFSET*, 4 }, {*NOP, IMPLICIT   , 2 }, {*SLO, ABSOLUTE_Y_OFFSET , 7 }, {*NOP, ABSOLUTE_X_OFFSET*, 4 }, { ORA, ABSOLUTE_X_OFFSET*, 4 }, { ASL, ABSOLUTE_X_OFFSET , 7 }, {*SLO, ABSOLUTE_X_OFFSET , 7 },
        { JSR, ABSOLUTE_JMP, 6 }, { AND, INDIRECT_X_OFFSET , 6 }, {*JAM, IMPLICIT , 1 }, {*RLA, INDIRECT_X_OFFSET ,8 }, { BIT, ZERO_PAGE         , 3 }, { AND, ZERO_PAGE         , 3 },{ ROL, ZERO_PAGE         , 5 }, {*RLA, ZERO_PAGE         , 5 }, { PLP, IMPLICIT, 4 }, { AND, IMMEDIATE         , 2 }, { ROL, ACCUMULATOR, 2 }, {*ANC, IMMEDIATE         , 2 }, { BIT, ABSOLUTE          , 4 }, { AND, ABSOLUTE          , 4 }, { ROL, ABSOLUTE          , 6 }, {*RLA, ABSOLUTE          , 6 },
        { BMI, RELATIVE*   , 2 }, { AND, INDIRECT_Y_OFFSET*, 5 }, {*JAM, IMPLICIT , 1 }, {*RLA, INDIRECT_Y_OFFSET ,8 }, {*NOP, ZERO_PAGE_X_OFFSET, 4 }, { AND, ZERO_PAGE_X_OFFSET, 4 },{ ROL, ZERO_PAGE_X_OFFSET, 6 }, {*RLA, ZERO_PAGE_X_OFFSET, 6 }, { SEC, IMPLICIT, 2 }, { AND, ABSOLUTE_Y_OFFSET*, 4 }, {*NOP, IMPLICIT   , 2 }, {*RLA, ABSOLUTE_Y_OFFSET , 7 }, {*NOP, ABSOLUTE_X_OFFSET*, 4 }, { AND, ABSOLUTE_X_OFFSET*, 4 }, { ROL, ABSOLUTE_X_OFFSET , 7 }, {*RLA, ABSOLUTE_X_OFFSET , 7 },
        { RTI, IMPLICIT    , 6 }, { EOR, INDIRECT_X_OFFSET , 6 }, {*JAM, IMPLICIT , 1 }, {*SRE, INDIRECT_X_OFFSET ,8 }, {*NOP, ZERO_PAGE         , 3 }, { EOR, ZERO_PAGE         , 3 },{ LSR, ZERO_PAGE         , 5 }, {*SRE, ZERO_PAGE         , 5 }, { PHA, IMPLICIT, 3 }, { EOR, IMMEDIATE         , 2 }, { LSR, ACCUMULATOR, 2 }, {*ALR, IMMEDIATE         , 2 }, { JMP, ABSOLUTE_JMP      , 3 }, { EOR, ABSOLUTE          , 4 }, { LSR, ABSOLUTE          , 6 }, {*SRE, ABSOLUTE          , 6 },
        { BVC, RELATIVE*   , 2 }, { EOR, INDIRECT_Y_OFFSET*, 5 }, {*JAM, IMPLICIT , 1 }, {*SRE, INDIRECT_Y_OFFSET ,8 }, {*NOP, ZERO_PAGE_X_OFFSET, 4 }, { EOR, ZERO_PAGE_X_OFFSET, 4 },{ LSR, ZERO_PAGE_X_OFFSET, 6 }, {*SRE, ZERO_PAGE_X_OFFSET, 6 }, { CLI, IMPLICIT, 2 }, { EOR, ABSOLUTE_Y_OFFSET*, 4 }, {*NOP, IMPLICIT   , 2 }, {*SRE, ABSOLUTE_Y_OFFSET , 7 }, {*NOP, ABSOLUTE_X_OFFSET*, 4 }, { EOR, ABSOLUTE_X_OFFSET*, 4 }, { LSR, ABSOLUTE_X_OFFSET , 7 }, {*SRE, ABSOLUTE_X_OFFSET , 7 },
        { RTS, IMPLICIT    , 6 }, { ADC, INDIRECT_X_OFFSET , 6 }, {*JAM, IMPLICIT , 2 }, {*RRA, INDIRECT_X_OFFSET ,8 }, {*NOP, ZERO_PAGE         , 3 }, { ADC, ZERO_PAGE         , 3 },{ ROR, ZERO_PAGE         , 5 }, {*RRA, ZERO_PAGE         , 5 }, { PLA, IMPLICIT, 4 }, { ADC, IMMEDIATE         , 2 }, { ROR, ACCUMULATOR, 2 }, {*ARR, IMMEDIATE         , 2 }, { JMP, INDIRECT          , 5 }, { ADC, ABSOLUTE          , 4 }, { ROR, ABSOLUTE          , 6 }, {*RRA, ABSOLUTE          , 6 },
        { BVS, RELATIVE*   , 2 }, { ADC, INDIRECT_Y_OFFSET*, 5 }, {*JAM, IMPLICIT , 2 }, {*RRA, INDIRECT_Y_OFFSET ,8 }, {*NOP, ZERO_PAGE_X_OFFSET, 4 }, { ADC, ZERO_PAGE_X_OFFSET, 4 },{ ROR, ZERO_PAGE_X_OFFSET, 6 }, {*RRA, ZERO_PAGE_X_OFFSET, 6 }, { SEI, IMPLICIT, 2 }, { ADC, ABSOLUTE_Y_OFFSET*, 4 }, {*NOP, IMPLICIT   , 2 }, {*RRA, ABSOLUTE_Y_OFFSET , 7 }, {*NOP, ABSOLUTE_X_OFFSET*, 4 }, { ADC, ABSOLUTE_X_OFFSET*, 4 }, { ROR, ABSOLUTE_X_OFFSET , 7 }, {*RRA, ABSOLUTE_X_OFFSET , 7 },
        {*NOP, IMMEDIATE   , 2 }, { STA, INDIRECT_X_OFFSET , 6 }, {*NOP, IMMEDIATE, 2 }, {*SAX, INDIRECT_X_OFFSET ,6 }, { STY, ZERO_PAGE         , 3 }, { STA, ZERO_PAGE         , 3 },{ STX, ZERO_PAGE         , 3 }, {*SAX, ZERO_PAGE         , 3 }, { DEY, IMPLICIT, 2 }, {*NOP, IMMEDIATE         , 2 }, { TXA, IMPLICIT   , 2 }, {*ANE, IMMEDIATE         , 2 }, { STY, ABSOLUTE          , 4 }, { STA, ABSOLUTE          , 4 }, { STX, ABSOLUTE          , 4 }, {*SAX, ABSOLUTE          , 4 },
        { BCC, RELATIVE*   , 2 }, { STA, INDIRECT_Y_OFFSET , 6 }, {*JAM, IMPLICIT , 1 }, {*SHA, INDIRECT_Y_OFFSET ,6 }, { STY, ZERO_PAGE_X_OFFSET, 4 }, { STA, ZERO_PAGE_X_OFFSET, 4 },{ STX, ZERO_PAGE_Y_OFFSET, 4 }, {*SAX, ZERO_PAGE_Y_OFFSET, 4 }, { TYA, IMPLICIT, 2 }, { STA, ABSOLUTE_Y_OFFSET , 5 }, { TXS, IMPLICIT   , 2 }, {*TAS, ABSOLUTE_Y_OFFSET , 5 }, {*SHY, ABSOLUTE_X_OFFSET , 5 }, { STA, ABSOLUTE_X_OFFSET , 5 }, {*SHX, ABSOLUTE_Y_OFFSET , 5 }, {*SHA, ABSOLUTE_Y_OFFSET , 5 },
        { LDY, IMMEDIATE   , 2 }, { LDA, INDIRECT_X_OFFSET , 6 }, { LDX, IMMEDIATE, 2 }, {*LAX, INDIRECT_X_OFFSET ,6 }, { LDY, ZERO_PAGE         , 3 }, { LDA, ZERO_PAGE         , 3 },{ LDX, ZERO_PAGE         , 3 }, {*LAX, ZERO_PAGE         , 3 }, { TAY, IMPLICIT, 2 }, { LDA, IMMEDIATE         , 2 }, { TAX, IMPLICIT   , 2 }, {*LXA, IMMEDIATE         , 2 }, { LDY, ABSOLUTE          , 4 }, { LDA, ABSOLUTE          , 4 }, { LDX, ABSOLUTE          , 4 }, {*LAX, ABSOLUTE          , 4 },
        { BCS, RELATIVE*   , 2 }, { LDA, INDIRECT_Y_OFFSET*, 5 }, {*JAM, IMPLICIT , 1 }, {*LAX, INDIRECT_Y_OFFSET*,5 }, { LDY, ZERO_PAGE_X_OFFSET, 4 }, { LDA, ZERO_PAGE_X_OFFSET, 4 },{ LDX, ZERO_PAGE_Y_OFFSET, 4 }, {*LAX, ZERO_PAGE_Y_OFFSET, 4 }, { CLV, IMPLICIT, 2 }, { LDA, ABSOLUTE_Y_OFFSET*, 4 }, { TSX, IMPLICIT   , 2 }, {*LAS, ABSOLUTE_Y_OFFSET*, 4 }, { LDY, ABSOLUTE_X_OFFSET*, 4 }, { LDA, ABSOLUTE_X_OFFSET*, 4 }, { LDX, ABSOLUTE_Y_OFFSET*, 4 }, {*LAX, ABSOLUTE_Y_OFFSET*, 4 },
        { CPY, IMMEDIATE   , 2 }, { CMP, INDIRECT_X_OFFSET , 6 }, {*NOP, IMMEDIATE, 2 }, {*DCP, INDIRECT_X_OFFSET ,8 }, { CPY, ZERO_PAGE         , 3 }, { CMP, ZERO_PAGE         , 3 },{ DEC, ZERO_PAGE         , 5 }, {*DCP, ZERO_PAGE         , 5 }, { INY, IMPLICIT, 2 }, { CMP, IMMEDIATE         , 2 }, { DEX, IMPLICIT   , 2 }, {*SBX, IMMEDIATE         , 2 }, { CPY, ABSOLUTE          , 4 }, { CMP, ABSOLUTE          , 4 }, { DEC, ABSOLUTE          , 6 }, {*DCP, ABSOLUTE          , 6 },
        { BNE, RELATIVE*   , 2 }, { CMP, INDIRECT_Y_OFFSET*, 5 }, {*JAM, IMPLICIT , 1 }, {*DCP, INDIRECT_Y_OFFSET ,8 }, {*NOP, ZERO_PAGE_X_OFFSET, 4 }, { CMP, ZERO_PAGE_X_OFFSET, 4 },{ DEC, ZERO_PAGE_X_OFFSET, 6 }, {*DCP, ZERO_PAGE_X_OFFSET, 6 }, { CLD, IMPLICIT, 2 }, { CMP, ABSOLUTE_Y_OFFSET*, 4 }, {*NOP, IMPLICIT   , 2 }, {*DCP, ABSOLUTE_Y_OFFSET , 7 }, {*NOP, ABSOLUTE_X_OFFSET*, 4 }, { CMP, ABSOLUTE_X_OFFSET*, 4 }, { DEC, ABSOLUTE_X_OFFSET , 7 }, {*DCP, ABSOLUTE_X_OFFSET , 7 },
        { CPX, IMMEDIATE   , 2 }, { SBC, INDIRECT_X_OFFSET , 6 }, {*NOP, IMMEDIATE, 2 }, {*ISB, INDIRECT_X_OFFSET ,8 }, { CPX, ZERO_PAGE         , 3 }, { SBC, ZERO_PAGE         , 3 },{ INC, ZERO_PAGE         , 5 }, {*ISB, ZERO_PAGE         , 5 }, { INX, IMPLICIT, 2 }, { SBC, IMMEDIATE         , 2 }, { NOP, IMPLICIT   , 2 }, {*SBC, IMMEDIATE         , 2 }, { CPX, ABSOLUTE          , 4 }, { SBC, ABSOLUTE          , 4 }, { INC, ABSOLUTE          , 6 }, {*ISB, ABSOLUTE          , 6 },
        { BEQ, RELATIVE*   , 2 }, { SBC, INDIRECT_Y_OFFSET*, 5 }, {*JAM, IMPLICIT , 1 }, {*ISB, INDIRECT_Y_OFFSET ,8 }, {*NOP, ZERO_PAGE_X_OFFSET, 4 }, { SBC, ZERO_PAGE_X_OFFSET, 4 },{ INC, ZERO_PAGE_X_OFFSET, 6 }, {*ISB, ZERO_PAGE_X_OFFSET, 6 }, { SED, IMPLICIT, 2 }, { SBC, ABSOLUTE_Y_OFFSET*, 4 }, {*NOP, IMPLICIT   , 2 }, {*ISB, ABSOLUTE_Y_OFFSET , 7 }, {*NOP, ABSOLUTE_X_OFFSET*, 4 }, { SBC, ABSOLUTE_X_OFFSET*, 4 }, { INC, ABSOLUTE_X_OFFSET , 7 }, {*ISB, ABSOLUTE_X_OFFSET , 7 },
    ]
}
