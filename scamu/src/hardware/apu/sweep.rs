use crate::hardware::{
    apu::pulse_channel::PulseChannelType, bit_ops::BitOps, constants::apu::register1_flags,
};

/// implementation of this: https://www.nesdev.org/wiki/APU_Sweep
#[derive(Default, Debug, Clone)]
pub struct Sweep {
    reload_flag: bool,
    enabled_flag: bool,
    negate_flag: bool,
    shift_count: u8,
    divier_timer: u8,
    divier_period: u8,
}

impl Sweep {
    pub fn set_register(&mut self, value: u8) {
        self.enabled_flag = value.get_flag_enabled(register1_flags::ENABLED);
        self.divier_period = value.get_bitfield(register1_flags::DIVIDER_PERIOD);
        self.negate_flag = value.get_flag_enabled(register1_flags::NEGATE);
        self.shift_count = value.get_bitfield(register1_flags::SHIFT_COUNT);
        self.reload_flag = true;
    }

    fn target_period(&self, pulse_timer_period: u16, channel: PulseChannelType) -> i32 {
        let mut change_ammount = (pulse_timer_period >> self.shift_count) as i32;
        if self.negate_flag {
            match channel {
                PulseChannelType::Pulse1 => change_ammount = 0 - change_ammount - 1,
                PulseChannelType::Pulse2 => change_ammount = 0 - change_ammount,
            };
        }
        pulse_timer_period as i32 + change_ammount
    }

    pub fn is_muted(&self, pulse_timer_period: u16, channel: PulseChannelType) -> bool {
        pulse_timer_period < 8 || self.target_period(pulse_timer_period, channel) > 0x7FF
    }

    pub fn tick(&mut self, pulse_timer_period: &mut u16, channel: PulseChannelType) {
        if self.divier_timer == 0
            && self.enabled_flag
            && self.shift_count != 0
            && !self.is_muted(*pulse_timer_period, channel)
        {
            *pulse_timer_period = self.target_period(*pulse_timer_period, channel).max(0) as u16;
        }

        if self.divier_timer == 0 || self.reload_flag {
            self.divier_timer = self.divier_period;
            self.reload_flag = false;
        } else {
            self.divier_timer -= 1;
        }
    }
}
