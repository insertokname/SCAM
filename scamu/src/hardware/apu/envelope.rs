use crate::hardware::{bit_ops::BitOps, constants::apu::register0_flags};

/// implementation of this: https://www.nesdev.org/wiki/APU_Envelope
#[derive(Default, Debug, Clone)]
pub struct Envelope {
    start_flag: bool,
    constant_volume_flag: bool,
    loop_flag: bool,
    volume: u8,
    divider_period: u8,
    divider_timer: u8,
    decay_level: u8,
}

impl Envelope {
    pub fn write_register(&mut self, address: u16, value: u8) {
        match address % 4 {
            0 => {
                self.volume = value.get_bitfield(register0_flags::ENVELOPE_VOLUME);
                self.divider_period = self.volume;
                self.loop_flag = value.get_flag_enabled(register0_flags::LOOP);
                self.constant_volume_flag =
                    value.get_flag_enabled(register0_flags::IS_CONSTANT_VOLUME);
            }
            3 => {
                self.start_flag = true;
            }
            _ => (),
        }
    }

    pub fn tick(&mut self) {
        if self.start_flag {
            self.start_flag = false;
            self.decay_level = 15;
            self.divider_timer = self.divider_period;
        } else {
            if self.divider_timer != 0 {
                self.divider_timer -= 1;
            } else {
                self.divider_timer = self.divider_period;
                if self.decay_level != 0 {
                    self.decay_level -= 1;
                } else if self.loop_flag {
                    self.decay_level = 15;
                }
            }
        }
    }
}

impl Iterator for Envelope {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.constant_volume_flag {
            Some(self.volume)
        } else {
            Some(self.decay_level)
        }
    }
}
