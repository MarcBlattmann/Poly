//! LZMA and the LZMA2 chunk format that xz uses, from the published
//! description of the algorithm: a binary range coder with adaptive
//! probabilities drives a literal / match / repeated-match state machine.

use crate::Error;
use alloc::vec;
use alloc::vec::Vec;

const PROB_INIT: u16 = 1024;
const MATCH_MIN_LEN: usize = 2;
const NUM_STATES: usize = 12;
const POS_STATES_MAX: usize = 16;
const END_POS_MODEL_INDEX: usize = 14;
const NUM_FULL_DISTANCES: usize = 128;
const NUM_ALIGN_BITS: u32 = 4;
const SPEC_POS_LEN: usize = NUM_FULL_DISTANCES - END_POS_MODEL_INDEX + 1;

/// The range decoder over one chunk's bytes.
struct Rc<'a> {
    src: &'a [u8],
    i: usize,
    range: u32,
    code: u32,
}

impl<'a> Rc<'a> {
    fn new(src: &'a [u8]) -> Result<Rc<'a>, Error> {
        if src.len() < 5 || src[0] != 0 {
            return Err(Error::Corrupt);
        }
        let code = u32::from_be_bytes([src[1], src[2], src[3], src[4]]);
        Ok(Rc { src, i: 5, range: 0xFFFF_FFFF, code })
    }

    #[inline]
    fn normalize(&mut self) -> Result<(), Error> {
        if self.range < (1 << 24) {
            let b = *self.src.get(self.i).ok_or(Error::Corrupt)?;
            self.i += 1;
            self.range <<= 8;
            self.code = (self.code << 8) | u32::from(b);
        }
        Ok(())
    }

    #[inline]
    fn bit(&mut self, prob: &mut u16) -> Result<u32, Error> {
        self.normalize()?;
        let bound = (self.range >> 11) * u32::from(*prob);
        if self.code < bound {
            self.range = bound;
            *prob += (2048 - *prob) >> 5;
            Ok(0)
        } else {
            self.range -= bound;
            self.code -= bound;
            *prob -= *prob >> 5;
            Ok(1)
        }
    }

    fn direct(&mut self, count: u32) -> Result<u32, Error> {
        let mut r = 0u32;
        for _ in 0..count {
            self.normalize()?;
            self.range >>= 1;
            self.code = self.code.wrapping_sub(self.range);
            let t = 0u32.wrapping_sub(self.code >> 31);
            self.code = self.code.wrapping_add(self.range & t);
            r = (r << 1).wrapping_add(t.wrapping_add(1));
        }
        Ok(r)
    }

    fn tree(&mut self, probs: &mut [u16], bits: u32) -> Result<u32, Error> {
        let mut m = 1usize;
        for _ in 0..bits {
            m = (m << 1) | self.bit(&mut probs[m])? as usize;
        }
        Ok(m as u32 - (1 << bits))
    }

    fn reverse_tree(&mut self, probs: &mut [u16], bits: u32) -> Result<u32, Error> {
        let mut m = 1usize;
        let mut r = 0u32;
        for i in 0..bits {
            let b = self.bit(&mut probs[m])?;
            m = (m << 1) | b as usize;
            r |= b << i;
        }
        Ok(r)
    }
}

/// The length coder (one for matches, one for repeated matches).
#[derive(Clone, Debug)]
struct LenCoder {
    choice: u16,
    choice2: u16,
    low: Vec<u16>,
    mid: Vec<u16>,
    high: [u16; 256],
}

impl LenCoder {
    fn new() -> LenCoder {
        LenCoder {
            choice: PROB_INIT,
            choice2: PROB_INIT,
            low: vec![PROB_INIT; 8 * POS_STATES_MAX],
            mid: vec![PROB_INIT; 8 * POS_STATES_MAX],
            high: [PROB_INIT; 256],
        }
    }

    /// The length minus [`MATCH_MIN_LEN`].
    fn decode(&mut self, rc: &mut Rc<'_>, pos_state: usize) -> Result<usize, Error> {
        if rc.bit(&mut self.choice)? == 0 {
            return Ok(rc.tree(&mut self.low[pos_state * 8..pos_state * 8 + 8], 3)? as usize);
        }
        if rc.bit(&mut self.choice2)? == 0 {
            return Ok(8 + rc.tree(&mut self.mid[pos_state * 8..pos_state * 8 + 8], 3)? as usize);
        }
        Ok(16 + rc.tree(&mut self.high, 8)? as usize)
    }
}

/// The decoder's adaptive state, which LZMA2 keeps across chunks.
#[derive(Debug)]
pub struct State {
    lc: u32,
    lp: u32,
    pb: u32,
    literal: Vec<u16>,
    is_match: Vec<u16>,
    is_rep: [u16; NUM_STATES],
    is_rep_g0: [u16; NUM_STATES],
    is_rep_g1: [u16; NUM_STATES],
    is_rep_g2: [u16; NUM_STATES],
    is_rep0_long: Vec<u16>,
    pos_slot: Vec<u16>,
    spec_pos: [u16; SPEC_POS_LEN],
    align: [u16; 16],
    len: LenCoder,
    rep_len: LenCoder,
    state: usize,
    reps: [usize; 4],
}

impl State {
    /// A fresh state for the properties `lc`, `lp`, `pb`.
    pub fn new(lc: u32, lp: u32, pb: u32) -> Result<State, Error> {
        if lc > 8 || lp > 4 || pb > 4 {
            return Err(Error::Corrupt);
        }
        Ok(State {
            lc,
            lp,
            pb,
            literal: vec![PROB_INIT; 0x300usize << (lc + lp)],
            is_match: vec![PROB_INIT; NUM_STATES * POS_STATES_MAX],
            is_rep: [PROB_INIT; NUM_STATES],
            is_rep_g0: [PROB_INIT; NUM_STATES],
            is_rep_g1: [PROB_INIT; NUM_STATES],
            is_rep_g2: [PROB_INIT; NUM_STATES],
            is_rep0_long: vec![PROB_INIT; NUM_STATES * POS_STATES_MAX],
            pos_slot: vec![PROB_INIT; 4 * 64],
            spec_pos: [PROB_INIT; SPEC_POS_LEN],
            align: [PROB_INIT; 16],
            len: LenCoder::new(),
            rep_len: LenCoder::new(),
            state: 0,
            reps: [0; 4],
        })
    }

    /// Splits an LZMA properties byte (`(pb * 5 + lp) * 9 + lc`).
    pub fn split_props(b: u8) -> Result<(u32, u32, u32), Error> {
        if b >= 9 * 5 * 5 {
            return Err(Error::Corrupt);
        }
        let b = u32::from(b);
        Ok((b % 9, (b / 9) % 5, b / 45))
    }

    fn distance(&mut self, rc: &mut Rc<'_>, len: usize) -> Result<u32, Error> {
        let len_state = len.min(3);
        let slot = rc.tree(&mut self.pos_slot[len_state * 64..len_state * 64 + 64], 6)?;
        if slot < 4 {
            return Ok(slot);
        }
        let direct = (slot >> 1) - 1;
        let mut dist = (2 | (slot & 1)) << direct;
        if (slot as usize) < END_POS_MODEL_INDEX {
            // Reverse bit tree whose probabilities start just before
            // index `dist - slot` of the special positions.
            let base = (dist - slot) as isize - 1;
            let mut m = 1usize;
            let mut r = 0u32;
            for i in 0..direct {
                let b = rc.bit(&mut self.spec_pos[(base + m as isize) as usize])?;
                m = (m << 1) | b as usize;
                r |= b << i;
            }
            dist += r;
        } else {
            dist = dist.wrapping_add(rc.direct(direct - NUM_ALIGN_BITS)? << NUM_ALIGN_BITS);
            dist = dist.wrapping_add(rc.reverse_tree(&mut self.align, NUM_ALIGN_BITS)?);
        }
        Ok(dist)
    }

    /// Decodes one LZMA chunk from `src` appending exactly `unpacked`
    /// bytes to `out` (the dictionary starts at `dict_start`), or until the
    /// end marker when `unpacked` is `None`.
    pub fn decode(
        &mut self,
        src: &[u8],
        unpacked: Option<usize>,
        out: &mut Vec<u8>,
        dict_start: usize,
        max_out: usize,
    ) -> Result<(), Error> {
        let mut rc = Rc::new(src)?;
        let start = out.len();
        let pb_mask = (1usize << self.pb) - 1;
        let lp_mask = (1usize << self.lp) - 1;
        loop {
            if let Some(n) = unpacked
                && out.len() - start >= n
            {
                return Ok(());
            }
            let pos = out.len() - dict_start;
            let pos_state = pos & pb_mask;
            let st = self.state;
            if rc.bit(&mut self.is_match[st * POS_STATES_MAX + pos_state])? == 0 {
                let prev = if out.len() > dict_start { out[out.len() - 1] } else { 0 };
                let lit_state = ((pos & lp_mask) << self.lc) + (usize::from(prev) >> (8 - self.lc));
                let base = 0x300 * lit_state;
                let probs = &mut self.literal[base..base + 0x300];
                let mut symbol = 1usize;
                if st >= 7 {
                    let dist = self.reps[0] + 1;
                    if dist > out.len() - dict_start {
                        return Err(Error::Corrupt);
                    }
                    let mut match_byte = usize::from(out[out.len() - dist]);
                    let mut offs = 0x100usize;
                    while symbol < 0x100 {
                        match_byte <<= 1;
                        let match_bit = match_byte & offs;
                        let b = rc.bit(&mut probs[offs + match_bit + symbol])?;
                        symbol = (symbol << 1) | b as usize;
                        if b == 0 {
                            offs &= !match_bit;
                        } else {
                            offs &= match_bit;
                        }
                    }
                } else {
                    while symbol < 0x100 {
                        symbol = (symbol << 1) | rc.bit(&mut probs[symbol])? as usize;
                    }
                }
                if out.len() >= max_out {
                    return Err(Error::TooLarge);
                }
                out.push(symbol as u8);
                self.state = if st < 4 {
                    0
                } else if st < 10 {
                    st - 3
                } else {
                    st - 6
                };
                continue;
            }
            let len;
            if rc.bit(&mut self.is_rep[st])? == 0 {
                self.reps[3] = self.reps[2];
                self.reps[2] = self.reps[1];
                self.reps[1] = self.reps[0];
                len = self.len.decode(&mut rc, pos_state)?;
                self.state = if st < 7 { 7 } else { 10 };
                let d = self.distance(&mut rc, len)?;
                if d == 0xFFFF_FFFF {
                    // The end marker.
                    return if unpacked.is_none() { Ok(()) } else { Err(Error::Corrupt) };
                }
                self.reps[0] = d as usize;
            } else {
                if out.len() == dict_start {
                    return Err(Error::Corrupt);
                }
                if rc.bit(&mut self.is_rep_g0[st])? == 0 {
                    if rc.bit(&mut self.is_rep0_long[st * POS_STATES_MAX + pos_state])? == 0 {
                        // A one-byte repeat.
                        self.state = if st < 7 { 9 } else { 11 };
                        let dist = self.reps[0] + 1;
                        if dist > out.len() - dict_start {
                            return Err(Error::Corrupt);
                        }
                        if out.len() >= max_out {
                            return Err(Error::TooLarge);
                        }
                        let b = out[out.len() - dist];
                        out.push(b);
                        continue;
                    }
                } else {
                    let dist;
                    if rc.bit(&mut self.is_rep_g1[st])? == 0 {
                        dist = self.reps[1];
                    } else {
                        if rc.bit(&mut self.is_rep_g2[st])? == 0 {
                            dist = self.reps[2];
                        } else {
                            dist = self.reps[3];
                            self.reps[3] = self.reps[2];
                        }
                        self.reps[2] = self.reps[1];
                    }
                    self.reps[1] = self.reps[0];
                    self.reps[0] = dist;
                }
                len = self.rep_len.decode(&mut rc, pos_state)?;
                self.state = if st < 7 { 8 } else { 11 };
            }
            let total = len + MATCH_MIN_LEN;
            let dist = self.reps[0] + 1;
            if dist > out.len() - dict_start {
                return Err(Error::Corrupt);
            }
            let limit = unpacked.map_or(usize::MAX, |n| start + n);
            if out.len() + total > limit.min(max_out) {
                return Err(if out.len() + total > max_out { Error::TooLarge } else { Error::Corrupt });
            }
            let from = out.len() - dist;
            for k in 0..total {
                let b = out[from + k];
                out.push(b);
            }
        }
    }
}

/// Decodes LZMA2 chunks (the payload of an xz block, filter 0x21) from
/// `src`; returns the decoded bytes and how many input bytes the chunks
/// took, end marker included. `dict_size` is the dictionary the stream
/// declares (not enforced beyond the memory limit `max_out`).
pub fn lzma2(src: &[u8], max_out: usize) -> Result<(Vec<u8>, usize), Error> {
    let mut out: Vec<u8> = Vec::with_capacity(max_out.min(1 << 20));
    let mut i = 0usize;
    let mut dict_start = 0usize;
    let mut state: Option<State> = None;
    let mut need_dict_reset = true;
    let mut need_props = true;
    loop {
        let control = *src.get(i).ok_or(Error::Corrupt)?;
        i += 1;
        if control == 0 {
            return Ok((out, i));
        }
        if control < 0x80 {
            // Uncompressed chunk: 1 resets the dictionary, 2 does not.
            if control > 2 {
                return Err(Error::Corrupt);
            }
            if control == 1 {
                dict_start = out.len();
                need_dict_reset = false;
            } else if need_dict_reset {
                return Err(Error::Corrupt);
            }
            let hdr = src.get(i..i + 2).ok_or(Error::Corrupt)?;
            let n = (usize::from(hdr[0]) << 8 | usize::from(hdr[1])) + 1;
            i += 2;
            let data = src.get(i..i + n).ok_or(Error::Corrupt)?;
            if out.len() + n > max_out {
                return Err(Error::TooLarge);
            }
            out.extend_from_slice(data);
            i += n;
            continue;
        }
        let hdr = src.get(i..i + 4).ok_or(Error::Corrupt)?;
        let unpacked = ((usize::from(control & 0x1f) << 16) | (usize::from(hdr[0]) << 8) | usize::from(hdr[1])) + 1;
        let packed = (usize::from(hdr[2]) << 8 | usize::from(hdr[3])) + 1;
        i += 4;
        let reset = (control >> 5) & 3;
        if reset == 3 {
            dict_start = out.len();
            need_dict_reset = false;
        } else if need_dict_reset {
            return Err(Error::Corrupt);
        }
        if reset >= 2 {
            let p = *src.get(i).ok_or(Error::Corrupt)?;
            i += 1;
            let (lc, lp, pb) = State::split_props(p)?;
            if lc + lp > 4 {
                return Err(Error::Corrupt);
            }
            state = Some(State::new(lc, lp, pb)?);
            need_props = false;
        } else if need_props {
            return Err(Error::Corrupt);
        } else if reset == 1 {
            let s = state.as_ref().ok_or(Error::Corrupt)?;
            state = Some(State::new(s.lc, s.lp, s.pb)?);
        }
        let chunk = src.get(i..i + packed).ok_or(Error::Corrupt)?;
        let s = state.as_mut().ok_or(Error::Corrupt)?;
        s.decode(chunk, Some(unpacked), &mut out, dict_start, max_out)?;
        i += packed;
    }
}
