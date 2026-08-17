//! Palette. Colours are 0xRRGGBB so they read like CSS at a glance.

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: u32,
    pub frame: u32,
    pub label: u32,
    pub fg: u32,
    pub bright: u32,
    pub ok: u32,
    pub warn: u32,
    pub crit: u32,
    pub cyan: u32,
    pub magenta: u32,
    pub yellow: u32,
}

/// Green phosphor on a CRT that has been on for twenty years.
pub const PHOSPHOR: Theme = Theme {
    bg: 0x05_08_06,
    frame: 0x1B_35_26,
    label: 0x4A_7A_5E,
    fg: 0x7F_BF_9A,
    bright: 0xD6_F7_E2,
    ok: 0x35_D0_6B,
    warn: 0xE0_B3_41,
    crit: 0xE8_5D_5D,
    cyan: 0x4F_D6_D6,
    magenta: 0xB0_7F_E0,
    yellow: 0xD8_D0_60,
};

/// The same hierarchy at two-thirds saturation — for long-staring eyes that
/// find the pure phosphor too insistent.
pub const SOFT: Theme = Theme {
    bg: 0x0A_0C_0B,
    frame: 0x24_35_2C,
    label: 0x5A_75_66,
    fg: 0x8F_B5_A0,
    bright: 0xC9_E4_D4,
    ok: 0x5F_BF_85,
    warn: 0xCB_A9_5C,
    crit: 0xC9_6F_6F,
    cyan: 0x6B_BB_BB,
    magenta: 0xA8_8F_C9,
    yellow: 0xBD_B2_73,
};

pub fn pct_color(t: &Theme, pct: f64) -> u32 {
    if pct >= 85.0 {
        t.crit
    } else if pct >= 65.0 {
        t.warn
    } else {
        t.ok
    }
}
