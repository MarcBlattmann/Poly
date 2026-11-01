//! The widget graph of one audio function group and the output path search.
//!
//! The driver reads every widget of an AFG into a fixed table of
//! [`Widget`]s (at most [`MAX_WIDGETS`], each with at most
//! [`MAX_CONNECTIONS`] connection list entries), then [`find_output`]
//! picks a pin and searches back from it to an audio output converter.
//! Everything here is pure; the codec's answers are already in the table.
//!
//! # Choosing the pin
//!
//! Candidates are pin complexes that can drive an output (pin capabilities
//! bit 4) and whose configuration default does not say "no physical
//! connection" (port connectivity 01b). By default device: line out first,
//! then speaker, then headphone out, then anything else; ties go to the
//! lower NID.
//!
//! # The search
//!
//! Depth-first from the pin through the connection lists, passing only
//! mixers and selectors, until an audio output converter is reached. At
//! most [`MAX_PATH`] nodes (pin and DAC included), and a node is entered at
//! most once per pin, so loops in the connection lists end the search.

use crate::codec::{AmpCaps, ConfigDefault, Connectivity, PinCaps, WidgetCaps, WidgetType, device};
use crate::format::{self, PCM_RATES_MASK, PCM_SIZES_MASK, PCM_SIZES_SHIFT, StreamFormat};

/// Widgets kept per audio function group; further ones are ignored.
pub const MAX_WIDGETS: usize = 64;
/// Connection list entries kept per widget (ranges expanded); further ones
/// are ignored.
pub const MAX_CONNECTIONS: usize = 32;
/// Nodes in an output path, DAC and pin included.
pub const MAX_PATH: usize = 8;

/// One widget as read from the codec. Fields the widget type does not have
/// are zero (no pin capabilities on a mixer, no PCM bits on a selector).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Widget {
    pub nid: u8,
    pub caps: WidgetCaps,
    /// Pin complexes: pin capabilities and configuration default.
    pub pin_caps: PinCaps,
    pub config: ConfigDefault,
    /// Amplifier capabilities in effect (the widget's own with the amp
    /// override bit, else the AFG's); zero without that amplifier.
    pub in_amp: AmpCaps,
    pub out_amp: AmpCaps,
    /// Converters: parameter 0x0A and 0x0B in effect (own with the format
    /// override bit, else the AFG's).
    pub pcm: u32,
    pub stream_formats: u32,
    conns: [u8; MAX_CONNECTIONS],
    conn_len: u8,
    /// The connection list held more entries than were kept.
    pub conn_truncated: bool,
}

impl Widget {
    pub const EMPTY: Widget = Widget {
        nid: 0,
        caps: WidgetCaps(0),
        pin_caps: PinCaps(0),
        config: ConfigDefault(0),
        in_amp: AmpCaps(0),
        out_amp: AmpCaps(0),
        pcm: 0,
        stream_formats: 0,
        conns: [0; MAX_CONNECTIONS],
        conn_len: 0,
        conn_truncated: false,
    };

    pub fn new(nid: u8, caps: WidgetCaps) -> Widget {
        Widget { nid, caps, ..Widget::EMPTY }
    }

    pub fn kind(&self) -> WidgetType {
        self.caps.kind()
    }

    /// The connection list, ranges expanded, in the codec's order: entry
    /// `i` is what connection select index `i` picks. NIDs above 255
    /// (long form) are kept as 0 so the indices stay right.
    pub fn connections(&self) -> &[u8] {
        &self.conns[..(self.conn_len as usize).min(MAX_CONNECTIONS)]
    }

    /// Append a connection; false (and `conn_truncated` set) once full.
    pub fn push_connection(&mut self, nid: u8) -> bool {
        let len = self.conn_len as usize;
        if len >= MAX_CONNECTIONS {
            self.conn_truncated = true;
            return false;
        }
        self.conns[len] = nid;
        self.conn_len += 1;
        true
    }

    /// The pin would be chosen at this rank (lower is better), `None` if it
    /// is not an output candidate.
    pub fn output_rank(&self) -> Option<u8> {
        if self.kind() != WidgetType::PinComplex
            || !self.pin_caps.output()
            || self.config.connectivity() == Connectivity::None
        {
            return None;
        }
        Some(match self.config.device() {
            device::LINE_OUT => 0,
            device::SPEAKER => 1,
            device::HP_OUT => 2,
            _ => 3,
        })
    }

    /// A headphone pin: default device HP out and headphone drive capable.
    pub fn is_headphone(&self) -> bool {
        self.config.device() == device::HP_OUT && self.pin_caps.headphone_drive()
    }
}

impl Default for Widget {
    fn default() -> Widget {
        Widget::EMPTY
    }
}

/// The output path found in the widget graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputPath {
    /// Codec address (SDI line).
    pub codec: u8,
    /// Audio function group NID.
    pub afg: u8,
    /// Audio output converter NID (`nodes[0]`).
    pub dac: u8,
    /// Pin complex NID (`nodes[len - 1]`).
    pub pin: u8,
    /// The path in signal order: `nodes[0]` the DAC, then mixers /
    /// selectors, `nodes[len - 1]` the pin. Entries from `len` on are 0.
    pub nodes: [u8; MAX_PATH],
    pub len: usize,
    /// For `nodes[i]` with `i >= 1`: the index in its connection list of
    /// `nodes[i - 1]` (what its connection select or input amplifier index
    /// is set to). `inputs[0]` is 0.
    pub inputs: [u8; MAX_PATH],
    /// The DAC's PCM rate bits (parameter 0x0A bits 11:0; R1 = 8 kHz in
    /// bit 0, see [`format::RATES`]).
    pub rates: u32,
    /// The DAC's PCM size bits, parameter 0x0A bits 20:16 shifted down to
    /// bits 4:0: bit 0 8-bit, 1 16-bit, 2 20-bit, 3 24-bit, 4 32-bit.
    pub sizes: u32,
    /// Channels the DAC takes (audio widget capabilities).
    pub max_channels: u8,
    /// The pin's preference rank: 0 line out, 1 speaker, 2 HP out, 3 other.
    pub rank: u8,
}

impl OutputPath {
    /// The path's nodes, DAC first.
    pub fn nodes(&self) -> &[u8] {
        &self.nodes[..self.len.min(MAX_PATH)]
    }

    /// Can the DAC take this format: a rate it lists, a size it lists, no
    /// more channels than it has?
    pub fn supports(&self, f: &StreamFormat) -> bool {
        let rate_ok = format::rate_bit(f.rate).is_some_and(|b| self.rates & b != 0);
        let size_ok = format::size_bit(f.bits).is_some_and(|b| self.sizes & b != 0);
        rate_ok && size_ok && f.channels >= 1 && f.channels <= self.max_channels && f.encode().is_some()
    }

    /// The rates the DAC lists, in Hz, ascending.
    pub fn rates_hz(&self) -> impl Iterator<Item = u32> {
        format::rates_hz(self.rates)
    }

    /// The sample sizes the DAC lists, in bits, ascending.
    pub fn sample_bits(&self) -> impl Iterator<Item = u8> {
        format::sizes_bits(self.sizes)
    }
}

/// Index of the widget with `nid` in `ws`.
pub fn position(ws: &[Widget], nid: u8) -> Option<usize> {
    if nid == 0 {
        return None;
    }
    ws.iter().position(|w| w.nid == nid)
}

/// The search stack: `nodes[d]` the node at depth `d` from the pin,
/// `inputs[d]` the connection index at depth `d` that leads to depth d + 1.
struct Stack {
    nodes: [u8; MAX_PATH],
    inputs: [u8; MAX_PATH],
}

fn search(ws: &[Widget], idx: usize, depth: usize, visited: &mut u64, st: &mut Stack) -> Option<usize> {
    let w = ws.get(idx)?;
    if depth > 0 {
        match w.kind() {
            WidgetType::AudioOutput => return Some(depth),
            WidgetType::Mixer | WidgetType::Selector => {}
            _ => return None,
        }
    }
    if depth + 1 >= MAX_PATH {
        return None;
    }
    for (i, &c) in w.connections().iter().enumerate() {
        let Some(j) = position(ws, c) else { continue };
        if j >= 64 || *visited & (1 << j) != 0 {
            continue;
        }
        *visited |= 1 << j;
        st.nodes[depth + 1] = c;
        st.inputs[depth] = i as u8;
        if let Some(d) = search(ws, j, depth + 1, visited, st) {
            return Some(d);
        }
    }
    None
}

/// The path from the pin at `ws[pin]` to a DAC, if there is one.
pub fn path_from(ws: &[Widget], pin: usize, codec: u8, afg: u8) -> Option<OutputPath> {
    let ws = &ws[..ws.len().min(MAX_WIDGETS)];
    let p = ws.get(pin)?;
    let rank = p.output_rank()?;
    let mut st = Stack { nodes: [0; MAX_PATH], inputs: [0; MAX_PATH] };
    st.nodes[0] = p.nid;
    let mut visited = 1u64 << pin;
    let last = search(ws, pin, 0, &mut visited, &mut st)?;
    let len = last + 1;
    let mut path = OutputPath {
        codec,
        afg,
        dac: st.nodes[last],
        pin: p.nid,
        nodes: [0; MAX_PATH],
        len,
        inputs: [0; MAX_PATH],
        rates: 0,
        sizes: 0,
        max_channels: 0,
        rank,
    };
    // Reverse: signal order, DAC first. nodes[i] = stack[last - i]; its
    // input (towards nodes[i - 1] = stack[last - i + 1]) is
    // stack.inputs[last - i].
    for i in 0..len {
        path.nodes[i] = st.nodes[last - i];
        if i > 0 {
            path.inputs[i] = st.inputs[last - i];
        }
    }
    let dac = &ws[position(ws, path.dac)?];
    path.rates = dac.pcm & PCM_RATES_MASK;
    path.sizes = (dac.pcm >> PCM_SIZES_SHIFT) & PCM_SIZES_MASK;
    path.max_channels = dac.caps.channels();
    Some(path)
}

/// The best output path of one AFG's widget table (see the module docs).
pub fn find_output(ws: &[Widget], codec: u8, afg: u8) -> Option<OutputPath> {
    let ws = &ws[..ws.len().min(MAX_WIDGETS)];
    for rank in 0..=3 {
        for (i, w) in ws.iter().enumerate() {
            if w.output_rank() == Some(rank)
                && let Some(p) = path_from(ws, i, codec, afg)
            {
                return Some(p);
            }
        }
    }
    None
}
