use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use better_default::Default;

use crate::hardware::{
    apu::{
        pulse_channel::{PulseChannel, PulseChannelType},
        triangle_channel::TriangleChannel,
    },
    bit_ops::BitOps,
    constants::{
        apu::{SAMPLE_QUEUE_SIZE, frame_counter_register, status_register},
        clock_rates::{APU_SAMPLE_RATE, CPU_CLOCK},
    },
    cpu::Cpu,
};

pub mod envelope;
pub mod length_counter;
pub mod pulse_channel;
pub mod sweep;
pub mod triangle_channel;

#[derive(Default, Clone, Copy, Debug)]
pub struct ApuTick {
    pub is_apu_cycle: bool,
    pub is_quarter_frame: bool,
    pub is_half_frame: bool,
}

/// https://www.nesdev.org/wiki/APU
#[derive(Default, Debug, Clone)]
pub struct Apu {
    /// If you are not using the default [MASTER_CLOCK](crate::hardware::constants::clock_rates::MASTER_CLOCK)
    /// value to tick the emulator, you should set this to your custom
    /// frequency you are ticking the nes at divided by 3 (the cpu runs
    /// 3 times slower than the nes clock).
    ///
    /// Default value is: [CPU_CLOCK] (which is just MASTER_CLOCK / 3)
    #[default(CPU_CLOCK)]
    pub cpu_clock_frequency: u64,
    /// Set this if you are using a custom sample rate. By default it
    /// is set to: [APU_SAMPLE_RATE]
    #[default(APU_SAMPLE_RATE)]
    pub apu_sample_rate: u64,

    #[default(PulseChannel::new(PulseChannelType::Pulse1))]
    pulse1: PulseChannel,
    #[default(PulseChannel::new(PulseChannelType::Pulse2))]
    pulse2: PulseChannel,
    triangle: TriangleChannel,

    sequencer_mode_flag: bool,
    interrupt_inhibit_flag: bool,
    frame_interrupt_flag: bool,

    cpu_total_cycles: usize,
    apu_total_cycles: usize,
    new_mode_flag: bool,
    new_mode_flag_cycle: usize,
    sampled_sound_total: f32,
    collected_samples: u32,
    sample_timer: f32,
    #[default(VecDeque::with_capacity(SAMPLE_QUEUE_SIZE))]
    sample_queue: VecDeque<f32>,
}

impl Apu {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn connect_cpu(&mut self, _cpu: Rc<RefCell<Cpu>>) {}

    pub fn queue_len(&self) -> usize {
        self.sample_queue.len()
    }

    pub fn read_register(&mut self, address: u16, peek: bool) -> u8 {
        if address != 0x4015 {
            return 0xFF;
        }
        if peek {
            return 0xFF;
        }
        let mut value = 0;
        value.set_flag_enabled(
            status_register::ENABLE_PULSE1,
            self.pulse1.is_length_counter_non_zero(),
        );
        value.set_flag_enabled(
            status_register::ENABLE_PULSE2,
            self.pulse2.is_length_counter_non_zero(),
        );
        value.set_flag_enabled(status_register::FRAME_INTERRUPT, self.frame_interrupt_flag);
        self.frame_interrupt_flag = false;
        self.sync_irq_line();
        value
    }

    pub fn write_register(&mut self, address: u16, value: u8) {
        match address {
            0x4000..0x4004 => self.pulse1.write_register(address, value),
            0x4004..0x4008 => self.pulse2.write_register(address, value),
            0x4008..0x400C => self.triangle.write_register(address, value),
            0x4015 => {
                self.pulse1
                    .set_enabled(value.get_flag_enabled(status_register::ENABLE_PULSE1));
                self.pulse2
                    .set_enabled(value.get_flag_enabled(status_register::ENABLE_PULSE2));
                self.triangle
                    .set_enabled(value.get_flag_enabled(status_register::ENABLE_TRIANGLE));
            }
            0x4017 => {
                self.interrupt_inhibit_flag =
                    value.get_flag_enabled(frame_counter_register::INTERRUPT_INHIBIT);
                if self.interrupt_inhibit_flag {
                    self.frame_interrupt_flag = false;
                    self.sync_irq_line();
                }
                self.new_mode_flag = value.get_flag_enabled(frame_counter_register::SEQUENCER_MODE);
                let offset = if self.cpu_total_cycles % 2 == 0 { 3 } else { 4 };
                self.new_mode_flag_cycle = self.cpu_total_cycles + offset;
            }
            _ => (),
        }
    }

    // TODO: fix this later
    fn sync_irq_line(&mut self) {}

    /// https://www.nesdev.org/wiki/APU_Mixer
    fn mix(&mut self) -> f32 {
        let pulse1 = self.pulse1.next().unwrap();
        let pulse2 = self.pulse2.next().unwrap();

        let pulse_out = if pulse1 + pulse2 == 0 {
            0.0
        } else {
            95.88 / ((8128.0 / (pulse1 as f32 + pulse2 as f32)) + 100.0)
        };

        let triangle = self.triangle.next().unwrap();
        let noise: u8 = 0;
        let dmc: u8 = 0;

        let tnd_out = if triangle + noise + dmc == 0 {
            0.0
        } else {
            159.79
                / (1.0
                    / ((triangle as f32 / 8227.0)
                        + (noise as f32 / 12241.0)
                        + (dmc as f32 / 22638.0))
                    + 100.0)
        };

        pulse_out + tnd_out
    }

    pub fn tick(&mut self) {
        let is_apu_cycle = self.cpu_total_cycles % 2 == 0;
        let mut immediate_frame_clock = false;

        if self.cpu_total_cycles == self.new_mode_flag_cycle {
            self.sequencer_mode_flag = self.new_mode_flag;
            self.apu_total_cycles = 0;
            immediate_frame_clock = self.sequencer_mode_flag;
        }

        if is_apu_cycle {
            self.apu_total_cycles += 1;
        }

        if !self.sequencer_mode_flag {
            if self.apu_total_cycles >= 14915 {
                self.apu_total_cycles = 0;
            }
        } else {
            if self.apu_total_cycles >= 18641 {
                self.apu_total_cycles = 0;
            }
        }

        #[rustfmt::skip]
        let mut apu_tick = if is_apu_cycle {
            ApuTick::default()
        } else if !self.sequencer_mode_flag {
            match self.apu_total_cycles {
                3728  => ApuTick { is_quarter_frame: true, is_half_frame: false, ..ApuTick::default() },
                7456  => ApuTick { is_quarter_frame: true, is_half_frame: true,  ..ApuTick::default() },
                11185 => ApuTick { is_quarter_frame: true, is_half_frame: false, ..ApuTick::default() },
                14914 => ApuTick { is_quarter_frame: true, is_half_frame: true,  ..ApuTick::default() },
                _     => ApuTick::default(),
            }
        } else {
            match self.apu_total_cycles {
                3728  => ApuTick { is_quarter_frame: true, is_half_frame: false, ..ApuTick::default() },
                7456  => ApuTick { is_quarter_frame: true, is_half_frame: true,  ..ApuTick::default() },
                11185 => ApuTick { is_quarter_frame: true, is_half_frame: false, ..ApuTick::default() },
                18640 => ApuTick { is_quarter_frame: true, is_half_frame: true,  ..ApuTick::default() },
                _     => ApuTick::default(),
            }
        };

        apu_tick.is_apu_cycle = is_apu_cycle;
        if immediate_frame_clock {
            apu_tick.is_quarter_frame = true;
            apu_tick.is_half_frame = true;
        }

        if !self.sequencer_mode_flag
            && !self.interrupt_inhibit_flag
            && (self.apu_total_cycles == 14914 || (self.apu_total_cycles == 0 && is_apu_cycle))
        {
            self.frame_interrupt_flag = true;
        }
        self.sync_irq_line();

        self.pulse1.tick(apu_tick);
        self.pulse2.tick(apu_tick);
        self.triangle.tick(apu_tick);

        self.sampled_sound_total += self.mix();
        self.collected_samples += 1;
        self.sample_timer += 1.0;

        let cycles_per_sample = self.cpu_clock_frequency as f32 / self.apu_sample_rate as f32;

        if self.sample_timer >= cycles_per_sample {
            self.sample_timer -= cycles_per_sample;

            let out = self.sampled_sound_total / self.collected_samples as f32;

            if self.sample_queue.len() >= SAMPLE_QUEUE_SIZE {
                self.sample_queue.pop_front();
            }
            self.sample_queue.push_back(out);

            self.sampled_sound_total = 0.0;
            self.collected_samples = 0;
        }

        self.cpu_total_cycles += 1;
    }
}

impl Iterator for Apu {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.sample_queue.pop_front()
    }
}
