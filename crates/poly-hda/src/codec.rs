//! Codec commands (HDA 1.0a §7.3): verbs, parameters, and the capability
//! words codecs answer with.
//!
//! # Command word (§7.3, "Command Format")
//!
//! Bits 31:28 the codec address, 27:20 the node ID, 19:0 the verb and its
//! payload. Verbs come in two shapes, and [`crate::Hda::verb`] takes the
//! 20-bit field ready-made:
//!
//! - 12-bit verb ID with an 8-bit payload: `id << 8 | payload`
//!   ([`short`]); the IDs are 0x7xx (set) and 0xFxx (get).
//! - 4-bit verb ID with a 16-bit payload: `id << 16 | payload` ([`long`]);
//!   the IDs are 0x1..0x5 (set) and 0x9..0xD (get).

/// A 12-bit verb with an 8-bit payload, as the 20-bit verb field.
pub const fn short(id: u16, payload: u8) -> u32 {
    ((id as u32 & 0xFFF) << 8) | payload as u32
}

/// A 4-bit verb with a 16-bit payload, as the 20-bit verb field.
pub const fn long(id: u8, payload: u16) -> u32 {
    ((id as u32 & 0xF) << 16) | payload as u32
}

/// The 32-bit command word for `verb` (a 20-bit field from [`short`] or
/// [`long`]) to node `nid` of codec `codec` (§7.3).
pub const fn command(codec: u8, nid: u8, verb: u32) -> u32 {
    ((codec as u32 & 0xF) << 28) | ((nid as u32) << 20) | (verb & 0xF_FFFF)
}

/// Verb IDs (§7.3.3).
pub mod verb {
    /// Get Parameter (§7.3.3.x, "Get Parameter"): payload the parameter ID.
    pub const GET_PARAMETER: u16 = 0xF00;
    /// Connection Select Control, get and set (§7.3.3, "Connection Select
    /// Control"): payload the connection index.
    pub const GET_CONNECTION_SELECT: u16 = 0xF01;
    pub const SET_CONNECTION_SELECT: u16 = 0x701;
    /// Get Connection List Entry (§7.3.3, "Get Connection List Entry"):
    /// payload the index of the first entry returned; four short-form or
    /// two long-form entries come back.
    pub const GET_CONNECTION_LIST_ENTRY: u16 = 0xF02;
    /// Power State, get and set (§7.3.3, "Power State"): payload bits 3:0
    /// the state (0 = D0 .. 3 = D3); the response has the state set in
    /// bits 3:0 and the actual state in bits 7:4.
    pub const GET_POWER_STATE: u16 = 0xF05;
    pub const SET_POWER_STATE: u16 = 0x705;
    /// Converter Stream, Channel (§7.3.3, "Converter Stream, Channel"):
    /// payload bits 7:4 the stream tag, 3:0 the lowest channel.
    pub const GET_CONVERTER_STREAM_CHANNEL: u16 = 0xF06;
    pub const SET_CONVERTER_STREAM_CHANNEL: u16 = 0x706;
    /// Pin Widget Control (§7.3.3, "Pin Widget Control"): see
    /// [`super::pinctl`].
    pub const GET_PIN_WIDGET_CONTROL: u16 = 0xF07;
    pub const SET_PIN_WIDGET_CONTROL: u16 = 0x707;
    /// EAPD/BTL Enable (§7.3.3, "EAPD/BTL Enable"): bit 1 EAPD.
    pub const GET_EAPD_BTL: u16 = 0xF0C;
    pub const SET_EAPD_BTL: u16 = 0x70C;
    /// Get Configuration Default (§7.3.3, "Configuration Default"): see
    /// [`super::ConfigDefault`].
    pub const GET_CONFIG_DEFAULT: u16 = 0xF1C;

    /// Converter Format, 4-bit verbs (§7.3.3, "Converter Format"): the
    /// payload is the 16-bit stream format of §3.7.1.
    pub const SET_CONVERTER_FORMAT: u8 = 0x2;
    pub const GET_CONVERTER_FORMAT: u8 = 0xA;
    /// Amplifier Gain/Mute, 4-bit verbs (§7.3.3, "Amplifier Gain/Mute"):
    /// see [`super::amp`].
    pub const SET_AMP_GAIN_MUTE: u8 = 0x3;
    pub const GET_AMP_GAIN_MUTE: u8 = 0xB;
}

/// Parameter IDs for Get Parameter (§7.3.4).
pub mod param {
    /// §7.3.4.1: vendor ID in bits 31:16, device ID in 15:0.
    pub const VENDOR_ID: u8 = 0x00;
    /// §7.3.4.2.
    pub const REVISION_ID: u8 = 0x02;
    /// §7.3.4.3: bits 23:16 the first subordinate node, 7:0 their count.
    pub const NODE_COUNT: u8 = 0x04;
    /// §7.3.4.4: bits 7:0 the function group type (1 = audio).
    pub const FUNCTION_GROUP_TYPE: u8 = 0x05;
    /// §7.3.4.5.
    pub const AFG_CAPS: u8 = 0x08;
    /// §7.3.4.6: see [`super::WidgetCaps`].
    pub const WIDGET_CAPS: u8 = 0x09;
    /// §7.3.4.7: PCM rates in bits 11:0, sizes in 20:16.
    pub const PCM: u8 = 0x0A;
    /// §7.3.4.8: bit 0 PCM, 1 float32, 2 AC-3.
    pub const STREAM_FORMATS: u8 = 0x0B;
    /// §7.3.4.9: see [`super::PinCaps`].
    pub const PIN_CAPS: u8 = 0x0C;
    /// §7.3.4.10: see [`super::AmpCaps`].
    pub const IN_AMP_CAPS: u8 = 0x0D;
    /// §7.3.4.11: bits 6:0 the entry count, bit 7 long form.
    pub const CONN_LIST_LEN: u8 = 0x0E;
    /// §7.3.4.12.
    pub const POWER_STATES: u8 = 0x0F;
    /// §7.3.4.10, output amplifier.
    pub const OUT_AMP_CAPS: u8 = 0x12;
}

/// Function group type of an audio function group (§7.3.4.4).
pub const FG_AUDIO: u32 = 0x01;

/// Connection list length parameter (§7.3.4.11).
pub mod conn {
    pub const LEN_MASK: u32 = 0x7F;
    pub const LONG_FORM: u32 = 1 << 7;
    /// Short-form entries: 8 bits, bit 7 marks a range, 6:0 the NID.
    pub const SHORT_RANGE: u32 = 1 << 7;
    pub const SHORT_NID: u32 = 0x7F;
    /// Long-form entries: 16 bits, bit 15 marks a range, 14:0 the NID.
    pub const LONG_RANGE: u32 = 1 << 15;
    pub const LONG_NID: u32 = 0x7FFF;
}

/// Pin Widget Control bits (§7.3.3, "Pin Widget Control").
pub mod pinctl {
    /// Headphone amplifier enable.
    pub const HP_EN: u8 = 1 << 7;
    /// Output enable.
    pub const OUT_EN: u8 = 1 << 6;
    /// Input enable.
    pub const IN_EN: u8 = 1 << 5;
}

/// EAPD/BTL Enable payload bits (§7.3.3, "EAPD/BTL Enable").
pub mod eapd {
    pub const BTL: u8 = 1 << 0;
    pub const EAPD: u8 = 1 << 1;
    pub const LR_SWAP: u8 = 1 << 2;
}

/// Set Amplifier Gain/Mute payload bits (§7.3.3, "Amplifier Gain/Mute").
pub mod amp {
    pub const OUTPUT: u16 = 1 << 15;
    pub const INPUT: u16 = 1 << 14;
    pub const LEFT: u16 = 1 << 13;
    pub const RIGHT: u16 = 1 << 12;
    pub const INDEX_SHIFT: u16 = 8;
    pub const MUTE: u16 = 1 << 7;
    pub const GAIN_MASK: u16 = 0x7F;

    /// The payload setting both channels of one amplifier.
    pub const fn set(output: bool, index: u8, mute: bool, gain: u8) -> u16 {
        (if output { OUTPUT } else { INPUT })
            | LEFT
            | RIGHT
            | (((index & 0xF) as u16) << INDEX_SHIFT)
            | (if mute { MUTE } else { 0 })
            | (gain as u16 & GAIN_MASK)
    }
}

/// Power states for Set Power State (§7.3.3).
pub mod power {
    pub const D0: u8 = 0;
    pub const D3: u8 = 3;
}

/// Audio widget types (§7.3.4.6, bits 23:20).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WidgetType {
    AudioOutput,
    AudioInput,
    Mixer,
    Selector,
    PinComplex,
    Power,
    VolumeKnob,
    BeepGenerator,
    VendorDefined,
    /// A reserved type code.
    Reserved(u8),
}

impl WidgetType {
    pub fn from_code(code: u8) -> WidgetType {
        match code & 0xF {
            0x0 => WidgetType::AudioOutput,
            0x1 => WidgetType::AudioInput,
            0x2 => WidgetType::Mixer,
            0x3 => WidgetType::Selector,
            0x4 => WidgetType::PinComplex,
            0x5 => WidgetType::Power,
            0x6 => WidgetType::VolumeKnob,
            0x7 => WidgetType::BeepGenerator,
            0xF => WidgetType::VendorDefined,
            c => WidgetType::Reserved(c),
        }
    }
}

/// Audio Widget Capabilities (§7.3.4.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WidgetCaps(pub u32);

impl WidgetCaps {
    pub fn kind(self) -> WidgetType {
        WidgetType::from_code((self.0 >> 20) as u8)
    }
    /// Bits 19:16, delay in samples.
    pub fn delay(self) -> u8 {
        ((self.0 >> 16) & 0xF) as u8
    }
    /// Channels: (bits 15:13 << 1 | bit 0) + 1.
    pub fn channels(self) -> u8 {
        ((((self.0 >> 13) & 7) << 1) | (self.0 & 1)) as u8 + 1
    }
    pub fn stereo(self) -> bool {
        self.0 & (1 << 0) != 0
    }
    pub fn in_amp(self) -> bool {
        self.0 & (1 << 1) != 0
    }
    pub fn out_amp(self) -> bool {
        self.0 & (1 << 2) != 0
    }
    /// The widget has its own amplifier capabilities (else the AFG's apply).
    pub fn amp_override(self) -> bool {
        self.0 & (1 << 3) != 0
    }
    /// The widget has its own PCM / stream format parameters (else the
    /// AFG's apply).
    pub fn format_override(self) -> bool {
        self.0 & (1 << 4) != 0
    }
    pub fn stripe(self) -> bool {
        self.0 & (1 << 5) != 0
    }
    pub fn proc_widget(self) -> bool {
        self.0 & (1 << 6) != 0
    }
    pub fn unsol_capable(self) -> bool {
        self.0 & (1 << 7) != 0
    }
    pub fn conn_list(self) -> bool {
        self.0 & (1 << 8) != 0
    }
    pub fn digital(self) -> bool {
        self.0 & (1 << 9) != 0
    }
    /// The widget supports the Power State verbs.
    pub fn power_control(self) -> bool {
        self.0 & (1 << 10) != 0
    }
    pub fn lr_swap(self) -> bool {
        self.0 & (1 << 11) != 0
    }
}

/// Pin Capabilities (§7.3.4.9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PinCaps(pub u32);

impl PinCaps {
    pub fn impedance_sense(self) -> bool {
        self.0 & (1 << 0) != 0
    }
    pub fn trigger_required(self) -> bool {
        self.0 & (1 << 1) != 0
    }
    pub fn presence_detect(self) -> bool {
        self.0 & (1 << 2) != 0
    }
    pub fn headphone_drive(self) -> bool {
        self.0 & (1 << 3) != 0
    }
    pub fn output(self) -> bool {
        self.0 & (1 << 4) != 0
    }
    pub fn input(self) -> bool {
        self.0 & (1 << 5) != 0
    }
    pub fn balanced(self) -> bool {
        self.0 & (1 << 6) != 0
    }
    pub fn eapd(self) -> bool {
        self.0 & (1 << 16) != 0
    }
}

/// Port connectivity, bits 31:30 of the configuration default (§7.3.3,
/// "Configuration Default").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Connectivity {
    Jack,
    None,
    Fixed,
    Both,
}

/// Default device codes, bits 23:20 of the configuration default.
pub mod device {
    pub const LINE_OUT: u8 = 0x0;
    pub const SPEAKER: u8 = 0x1;
    pub const HP_OUT: u8 = 0x2;
    pub const CD: u8 = 0x3;
    pub const SPDIF_OUT: u8 = 0x4;
    pub const DIGITAL_OTHER_OUT: u8 = 0x5;
    pub const MODEM_LINE: u8 = 0x6;
    pub const MODEM_HANDSET: u8 = 0x7;
    pub const LINE_IN: u8 = 0x8;
    pub const AUX: u8 = 0x9;
    pub const MIC_IN: u8 = 0xA;
    pub const TELEPHONY: u8 = 0xB;
    pub const SPDIF_IN: u8 = 0xC;
    pub const DIGITAL_OTHER_IN: u8 = 0xD;
    pub const OTHER: u8 = 0xF;
}

/// The configuration default of a pin (§7.3.3, "Configuration Default").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ConfigDefault(pub u32);

impl ConfigDefault {
    pub fn connectivity(self) -> Connectivity {
        match self.0 >> 30 {
            0 => Connectivity::Jack,
            1 => Connectivity::None,
            2 => Connectivity::Fixed,
            _ => Connectivity::Both,
        }
    }
    /// Bits 29:24.
    pub fn location(self) -> u8 {
        ((self.0 >> 24) & 0x3F) as u8
    }
    /// Bits 23:20, see [`device`].
    pub fn device(self) -> u8 {
        ((self.0 >> 20) & 0xF) as u8
    }
    /// Bits 19:16.
    pub fn connection_type(self) -> u8 {
        ((self.0 >> 16) & 0xF) as u8
    }
    /// Bits 15:12.
    pub fn color(self) -> u8 {
        ((self.0 >> 12) & 0xF) as u8
    }
    /// Bits 11:8.
    pub fn misc(self) -> u8 {
        ((self.0 >> 8) & 0xF) as u8
    }
    /// Bits 7:4.
    pub fn association(self) -> u8 {
        ((self.0 >> 4) & 0xF) as u8
    }
    /// Bits 3:0.
    pub fn sequence(self) -> u8 {
        (self.0 & 0xF) as u8
    }
}

/// Amplifier Capabilities (§7.3.4.10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AmpCaps(pub u32);

impl AmpCaps {
    /// Bits 6:0: the gain step that is 0 dB.
    pub fn offset(self) -> u8 {
        (self.0 & 0x7F) as u8
    }
    /// Bits 14:8: the number of steps minus one (the largest gain value).
    pub fn steps(self) -> u8 {
        ((self.0 >> 8) & 0x7F) as u8
    }
    /// Bits 22:16: step size in 0.25 dB units, minus one.
    pub fn step_size(self) -> u8 {
        ((self.0 >> 16) & 0x7F) as u8
    }
    pub fn mute_capable(self) -> bool {
        self.0 & (1 << 31) != 0
    }
    /// The gain value for 0 dB, kept within the advertised range.
    pub fn zero_db(self) -> u8 {
        self.offset().min(self.steps())
    }
}
