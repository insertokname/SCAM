use crate::hardware::{
    apu::{ApuTick, envelope::Envelope, length_counter::LengthCounter, sweep::Sweep},
    bit_ops::BitOps,
    constants::apu::{PULSE_WAVEFORMS, register0_flags, register2_flags, register3_flags},
};

#[derive(Default, Debug, Clone, Copy)]
pub enum PulseChannelType {
    #[default]
    Pulse1,
    Pulse2,
}

/// implementation of this: https://www.nesdev.org/wiki/APU_Pulse
#[derive(Default, Debug, Clone)]
pub struct PulseChannel {
    waveform: u8,
    sequence_step: u8,
    divider_period: u16,
    divider_timer: u16,

    envelope: Envelope,
    length_counter: LengthCounter,
    sweep: Sweep,

    channel_type: PulseChannelType,

    register0: u8,
    register1: u8,
    register2: u8,
    register3: u8,
}

impl PulseChannel {
    pub fn new(channel_type: PulseChannelType) -> Self {
        Self {
            channel_type,
            ..Default::default()
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.length_counter.set_enabled(enabled);
    }

    pub fn is_length_counter_non_zero(&self) -> bool {
        self.length_counter.is_non_zero()
    }

    pub fn write_register(&mut self, address: u16, value: u8) {
        match address % 4 {
            0 => {
                self.register0 = value;
                let duty_cycle = self.register0.get_bitfield(register0_flags::DUTY_CYCLE);
                self.waveform =
                    PULSE_WAVEFORMS[duty_cycle as usize].rotate_right(self.sequence_step as u32);

                self.envelope.write_register(address, value);

                let halt_length_counter = self
                    .register0
                    .get_flag_enabled(register0_flags::LENGTH_COUNTER_HALT);
                self.length_counter.halt_length_counter = halt_length_counter;
            }
            1 => {
                self.register1 = value;
                self.sweep.set_register(value);
            }
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
                let duty_cycle_id = self.register0.get_bitfield(register0_flags::DUTY_CYCLE);
                self.sequence_step = 0;
                self.waveform = PULSE_WAVEFORMS[duty_cycle_id as usize];

                self.envelope.write_register(address, value);

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
        if tick.is_apu_cycle {
            if self.divider_timer == 0 {
                self.waveform = self.waveform.rotate_right(1);
                self.sequence_step = (self.sequence_step + 1) & 0b00000111;
                self.divider_timer = self.divider_period;
            } else {
                self.divider_timer -= 1;
            }
        }

        if tick.is_half_frame {
            self.length_counter.tick();
            self.sweep.tick(&mut self.divider_period, self.channel_type);
        }

        if tick.is_quarter_frame {
            self.envelope.tick();
        }
    }
}

impl Iterator for PulseChannel {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        let sequencer_output = ((self.waveform & 0x80) != 0) as u8;
        let not_muted = (!self.sweep.is_muted(self.divider_period, self.channel_type)) as u8;
        Some(sequencer_output * not_muted * self.envelope.next()? * self.length_counter.next()?)
    }
}
