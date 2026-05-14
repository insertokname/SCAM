use better_default::Default;

use crate::hardware::{
    apu::{ApuTick, length_counter::LengthCounter},
    bit_ops::BitOps,
    constants::apu::{TRIANGLE_WAVEFORMS, register2_flags, register3_flags, triangle_register0},
};

/// implementation of: https://www.nesdev.org/wiki/APU_Triangle
#[derive(Default, Debug, Clone)]
pub struct TriangleChannel {
    control_flag: bool,
    linear_reload_flag: bool,
    divider_period: u16,
    divider_timer: u16,
    linear_period: u8,
    linear_timer: u8,

    waveform_index: usize,

    length_counter: LengthCounter,

    register0: u8,
    register2: u8,
    register3: u8,
}

impl TriangleChannel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.length_counter.set_enabled(enabled);
    }

    pub fn write_register(&mut self, address: u16, value: u8) {
        match address % 4 {
            0 => {
                self.register0 = value;
                self.linear_period = self
                    .register0
                    .get_bitfield(triangle_register0::COUNTER_RELOAD);
                self.control_flag = self
                    .register0
                    .get_flag_enabled(triangle_register0::CONTROL_FLAG);
                let length_counter_halt = self
                    .register0
                    .get_flag_enabled(triangle_register0::LENGTH_COUNTER_HALT);
                self.length_counter.halt_length_counter = length_counter_halt;
            }
            1 => {}
            2 => {
                self.register2 = value;
                self.divider_period
                    .set_bitmasked(register2_flags::TIMER_LOW as u16, self.register2 as u16);
            }
            3 => {
                self.register3 = value;
                self.divider_period.set_bitmasked(
                    (register3_flags::TIMER_HIGH as u16) << 8,
                    (self.register3 as u16) << 8,
                );
                self.divider_period += 1;
                self.divider_period = self.divider_period.get_bitmasked(
                    ((register3_flags::TIMER_HIGH as u16) << 8) | register2_flags::TIMER_LOW as u16,
                );
                self.linear_reload_flag = true;

                let length_counter_load = self
                    .register3
                    .get_bitfield(register3_flags::LENGTH_COUNTER_LOAD);
                self.length_counter
                    .set_length_counter_load(length_counter_load);
            }
            _ => (),
        }
    }

    pub fn tick(&mut self, tick: ApuTick) {
        if tick.is_quarter_frame {
            if self.linear_reload_flag {
                self.linear_timer = self.linear_period;
            } else if self.linear_timer > 0 {
                self.linear_timer -= 1
            }

            if !self.control_flag {
                self.linear_reload_flag = false;
            }
        }

        if tick.is_half_frame {
            self.length_counter.tick();
        }

        // ticks on every tick, not only apu ticks
        if self.divider_timer == 0 {
            self.divider_timer = self.divider_period;

            if self.length_counter.next().unwrap() != 0 && self.linear_timer != 0 {
                self.waveform_index += 1;
                self.waveform_index %= TRIANGLE_WAVEFORMS.len();
            }
        } else {
            self.divider_timer -= 1;
        }
    }
}

impl Iterator for TriangleChannel {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        Some(
            TRIANGLE_WAVEFORMS[self.waveform_index]
                * self.length_counter.next()?
                * (self.linear_timer != 0) as u8,
        )
    }
}
