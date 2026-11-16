//! The immediate context (ADR 0211): what it binds (ID3D11DeviceContext.PSSetShaderResources.yaml,
//! OMSetRenderTargets.yaml), and the work it does at once: maps, copies, updates, clears, mip
//! generation, resolves and queries. Draws and dispatches change nothing (no shader can be made,
//! ID3D11DeviceContext.Draw.yaml). Bindings hold hidden references (`texture::hold`): the public
//! counts of the bound objects do not change.

use crate::com::*;
use crate::device::{Device, device_of};
use crate::objects;
use crate::texture::{self, Kind, Tex, ViewKind};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::Ordering;
use poly_d3d::{clear, format, resource, view};

const STAGES: usize = 6;
pub const SRV_SLOTS: usize = 128;
pub const SAMPLER_SLOTS: usize = 16;
pub const CB_SLOTS: usize = 14;
pub const VB_SLOTS: usize = 32;

type P = *mut c_void;

/// The context's bindings.
pub struct State {
    srv: [[P; SRV_SLOTS]; STAGES],
    sampler: [[P; SAMPLER_SLOTS]; STAGES],
    cb: [[P; CB_SLOTS]; STAGES],
    shader: [P; STAGES],
    vb: [(P, u32, u32); VB_SLOTS],
    ib: (P, u32, u32),
    layout: P,
    topology: u32,
    rtv: [P; 8],
    dsv: P,
    blend: (P, [f32; 4], u32),
    depth: (P, u32),
    raster: P,
    viewports: Vec<[f32; 6]>,
    scissors: Vec<[i32; 4]>,
    predicate: (P, i32),
}

// SAFETY: raw pointers of objects of this DLL, reached with the device's lock held.
unsafe impl Send for State {}

impl State {
    pub fn new() -> State {
        let n = core::ptr::null_mut();
        State {
            srv: [[n; SRV_SLOTS]; STAGES],
            sampler: [[n; SAMPLER_SLOTS]; STAGES],
            cb: [[n; CB_SLOTS]; STAGES],
            shader: [n; STAGES],
            vb: [(n, 0, 0); VB_SLOTS],
            ib: (n, 0, 0),
            layout: n,
            topology: 0,
            rtv: [n; 8],
            dsv: n,
            blend: (n, [1.0; 4], u32::MAX),
            depth: (n, 0),
            raster: n,
            viewports: Vec::new(),
            scissors: Vec::new(),
            predicate: (n, 0),
        }
    }

    /// Every bound object (to drop the hidden references).
    fn all(&self) -> Vec<P> {
        let mut v = Vec::new();
        for s in 0..STAGES {
            v.extend(self.srv[s].iter().chain(&self.sampler[s]).chain(&self.cb[s]).copied());
            v.push(self.shader[s]);
        }
        v.extend(self.vb.iter().map(|b| b.0));
        v.extend([self.ib.0, self.layout, self.dsv, self.blend.0, self.depth.0, self.raster, self.predicate.0]);
        v.extend(self.rtv.iter().copied());
        v.retain(|p| !p.is_null());
        v
    }
}

impl Default for State {
    fn default() -> Self {
        State::new()
    }
}

/// Replace a binding: a hidden reference on the new object, the old one's dropped.
unsafe fn set(slot: &mut P, p: P) {
    if *slot == p {
        return;
    }
    if !p.is_null() {
        // SAFETY: a live object of this DLL.
        unsafe { texture::hold(p) };
    }
    let old = core::mem::replace(slot, p);
    if !old.is_null() {
        // SAFETY: the hidden reference this binding held.
        unsafe { texture::unhold(old) };
    }
}

/// A binding returned to the caller: a new public reference.
unsafe fn give(p: P) -> P {
    if !p.is_null() {
        // SAFETY: a live bound object.
        unsafe { add_ref(p) };
    }
    p
}

/// The device of a context (`this` is the context part).
unsafe fn dev<'a>(this: P) -> &'a Device {
    // SAFETY: the context is a part of a live device.
    unsafe { device_of(crate::com::part_owner(this)) }
}

unsafe fn with_state<R>(this: P, f: impl FnOnce(&mut State) -> R) -> R {
    // SAFETY: a live context.
    unsafe { dev(this) }.pipe.with(f)
}

/// The caller's array of `n` pointers.
unsafe fn slice<'a, T>(p: *const T, n: u32) -> &'a [T] {
    if p.is_null() || n == 0 {
        &[]
    } else {
        // SAFETY: the caller's array of `n` elements.
        unsafe { core::slice::from_raw_parts(p, n as usize) }
    }
}

/// The resource a shader-resource or render-target view sees.
unsafe fn view_res(p: P) -> P {
    // SAFETY: as `texture::view`.
    unsafe { texture::view(p, ViewKind::Srv).or_else(|| texture::view(p, ViewKind::Rtv)) }.map_or(core::ptr::null_mut(), |v| v.res)
}

// ----- the shader stages: VS 0, HS 1, DS 2, GS 3, PS 4, CS 5

unsafe fn set_srvs(this: P, stage: usize, start: u32, n: u32, v: *const P) {
    // SAFETY: the caller's array.
    let views = unsafe { slice(v, n) };
    // SAFETY: a live context.
    unsafe {
        with_state(this, |s| {
            for (i, &p) in views.iter().enumerate() {
                let slot = start as usize + i;
                if slot >= SRV_SLOTS {
                    break;
                }
                // A resource bound as a render target is not bound for reading (the hazard rule).
                let r = view_res(p);
                let p = if !r.is_null() && s.rtv.iter().any(|&t| !t.is_null() && view_res(t) == r) { core::ptr::null_mut() } else { p };
                set(&mut s.srv[stage][slot], p);
            }
        })
    }
}

unsafe fn get_list<const N: usize>(list: &[P; N], start: u32, n: u32, out: *mut P) {
    if out.is_null() {
        return;
    }
    for i in 0..n as usize {
        let slot = start as usize + i;
        let p = if slot < N { list[slot] } else { core::ptr::null_mut() };
        // SAFETY: the caller's array of `n` outputs; a bound object.
        unsafe { *out.add(i) = give(p) };
    }
}

unsafe fn set_list<const N: usize>(list: &mut [P; N], start: u32, n: u32, v: *const P) {
    // SAFETY: the caller's array.
    for (i, &p) in unsafe { slice(v, n) }.iter().enumerate() {
        if start as usize + i < N {
            // SAFETY: a binding of a live object.
            unsafe { set(&mut list[start as usize + i], p) };
        }
    }
}

macro_rules! stage_methods {
    ($stage:expr, $set_srv:ident, $get_srv:ident, $set_smp:ident, $get_smp:ident, $set_cb:ident, $get_cb:ident, $set_sh:ident, $get_sh:ident) => {
        pub unsafe extern "system" fn $set_srv(this: P, start: u32, n: u32, v: *const P) {
            // SAFETY: the caller's arguments.
            unsafe { set_srvs(this, $stage, start, n, v) }
        }
        pub unsafe extern "system" fn $get_srv(this: P, start: u32, n: u32, out: *mut P) {
            // SAFETY: the caller's arguments.
            unsafe { with_state(this, |s| get_list(&s.srv[$stage], start, n, out)) }
        }
        pub unsafe extern "system" fn $set_smp(this: P, start: u32, n: u32, v: *const P) {
            // SAFETY: the caller's arguments.
            unsafe { with_state(this, |s| set_list(&mut s.sampler[$stage], start, n, v)) }
        }
        pub unsafe extern "system" fn $get_smp(this: P, start: u32, n: u32, out: *mut P) {
            // SAFETY: the caller's arguments.
            unsafe { with_state(this, |s| get_list(&s.sampler[$stage], start, n, out)) }
        }
        pub unsafe extern "system" fn $set_cb(this: P, start: u32, n: u32, v: *const P) {
            // SAFETY: the caller's arguments.
            unsafe { with_state(this, |s| set_list(&mut s.cb[$stage], start, n, v)) }
        }
        pub unsafe extern "system" fn $get_cb(this: P, start: u32, n: u32, out: *mut P) {
            // SAFETY: the caller's arguments.
            unsafe { with_state(this, |s| get_list(&s.cb[$stage], start, n, out)) }
        }
        pub unsafe extern "system" fn $set_sh(this: P, sh: P, _: *const P, _: u32) {
            // SAFETY: the caller's arguments.
            unsafe { with_state(this, |s| set(&mut s.shader[$stage], sh)) }
        }
        pub unsafe extern "system" fn $get_sh(this: P, out: *mut P, _: *mut P, n: *mut u32) {
            // SAFETY: the caller's outputs.
            unsafe {
                with_state(this, |s| put(out, give(s.shader[$stage])));
                put(n, 0);
            }
        }
    };
}

stage_methods!(0, vs_set_srv, vs_get_srv, vs_set_smp, vs_get_smp, vs_set_cb, vs_get_cb, vs_set_sh, vs_get_sh);
stage_methods!(1, hs_set_srv, hs_get_srv, hs_set_smp, hs_get_smp, hs_set_cb, hs_get_cb, hs_set_sh, hs_get_sh);
stage_methods!(2, ds_set_srv, ds_get_srv, ds_set_smp, ds_get_smp, ds_set_cb, ds_get_cb, ds_set_sh, ds_get_sh);
stage_methods!(3, gs_set_srv, gs_get_srv, gs_set_smp, gs_get_smp, gs_set_cb, gs_get_cb, gs_set_sh, gs_get_sh);
stage_methods!(4, ps_set_srv, ps_get_srv, ps_set_smp, ps_get_smp, ps_set_cb, ps_get_cb, ps_set_sh, ps_get_sh);
stage_methods!(5, cs_set_srv, cs_get_srv, cs_set_smp, cs_get_smp, cs_set_cb, cs_get_cb, cs_set_sh, cs_get_sh);

/// `XSSetConstantBuffers1(Start, Num, ppBuffers, pFirstConstant, pNumConstants)`: the ranges are
/// not used (no shader reads them).
macro_rules! cb1 {
    ($set:ident, $get:ident, $s:ident, $g:ident) => {
        pub unsafe extern "system" fn $set(this: P, start: u32, n: u32, v: *const P, _: *const u32, _: *const u32) {
            // SAFETY: the caller's arguments.
            unsafe { $s(this, start, n, v) }
        }
        pub unsafe extern "system" fn $get(this: P, start: u32, n: u32, out: *mut P, first: *mut u32, num: *mut u32) {
            // SAFETY: the caller's arguments.
            unsafe { $g(this, start, n, out) };
            for i in 0..n as usize {
                // SAFETY: the caller's optional arrays of `n`.
                unsafe {
                    if !first.is_null() {
                        *first.add(i) = 0;
                    }
                    if !num.is_null() {
                        *num.add(i) = 4096;
                    }
                }
            }
        }
    };
}

cb1!(vs_set_cb1, vs_get_cb1, vs_set_cb, vs_get_cb);
cb1!(hs_set_cb1, hs_get_cb1, hs_set_cb, hs_get_cb);
cb1!(ds_set_cb1, ds_get_cb1, ds_set_cb, ds_get_cb);
cb1!(gs_set_cb1, gs_get_cb1, gs_set_cb, gs_get_cb);
cb1!(ps_set_cb1, ps_get_cb1, ps_set_cb, ps_get_cb);
cb1!(cs_set_cb1, cs_get_cb1, cs_set_cb, cs_get_cb);

// ----- input assembler

pub unsafe extern "system" fn ia_set_input_layout(this: P, l: P) {
    // SAFETY: the caller's arguments.
    unsafe { with_state(this, |s| set(&mut s.layout, l)) }
}

pub unsafe extern "system" fn ia_get_input_layout(this: P, out: *mut P) {
    // SAFETY: the caller's output.
    unsafe { with_state(this, |s| put(out, give(s.layout))) }
}

pub unsafe extern "system" fn ia_set_vertex_buffers(this: P, start: u32, n: u32, v: *const P, strides: *const u32, offs: *const u32) {
    // SAFETY: the caller's arrays of `n`.
    unsafe {
        let (v, st, of) = (slice(v, n), slice(strides, n), slice(offs, n));
        with_state(this, |s| {
            for (i, &p) in v.iter().enumerate() {
                let slot = start as usize + i;
                if slot < VB_SLOTS {
                    set(&mut s.vb[slot].0, p);
                    s.vb[slot].1 = st.get(i).copied().unwrap_or(0);
                    s.vb[slot].2 = of.get(i).copied().unwrap_or(0);
                }
            }
        })
    }
}

pub unsafe extern "system" fn ia_get_vertex_buffers(this: P, start: u32, n: u32, out: *mut P, strides: *mut u32, offs: *mut u32) {
    // SAFETY: the caller's optional arrays of `n`.
    unsafe {
        with_state(this, |s| {
            for i in 0..n as usize {
                let slot = start as usize + i;
                let b = if slot < VB_SLOTS { s.vb[slot] } else { (core::ptr::null_mut(), 0, 0) };
                if !out.is_null() {
                    *out.add(i) = give(b.0);
                }
                if !strides.is_null() {
                    *strides.add(i) = b.1;
                }
                if !offs.is_null() {
                    *offs.add(i) = b.2;
                }
            }
        })
    }
}

pub unsafe extern "system" fn ia_set_index_buffer(this: P, b: P, f: u32, off: u32) {
    // SAFETY: the caller's arguments.
    unsafe {
        with_state(this, |s| {
            set(&mut s.ib.0, b);
            s.ib.1 = if b.is_null() { 0 } else { f };
            s.ib.2 = if b.is_null() { 0 } else { off };
        })
    }
}

pub unsafe extern "system" fn ia_get_index_buffer(this: P, out: *mut P, f: *mut u32, off: *mut u32) {
    // SAFETY: the caller's optional outputs.
    unsafe {
        with_state(this, |s| {
            put(out, give(s.ib.0));
            put(f, s.ib.1);
            put(off, s.ib.2);
        })
    }
}

pub unsafe extern "system" fn ia_set_topology(this: P, t: u32) {
    // SAFETY: a live context.
    unsafe { with_state(this, |s| s.topology = t) }
}

pub unsafe extern "system" fn ia_get_topology(this: P, out: *mut u32) {
    // SAFETY: the caller's output.
    unsafe { with_state(this, |s| put(out, s.topology)) }
}

// ----- output merger and rasterizer

pub unsafe extern "system" fn om_set_render_targets(this: P, n: u32, v: *const P, dsv: P) {
    // SAFETY: the caller's array.
    let views = unsafe { slice(v, n.min(8)) };
    // The same view twice, or two views of one resource, bind nothing (OMSetRenderTargets.yaml
    // `refused_bindings`).
    let mut seen: Vec<P> = Vec::new();
    let mut refused = false;
    for &p in views {
        if p.is_null() {
            continue;
        }
        // SAFETY: a view of this DLL.
        let r = unsafe { view_res(p) };
        if seen.contains(&p) || seen.contains(&r) {
            refused = true;
        }
        seen.push(p);
        seen.push(r);
    }
    // SAFETY: a live context; bindings of live objects.
    unsafe {
        with_state(this, |s| {
            for i in 0..8 {
                let p = if refused { core::ptr::null_mut() } else { views.get(i).copied().unwrap_or(core::ptr::null_mut()) };
                set(&mut s.rtv[i], p);
                // Unbind the shader-resource views of a resource now written.
                if !p.is_null() {
                    let r = view_res(p);
                    for st in 0..STAGES {
                        for slot in 0..SRV_SLOTS {
                            let b = s.srv[st][slot];
                            if !b.is_null() && view_res(b) == r {
                                set(&mut s.srv[st][slot], core::ptr::null_mut());
                            }
                        }
                    }
                }
            }
            set(&mut s.dsv, if refused { core::ptr::null_mut() } else { dsv });
        })
    }
}

pub unsafe extern "system" fn om_set_rt_and_uavs(this: P, n: u32, v: *const P, dsv: P, _: u32, _: u32, _: *const P, _: *const u32) {
    // D3D11_KEEP_RENDER_TARGETS_AND_DEPTH_STENCIL (0xffffffff) keeps the targets.
    if n != u32::MAX {
        // SAFETY: the caller's arguments.
        unsafe { om_set_render_targets(this, n, v, dsv) };
    }
}

pub unsafe extern "system" fn om_get_render_targets(this: P, n: u32, out: *mut P, dsv: *mut P) {
    // SAFETY: the caller's optional outputs.
    unsafe {
        with_state(this, |s| {
            get_list(&s.rtv, 0, n, out);
            put(dsv, give(s.dsv));
        })
    }
}

pub unsafe extern "system" fn om_get_rt_and_uavs(this: P, n: u32, out: *mut P, dsv: *mut P, _: u32, nu: u32, uavs: *mut P) {
    // SAFETY: the caller's optional outputs.
    unsafe {
        om_get_render_targets(this, n, out, dsv);
        for i in 0..nu as usize {
            if !uavs.is_null() {
                *uavs.add(i) = core::ptr::null_mut();
            }
        }
    }
}

pub unsafe extern "system" fn om_set_blend_state(this: P, b: P, factor: *const [f32; 4], mask: u32) {
    // SAFETY: the caller's arguments.
    unsafe {
        let f = if factor.is_null() { [1.0; 4] } else { *factor };
        with_state(this, |s| {
            set(&mut s.blend.0, b);
            s.blend.1 = f;
            s.blend.2 = mask;
        })
    }
}

pub unsafe extern "system" fn om_get_blend_state(this: P, out: *mut P, factor: *mut [f32; 4], mask: *mut u32) {
    // SAFETY: the caller's optional outputs.
    unsafe {
        with_state(this, |s| {
            put(out, give(s.blend.0));
            put(factor, s.blend.1);
            put(mask, s.blend.2);
        })
    }
}

pub unsafe extern "system" fn om_set_depth_state(this: P, d: P, r: u32) {
    // SAFETY: the caller's arguments.
    unsafe {
        with_state(this, |s| {
            set(&mut s.depth.0, d);
            s.depth.1 = r & 0xFF;
        })
    }
}

pub unsafe extern "system" fn om_get_depth_state(this: P, out: *mut P, r: *mut u32) {
    // SAFETY: the caller's optional outputs.
    unsafe {
        with_state(this, |s| {
            put(out, give(s.depth.0));
            put(r, s.depth.1);
        })
    }
}

pub unsafe extern "system" fn rs_set_state(this: P, r: P) {
    // SAFETY: the caller's arguments.
    unsafe { with_state(this, |s| set(&mut s.raster, r)) }
}

pub unsafe extern "system" fn rs_get_state(this: P, out: *mut P) {
    // SAFETY: the caller's output.
    unsafe { with_state(this, |s| put(out, give(s.raster))) }
}

/// `RSSetViewports`: more than 16, or any viewport with a NaN or infinite value, a negative size,
/// or depths outside [0, 1] or reversed, is ignored entirely.
pub unsafe extern "system" fn rs_set_viewports(this: P, n: u32, v: *const [f32; 6]) {
    // SAFETY: the caller's array.
    let vps = unsafe { slice(v, n) };
    if n > 16 || (n > 0 && vps.is_empty()) {
        return;
    }
    let ok = vps.iter().all(|v| {
        v.iter().all(|x| x.is_finite()) && v[2] >= 0.0 && v[3] >= 0.0 && (0.0..=1.0).contains(&v[4]) && (0.0..=1.0).contains(&v[5]) && v[4] <= v[5]
    });
    if ok {
        // SAFETY: a live context.
        unsafe { with_state(this, |s| s.viewports = vps.to_vec()) };
    }
}

pub unsafe extern "system" fn rs_set_scissors(this: P, n: u32, v: *const [i32; 4]) {
    // SAFETY: the caller's array.
    let r = unsafe { slice(v, n) };
    if n > 16 || (n > 0 && r.is_empty()) || r.iter().any(|r| r[2] < r[0] || r[3] < r[1]) {
        return;
    }
    // SAFETY: a live context.
    unsafe { with_state(this, |s| s.scissors = r.to_vec()) };
}

unsafe fn get_rects<T: Copy + Default>(list: &[T], n: *mut u32, out: *mut T) {
    if n.is_null() {
        return;
    }
    // SAFETY: the caller's count.
    let room = unsafe { *n } as usize;
    if out.is_null() {
        // SAFETY: as above.
        unsafe { *n = list.len() as u32 };
        return;
    }
    for i in 0..room {
        // SAFETY: the caller's array of `room`.
        unsafe { *out.add(i) = list.get(i).copied().unwrap_or_default() };
    }
    // SAFETY: as above.
    unsafe { *n = list.len().min(room) as u32 };
}

pub unsafe extern "system" fn rs_get_viewports(this: P, n: *mut u32, out: *mut [f32; 6]) {
    // SAFETY: the caller's arguments.
    unsafe { with_state(this, |s| get_rects(&s.viewports, n, out)) }
}

pub unsafe extern "system" fn rs_get_scissors(this: P, n: *mut u32, out: *mut [i32; 4]) {
    // SAFETY: the caller's arguments.
    unsafe { with_state(this, |s| get_rects(&s.scissors, n, out)) }
}

pub unsafe extern "system" fn set_predication(this: P, p: P, v: i32) {
    // SAFETY: the caller's arguments.
    unsafe {
        with_state(this, |s| {
            set(&mut s.predicate.0, p);
            s.predicate.1 = i32::from(v != 0);
        })
    }
}

pub unsafe extern "system" fn get_predication(this: P, out: *mut P, v: *mut i32) {
    // SAFETY: the caller's optional outputs.
    unsafe {
        with_state(this, |s| {
            put(out, give(s.predicate.0));
            put(v, s.predicate.1);
        })
    }
}

/// `ClearState`: every binding back to the default.
pub unsafe extern "system" fn clear_state(this: P) {
    // SAFETY: a live context.
    let old = unsafe { with_state(this, |s| core::mem::take(s)) };
    for p in old.all() {
        // SAFETY: the hidden references the bindings held.
        unsafe { texture::unhold(p) };
    }
}

/// Drop every binding (the device goes).
pub fn release_all(s: &mut State) {
    let old = core::mem::take(s);
    for p in old.all() {
        // SAFETY: the hidden references the bindings held.
        unsafe { texture::unhold(p) };
    }
}

// ----- draws: nothing is drawn without shaders

pub unsafe extern "system" fn draw2(_: P, _: u32, _: u32) {}
pub unsafe extern "system" fn draw3(_: P, _: u32, _: u32, _: i32) {}
pub unsafe extern "system" fn draw4(_: P, _: u32, _: u32, _: u32, _: u32) {}
pub unsafe extern "system" fn draw5(_: P, _: u32, _: u32, _: u32, _: i32, _: u32) {}
pub unsafe extern "system" fn draw0(_: P) {}
pub unsafe extern "system" fn draw_indirect(_: P, _: P, _: u32) {}
pub unsafe extern "system" fn dispatch(_: P, _: u32, _: u32, _: u32) {}
pub unsafe extern "system" fn so_set_targets(_: P, _: u32, _: *const P, _: *const u32) {}
pub unsafe extern "system" fn so_get_targets(_: P, n: u32, out: *mut P) {
    for i in 0..n as usize {
        if !out.is_null() {
            // SAFETY: the caller's array of `n`.
            unsafe { *out.add(i) = core::ptr::null_mut() };
        }
    }
}

// ----- Map / Unmap

/// `D3D11_MAP` (d3d11.h) and `D3D11_MAP_FLAG_DO_NOT_WAIT`.
const MAP_READ: u32 = 1;
const MAP_WRITE_NO_OVERWRITE: u32 = 5;
const MAP_DO_NOT_WAIT: u32 = 0x10_0000;

/// `Map(pResource, Subresource, MapType, MapFlags, pMappedResource)` (ID3D11DeviceContext.Map.yaml).
pub unsafe extern "system" fn map(this: P, res: P, sub: u32, kind: u32, flags: u32, out: *mut u8) -> HRESULT {
    let fail = |hr: HRESULT| {
        if !out.is_null() {
            // SAFETY: the caller's D3D11_MAPPED_SUBRESOURCE: only pData is cleared (Map.yaml:
            // E_INVALIDARG with a NULL data pointer; the pitches are left as they were).
            unsafe { (out as *mut *mut u8).write(core::ptr::null_mut()) };
        }
        hr
    };
    // SAFETY: a live context.
    if unsafe { dev(this) }.removed.load(Ordering::Acquire) {
        return fail(DXGI_ERROR_DEVICE_REMOVED);
    }
    // SAFETY: the caller's resource.
    let Some(t) = (unsafe { texture::res(res) }) else { return fail(E_INVALIDARG) };
    if out.is_null() || flags & !MAP_DO_NOT_WAIT != 0 || !(MAP_READ..=MAP_WRITE_NO_OVERWRITE).contains(&kind) {
        return fail(E_INVALIDARG);
    }
    let (usage, cpu) = match &t.kind {
        Kind::Buffer(b) => (b.usage, b.cpu),
        Kind::Tex2d(d) => (d.usage, d.cpu),
    };
    let allowed = match usage {
        resource::usage::STAGING => (1..=3).contains(&kind) && (kind != 1 || cpu & resource::cpu::READ != 0) && (kind == 1 || cpu & resource::cpu::WRITE != 0),
        resource::usage::DYNAMIC => kind == 4 || (kind == 5 && t.is_buffer()),
        _ => false,
    };
    if !allowed {
        return fail(E_INVALIDARG);
    }
    let Some((bytes, s)) = t.sub_bytes(sub) else { return fail(E_INVALIDARG) };
    // SAFETY: the caller's D3D11_MAPPED_SUBRESOURCE: pData, RowPitch, DepthPitch.
    unsafe {
        (out as *mut *mut u8).write(bytes.as_mut_ptr());
        (out.add(8) as *mut u32).write(s.pitch as u32);
        (out.add(12) as *mut u32).write((s.pitch * s.rows) as u32);
    }
    S_OK
}

pub unsafe extern "system" fn unmap(_: P, _: P, _: u32) {}

// ----- copies

/// Remove the device (an invalid box on WARP, CopyResource.yaml `invalid_boxes_remove_device`).
fn remove(d: &Device) {
    if !d.removed.swap(true, Ordering::AcqRel) {
        log("d3d11: device removed (an invalid copy box)");
    }
}

/// `CopyResource(pDstResource, pSrcResource)`: every subresource.
pub unsafe extern "system" fn copy_resource(this: P, dst: P, src: P) {
    // SAFETY: the caller's resources.
    let (Some(d), Some(s)) = (unsafe { texture::res(dst) }, unsafe { texture::res(src) }) else { return };
    if !texture::unlocked(s) || !texture::unlocked(d) {
        return;
    }
    let _ = this;
    if d.is_buffer() != s.is_buffer() {
        return;
    }
    if !d.is_buffer() && !texture::same_family(d.format(), s.format()) {
        return;
    }
    let n = d.subresources().min(s.subresources());
    for i in 0..n {
        texture::copy_region(d, i, 0, 0, s, i, 0, 0, usize::MAX, usize::MAX);
    }
}

/// `CopySubresourceRegion(pDst, DstSubresource, DstX, DstY, DstZ, pSrc, SrcSubresource, pSrcBox)`.
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn copy_subresource_region(this: P, dst: P, ds: u32, dx: u32, dy: u32, dz: u32, src: P, ss: u32, bx: *const [u32; 6]) {
    // SAFETY: the caller's resources.
    let (Some(d), Some(s)) = (unsafe { texture::res(dst) }, unsafe { texture::res(src) }) else { return };
    // SAFETY: a live context.
    let device = unsafe { dev(this) };
    let (Some(sd), Some(dd)) = (s.sub(ss), d.sub(ds)) else { return };
    let (sx, sy, w, h) = if bx.is_null() {
        (0, 0, sd.width, sd.height)
    } else {
        // SAFETY: the caller's D3D11_BOX: left, top, front, right, bottom, back.
        let b = unsafe { *bx };
        if b[3] < b[0] || b[4] < b[1] || b[5] < b[2] {
            remove(device);
            return;
        }
        if b[0] == b[3] || b[1] == b[4] || b[2] == b[5] {
            return;
        }
        if b[3] > sd.width || b[4] > sd.height || b[2] != 0 || b[5] != 1 {
            remove(device);
            return;
        }
        (b[0], b[1], b[3] - b[0], b[4] - b[1])
    };
    if !bx.is_null() && (dx + w > dd.width || dy + h > dd.height || dz != 0) {
        remove(device);
        return;
    }
    if !texture::unlocked(s) || !texture::unlocked(d) {
        return;
    }
    texture::copy_region(d, ds, dx as usize, dy as usize, s, ss, sx as usize, sy as usize, w as usize, h as usize);
}

/// `UpdateSubresource(pDstResource, DstSubresource, pDstBox, pSrcData, SrcRowPitch, SrcDepthPitch)`.
pub unsafe extern "system" fn update_subresource(this: P, dst: P, sub: u32, bx: *const [u32; 6], data: *const u8, pitch: u32, _: u32) {
    // SAFETY: the caller's resource.
    let Some(t) = (unsafe { texture::res(dst) }) else { return };
    // SAFETY: a live context.
    let device = unsafe { dev(this) };
    let Some(s) = t.sub(sub) else { return };
    if data.is_null() {
        return;
    }
    let (x, y, w, h) = if bx.is_null() {
        (0, 0, s.width, s.height)
    } else {
        // SAFETY: the caller's box.
        let b = unsafe { *bx };
        if b[0] == b[3] || b[1] == b[4] {
            return;
        }
        if b[3] < b[0] || b[4] < b[1] || b[3] > s.width || b[4] > s.height {
            remove(device);
            return;
        }
        (b[0], b[1], b[3] - b[0], b[4] - b[1])
    };
    if !texture::unlocked(t) {
        return;
    }
    let bytes = t.data();
    if t.is_buffer() {
        // SAFETY: the caller's `w` bytes.
        let from = unsafe { core::slice::from_raw_parts(data, w as usize) };
        bytes[x as usize..(x + w) as usize].copy_from_slice(from);
        return;
    }
    let (row, _) = format::pitch_rows(t.format(), w, 1);
    let bpp = row / w.max(1);
    for r in 0..h as usize {
        // SAFETY: the caller's rows, `pitch` apart, of `row` bytes.
        let from = unsafe { core::slice::from_raw_parts(data.add(r * pitch as usize), row as usize) };
        let at = s.offset + (y as usize + r) * s.pitch + x as usize * bpp as usize;
        bytes[at..at + row as usize].copy_from_slice(from);
    }
}

pub unsafe extern "system" fn update_subresource1(this: P, dst: P, sub: u32, bx: *const [u32; 6], data: *const u8, pitch: u32, dp: u32, _: u32) {
    // SAFETY: the caller's arguments.
    unsafe { update_subresource(this, dst, sub, bx, data, pitch, dp) }
}

#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn copy_subresource_region1(this: P, dst: P, ds: u32, dx: u32, dy: u32, dz: u32, src: P, ss: u32, bx: *const [u32; 6], _: u32) {
    // SAFETY: the caller's arguments.
    unsafe { copy_subresource_region(this, dst, ds, dx, dy, dz, src, ss, bx) }
}

/// `ResolveSubresource`: the texture keeps one sample per texel, so a resolve is a copy.
pub unsafe extern "system" fn resolve_subresource(this: P, dst: P, ds: u32, src: P, ss: u32, _: u32) {
    // SAFETY: the caller's arguments.
    unsafe { copy_subresource_region(this, dst, ds, 0, 0, 0, src, ss, core::ptr::null()) }
}

// ----- clears

/// The subresources a view covers: (mip, first slice, slices) of a texture.
fn subs_of(t: &Tex, mip: u32, first: u32, n: u32) -> Vec<u32> {
    let Some(d) = t.desc() else { return alloc::vec![0] };
    (first..first + n).map(|s| mip + s * d.mips).collect()
}

/// Fill subresource `sub` of `t` with the texel `px`.
fn fill(t: &Tex, sub: u32, px: &[u8]) {
    let Some((bytes, s)) = t.sub_bytes(sub) else { return };
    let w = s.width as usize * px.len();
    for r in 0..s.rows {
        for chunk in bytes[r * s.pitch..r * s.pitch + w].chunks_exact_mut(px.len()) {
            chunk.copy_from_slice(px);
        }
    }
}

/// `ClearRenderTargetView(pRenderTargetView, ColorRGBA)`.
pub unsafe extern "system" fn clear_render_target_view(_: P, rtv: P, rgba: *const [f32; 4]) {
    // SAFETY: the caller's view.
    let Some(v) = (unsafe { texture::view(rtv, ViewKind::Rtv) }) else { return };
    // SAFETY: the view's resource.
    let Some(t) = (unsafe { texture::res(v.res) }) else { return };
    if rgba.is_null() || !texture::unlocked(t) {
        return;
    }
    // SAFETY: the caller's four floats.
    let c = unsafe { *rgba };
    let Some(px) = clear::color_texel(v.desc[0], c) else {
        log("stub: d3d11: ClearRenderTargetView of a format without an 8-bit clear");
        return;
    };
    let d: [u32; 5] = [v.desc[0], v.desc[1], v.desc[2], v.desc[3], v.desc[4]];
    let (mip, first, n) = view::rtv_target(&d);
    for sub in subs_of(t, mip, first, n) {
        fill(t, sub, &px);
    }
}

/// `ClearView(pView, Color, pRect, NumRects)` of a render target view: the whole view.
pub unsafe extern "system" fn clear_view(this: P, v: P, rgba: *const [f32; 4], _: *const [i32; 4], _: u32) {
    // SAFETY: the caller's arguments.
    unsafe { clear_render_target_view(this, v, rgba) }
}

/// `ClearDepthStencilView(pDepthStencilView, ClearFlags, Depth, Stencil)`: the planes the view
/// does not make read-only.
pub unsafe extern "system" fn clear_depth_stencil_view(_: P, dsv: P, flags: u32, depth: f32, stencil: u8) {
    // SAFETY: the caller's view.
    let Some(v) = (unsafe { texture::view(dsv, ViewKind::Dsv) }) else { return };
    // SAFETY: the view's resource.
    let Some(t) = (unsafe { texture::res(v.res) }) else { return };
    // D3D11_DSV_READ_ONLY_DEPTH 1, READ_ONLY_STENCIL 2.
    let flags = flags & !v.desc[2] & 3;
    if flags == 0 {
        return;
    }
    let (mip, first, n) = view::dsv_target(&v.desc);
    let bpp = format::texel_bytes(v.desc[0]) as usize;
    for sub in subs_of(t, mip, first, n) {
        let Some((bytes, s)) = t.sub_bytes(sub) else { continue };
        for r in 0..s.rows {
            for px in bytes[r * s.pitch..r * s.pitch + s.width as usize * bpp].chunks_exact_mut(bpp) {
                clear::clear_depth_texel(v.desc[0], px, flags, depth, stencil);
            }
        }
    }
}

pub unsafe extern "system" fn clear_uav_uint(_: P, _: P, _: *const [u32; 4]) {}
pub unsafe extern "system" fn clear_uav_float(_: P, _: P, _: *const [f32; 4]) {}

/// `GenerateMips(pShaderResourceView)`: levels MostDetailedMip + 1 up to MipLevels - 1 of a
/// texture made with GENERATE_MIPS, each the box-filtered level above it (GenerateMips.yaml).
pub unsafe extern "system" fn generate_mips(_: P, srv: P) {
    // SAFETY: the caller's view.
    let Some(v) = (unsafe { texture::view(srv, ViewKind::Srv) }) else { return };
    // SAFETY: the view's resource.
    let Some(t) = (unsafe { texture::res(v.res) }) else { return };
    let Some(d) = t.desc().copied() else { return };
    if d.misc & resource::misc::GENERATE_MIPS == 0 || format::texel_bytes(d.format) != 4 {
        return;
    }
    let (most, end) = (v.desc[2], v.desc[3]);
    let (first, slices) = match v.desc[1] {
        5 => (v.desc[4], v.desc[5]),
        10 => (v.desc[4], v.desc[5] * 6),
        9 => (0, 6),
        _ => (0, 1),
    };
    let srgb = format::is_srgb(v.desc[0]);
    for slice in first..first + slices {
        for level in most + 1..end.min(d.mips) {
            let (src, dst) = (level - 1 + slice * d.mips, level + slice * d.mips);
            let (Some(ss), Some(ds)) = (t.sub(src), t.sub(dst)) else { continue };
            let bytes = t.data();
            let mut out = alloc::vec![0u8; ds.pitch * ds.rows];
            for y in 0..ds.height as usize {
                for x in 0..ds.width as usize {
                    for c in 0..4 {
                        let mut sum = 0.0f64;
                        let mut n = 0.0;
                        for (sy, sx) in [(2 * y, 2 * x), (2 * y, 2 * x + 1), (2 * y + 1, 2 * x), (2 * y + 1, 2 * x + 1)] {
                            if sy < ss.height as usize && sx < ss.width as usize {
                                let b = bytes[ss.offset + sy * ss.pitch + sx * 4 + c];
                                sum += if srgb && c != 3 { clear::srgb_to_linear(b) } else { b as f64 / 255.0 };
                                n += 1.0;
                            }
                        }
                        let avg = (sum / n) as f32;
                        out[y * ds.pitch + x * 4 + c] = if srgb && c != 3 { clear::srgb8(avg) } else { clear::unorm8(avg) };
                    }
                }
            }
            bytes[ds.offset..ds.offset + out.len()].copy_from_slice(&out);
        }
    }
}

pub unsafe extern "system" fn set_resource_min_lod(_: P, _: P, _: f32) {}
pub unsafe extern "system" fn get_resource_min_lod(_: P, _: P) -> f32 {
    0.0
}

// ----- queries

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn QueryPerformanceCounter(v: *mut i64) -> i32;
    fn QueryPerformanceFrequency(v: *mut i64) -> i32;
}

fn now() -> u64 {
    let mut v = 0i64;
    // SAFETY: our output.
    unsafe { QueryPerformanceCounter(&mut v) };
    v as u64
}

fn freq() -> u64 {
    let mut v = 0i64;
    // SAFETY: our output.
    unsafe { QueryPerformanceFrequency(&mut v) };
    v as u64
}

pub unsafe extern "system" fn begin(_: P, q: P) {
    // SAFETY: the caller's query.
    if let Some(o) = unsafe { objects::query_obj(q) } {
        o.q.with(|q| q.begin());
    }
}

pub unsafe extern "system" fn end(_: P, q: P) {
    // SAFETY: the caller's query.
    if let Some(o) = unsafe { objects::query_obj(q) } {
        let t = now();
        o.q.with(|q| q.end(t));
    }
}

/// `GetData(pAsync, pData, DataSize, GetDataFlags)`.
pub unsafe extern "system" fn get_data(_: P, q: P, data: *mut u8, size: u32, flags: u32) -> HRESULT {
    // SAFETY: the caller's query.
    let Some(o) = (unsafe { objects::query_obj(q) }) else { return E_INVALIDARG };
    let f = freq();
    match o.q.with(|q| q.get_data(!data.is_null(), size, flags, f)) {
        Ok(bytes) => {
            if !bytes.is_empty() {
                // SAFETY: the caller's buffer of `size` >= bytes.len() bytes.
                unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len()) };
            }
            S_OK
        }
        Err(hr) => hr,
    }
}

/// Whether a device's context state needs nothing more (the device goes).
pub fn unused(_: &Device) {}
