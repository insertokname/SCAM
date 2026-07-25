use scamm::cartridge::Cartridge;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
};

use crate::hardware::{
    apu::Apu,
    cpu::{Cpu, DmaState},
    cpu_bus::CpuBus,
    ppu::Ppu,
};
use scamm::ResetKind;

pub struct Nes {
    total_cycles: u64,
    pub bus: CpuBus,
    pub cpu: Rc<RefCell<Cpu>>,
    pub ppu: Rc<RefCell<Ppu>>,
    pub apu: Arc<Mutex<Apu>>,
    cartridge: Option<Rc<RefCell<Cartridge>>>,
}

impl Nes {
    pub fn new() -> Self {
        let mut bus = CpuBus::new();
        let cpu = Rc::new(RefCell::new(Cpu::new()));
        let ppu = Rc::new(RefCell::new(Ppu::new()));
        let apu = Arc::new(Mutex::new(Apu::new()));
        bus.connect_ppu(ppu.clone());
        bus.connect_apu(apu.clone());
        apu.lock().unwrap().connect_cpu(cpu.clone());
        ppu.borrow_mut().connect_cpu(cpu.clone());
        Self {
            total_cycles: 0,
            bus,
            cpu,
            ppu,
            apu,
            cartridge: None,
        }
    }

    pub fn new_with_cartridge(cartridge: Cartridge) -> Self {
        let cartridge_rc = Rc::new(RefCell::new(cartridge));
        let mut out = Self {
            total_cycles: 0,
            bus: CpuBus::new(),
            cpu: Rc::new(RefCell::new(Cpu::new())),
            ppu: Rc::new(RefCell::new(Ppu::new())),
            apu: Arc::new(Mutex::new(Apu::new())),
            cartridge: Some(cartridge_rc.clone()),
        };
        out.bus.insert_cartridge(cartridge_rc.clone());
        out.bus.connect_ppu(out.ppu.clone());
        out.bus.connect_apu(out.apu.clone());
        out.apu.lock().unwrap().connect_cpu(out.cpu.clone());
        out.ppu.borrow_mut().insert_cartridge(cartridge_rc);
        out.ppu.borrow_mut().connect_cpu(out.cpu.clone());
        out
    }

    pub fn insert_cartridge(&mut self, cartridge: Cartridge) {
        let cartridge = Rc::new(RefCell::new(cartridge));
        self.bus.insert_cartridge(cartridge.clone());
        self.ppu.borrow_mut().insert_cartridge(cartridge.clone());
        self.cartridge = Some(cartridge);
    }

    pub fn is_resetting(&self) -> bool {
        self.cpu.borrow().is_resetting()
    }

    pub fn reset(&mut self) {
        if let Some(cartridge) = &self.cartridge {
            cartridge.borrow_mut().reset_mapper(ResetKind::ResetButton);
        }
        self.cpu.borrow_mut().reset(&self.bus);
    }

    pub fn reset_with_program_counter(&mut self, program_counter: u16) {
        if let Some(cartridge) = &self.cartridge {
            cartridge.borrow_mut().reset_mapper(ResetKind::ResetButton);
        }
        self.cpu
            .borrow_mut()
            .reset_with_program_counter(program_counter);
    }

    /// ticks 4 times faster than the real nes would
    /// This means it should be clocked at a frequency of: [MASTER_CLOCK](crate::hardware::constants::clock_rates::MASTER_CLOCK)
    pub fn tick(&mut self) -> Option<(u32, u32, u8, u8)> {
        let out = self.ppu.borrow_mut().tick();
        if self.total_cycles % 3 == 0 {
            let (mapper_irq, expansion_audio) = self
                .cartridge
                .as_ref()
                .map(|cartridge| {
                    let mut cartridge = cartridge.borrow_mut();
                    cartridge.clock_cpu();
                    (cartridge.irq_pending(), cartridge.expansion_audio_sample())
                })
                .unwrap_or((false, 0.0));
            let apu_irq = {
                let mut apu = self.apu.lock().unwrap();
                apu.set_expansion_audio_sample(expansion_audio);
                apu.tick();
                apu.irq_pending()
            };
            self.cpu.borrow_mut().is_triggered_irq = apu_irq || mapper_irq;
            let mut dma_status = self.cpu.borrow().dma_status.clone();
            match &mut dma_status {
                DmaState::None => self.cpu.borrow_mut().tick(&mut self.bus),
                DmaState::Initializing { page } => {
                    if self.total_cycles % 2 == 1 {
                        self.cpu.borrow_mut().dma_status = DmaState::Transfering {
                            page: *page,
                            index: 0,
                            fetched_value: 0,
                        };
                    }
                }
                DmaState::Transfering {
                    page,
                    index,
                    fetched_value,
                } => {
                    if self.total_cycles % 2 == 0 {
                        *fetched_value = self.bus.read(*index as u16 + *page as u16 * 0x100);
                        self.cpu.borrow_mut().dma_status = dma_status;
                    } else {
                        self.ppu.borrow_mut().oam[*index as usize] = *fetched_value;

                        if *index == 0xFF {
                            self.cpu.borrow_mut().dma_status = DmaState::None;
                        } else {
                            *index += 1;
                            self.cpu.borrow_mut().dma_status = dma_status;
                        }
                    }
                }
            }
        }

        // if self.total_cycles % 4 == 0 {
        //     self.ppu.borrow_mut().tick();
        // }
        // if self.total_cycles % 12 == 0 {
        //     self.cpu.borrow_mut().tick(&mut self.bus);
        // }
        self.total_cycles += 1;
        out
    }

    pub fn write_memory(&mut self, start: u16, memory: &[u8]) {
        for i in 0..memory.len() {
            self.bus.write(start + i as u16, memory[i]);
        }
    }
}
