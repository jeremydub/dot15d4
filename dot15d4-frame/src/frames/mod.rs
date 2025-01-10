use crate::{Error, Result};

use crate::{FrameControl, FrameType, FrameVersion};

pub(crate) mod beacon;
mod data;

pub use beacon::BeaconFrame;
pub use beacon::EnhancedBeaconFrame;
pub use data::DataFrame;

/// A high-level representation of an IEEE 802.15.4 frame.
pub enum Frame<T: AsRef<[u8]>> {
    /// A beacon frame.
    Beacon(BeaconFrame<T>),
    /// An enhanced beacon frame.
    EnhancedBeacon(EnhancedBeaconFrame<T>),
    /// A data frame.
    Data(DataFrame<T>),
}

impl<T: AsRef<[u8]>> Frame<T> {
    /// Create a new [`Frame`] from a given buffer.
    pub fn new(buffer: T) -> Result<Self> {
        if buffer.as_ref().len() < 2 {
            return Err(Error);
        }

        let frame_control = FrameControl::new(&buffer.as_ref()[..2])?;

        match frame_control.frame_type() {
            FrameType::Beacon => match frame_control.frame_version() {
                FrameVersion::Ieee802154_2003 | FrameVersion::Ieee802154_2006 => {
                    Ok(Frame::Beacon(BeaconFrame::new(buffer)?))
                }
                FrameVersion::Ieee802154_2020 => {
                    Ok(Frame::EnhancedBeacon(EnhancedBeaconFrame::new(buffer)?))
                }
                FrameVersion::Unknown => Err(Error),
            },
            FrameType::Data => Ok(Frame::Data(DataFrame::new(buffer)?)),
            _ => Err(Error),
        }
    }

    /// Convert the [`Frame`] into a [`BeaconFrame`].
    ///
    /// # Panics
    /// Panics if the frame is not a beacon frame.
    pub fn into_beacon(self) -> BeaconFrame<T> {
        match self {
            Frame::Beacon(frame) => frame,
            _ => panic!("not a beacon"),
        }
    }

    /// Convert the [`Frame`] into an [`EnhancedBeaconFrame`].
    ///
    /// # Panics
    /// Panics if the frame is not an enhanced beacon frame.
    pub fn into_enhanced_beacon(self) -> EnhancedBeaconFrame<T> {
        match self {
            Frame::EnhancedBeacon(frame) => frame,
            _ => panic!("not an enhanced beacon"),
        }
    }

    /// Convert the [`Frame`] into a [`DataFrame`].
    ///
    /// # Panics
    /// Panics if the frame is not a data frame.
    pub fn into_data(self) -> DataFrame<T> {
        match self {
            Frame::Data(frame) => frame,
            _ => panic!("not a data frame"),
        }
    }
}