use funty::Unsigned;

pub trait BitOps: Unsigned {
    fn set_bitfield(&mut self, field: Self, value: Self) {
        *self &= !field;
        *self |= (value << field.trailing_zeros()) & field;
    }

    fn get_bitfield(self, field: Self) -> Self {
        (self & field) >> field.trailing_zeros()
    }

    fn set_bitmasked(&mut self, mask: Self, value: Self) {
        *self &= !mask;
        *self |= value & mask;
    }

    fn get_bitmasked(self, mask: Self) -> Self {
        self & mask
    }

    fn set_flag_enabled(&mut self, flag: Self, enabled: bool) {
        if enabled {
            *self |= flag;
        } else {
            *self &= !flag;
        }
    }

    fn get_flag_enabled(self, flag: Self) -> bool {
        (self & flag) == flag
    }
}

impl<T: Unsigned> BitOps for T {}
