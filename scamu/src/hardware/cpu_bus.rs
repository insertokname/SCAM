use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, Mutex},
};

use crate::hardware::{
    apu::Apu,
    bit_ops::BitOps,
    cartrige::{Cartrige, cartrige_access::CartrigeAccess},
    ppu::Ppu,
};

use super::constants;

pub struct CpuBus {
    cpu_ram: [u8; constants::cpu::RAM_SIZE],
    cartrige: Option<Rc<RefCell<Cartrige>>>,
    apu: Option<Arc<Mutex<Apu>>>,
    ppu: Option<Rc<RefCell<Ppu>>>,
    open_bus: Cell<u8>,
    controller_state: [Cell<u8>; 2],
    controller_shift: [Cell<u8>; 2],
    controller_strobe: Cell<bool>,
}

impl CpuBus {
    pub fn new() -> Self {
        Self {
            cpu_ram: [0; constants::cpu::RAM_SIZE],
            cartrige: None,
            apu: None,
            ppu: None,
            open_bus: Cell::new(0),
            controller_state: std::array::from_fn(|_| Cell::new(0)),
            controller_shift: std::array::from_fn(|_| Cell::new(0)),
            controller_strobe: Cell::new(false),
        }
    }

    pub fn insert_cartrige(&mut self, cartrige: Rc<RefCell<Cartrige>>) {
        self.cartrige = Some(cartrige);
    }

    pub fn connect_ppu(&mut self, ppu: Rc<RefCell<Ppu>>) {
        self.ppu = Some(ppu);
    }

    pub fn connect_apu(&mut self, apu: Arc<Mutex<Apu>>) {
        self.apu = Some(apu);
    }

    pub fn read(&self, address: u16) -> u8 {
        self.read_inner(address, false)
    }

    /// Same as [CpuBus::read] but doesn't mutate state (used here for debugging)
    pub fn peek(&self, address: u16) -> u8 {
        self.read_inner(address, true)
    }

    pub(crate) fn read_inner(&self, address: u16, peek: bool) -> u8 {
        let result = match address {
            0x0..0x2000 => self.cpu_ram[address as usize & (constants::cpu::RAM_SIZE - 1)],
            0x2000..0x4000 => self
                .ppu
                .as_ref()
                .map(|c| c.borrow_mut().read_register_inner(address, peek))
                .unwrap_or(0),
            0x4016 => self.read_controller(0, peek),
            0x4017 => self.read_controller(1, peek),
            0x4000..0x4020 => self
                .apu
                .as_ref()
                .map(|a| a.lock().unwrap().read_register(address, peek))
                .unwrap_or(self.open_bus.get()),
            0x4020.. => self
                .cartrige
                .as_ref()
                .map(|c| {
                    c.borrow_mut()
                        .read(CartrigeAccess::CpuAccess { address })
                        .unwrap_or_else(|| self.open_bus.get())
                })
                .unwrap_or(0x0),
        };

        if !peek {
            self.open_bus.set(result);
        }
        return result;
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0..0x2000 => self.cpu_ram[address as usize & (constants::cpu::RAM_SIZE - 1)] = value,
            0x2000..0x4000 | 0x4014 => self
                .ppu
                .as_ref()
                .map(|c| c.borrow_mut().write_register(address, value))
                .unwrap_or(()),
            0x4016 => {
                let strobe = value & 1 != 0;
                let prev_strobe = self.controller_strobe.replace(strobe);

                if strobe || (prev_strobe && !strobe) {
                    self.controller_state
                        .iter()
                        .zip(self.controller_shift.iter())
                        .for_each(|(state, shift)| shift.set(state.get()));
                }
            }
            0x4000..0x4020 => self
                .apu
                .as_ref()
                .map(|a| a.lock().unwrap().write_register(address, value))
                .unwrap_or(()),
            0x4020.. => self
                .cartrige
                .as_ref()
                .map(|c| {
                    c.borrow_mut()
                        .write(CartrigeAccess::CpuAccess { address }, value)
                })
                .unwrap_or(()),
        }
    }

    pub fn read_u16(&self, address: u16) -> u16 {
        let pointer_low = self.read(address) as u16;
        let pointer_high = self.read(address + 1) as u16;
        pointer_high << 8 | pointer_low
    }

    pub fn peek_u16(&self, address: u16) -> u16 {
        let pointer_low = self.peek(address) as u16;
        let pointer_high = self.peek(address + 1) as u16;
        pointer_high << 8 | pointer_low
    }

    pub fn write_u16(&mut self, address: u16, value: u16) {
        let value_low = (value & 0x00FF) as u8;
        let value_high = (value >> 8) as u8;

        self.cpu_ram[address as usize] = value_low;
        self.cpu_ram[(address + 1) as usize] = value_high;
    }

    pub fn write_memory(&mut self, start: u16, memory: &[u8]) {
        for i in 0..memory.len() {
            self.write(start + i as u16, memory[i]);
        }
    }

    pub fn set_controller_button(&mut self, controller_index: usize, button: u8, pressed: bool) {
        if controller_index >= self.controller_state.len() {
            return;
        }

        let mut state = self.controller_state[controller_index].get();
        state.set_flag_enabled(button, pressed);

        self.controller_state[controller_index].set(state);
        if self.controller_strobe.get() {
            self.controller_shift[controller_index].set(state);
        }
    }

    fn read_controller(&self, controller_index: usize, peek: bool) -> u8 {
        if self.controller_strobe.get() {
            return self.controller_state[controller_index].get() & 1;
        }

        let shift = self.controller_shift[controller_index].get();
        let out = shift & 1;

        if !peek {
            self.controller_shift[controller_index].set((shift >> 1) | 0x80);
        }

        out
    }
}
