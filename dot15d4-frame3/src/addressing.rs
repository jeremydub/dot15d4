#[derive(Clone, Copy, Debug)]
pub enum Address {
    Absent,
    Short(u16),    // PAN ID
    Extended(u16), // PAN ID
} // 4 bytes

impl Address {
    pub const fn length(&self) -> usize {
        match self {
            Address::Absent => 0,
            Address::Short(_) => 2,
            Address::Extended(_) => 8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PanIdCompression {
    Yes,
    No,
    Legacy,
} // 1 byte

#[derive(Clone, Copy, Debug)]
pub struct Addressing {
    pub(crate) dst: Address,
    pub(crate) src: Address,
    pub(crate) pan: PanIdCompression,
} // 10 bytes

impl Addressing {
    pub const fn new(dst: Address, src: Address, pan: PanIdCompression) -> Self {
        Self { dst, src, pan }
    }

    pub const fn none() -> Self {
        Addressing::new(Address::Absent, Address::Absent, PanIdCompression::No)
    }

    pub const fn new_legacy_addressing(dst: Address, src: Address) -> Self {
        Self {
            dst,
            src,
            pan: PanIdCompression::Legacy,
        }
    }

    const fn addr_len(&self) -> usize {
        self.dst.length() + self.src.length()
    }

    pub const fn length(&self) -> usize {
        self.addr_len()
            + if matches!(self.pan, PanIdCompression::Legacy) {
                // IEEE 802.15.4-2006 or earlier.
                match (self.dst, self.src) {
                    (Address::Short(dst_pan), Address::Short(src_pan))
                    | (Address::Short(dst_pan), Address::Extended(src_pan))
                    | (Address::Extended(dst_pan), Address::Short(src_pan))
                    | (Address::Extended(dst_pan), Address::Extended(src_pan)) => {
                        if dst_pan == src_pan {
                            2
                        } else {
                            4
                        }
                    }

                    (Address::Absent, Address::Extended(_))
                    | (Address::Absent, Address::Short(_))
                    | (Address::Short(_), Address::Absent)
                    | (Address::Extended(_), Address::Absent) => 2,

                    (Address::Absent, Address::Absent) => 0,
                }
            } else {
                // IEEE 802.15.4-2015 and beyond.
                match (self.dst, self.src, self.pan) {
                    (Address::Absent, Address::Absent, PanIdCompression::No)
                    | (Address::Short(_), Address::Absent, PanIdCompression::Yes)
                    | (Address::Extended(_), Address::Absent, PanIdCompression::Yes)
                    | (Address::Absent, Address::Short(_), PanIdCompression::Yes)
                    | (Address::Absent, Address::Extended(_), PanIdCompression::Yes)
                    | (Address::Extended(_), Address::Extended(_), PanIdCompression::Yes) => 0,

                    (Address::Absent, Address::Absent, PanIdCompression::Yes)
                    | (Address::Short(_), Address::Absent, PanIdCompression::No)
                    | (Address::Extended(_), Address::Absent, PanIdCompression::No)
                    | (Address::Absent, Address::Short(_), PanIdCompression::No)
                    | (Address::Absent, Address::Extended(_), PanIdCompression::No)
                    | (Address::Extended(_), Address::Extended(_), PanIdCompression::No) => 2,

                    (Address::Short(dst_pan), Address::Short(src_pan), _)
                    | (Address::Short(dst_pan), Address::Extended(src_pan), _)
                    | (Address::Extended(dst_pan), Address::Short(src_pan), _) => {
                        if dst_pan == src_pan {
                            debug_assert!(matches!(self.pan, PanIdCompression::Yes), "undefined");
                            2
                        } else {
                            debug_assert!(matches!(self.pan, PanIdCompression::No), "undefined");
                            4
                        }
                    }

                    (_, _, PanIdCompression::Legacy) => unreachable!(),
                }
            }
    }

    pub const fn pan_id_compression(&self) -> bool {
        match self.pan {
            PanIdCompression::Yes => {
                return true;
            }
            PanIdCompression::No => {
                return false;
            }
            PanIdCompression::Legacy => match (self.dst, self.src) {
                (Address::Short(dst_pan), Address::Short(src_pan))
                | (Address::Short(dst_pan), Address::Extended(src_pan))
                | (Address::Extended(dst_pan), Address::Short(src_pan))
                | (Address::Extended(dst_pan), Address::Extended(src_pan)) => {
                    if dst_pan == src_pan {
                        true
                    } else {
                        false
                    }
                }

                _ => false,
            },
        }
    }

    pub const fn dst_addr_mode(&self) -> AddressingMode {
        match self.dst {
            Address::Absent => AddressingMode::Absent,
            Address::Short(_) => AddressingMode::Short,
            Address::Extended(_) => AddressingMode::Extended,
        }
    }

    pub const fn src_addr_mode(&self) -> AddressingMode {
        match self.src {
            Address::Absent => AddressingMode::Absent,
            Address::Short(_) => AddressingMode::Short,
            Address::Extended(_) => AddressingMode::Extended,
        }
    }
}

/// IEEE 802.15.4 addressing mode.
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub enum AddressingMode {
    /// The address is absent.
    Absent = 0b00,
    /// The address is a short address.
    Short = 0b10,
    /// The address is an extended address.
    Extended = 0b11,
    /// Unknown addressing mode.
    Unknown,
}

impl From<u8> for AddressingMode {
    fn from(value: u8) -> Self {
        match value {
            0b00 => Self::Absent,
            0b10 => Self::Short,
            0b11 => Self::Extended,
            _ => Self::Unknown,
        }
    }
}
