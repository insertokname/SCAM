use std::{cell::RefCell, rc::Rc};

use crate::hardware::{
    bit_ops::BitOps,
    cartrige::{Cartrige, cartrige_access::CartrigeAccess},
    constants::{
        self,
        ppu::{
            NAMETABLE_SIZE,
            control_flags::{self, SPRITE_SIZE},
            mask_flags::{self, SHOW_LEFTMOST_BACKGROUND, SHOW_LEFTMOST_SPRITE},
            sprite_attributes, sprite_tile_id,
            status_flags::{self, SPRITE_0_HIT, SPRITE_OVERFLOW},
            vram_sections::*,
        },
    },
    cpu::{Cpu, DmaState},
    ppu::pallet_memory::PalletMemory,
};

pub mod pallet_memory;

pub type BackgroundSprite = [[u8; 8]; 8];
pub type PatternTable = [[BackgroundSprite; 16]; 32];

/// https://www.nesdev.org/wiki/PPU_OAM#OAM_(Sprite)_Data
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sprite {
    y: u8,
    tile_id: u8,
    attributes: u8,
    x: u8,
}

#[derive(Debug, Clone, Default)]
pub enum SpriteRenderingState {
    #[default]
    Idle,
    Initializing,
    Evaluating {
        eval_state: SpriteEvaluation,
        temp_oam_address: u8,
    },
    Fetching {
        temp_oam_address: u8,
        temp_sprite: Sprite,
        temp_fetch_addr: u16,
    },
}

#[derive(Debug, Clone)]
pub enum SpriteEvaluation {
    Read,
    Write {
        fetched_byte: u8,
    },
    TransferRead {
        transfer_byte_count: u8,
    },
    TransferWrite {
        fetched_byte: u8,
        transfer_byte_count: u8,
    },
    OverflowRead,
    OverflowWrite {
        fetched_byte: u8,
    },
    OverflowTransferRead {
        transfer_byte_count: u8,
    },
    OverflowTransferWrite {
        fetched_byte: u8,
        transfer_byte_count: u8,
    },
    WaitingHBlankRead,
    WaitingHBlankWrite {
        fetched_byte: u8,
    },
}

pub struct Ppu {
    cpu: Option<Rc<RefCell<Cpu>>>,
    cartrige: Option<Rc<RefCell<Cartrige>>>,
    scanline: u32,
    dot: u32,
    pub pallet_memory: PalletMemory,
    nametable_memory: [u8; NAMETABLE_SIZE * 4],
    open_bus: u8,
    vram_address: u16,
    temp_vram_address: u16,
    fine_x: u8,
    is_writing_low_byte: bool,
    /// more info about this: https://www.nesdev.org/wiki/PPU_registers#The_PPUDATA_read_buffer
    ppu_data_read_buffer: u8,
    pub control_register: u8,
    mask_register: u8,
    status_register: u8,
    oam_address_register: u8,
    pub oam: [u8; 256],
    temp_oam: [u8; 32],
    // -temp_oam_address: u8,
    renderer_sprite_id: u8,
    renderer_attribute_lsb: u8,
    renderer_attribute_msb: u8,
    renderer_pattern_msb: u8,
    renderer_pattern_lsb: u8,
    renderer_shift_pattern_msb: u16,
    renderer_shift_pattern_lsb: u16,
    renderer_shift_attribute_lsb: u16,
    renderer_shift_attribute_msb: u16,
    renderer_sprite_state: SpriteRenderingState,
    // renderer_oam_latch: u8,
    // renderer_sprite_eval_state: SpriteEvaluation,
    // renderer_temp_sprite: Sprite,
    // renderer_sprite_fetch_addr: u16,
    renderer_sprite_shift_lsb: [u8; 8],
    renderer_sprite_shift_msb: [u8; 8],
    renderer_sprite_x_counter: [u8; 8],
    renderer_sprite_attributes: [u8; 8],
    renderer_sprite_orig_indexes: [u8; 8],
    is_odd_frame: bool,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            cpu: None,
            cartrige: None,
            scanline: 0,
            dot: 0,
            pallet_memory: PalletMemory::default(),
            nametable_memory: [0; NAMETABLE_SIZE * 4],
            open_bus: 0,
            vram_address: 0,
            temp_vram_address: 0,
            fine_x: 0,
            is_writing_low_byte: false,
            ppu_data_read_buffer: 0,
            control_register: 0,
            mask_register: 0,
            status_register: 0,
            oam_address_register: 0,
            oam: [0; 256],
            temp_oam: [0; 32],
            // temp_oam_address: 0,
            renderer_sprite_id: 0,
            renderer_attribute_lsb: 0,
            renderer_attribute_msb: 0,
            renderer_pattern_msb: 0,
            renderer_pattern_lsb: 0,
            renderer_shift_pattern_msb: 0,
            renderer_shift_pattern_lsb: 0,
            renderer_shift_attribute_lsb: 0,
            renderer_shift_attribute_msb: 0,
            // renderer_oam_latch: 0,
            renderer_sprite_state: SpriteRenderingState::default(),
            // renderer_sprite_eval_state: SpriteEvaluation::default(),
            // renderer_temp_sprite: Sprite::default(),
            // renderer_sprite_fetch_addr: 0,
            renderer_sprite_shift_lsb: [0; 8],
            renderer_sprite_shift_msb: [0; 8],
            renderer_sprite_x_counter: [0; 8],
            renderer_sprite_attributes: [0; 8],
            renderer_sprite_orig_indexes: [0; 8],
            is_odd_frame: false,
        }
    }

    pub fn insert_cartrige(&mut self, cartrige: Rc<RefCell<Cartrige>>) {
        self.cartrige = Some(cartrige);
    }

    pub fn connect_cpu(&mut self, cpu: Rc<RefCell<Cpu>>) {
        self.cpu = Some(cpu);
    }

    pub fn read_register(&mut self, address: u16) -> u8 {
        self.read_register_inner(address, false)
    }

    pub fn peek_register(&mut self, address: u16) -> u8 {
        self.read_register_inner(address, true)
    }

    pub(crate) fn read_register_inner(&mut self, address: u16, peek: bool) -> u8 {
        let out = match address % 0x8 {
            0x2 => {
                if !peek {
                    self.is_writing_low_byte = false;
                }
                let out = self.status_register.get_bitmasked(!status_flags::OPEN_BUS)
                    + (self.open_bus & status_flags::OPEN_BUS);
                if !peek {
                    self.status_register
                        .set_flag_enabled(status_flags::VBLANK, false);
                }
                out
            }
            0x4 => {
                // accoring to https://www.nesdev.org/wiki/PPU_sprite_evaluation
                // in the first stage of sprite evaluation, oam addr always returns 0xFF
                if matches!(self.scanline, (0..=239)) && matches!(self.dot, (1..=64)) {
                    0xFF
                } else {
                    self.oam[self.oam_address_register as usize]
                }
            }
            0x7 => {
                // TODO: pallete memory is handled differently: https://www.nesdev.org/wiki/PPU_registers#Reading_palette_RAM
                // if address < 0x3F00 {
                let out = self.ppu_data_read_buffer;
                if !peek {
                    self.ppu_data_read_buffer = self.read_ppu_bus(self.vram_address);
                    self.increment_vram_address();
                }
                out
                // } else {
                // }
            }
            _ => self.open_bus, // TODO: impl rest of registers
        };
        if !peek {
            self.open_bus = out;
        }
        out
    }

    pub fn write_register(&mut self, address: u16, value: u8) {
        self.open_bus = value;

        if address == 0x4014 {
            if let Some(cpu) = self.cpu.as_ref() {
                // TODO: fix this stupid bullshit
                unsafe {
                    (*cpu.as_ptr()).dma_status = DmaState::Initializing { page: value };
                }
            }
        }

        match address % 0x8 {
            // TODO: IMPL PROPERLY
            0x0 => {
                self.control_register = value;
                let base_nametable_address = self
                    .control_register
                    .get_bitfield(control_flags::BASE_NAMETABLE_ADDRESS);
                self.temp_vram_address
                    .set_bitfield(BASE_NAMETABLE_ADDRESS, base_nametable_address as u16);
            }
            0x1 => {
                self.mask_register = value;
            }
            0x3 => {
                self.oam_address_register = value;
            }
            0x4 => {
                self.oam[self.oam_address_register as usize] = value;
                self.oam_address_register += 1;
            }
            0x5 => {
                if !self.is_writing_low_byte {
                    self.fine_x = value & 0b111;
                    self.temp_vram_address
                        .set_bitfield(COARSE_X, value as u16 >> 3);
                    self.is_writing_low_byte = true;
                } else {
                    self.temp_vram_address
                        .set_bitfield(FINE_Y, value as u16 & 0b111);
                    self.temp_vram_address
                        .set_bitfield(COARSE_Y, value as u16 >> 3);
                    self.is_writing_low_byte = false;
                }
            }
            0x6 => {
                if !self.is_writing_low_byte {
                    self.temp_vram_address =
                        ((value as u16) << 8) + (self.temp_vram_address.get_bitmasked(0xFF));
                    self.is_writing_low_byte = true;
                } else {
                    self.temp_vram_address =
                        (self.temp_vram_address.get_bitmasked(0xFF00)) + (value as u16);
                    self.is_writing_low_byte = false;
                    self.vram_address = self.temp_vram_address;
                }
            }
            0x7 => {
                self.write(self.vram_address, value);

                self.increment_vram_address();
            }
            _ => (), // TODO: impl rest of register writes
        };
    }

    pub fn read_ppu_bus(&self, address: u16) -> u8 {
        let result = match address {
            0x0..0x2000 => self
                .cartrige
                .as_ref()
                .map(|c| c.borrow_mut().read(CartrigeAccess::PpuAccess { address }))
                .flatten()
                .unwrap_or(0x0),
            0x2000..0x3F00 => {
                self.nametable_memory[self.map_nametable_address(address) as usize - 0x2000]
            }
            0x3F00..0x4000 => self.pallet_memory.read_address(address),
            _ => 0,
        };
        return result;
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0..0x2000 => {
                _ = self.cartrige.as_ref().map(|c| {
                    c.borrow_mut()
                        .write(CartrigeAccess::PpuAccess { address }, value)
                })
            }
            0x2000..0x3F00 => {
                self.nametable_memory[self.map_nametable_address(address) as usize - 0x2000] = value
            }
            0x3F00..0x4000 => {
                self.pallet_memory.write_address(address, value);
            }
            _ => (),
        };
    }

    pub fn tick(&mut self) -> Option<(u32, u32, u8, u8)> {
        let enabled_background_rendering = self
            .mask_register
            .get_flag_enabled(mask_flags::ENABLE_BG_RENDERING);
        let enabled_sprite_rendering = {
            self.mask_register
                .get_flag_enabled(mask_flags::ENABLE_SPRITE_RENDERING)
        };
        let enabled_rendering = enabled_background_rendering || enabled_sprite_rendering;

        let scanline_background_visible = matches!(self.scanline, (0..=239) | 261);
        let dot_background_fetch = matches!(self.dot, (2..=256) | (321..=336));

        // implementation of this: https://www.nesdev.org/w/images/default/4/4f/Ppu.svg
        if enabled_rendering {
            // bg rendering section
            if scanline_background_visible && dot_background_fetch {
                self.renderer_shift_attribute_lsb <<= 1;
                self.renderer_shift_attribute_msb <<= 1;
                self.renderer_shift_pattern_lsb <<= 1;
                self.renderer_shift_pattern_msb <<= 1;

                match (self.dot - 1) % 8 + 1 {
                    // load shifters + last tick of NT
                    2 => {
                        self.renderer_sprite_id = self.read_ppu_bus(
                            0x2000 | (self.vram_address.get_bitfield(NAMETABLE_OFFSET)),
                        )
                    }
                    // last tick of AT
                    4 => {
                        let mut attributes = self.read_ppu_bus(
                            0x23C0
                                | self.vram_address.get_bitmasked(BASE_NAMETABLE_ADDRESS)
                                | (self.vram_address.get_bitfield(COARSE_X) >> 2)
                                | (self.vram_address.get_bitfield(COARSE_Y) >> 2 << 3),
                        );

                        if (self.vram_address.get_bitfield(COARSE_Y) & 2) != 0 {
                            attributes >>= 4;
                        }
                        if (self.vram_address.get_bitfield(COARSE_X) & 2) != 0 {
                            attributes >>= 2;
                        }
                        self.renderer_attribute_msb = (attributes >> 1) & 1;
                        self.renderer_attribute_lsb = attributes & 1;
                    }
                    // last tick of BG LSBIT
                    6 => {
                        // info on pattern tables: https://www.nesdev.org/wiki/PPU_pattern_tables
                        self.renderer_pattern_lsb = self.read_ppu_bus(
                            self.get_background_pattern_address()
                                + self.renderer_sprite_id as u16 * 16
                                + self.vram_address.get_bitfield(FINE_Y),
                        )
                    }
                    // last tick of BG MSBIT + increment horizontaly/vertically
                    8 => {
                        self.renderer_pattern_msb = self.read_ppu_bus(
                            self.get_background_pattern_address()
                                + self.renderer_sprite_id as u16 * 16
                                + self.vram_address.get_bitfield(FINE_Y)
                                + 8,
                        );

                        // read more about incrementation: https://www.nesdev.org/wiki/PPU_scrolling#Wrapping_around
                        let mut coarse_x = self.vram_address.get_bitfield(COARSE_X);
                        if coarse_x == 31 {
                            self.vram_address ^= BASE_NAMETABLE_ADDRESS_X;
                            coarse_x = 0;
                        } else {
                            coarse_x += 1;
                        }
                        self.vram_address.set_bitfield(COARSE_X, coarse_x);

                        if self.dot == 256 {
                            let mut fine_y = self.vram_address.get_bitfield(FINE_Y);
                            if fine_y < 7 {
                                fine_y += 1;
                            } else {
                                fine_y = 0;
                                let mut coarse_y = self.vram_address.get_bitfield(COARSE_Y);
                                if coarse_y == 29 {
                                    coarse_y = 0;
                                    self.vram_address ^= BASE_NAMETABLE_ADDRESS_Y;
                                } else if coarse_y == 31 {
                                    coarse_y = 0;
                                } else {
                                    coarse_y += 1;
                                }
                                self.vram_address.set_bitfield(COARSE_Y, coarse_y);
                            }
                            self.vram_address.set_bitfield(FINE_Y, fine_y);
                        }

                        self.renderer_shift_pattern_msb = (self.renderer_shift_pattern_msb
                            & 0xFF00)
                            | self.renderer_pattern_msb as u16;
                        self.renderer_shift_pattern_lsb = (self.renderer_shift_pattern_lsb
                            & 0xFF00)
                            | self.renderer_pattern_lsb as u16;

                        self.renderer_shift_attribute_msb = (self.renderer_shift_attribute_msb
                            & 0xFF00)
                            | self.renderer_attribute_msb as u16 * 0xFF;
                        self.renderer_shift_attribute_lsb = (self.renderer_shift_attribute_lsb
                            & 0xFF00)
                            | self.renderer_attribute_lsb as u16 * 0xFF;
                    }
                    _ => (),
                }
            }

            if scanline_background_visible && self.dot == 257 {
                self.vram_address.set_bitmasked(
                    COARSE_X | BASE_NAMETABLE_ADDRESS_X,
                    self.temp_vram_address
                        .get_bitmasked(COARSE_X | BASE_NAMETABLE_ADDRESS_X),
                );
            }

            // implementation of this: https://www.nesdev.org/wiki/PPU_sprite_evaluation
            match self.scanline {
                0..=239 => {
                    if matches!(self.dot, (1..=256)) {
                        for i in 0..8 {
                            if self.renderer_sprite_x_counter[i] > 0 {
                                self.renderer_sprite_x_counter[i] -= 1;
                            } else {
                                self.renderer_sprite_shift_lsb[i] <<= 1;
                                self.renderer_sprite_shift_msb[i] <<= 1;
                            }
                        }
                    };

                    // update the sprite rendering state if required
                    if !matches!(
                        (&self.renderer_sprite_state, self.dot),
                        (SpriteRenderingState::Initializing, 1..=64)
                            | (SpriteRenderingState::Evaluating { .. }, 65..=256)
                            | (SpriteRenderingState::Fetching { .. }, 257..=320)
                    ) {
                        self.renderer_sprite_state = match self.dot {
                            1..=64 => SpriteRenderingState::Initializing,
                            65..=256 => SpriteRenderingState::Evaluating {
                                eval_state: SpriteEvaluation::Read,
                                temp_oam_address: 0,
                            },
                            257..=320 => SpriteRenderingState::Fetching {
                                temp_oam_address: 0,
                                temp_sprite: Sprite::default(),
                                temp_fetch_addr: 0,
                            },
                            _ => SpriteRenderingState::Idle,
                        };
                    }

                    let mut state = self.renderer_sprite_state.clone();
                    match &mut state {
                        SpriteRenderingState::Initializing => {
                            if (self.dot - 1) % 2 == 1 {
                                self.temp_oam[((self.dot - 1) / 2) as usize] = 0xFF;
                            }
                        }
                        SpriteRenderingState::Evaluating {
                            eval_state,
                            temp_oam_address,
                        } => {
                            *eval_state = match *eval_state {
                                SpriteEvaluation::Read => {
                                    let fetched_byte = self.oam[self.oam_address_register as usize];
                                    SpriteEvaluation::Write { fetched_byte }
                                }
                                SpriteEvaluation::Write { fetched_byte } => {
                                    self.temp_oam[*temp_oam_address as usize] = fetched_byte;

                                    let sprite_height =
                                        if self.control_register.get_flag_enabled(SPRITE_SIZE) {
                                            16
                                        } else {
                                            8
                                        };

                                    if (self.scanline & 0xFF) as u8 - fetched_byte < sprite_height {
                                        self.renderer_sprite_orig_indexes
                                            [(*temp_oam_address / 4) as usize] =
                                            self.oam_address_register / 4;
                                        *temp_oam_address += 1;
                                        self.oam_address_register += 1;
                                        // 1a: copy leftover sprite data
                                        SpriteEvaluation::TransferRead {
                                            transfer_byte_count: 3,
                                        }
                                    } else {
                                        let old = self.oam_address_register;
                                        self.oam_address_register += 4;
                                        // 2a: overflowed, all sprites evaluated
                                        if old > self.oam_address_register {
                                            SpriteEvaluation::WaitingHBlankRead
                                        // 2b: more sprites to be evaluated
                                        } else {
                                            SpriteEvaluation::Read
                                        }
                                    }
                                }
                                SpriteEvaluation::TransferRead {
                                    transfer_byte_count,
                                } => {
                                    let fetched_byte = self.oam[self.oam_address_register as usize];
                                    SpriteEvaluation::TransferWrite {
                                        fetched_byte,
                                        transfer_byte_count,
                                    }
                                }
                                SpriteEvaluation::TransferWrite {
                                    fetched_byte,
                                    transfer_byte_count,
                                } => {
                                    self.temp_oam[*temp_oam_address as usize] = fetched_byte;

                                    *temp_oam_address += 1;
                                    self.oam_address_register += 1;

                                    // copy leftover bytes to secondary oam
                                    if transfer_byte_count - 1 > 0 {
                                        SpriteEvaluation::TransferRead {
                                            transfer_byte_count: transfer_byte_count - 1,
                                        }
                                    }
                                    // 2a: all sprites evaluated, wait for hblank
                                    else if self.oam_address_register == 0 {
                                        SpriteEvaluation::WaitingHBlankRead
                                    }
                                    // 2c: secondary oam full, check overflow
                                    else if *temp_oam_address == 32 {
                                        SpriteEvaluation::OverflowRead {}
                                    }
                                    // 2b: more sprites to be evaluated, go to 1
                                    else {
                                        SpriteEvaluation::Read
                                    }
                                }
                                SpriteEvaluation::OverflowRead {} => {
                                    let fetched_byte = self.oam[self.oam_address_register as usize];
                                    SpriteEvaluation::OverflowWrite { fetched_byte }
                                }
                                SpriteEvaluation::OverflowWrite { fetched_byte } => {
                                    let sprite_height =
                                        if self.control_register.get_flag_enabled(SPRITE_SIZE) {
                                            16
                                        } else {
                                            8
                                        };

                                    if (self.scanline & 0xFF) as u8 - fetched_byte < sprite_height {
                                        self.status_register
                                            .set_flag_enabled(SPRITE_OVERFLOW, true);
                                        self.oam_address_register += 1;

                                        if self.oam_address_register == 0 {
                                            SpriteEvaluation::WaitingHBlankRead
                                        } else {
                                            SpriteEvaluation::OverflowTransferRead {
                                                transfer_byte_count: 3,
                                            }
                                        }
                                    } else {
                                        let prev = self.oam_address_register;
                                        // increment n and m separatley
                                        self.oam_address_register =
                                            ((self.oam_address_register + 4) & 0xFC)
                                                | ((self.oam_address_register + 1) & 0x03);

                                        // 3b: n overflowed, going to 4
                                        if prev > self.oam_address_register {
                                            SpriteEvaluation::WaitingHBlankRead
                                        // 3b: going back to 3 since n didn't overflow
                                        } else {
                                            SpriteEvaluation::OverflowRead
                                        }
                                    }
                                }
                                SpriteEvaluation::OverflowTransferRead {
                                    transfer_byte_count,
                                } => {
                                    let fetched_byte = self.oam[self.oam_address_register as usize];
                                    SpriteEvaluation::OverflowTransferWrite {
                                        fetched_byte,
                                        transfer_byte_count,
                                    }
                                }
                                SpriteEvaluation::OverflowTransferWrite {
                                    transfer_byte_count,
                                    ..
                                } => {
                                    self.oam_address_register += 1;
                                    if self.oam_address_register == 0
                                        || transfer_byte_count - 1 == 0
                                    {
                                        SpriteEvaluation::WaitingHBlankRead
                                    } else {
                                        SpriteEvaluation::OverflowTransferRead {
                                            transfer_byte_count: transfer_byte_count - 1,
                                        }
                                    }
                                }
                                SpriteEvaluation::WaitingHBlankRead => {
                                    let fetched_byte = self.oam[self.oam_address_register as usize];
                                    SpriteEvaluation::WaitingHBlankWrite { fetched_byte }
                                }
                                SpriteEvaluation::WaitingHBlankWrite { .. } => {
                                    self.oam_address_register += 4;
                                    SpriteEvaluation::WaitingHBlankRead
                                }
                            };
                        }
                        SpriteRenderingState::Fetching {
                            temp_oam_address,
                            temp_sprite,
                            temp_fetch_addr,
                        } => {
                            self.oam_address_register = 0;

                            let sprite_idx = ((self.dot - 257) / 8) as usize;
                            let tick = (self.dot - 257) % 8;
                            match tick {
                                0 => {
                                    temp_sprite.y = self.temp_oam[*temp_oam_address as usize];
                                    *temp_oam_address += 1;
                                }
                                1 => {
                                    temp_sprite.tile_id = self.temp_oam[*temp_oam_address as usize];
                                    *temp_oam_address += 1;
                                }
                                2 => {
                                    temp_sprite.attributes =
                                        self.temp_oam[*temp_oam_address as usize];
                                    self.renderer_sprite_attributes[sprite_idx] =
                                        temp_sprite.attributes;
                                    *temp_oam_address += 1;
                                }
                                3 => {
                                    temp_sprite.x = self.temp_oam[*temp_oam_address as usize];
                                    self.renderer_sprite_x_counter[sprite_idx] = temp_sprite.x;
                                }
                                4 => {
                                    let tall_sprites = self
                                        .control_register
                                        .get_flag_enabled(control_flags::SPRITE_SIZE);
                                    let height: u8 = if tall_sprites { 16 } else { 8 };
                                    let flipped_vertically = temp_sprite
                                        .attributes
                                        .get_flag_enabled(sprite_attributes::FLIP_VERTICALLY);

                                    let sprite_pattern_table_address = 0x1000
                                        * if tall_sprites {
                                            temp_sprite
                                                .tile_id
                                                .get_flag_enabled(sprite_tile_id::BANK)
                                        } else {
                                            self.control_register.get_flag_enabled(
                                                control_flags::SPRITE_PATTERN_TABLE_ADDR,
                                            )
                                        } as u16;

                                    let mut tile_id = if tall_sprites {
                                        temp_sprite.tile_id.get_bitmasked(sprite_tile_id::TILE_ID)
                                    } else {
                                        temp_sprite.tile_id
                                    } as u16;

                                    let mut row = self.scanline as u16 - temp_sprite.y as u16;

                                    if tall_sprites && row >= 8 {
                                        tile_id += 1;
                                        row -= 8;
                                    }

                                    let row = if flipped_vertically {
                                        (height - 1) as u16 - row
                                    } else {
                                        row
                                    };

                                    *temp_fetch_addr =
                                        sprite_pattern_table_address + tile_id * 16 + row;
                                }
                                5 => {
                                    let mut fetched_byte = self.read_ppu_bus(*temp_fetch_addr);
                                    if temp_sprite
                                        .attributes
                                        .get_flag_enabled(sprite_attributes::FLIP_HORIZONTALLY)
                                    {
                                        fetched_byte = fetched_byte.reverse_bits();
                                    }

                                    let tall_sprites = self
                                        .control_register
                                        .get_flag_enabled(control_flags::SPRITE_SIZE);
                                    let row = self.scanline as u16 - temp_sprite.y as u16;
                                    if !(row < if tall_sprites { 16 } else { 8 }) {
                                        fetched_byte = 0;
                                    }

                                    self.renderer_sprite_shift_lsb[sprite_idx] = fetched_byte;
                                }
                                6 => {
                                    *temp_fetch_addr += 8;
                                }
                                7 => {
                                    let mut fetched_byte = self.read_ppu_bus(*temp_fetch_addr);
                                    if temp_sprite
                                        .attributes
                                        .get_flag_enabled(sprite_attributes::FLIP_HORIZONTALLY)
                                    {
                                        fetched_byte = fetched_byte.reverse_bits();
                                    }

                                    let tall_sprites = self
                                        .control_register
                                        .get_flag_enabled(control_flags::SPRITE_SIZE);
                                    let row = self.scanline as u16 - temp_sprite.y as u16;
                                    if !(row < if tall_sprites { 16 } else { 8 }) {
                                        fetched_byte = 0;
                                    }

                                    self.renderer_sprite_shift_msb[sprite_idx] = fetched_byte;

                                    *temp_sprite = Sprite::default();
                                    *temp_oam_address += 1;
                                }
                                _ => unreachable!(),
                            }
                        }
                        SpriteRenderingState::Idle => {}
                    }
                    self.renderer_sprite_state = state;
                }
                _ => {}
            }
        }

        if self.scanline == 241 && self.dot == 1 {
            if self
                .control_register
                .get_flag_enabled(control_flags::VBLANK_NMI)
                && let Some(cpu) = self.cpu.as_ref()
            {
                cpu.borrow_mut().is_triggered_nmi = true;
            }
            self.status_register
                .set_flag_enabled(status_flags::VBLANK, true);
        }
        if self.scanline == 261 && self.dot == 1 {
            self.status_register
                .set_flag_enabled(status_flags::VBLANK, false);
            self.status_register
                .set_flag_enabled(status_flags::SPRITE_0_HIT, false);
            self.status_register
                .set_flag_enabled(status_flags::SPRITE_OVERFLOW, false);
        }
        if enabled_rendering && self.scanline == 261 && matches!(self.dot, (280..305)) {
            self.vram_address.set_bitmasked(
                COARSE_Y | FINE_Y | BASE_NAMETABLE_ADDRESS_Y,
                self.temp_vram_address
                    .get_bitmasked(COARSE_Y | FINE_Y | BASE_NAMETABLE_ADDRESS_Y),
            );
        }

        let mut out = None;
        let pixel_in_display = matches!(self.dot, (1..=256)) && matches!(self.scanline, (0..=239));
        if pixel_in_display && enabled_background_rendering {
            let fine_x_selector = 1 << (15 - self.fine_x);

            let pattern_lsb = self
                .renderer_shift_pattern_lsb
                .get_flag_enabled(fine_x_selector) as u8;
            let pattern_msb = self
                .renderer_shift_pattern_msb
                .get_flag_enabled(fine_x_selector) as u8;

            let pattern = (pattern_msb << 1) | pattern_lsb;

            let attrib_lsb = self
                .renderer_shift_attribute_lsb
                .get_flag_enabled(fine_x_selector) as u8;
            let attrib_msb = self
                .renderer_shift_attribute_msb
                .get_flag_enabled(fine_x_selector) as u8;

            let attrib = (attrib_msb << 1) | attrib_lsb;

            out = Some((self.dot - 1, self.scanline, pattern, attrib));
        }

        if pixel_in_display && enabled_sprite_rendering {
            let (_, _, bg_pattern, bg_attrib) = out.unwrap_or_else(|| (0, 0, 0, 0));

            let (fg_pattern, fg_attrib, priority, orig_index) = (0..8)
                .find_map(|sprite_idx| {
                    if self.renderer_sprite_x_counter[sprite_idx] != 0 {
                        return None;
                    }

                    let lsb = self.renderer_sprite_shift_lsb[sprite_idx];
                    let msb = self.renderer_sprite_shift_msb[sprite_idx];
                    let orig_index = self.renderer_sprite_orig_indexes[sprite_idx];

                    let pattern_lsb = lsb.get_bitfield(0x80);
                    let pattern_msb = msb.get_bitfield(0x80);
                    let pattern = (pattern_msb << 1) | pattern_lsb;

                    let attributes = self.renderer_sprite_attributes[sprite_idx];
                    let attrib = attributes.get_bitfield(sprite_attributes::PALLETE) + 4;
                    let priority = attributes.get_flag_enabled(sprite_attributes::PRIORITY);

                    if pattern != 0 {
                        Some((pattern, attrib, priority, orig_index))
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            let leftmost_rendering = self
                .status_register
                .get_flag_enabled(SHOW_LEFTMOST_BACKGROUND)
                && self.status_register.get_flag_enabled(SHOW_LEFTMOST_SPRITE);

            if orig_index == 0
                && enabled_rendering
                && bg_pattern != 0
                && fg_pattern != 0
                && self.dot != 255
                && !self.status_register.get_flag_enabled(SPRITE_0_HIT)
                && (leftmost_rendering || !matches!(self.dot, 0..=7))
            {
                self.status_register.set_flag_enabled(SPRITE_0_HIT, true);
            }

            let (pattern, attrib) = if bg_pattern == 0 {
                (fg_pattern, fg_attrib)
            } else if fg_pattern == 0 {
                (bg_pattern, bg_attrib)
            } else {
                if priority {
                    (bg_pattern, bg_attrib)
                } else {
                    (fg_pattern, fg_attrib)
                }
            };

            out = Some((self.dot - 1, self.scanline, pattern, attrib));
        }

        if enabled_rendering && self.scanline == 261 && self.dot == 339 && self.is_odd_frame {
            self.dot = 0;
            self.scanline = 0;
            self.is_odd_frame = !self.is_odd_frame;
        } else {
            self.dot += 1;
            if self.dot > 340 {
                self.scanline += 1;
                if self.scanline > 261 {
                    self.scanline = 0;
                    self.is_odd_frame = !self.is_odd_frame;
                }
                self.dot = 0;
            }
        }

        out
    }

    pub fn get_pixel_color(&self, i: usize, j: usize) -> u32 {
        let i_tile = i / 8;
        let j_tile = j / 8;
        let index = i_tile * 32 + j_tile;
        let sprite = self.nametable_memory[index];
        let pixel_i = i % 8;
        let pixel_j = j % 8;
        let pallet_collor_id = self.get_sprite_pixel_pallet(sprite, pixel_i as u8, pixel_j as u8);

        let attr_index = i_tile / 4 * 8 + j_tile / 4;
        let attr_value = self.nametable_memory[0x3c0 + attr_index as usize];
        let shift = ((i_tile / 2) % 2) * 4 + ((j_tile / 2) % 2) * 2;
        let pallet_index = (attr_value >> shift) & 0b11;
        let color_id = self
            .pallet_memory
            .read_index(pallet_index as u16, pallet_collor_id as u16);
        constants::ppu::COLORS[color_id as usize]
    }

    fn increment_vram_address(&mut self) {
        let inc_amount = if self
            .control_register
            .get_flag_enabled(control_flags::VRAM_INC)
        {
            32
        } else {
            1
        };
        self.vram_address = self.vram_address.wrapping_add(inc_amount);
    }

    fn get_background_pattern_address(&self) -> u16 {
        if self
            .control_register
            .get_flag_enabled(control_flags::BG_PATTERN_TABLE_ADDR)
        {
            0x1000
        } else {
            0x0
        }
    }

    fn get_sprite_pixel_pallet(&self, sprite: u8, pixel_i: u8, pixel_j: u8) -> u8 {
        let mut background_nametable_address = 0;
        if self
            .control_register
            .get_flag_enabled(control_flags::BG_PATTERN_TABLE_ADDR)
        {
            background_nametable_address = 0x1000;
        }
        let first_byte = self
            .read_cartrige(background_nametable_address + (sprite as u16) * 16 + pixel_i as u16);
        let second_byte = self.read_cartrige(
            background_nametable_address + (sprite as u16) * 16 + pixel_i as u16 + 8,
        );

        let lsb = (first_byte >> (7 - pixel_j)) & 1;
        let msb = (second_byte >> (7 - pixel_j)) & 1;
        (msb << 1) + lsb
    }

    fn read_cartrige(&self, address: u16) -> u8 {
        self.cartrige
            .as_ref()
            .map(|c| c.borrow_mut().read(CartrigeAccess::PpuAccess { address }))
            .flatten()
            .unwrap_or(0)
    }

    pub fn process_pattern_table(&self) -> PatternTable {
        let mut out: [[[[u8; 8]; 8]; 16]; 32] = [[[[0; 8]; 8]; 16]; 32];
        for i in 0..32 {
            for j in 0..16 {
                out[i as usize][j as usize] = self.process_sprite(i, j);
            }
        }
        out
    }

    fn process_sprite(&self, sprite_i: u16, sprite_j: u16) -> BackgroundSprite {
        let mut out = [[0; 8]; 8];
        for i in 0..8 {
            let first_byte_address = sprite_i * 256 + sprite_j * 16 + i;
            let second_byte_address = first_byte_address + 8;
            let first_byte = self
                .cartrige
                .as_ref()
                .map(|c| {
                    c.borrow_mut().read(CartrigeAccess::PpuAccess {
                        address: first_byte_address,
                    })
                })
                .flatten()
                .unwrap_or(0);

            let second_byte = self
                .cartrige
                .as_ref()
                .map(|c| {
                    c.borrow_mut().read(CartrigeAccess::PpuAccess {
                        address: second_byte_address,
                    })
                })
                .flatten()
                .unwrap_or(0);

            for j in 0..8 {
                let lsb = (first_byte >> (7 - j)) & 1;
                let msb = (second_byte >> (7 - j)) & 1;

                let palette = (msb << 1) + lsb;
                out[i as usize][j as usize] = palette;
            }
        }
        out
    }

    fn map_nametable_address(&self, address: u16) -> u16 {
        self.cartrige
            .as_ref()
            .map(|c| c.borrow().map_nametable(address))
            .unwrap_or_else(|| address)
    }
}
