use crate::hardware::{
    bit_ops::BitOps,
    constants::cpu::flags::*,
    cpu::instructions::{INSTRUCTIONS_LOOKUP, InstructionTrait},
    cpu_bus::CpuBus,
};

mod addressing_modes;
mod instructions;
mod operations;

#[derive(Default, Debug, Clone, Copy)]
pub enum DmaState {
    #[default]
    None,
    Initializing {
        page: u8,
    },
    Transfering {
        page: u8,
        index: u8,
        fetched_value: u8,
    },
}

#[derive(Debug, Clone)]
pub struct Cpu {
    accumulator: u8,
    x: u8,
    y: u8,
    program_counter: u16,
    stack_pointer: u8,
    status: u8,
    cycles_left: u8,
    total_cycles: u64,
    is_resetting: bool,
    is_jammed: bool, // Caused by the JAM instruction
    pub is_triggered_nmi: bool,
    pub is_triggered_irq: bool,
    pub dma_status: DmaState,
}

// TODO: impl interupts
impl Cpu {
    pub fn new() -> Self {
        Self {
            accumulator: 0,
            x: 0,
            y: 0,
            program_counter: 0,
            stack_pointer: 0xFD,
            status: UNUSED | INTERRUPT_DISABLE,
            cycles_left: 0,
            total_cycles: 7,
            is_resetting: false,
            is_jammed: false,
            is_triggered_irq: false,
            is_triggered_nmi: false,
            dma_status: DmaState::None,
        }
    }

    pub fn is_resetting(&self) -> bool {
        self.is_resetting
    }

    pub fn reset(&mut self, bus: &CpuBus) {
        *self = Self::new();
        self.program_counter = bus.read_u16(0xFFFC);
        self.is_jammed = false;
        self.is_resetting = false;
    }

    pub fn reset_with_program_counter(&mut self, program_counter: u16) {
        *self = Self::new();
        self.program_counter = program_counter;
        self.is_jammed = false;
        self.is_resetting = false;
    }

    pub fn get_program_counter(&self) -> u16 {
        self.program_counter
    }

    pub fn push_stack(&mut self, value: u8, bus: &mut CpuBus) {
        bus.write(0x100 + self.stack_pointer as u16, value);
        self.stack_pointer = self.stack_pointer.wrapping_sub(1);
    }

    pub fn pop_stack(&mut self, bus: &CpuBus) -> u8 {
        self.stack_pointer = self.stack_pointer.wrapping_add(1);
        bus.read(0x100 + self.stack_pointer as u16)
    }

    pub fn push_stack_u16(&mut self, value: u16, bus: &mut CpuBus) {
        let high = (value >> 8) as u8;
        let low = value as u8;

        self.push_stack(high, bus);
        self.push_stack(low, bus);
    }

    pub fn pop_stack_u16(&mut self, bus: &CpuBus) -> u16 {
        let low = self.pop_stack(bus) as u16;
        let high = self.pop_stack(bus) as u16;
        (high << 8) | low
    }

    pub fn get_cycles_left(&self) -> u8 {
        self.cycles_left
    }

    pub fn get_total_cycles(&self) -> u64 {
        self.total_cycles
    }

    pub fn get_next_instruction(&mut self, bus: &CpuBus) -> Box<dyn InstructionTrait> {
        let instruction_code = bus.peek(self.program_counter);

        self.program_counter += 1;

        let next_instruction = (&INSTRUCTIONS_LOOKUP[instruction_code as usize]).create(self, bus);

        self.program_counter += next_instruction.next_instruction_offset();

        return next_instruction;
    }

    pub fn tick(&mut self, bus: &mut CpuBus) {
        if self.is_jammed {
            return;
        }

        if self.is_resetting {
            self.is_resetting = false;
        }

        if self.is_triggered_nmi
            || (self.is_triggered_irq && !self.status.get_flag_enabled(INTERRUPT_DISABLE))
        {
            self.push_stack_u16(self.program_counter, bus);
            self.push_stack(self.status, bus);
            self.status.set_flag_enabled(INTERRUPT_DISABLE, true);

            if self.is_triggered_nmi {
                self.program_counter = bus.read_u16(0xFFFA);
            } else {
                self.program_counter = bus.read_u16(0xFFFE);
            }

            self.is_triggered_nmi = false;
            self.is_triggered_irq = false;
        }

        if self.cycles_left > 0 {
            self.cycles_left -= 1;
        } else {
            let instruction_location = self.program_counter;
            let instruction_code = bus.peek(self.program_counter);

            self.program_counter += 1;

            let mut next_instruction =
                (&INSTRUCTIONS_LOOKUP[instruction_code as usize]).create(self, bus);

            // We are incrementing the program counter to the first location
            // after the immediate value. This is the expected behaviour
            // on the 6502 so yeah
            self.program_counter += next_instruction.next_instruction_offset();

            let length = 1 + next_instruction.next_instruction_offset() as usize;
            let mut bytes = Vec::with_capacity(length);
            for i in 0..length {
                bytes.push(bus.peek(instruction_location + i as u16));
            }
            let byte_str = match length {
                1 => format!("{:02X}      ", bytes[0]),
                2 => format!("{:02X} {:02X}   ", bytes[0], bytes[1]),
                3 => format!("{:02X} {:02X} {:02X}", bytes[0], bytes[1], bytes[2]),
                _ => unreachable!(),
            };
            let disasm = next_instruction.disassemble_instruction();
            log::info!(
                "{:04X}  {} {:<33}A:{:02X} X:{:02X} Y:{:02X} P:{:02X} SP:{:02X} CYC:{}",
                instruction_location,
                byte_str,
                disasm,
                self.accumulator,
                self.x,
                self.y,
                self.status,
                self.stack_pointer,
                self.total_cycles
            );

            let required_cycles = next_instruction.execute(self, bus);
            self.cycles_left += required_cycles;

            // We treat this whole loading and executing as 1 cycle
            // and the other ones are stored in the left_cycles and we
            // will artificially drain the left_cycles in the next ticks
            self.cycles_left -= 1;
            self.total_cycles = self.total_cycles + required_cycles as u64;
        }
    }
}
