use crate::mpdu::{MacFrameConfig, MpduBuilder};
use core::marker::PhantomData;

#[derive(Clone, Copy, Debug)]
pub enum SecurityLevel {
    Mic32,
    Mic64,
    Mic128,
    EncMic32,
    EncMic64,
    EncMic128,
} // 1 byte

impl SecurityLevel {
    pub const fn length(&self) -> usize {
        match self {
            SecurityLevel::Mic32 | SecurityLevel::EncMic32 => 32,
            SecurityLevel::Mic64 | SecurityLevel::EncMic64 => 64,
            SecurityLevel::Mic128 | SecurityLevel::EncMic128 => 128,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Security {
    Implicit(SecurityLevel),
    SourceNone(SecurityLevel),
    Source4Byte(SecurityLevel),
    Source8Byte(SecurityLevel),
} // 2 bytes

impl Security {
    pub const fn length(&self, tsch_mode: bool) -> usize {
        let frame_counter_len = if tsch_mode { 0 } else { 4 };

        let (key_id_len, sec_level) = match self {
            Security::Implicit(sec_level) => (1, sec_level),
            Security::SourceNone(sec_level) => (2, sec_level),
            Security::Source4Byte(sec_level) => (6, sec_level),
            Security::Source8Byte(sec_level) => (10, sec_level),
        };

        frame_counter_len + key_id_len + sec_level.length()
    }
}

#[derive(Debug)]
pub struct MacSecurityConfig;

impl<'frame> MpduBuilder<'frame, MacSecurityConfig> {
    #[cfg(feature = "security")]
    pub const fn with_security(self, security: Security) -> MpduBuilder<'frame, MacFrameConfig> {
        MpduBuilder {
            tsch_mode: self.tsch_mode,
            seq_nr: self.seq_nr,
            addressing: self.addressing,
            security: Some(security),
            #[cfg(feature = "tsch")]
            ies: self.ies,
            stage: PhantomData,
        }
    }

    pub const fn without_security(self) -> MpduBuilder<'frame, MacFrameConfig> {
        MpduBuilder {
            tsch_mode: self.tsch_mode,
            seq_nr: self.seq_nr,
            addressing: self.addressing,
            #[cfg(feature = "security")]
            security: None,
            #[cfg(feature = "tsch")]
            ies: self.ies,
            stage: PhantomData,
        }
    }
}
