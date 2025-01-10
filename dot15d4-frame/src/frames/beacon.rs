#![allow(missing_docs)]

use dot15d4_macros::frame;

use crate::{Address, Error, Result};

use crate::{
    AddressingFields, AddressingMode, AuxiliarySecurityHeader, FrameControl, FrameType,
    FrameVersion, InformationElements,
};

/// A reader/writer for an IEEE 802.15.4 Beacon frame.
pub struct BeaconFrame<T: AsRef<[u8]>> {
    buffer: T,
}

impl<T: AsRef<[u8]>> BeaconFrame<T> {
    #[allow(unused)]
    pub fn new(buffer: T) -> Result<Self> {
        todo!();
    }

    #[allow(unused)]
    fn check_len(&self) -> bool {
        todo!();
    }

    pub fn new_unchecked(buffer: T) -> Self {
        Self { buffer }
    }

    /// Return a [`FrameControl`] reader.
    pub fn frame_control(&self) -> FrameControl<&'_ [u8]> {
        FrameControl::new_unchecked(&self.buffer.as_ref()[..2])
    }

    /// Return the sequence number of the frame.
    pub fn sequence_number(&self) -> u8 {
        self.buffer.as_ref()[2]
    }

    /// Return an [`AddressingFields`] reader.
    pub fn addressing(&self) -> AddressingFields<&'_ [u8], &'_ [u8]> {
        AddressingFields::new_unchecked(&self.buffer.as_ref()[3..], self.frame_control())
    }

    pub fn auxiliary_security_header(&self) -> Option<AuxiliarySecurityHeader<&'_ [u8]>> {
        let fc = self.frame_control();

        if fc.security_enabled() {
            let mut offset = 3;
            offset += self.addressing().len();

            Some(AuxiliarySecurityHeader::new(
                &self.buffer.as_ref()[offset..],
            ))
        } else {
            None
        }
    }

    pub fn superframe_specification(&self) -> SuperframeSpecification<&'_ [u8]> {
        let mut offset = 3;
        offset += self.addressing().len();

        if self.frame_control().security_enabled() {
            offset += self.auxiliary_security_header().unwrap().len();
        }

        SuperframeSpecification::new_unchecked(&self.buffer.as_ref()[offset..][..2])
    }

    pub fn gts_info(&self) -> GtsInfo<&'_ [u8]> {
        let mut offset = 3;
        offset += self.addressing().len();

        if self.frame_control().security_enabled() {
            offset += self.auxiliary_security_header().unwrap().len();
        }

        offset += 2; // Superframe specification

        GtsInfo::new_unchecked(&self.buffer.as_ref()[offset..])
    }

    pub fn pending_address(&self) -> PendingAddress<&'_ [u8]> {
        let mut offset = 3;
        offset += self.addressing().len();

        if self.frame_control().security_enabled() {
            offset += self.auxiliary_security_header().unwrap().len();
        }

        offset += 2; // Superframe specification
        offset += self.gts_info().len();

        PendingAddress::new_unchecked(&self.buffer.as_ref()[offset..])
    }
}

impl<'f, T: AsRef<[u8]> + ?Sized> BeaconFrame<&'f T> {
    /// Return the payload of the frame.
    pub fn payload(&self) -> Option<&'f [u8]> {
        let mut offset = 3;
        offset += self.addressing().len();

        if self.frame_control().security_enabled() {
            offset += self.auxiliary_security_header().unwrap().len();
        }

        offset += 2; // Superframe specification
        offset += self.gts_info().len();
        offset += self.pending_address().len();

        Some(&self.buffer.as_ref()[offset..])
    }
}
/// A reader/writer for an IEEE 802.15.4 Enhanced Beacon frame.
pub struct EnhancedBeaconFrame<T: AsRef<[u8]>> {
    buffer: T,
}

impl<T: AsRef<[u8]>> EnhancedBeaconFrame<T> {
    pub fn new(buffer: T) -> Result<Self> {
        let b = Self::new_unchecked(buffer);

        if !b.check_len() {
            return Err(Error);
        }

        let fc = b.frame_control();

        if fc.security_enabled() {
            return Err(Error);
        }

        if fc.frame_type() == FrameType::Unknown {
            return Err(Error);
        }

        if fc.frame_version() == FrameVersion::Unknown {
            return Err(Error);
        }

        if fc.dst_addressing_mode() == AddressingMode::Unknown {
            return Err(Error);
        }

        if fc.src_addressing_mode() == AddressingMode::Unknown {
            return Err(Error);
        }

        Ok(b)
    }

    fn check_len(&self) -> bool {
        let buffer = self.buffer.as_ref();

        if buffer.len() < 2 || buffer.len() > 127 {
            return false;
        }

        let fc = self.frame_control();

        if !fc.sequence_number_suppression() && buffer.len() < 3 {
            return false;
        }

        true
    }

    pub fn new_unchecked(buffer: T) -> Self {
        Self { buffer }
    }

    /// Return a [`FrameControl`] reader.
    pub fn frame_control(&self) -> FrameControl<&'_ [u8]> {
        FrameControl::new_unchecked(&self.buffer.as_ref()[..2])
    }

    /// Return the sequence number of the frame.
    pub fn sequence_number(&self) -> Option<u8> {
        if self.frame_control().sequence_number_suppression() {
            None
        } else {
            Some(self.buffer.as_ref()[2])
        }
    }

    /// Return an [`AddressingFields`] reader.
    pub fn addressing(&self) -> Option<AddressingFields<&'_ [u8], &'_ [u8]>> {
        let fc = self.frame_control();

        if fc.sequence_number_suppression() {
            AddressingFields::new(&self.buffer.as_ref()[2..], fc).ok()
        } else {
            AddressingFields::new(&self.buffer.as_ref()[3..], fc).ok()
        }
    }

    pub fn auxiliary_security_header(&self) -> Option<AuxiliarySecurityHeader<&'_ [u8]>> {
        let fc = self.frame_control();

        if fc.security_enabled() {
            let mut offset = 2;

            offset += !fc.sequence_number_suppression() as usize;

            if let Some(af) = self.addressing() {
                offset += af.len();
            }

            Some(AuxiliarySecurityHeader::new(
                &self.buffer.as_ref()[offset..],
            ))
        } else {
            None
        }
    }

    /// Return an [`InformationElements`] reader.
    pub fn information_elements(&self) -> Option<InformationElements<&'_ [u8]>> {
        let fc = self.frame_control();
        if fc.information_elements_present() {
            let mut offset = 2;
            offset += !fc.sequence_number_suppression() as usize;

            if let Some(af) = self.addressing() {
                offset += af.len();
            }

            Some(InformationElements::new(&self.buffer.as_ref()[offset..]).ok()?)
        } else {
            None
        }
    }
}

impl<'f, T: AsRef<[u8]> + ?Sized> EnhancedBeaconFrame<&'f T> {
    /// Return the payload of the frame.
    pub fn payload(&self) -> Option<&'f [u8]> {
        let fc = self.frame_control();

        let mut offset = 0;
        offset += 2;

        if !fc.sequence_number_suppression() {
            offset += 1;
        }

        if let Some(af) = self.addressing() {
            offset += af.len();
        }

        if fc.security_enabled() {
            offset += self.auxiliary_security_header().unwrap().len();
        }

        if fc.information_elements_present() {
            if let Some(ie) = self.information_elements() {
                offset += ie.len();
            }
        }

        if self.buffer.as_ref().len() <= offset {
            return None;
        }

        Some(&self.buffer.as_ref()[offset..])
    }
}

#[frame]
#[derive(Debug)]
/// A reader/writer for the IEEE 802.15.4 Superframe Specification Header
/// Information Element.
pub struct SuperframeSpecification {
    /// Return the beacon order field value.
    #[bits(4)]
    #[into(BeaconOrder)]
    beacon_order: u8,
    /// Return the superframe order field value.
    #[bits(4)]
    #[into(SuperframeOrder)]
    superframe_order: u8,
    #[bits(4)]
    /// Return the final cap slot field value.
    final_cap_slot: u8,
    #[bits(1)]
    /// Return the battery life extension field value.
    battery_life_extension: bool,
    #[bits(1)]
    _reserved: bool,
    #[bits(1)]
    /// Return the PAN coordinator field value.
    pan_coordinator: bool,
    #[bits(1)]
    /// Return the association permit field value.
    association_permit: bool,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
#[repr(u8)]
/// Indicates the frequency at which the beacon is transmitted.
pub enum BeaconOrder {
    /// The beacon is transmitted at an interval:
    /// `base_super_frame_duration * 2^{beacon_order}`.
    Order(u8),
    /// The beacon is transmitted on demand.
    OnDemand,
}

impl From<u8> for BeaconOrder {
    fn from(value: u8) -> Self {
        match value {
            value @ 0..=14 => Self::Order(value),
            _ => Self::OnDemand,
        }
    }
}
impl From<BeaconOrder> for u8 {
    fn from(value: BeaconOrder) -> Self {
        match value {
            BeaconOrder::Order(value) => value,
            BeaconOrder::OnDemand => 15,
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
#[repr(u8)]
///  The lenght of the active portion of the superframe.
pub enum SuperframeOrder {
    /// The superframe duration is calculated with:
    /// `base_super_frame_duration * 2^{superframe_order}`
    Order(u8),
    /// The superframe is inactive after the the beacon.
    Inactive,
}

impl From<u8> for SuperframeOrder {
    fn from(value: u8) -> Self {
        match value {
            value @ 0..=14 => Self::Order(value),
            _ => Self::Inactive,
        }
    }
}
impl From<SuperframeOrder> for u8 {
    fn from(value: SuperframeOrder) -> Self {
        match value {
            SuperframeOrder::Order(value) => value,
            SuperframeOrder::Inactive => 15,
        }
    }
}

#[frame]
/// A reader/writer for the IEEE 802.15.4 GTS Info field.
pub struct GtsInfo {
    #[bits(8)]
    gts_spec: GtsSpecification,
    #[bits(1)]
    #[into(GtsDirection)]
    gts_direction: u8,
}

impl<T: AsRef<[u8]>> GtsInfo<T> {
    pub fn len(&self) -> usize {
        // TODO: check auto-generated code
        1 + self.gts_spec().unwrap().descriptor_count() as usize * GtsSlot::<T>::size()
    }
}

/// Guaranteed Timeslot Descriptor
#[frame(no_constructor)]
pub struct GtsSlot {
    /// Short address of the intended device.
    #[bytes(2)]
    #[into(crate::Address)]
    short_address: &[u8],
    /// Superframe slot at which the GTS is to begin.
    #[bits(4)]
    starting_slot: u8,
    /// Number of contiguous superframe slots over which the GTS is active.
    #[bits(4)]
    length: u8,

    /// The GTS slot direction.
    #[field]
    direction: GtsDirection,
}

impl<T: AsRef<[u8]>> GtsSlot<T> {
    /// Create a new [`#name`] reader/writer from a given buffer.
    pub fn new(buffer: T, direction: GtsDirection) -> Result<Self> {
        let s = Self::new_unchecked(buffer, direction);

        if !s.check_len() {
            return Err(Error);
        }

        Ok(s)
    }

    /// Returns `false` if the buffer is too short to contain this structure.
    fn check_len(&self) -> bool {
        self.buffer.as_ref().len() >= Self::size()
    }

    /// Create a new [`#name`] reader/writer from a given buffer without length
    /// checking.
    pub fn new_unchecked(buffer: T, direction: GtsDirection) -> Self {
        Self { buffer, direction }
    }
}

impl<T: AsRef<[u8]>> GtsSpecification<T> {
    /// Return a [`GtsSlotIterator`].
    pub fn slots(&self) -> GtsSlotIterator {
        if self.descriptor_count() == 0 {
            GtsSlotIterator {
                data: &[],
                count: 0,
                terminated: true,
            }
        } else {
            GtsSlotIterator {
                data: &self.buffer.as_ref()[1..]
                    [..1 + self.descriptor_count() as usize * GtsSlot::<T>::size()],
                count: 0,
                terminated: false,
            }
        }
    }
}

/// An [`Iterator`] over GTS slots.
pub struct GtsSlotIterator<'f> {
    data: &'f [u8],
    count: usize,
    terminated: bool,
}

impl<'f> Iterator for GtsSlotIterator<'f> {
    type Item = GtsSlot<&'f [u8]>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.terminated {
            None
        } else {
            const L: usize = GtsSlot::<&[u8]>::size();
            if 1 + self.count * L >= self.data.len() {
                return None;
            }

            let direction = GtsDirection::from((self.data[0] >> self.count) & 0b1);
            let descriptor = GtsSlot::new(&self.data[1 + self.count * L..], direction).ok()?;

            self.count += 1;
            if 1 + self.count * L >= self.data.len() {
                self.terminated = true;
            }

            Some(descriptor)
        }
    }
}

#[frame]
#[derive(Debug)]
/// Guaranteed Time Slot specification.
pub struct GtsSpecification {
    #[bits(3)]
    /// GTS descriptor count.
    descriptor_count: u8,
    #[bits(4)]
    _reserved: u8,
    /// GTS is permitted.
    #[bits(1)]
    gts_permit: bool,
}

/// GTS direciton.
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum GtsDirection {
    /// GTS Receive direction.
    Receive,
    /// GTS Transmit direction.
    Transmit,
}

impl From<u8> for GtsDirection {
    fn from(value: u8) -> Self {
        match value {
            0b0 => Self::Receive,
            _ => Self::Transmit,
        }
    }
}

impl From<GtsDirection> for u8 {
    fn from(value: GtsDirection) -> Self {
        match value {
            GtsDirection::Receive => 0b0,
            GtsDirection::Transmit => 0b1,
        }
    }
}

#[frame]
pub struct PendingAddress {
    #[bits(8)]
    pending_address_spec: PendingAddressSpecification,
}

impl<T: AsRef<[u8]>> PendingAddress<T> {
    pub fn len(&self) -> usize {
        let spec = self.pending_address_spec().unwrap();
        1 + spec.short_address_pending() as usize * 2 + spec.extended_address_pending() as usize * 8
    }

    pub fn pending_addresses(&self) -> PendingAddressIterator {
        let spec = self.pending_address_spec().unwrap();
        PendingAddressIterator::new(
            &self.buffer.as_ref()[1..][..self.len() - 1],
            spec.short_address_pending(),
            spec.extended_address_pending(),
        )
    }
}

#[frame]
pub struct PendingAddressSpecification {
    #[bits(3)]
    short_address_pending: u8,
    #[bits(1)]
    _reserved: u8,
    #[bits(3)]
    extended_address_pending: u8,
    #[bits(1)]
    _reserved: u8,
}

pub struct PendingAddressIterator<'f> {
    data: &'f [u8],
    short_addresses: u8,
    extended_addresses: u8,
    terminated: bool,
}

impl<'f> PendingAddressIterator<'f> {
    pub fn new(data: &'f [u8], short_addresses: u8, extended_addresses: u8) -> Self {
        Self {
            data,
            short_addresses,
            extended_addresses,
            terminated: false,
        }
    }
}

impl Iterator for PendingAddressIterator<'_> {
    type Item = Address;

    fn next(&mut self) -> Option<Self::Item> {
        if self.terminated {
            None
        } else {
            if self.short_addresses > 0 {
                if self.data.len() < 2 {
                    return None;
                }

                let address = Address::Short([self.data[0], self.data[1]]);
                self.data = &self.data[2..];
                self.short_addresses -= 1;

                return Some(address);
            }

            if self.extended_addresses > 0 {
                if self.data.len() < 8 {
                    return None;
                }

                let mut address = [0; 8];
                address.copy_from_slice(&self.data[..8]);
                self.data = &self.data[8..];
                self.extended_addresses -= 1;

                return Some(Address::Extended(address));
            }

            self.terminated = true;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superframe_specification() {
        let data = [0xff, 0x0f];
        let ie = SuperframeSpecification::new(&data).unwrap();
        assert_eq!(ie.beacon_order(), BeaconOrder::OnDemand);
        assert_eq!(ie.superframe_order(), SuperframeOrder::Inactive);
        assert_eq!(ie.final_cap_slot(), 0x0f);
        assert!(!ie.battery_life_extension());
        assert!(!ie.pan_coordinator());
        assert!(!ie.association_permit());
    }

    #[test]
    fn gts_specification() {
        use crate::Address;

        let data = [0b0000_0000];
        let gts = GtsSpecification::new(&data).unwrap();
        assert_eq!(gts.descriptor_count(), 0);
        assert!(!gts.gts_permit());

        let data = [0b1000_0000];
        let gts = GtsSpecification::new(&data).unwrap();
        assert_eq!(gts.descriptor_count(), 0);
        assert!(gts.gts_permit());

        assert!(gts.slots().next().is_none());

        let data = [0x82, 0x01, 0x34, 0x12, 0x11, 0x78, 0x56, 0x14];
        let gts = GtsSpecification::new(&data).unwrap();

        assert!(gts.gts_permit());
        assert_eq!(gts.descriptor_count(), 2);

        let mut slots = gts.slots();
        let s0 = slots.next().unwrap();
        assert_eq!(s0.short_address(), Address::Short([0x34, 0x12]));
        assert_eq!(s0.starting_slot(), 1);
        assert_eq!(s0.length(), 1);
        assert_eq!(s0.direction(), GtsDirection::Transmit);

        let s1 = slots.next().unwrap();
        assert_eq!(s1.short_address(), Address::Short([0x78, 0x56]));
        assert_eq!(s1.starting_slot(), 4);
        assert_eq!(s1.length(), 1);
        assert_eq!(s1.direction(), GtsDirection::Receive);

        assert!(slots.next().is_none());
    }

    #[test]
    fn gts_slot() {
        use crate::Address;
        let data = [0xab, 0xcd, 0b0101_1010];
        let slot = GtsSlot::new(&data[..], GtsDirection::Transmit).unwrap();
        assert_eq!(slot.short_address(), Address::Short([0xab, 0xcd]));
        assert_eq!(slot.starting_slot(), 0b1010);
        assert_eq!(slot.length(), 0b0101);
        assert_eq!(slot.direction(), GtsDirection::Transmit);
    }
}