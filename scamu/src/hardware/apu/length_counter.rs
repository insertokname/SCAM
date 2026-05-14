use crate::hardware::constants::apu::LENGTH_COUNTER_TABLE;

/// implementation of this: https://www.nesdev.org/wiki/APU_Length_Counter
#[derive(Default, Debug, Clone)]
pub struct LengthCounter {
    enabled: bool,
    pub halt_length_counter: bool,
    length_counter: u8,
}

impl LengthCounter {
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !self.enabled {
            self.length_counter = 0;
        }
    }

    pub fn set_length_counter_load(&mut self, length_counter_load: u8) {
        if !self.enabled {
            return;
        }
        self.length_counter = LENGTH_COUNTER_TABLE[length_counter_load as usize];
    }

    pub fn tick(&mut self) {
        if !self.halt_length_counter && self.length_counter != 0 {
            self.length_counter -= 1;
        }
    }

    pub fn is_non_zero(&self) -> bool {
        self.length_counter != 0
    }
}

impl Iterator for LengthCounter {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.length_counter == 0 {
            Some(0)
        } else {
            Some(1)
        }
    }
}
