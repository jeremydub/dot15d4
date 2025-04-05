pub struct MacPib {
    // see IEEE 802.15.4-2020, section 8.4.3.1, table 8-94
    pub mac_security_enabled: bool,

    // see IEEE 802.15.4-2020, section 8.4.3.2, table 8-95
    pub mac_tsch_enabled: bool,
    pub mac_le_hs_enabled: bool,

    // see IEEE 802.15.4-2020, section 9.5, table 9-8
    pub sec_frame_counter: u32,
}
