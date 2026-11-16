//! Buffer and 2D texture descriptions (ID3D11Device.CreateBuffer.yaml, ID3D11Device.CreateTexture2D.yaml,
//! IDXGIResource1.CreateSharedHandle.yaml): which are valid, and the description `GetDesc` returns.

use crate::format;
use crate::hr::*;

/// `D3D11_USAGE` (d3d11.h).
pub mod usage {
    pub const DEFAULT: u32 = 0;
    pub const IMMUTABLE: u32 = 1;
    pub const DYNAMIC: u32 = 2;
    pub const STAGING: u32 = 3;
}

/// `D3D11_BIND_FLAG` (d3d11.h).
pub mod bind {
    pub const VERTEX_BUFFER: u32 = 0x1;
    pub const INDEX_BUFFER: u32 = 0x2;
    pub const CONSTANT_BUFFER: u32 = 0x4;
    pub const SHADER_RESOURCE: u32 = 0x8;
    pub const STREAM_OUTPUT: u32 = 0x10;
    pub const RENDER_TARGET: u32 = 0x20;
    pub const DEPTH_STENCIL: u32 = 0x40;
    pub const UNORDERED_ACCESS: u32 = 0x80;
    pub const DECODER: u32 = 0x200;
    pub const VIDEO_ENCODER: u32 = 0x400;
}

/// `D3D11_CPU_ACCESS_FLAG` (d3d11.h).
pub mod cpu {
    pub const WRITE: u32 = 0x10000;
    pub const READ: u32 = 0x20000;
}

/// `D3D11_RESOURCE_MISC_FLAG` (d3d11.h).
pub mod misc {
    pub const GENERATE_MIPS: u32 = 0x1;
    pub const SHARED: u32 = 0x2;
    pub const TEXTURECUBE: u32 = 0x4;
    pub const DRAWINDIRECT_ARGS: u32 = 0x10;
    pub const BUFFER_ALLOW_RAW_VIEWS: u32 = 0x20;
    pub const BUFFER_STRUCTURED: u32 = 0x40;
    pub const SHARED_KEYEDMUTEX: u32 = 0x100;
    pub const GDI_COMPATIBLE: u32 = 0x200;
    pub const SHARED_NTHANDLE: u32 = 0x800;
    pub const RESTRICTED_CONTENT: u32 = 0x1000;
    pub const RESTRICT_SHARED_RESOURCE: u32 = 0x2000;
    pub const GUARDED: u32 = 0x8000;
    pub const TILED: u32 = 0x40000;
}

/// `D3D11_BUFFER_DESC`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct BufferDesc {
    pub byte_width: u32,
    pub usage: u32,
    pub bind: u32,
    pub cpu: u32,
    pub misc: u32,
    pub stride: u32,
}

/// Check a buffer description; `data` is whether initial data with a non-NULL `pSysMem` was given
/// (`Some(false)`: a `D3D11_SUBRESOURCE_DATA` with a NULL pointer). Returns the description
/// `GetDesc` reports.
pub fn check_buffer(d: &BufferDesc, data: Option<bool>) -> Result<BufferDesc, HRESULT> {
    use bind::*;
    let bad = Err(E_INVALIDARG);
    if d.byte_width == 0 || d.usage > usage::STAGING || data == Some(false) {
        return bad;
    }
    if d.bind & DEPTH_STENCIL != 0 || d.cpu & !(cpu::READ | cpu::WRITE) != 0 {
        return bad;
    }
    let shader = SHADER_RESOURCE | UNORDERED_ACCESS;
    if d.bind & CONSTANT_BUFFER != 0
        && (!d.byte_width.is_multiple_of(16)
            || d.bind & (VERTEX_BUFFER | INDEX_BUFFER | shader | STREAM_OUTPUT | RENDER_TARGET) != 0)
    {
        return bad;
    }
    match d.usage {
        usage::DEFAULT => {
            if d.cpu != 0 && d.bind & shader == 0 {
                return bad;
            }
        }
        usage::IMMUTABLE => {
            let allowed = VERTEX_BUFFER | INDEX_BUFFER | CONSTANT_BUFFER | SHADER_RESOURCE;
            if data.is_none() || d.cpu != 0 || d.bind == 0 || d.bind & !allowed & 0xFF != 0 {
                return bad;
            }
        }
        usage::DYNAMIC => {
            let allowed = VERTEX_BUFFER | INDEX_BUFFER | CONSTANT_BUFFER | SHADER_RESOURCE;
            if d.cpu != cpu::WRITE || d.bind == 0 || d.bind & !allowed & 0xFF != 0 {
                return bad;
            }
        }
        _ => {
            if d.bind != 0 || d.cpu == 0 {
                return bad;
            }
        }
    }
    let known = misc::SHARED
        | misc::DRAWINDIRECT_ARGS
        | misc::BUFFER_ALLOW_RAW_VIEWS
        | misc::BUFFER_STRUCTURED
        | misc::SHARED_KEYEDMUTEX
        | misc::GUARDED
        | misc::TILED
        | 0x10_0000;
    if d.misc & !known != 0
        || d.misc & (misc::SHARED | misc::SHARED_KEYEDMUTEX) == misc::SHARED | misc::SHARED_KEYEDMUTEX
    {
        return bad;
    }
    let structured = d.misc & misc::BUFFER_STRUCTURED != 0;
    let raw = d.misc & misc::BUFFER_ALLOW_RAW_VIEWS != 0;
    if structured {
        if d.stride == 0 || d.stride > 2048 || !d.byte_width.is_multiple_of(d.stride) {
            return bad;
        }
        if d.bind & (VERTEX_BUFFER | INDEX_BUFFER | CONSTANT_BUFFER | RENDER_TARGET | STREAM_OUTPUT) != 0 {
            return bad;
        }
    }
    if raw && (structured || d.bind & shader == 0 || d.usage == usage::STAGING) {
        return bad;
    }
    if d.misc & misc::DRAWINDIRECT_ARGS != 0
        && (structured || d.usage == usage::STAGING || !d.byte_width.is_multiple_of(4))
    {
        return bad;
    }
    Ok(BufferDesc { stride: if structured { d.stride } else { 0 }, ..*d })
}

/// `D3D11_TEXTURE2D_DESC` (the sample description as two words).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Tex2dDesc {
    pub width: u32,
    pub height: u32,
    pub mips: u32,
    pub array: u32,
    pub format: u32,
    pub samples: u32,
    pub quality: u32,
    pub usage: u32,
    pub bind: u32,
    pub cpu: u32,
    pub misc: u32,
}

impl Tex2dDesc {
    pub fn subresources(&self) -> u32 {
        self.mips * self.array
    }
    /// The size of mip `level`.
    pub fn mip_size(&self, level: u32) -> (u32, u32) {
        ((self.width >> level).max(1), (self.height >> level).max(1))
    }
}

/// The length of the full mip chain of a `w` x `h` texture.
pub fn full_chain(w: u32, h: u32) -> u32 {
    32 - w.max(h).max(1).leading_zeros()
}

/// What a texture of a format can be bound as (CreateTexture2D.yaml `formats_*`).
mod caps {
    pub const SRV: u32 = 1;
    pub const RTV: u32 = 2;
    pub const DSV: u32 = 4;
    pub const UAV: u32 = 8;
    pub const STAGING: u32 = 16;
    pub const DYNAMIC: u32 = 32;
}

fn format_caps(f: u32) -> u32 {
    use caps::*;
    if !format::known(f) || matches!(f, 0 | 106 | 188) {
        return 0;
    }
    // B8G8R8X8_TYPELESS may be an unordered-access texture though its typed members may not
    // (CreateTexture2D.yaml `formats_srv_rtv_uav`, `formats_no_uav`).
    if f == 92 {
        return SRV | RTV | UAV | STAGING | DYNAMIC;
    }
    if let Some(fam) = format::family(f)
        && fam[0] == f
    {
        // A typeless texture can be anything one of its typed formats can be.
        return fam[1..].iter().fold(0, |c, &m| c | format_caps(m));
    }
    match f {
        20 | 40 | 45 | 55 => DSV | STAGING,
        21 | 22 | 46 | 47 | 67..=84 | 94..=99 | 113 | 114 => SRV | STAGING | DYNAMIC,
        107..=109 => SRV | UAV | STAGING | DYNAMIC,
        88 | 93 => SRV | RTV | STAGING | DYNAMIC,
        _ => SRV | RTV | UAV | STAGING | DYNAMIC,
    }
}

/// Check a 2D texture description. `data` is `None` without initial data, else whether every
/// subresource's `D3D11_SUBRESOURCE_DATA` has a non-NULL pointer and a non-zero row pitch. Returns
/// the description `GetDesc` reports (MipLevels 0 replaced by the chain's length).
pub fn check_tex2d(d: &Tex2dDesc, data: Option<bool>) -> Result<Tex2dDesc, HRESULT> {
    use bind::*;
    let bad = Err(E_INVALIDARG);
    if d.width == 0 || d.height == 0 || d.width > 32768 || d.height > 32768 {
        return bad;
    }
    let chain = full_chain(d.width, d.height);
    let mips = if d.mips == 0 { chain } else { d.mips };
    if mips > chain || d.array == 0 || d.array > 2048 {
        return bad;
    }
    if !matches!(d.samples, 1 | 2 | 4 | 8 | 16) || d.quality != 0 {
        return bad;
    }
    if d.samples > 1 && (mips > 1 || d.usage == usage::STAGING) {
        return bad;
    }
    let allowed_bind = SHADER_RESOURCE | RENDER_TARGET | DEPTH_STENCIL | UNORDERED_ACCESS | DECODER | VIDEO_ENCODER;
    if d.bind & !allowed_bind != 0 || d.cpu & !(cpu::READ | cpu::WRITE) != 0 || d.usage > usage::STAGING {
        return bad;
    }
    if data == Some(false) {
        return bad;
    }
    let c = format_caps(d.format);
    if c == 0 {
        return bad;
    }
    let need = |b: u32, cap: u32| d.bind & b == 0 || c & cap != 0;
    if !need(SHADER_RESOURCE, caps::SRV)
        || !need(RENDER_TARGET, caps::RTV)
        || !need(DEPTH_STENCIL, caps::DSV)
        || !need(UNORDERED_ACCESS, caps::UAV)
    {
        return bad;
    }
    if d.bind & DEPTH_STENCIL != 0 && d.bind & (RENDER_TARGET | UNORDERED_ACCESS) != 0 {
        return bad;
    }
    match d.usage {
        usage::DEFAULT => {}
        usage::IMMUTABLE => {
            if data.is_none() || d.cpu != 0 || d.bind & !SHADER_RESOURCE != 0 || d.bind == 0 {
                return bad;
            }
        }
        usage::DYNAMIC => {
            if d.bind != SHADER_RESOURCE || d.cpu != cpu::WRITE || c & caps::DYNAMIC == 0 {
                return bad;
            }
        }
        _ => {
            if d.bind != 0 || d.cpu == 0 || c & caps::STAGING == 0 {
                return bad;
            }
        }
    }
    check_tex_misc(d)?;
    Ok(Tex2dDesc { mips, ..*d })
}

fn check_tex_misc(d: &Tex2dDesc) -> Result<(), HRESULT> {
    use misc::*;
    let bad = Err(E_INVALIDARG);
    let known = GENERATE_MIPS
        | SHARED
        | TEXTURECUBE
        | SHARED_KEYEDMUTEX
        | GDI_COMPATIBLE
        | SHARED_NTHANDLE
        | RESTRICTED_CONTENT
        | RESTRICT_SHARED_RESOURCE;
    if d.misc & !known != 0 {
        return bad;
    }
    if d.misc & GENERATE_MIPS != 0
        && d.bind & (bind::RENDER_TARGET | bind::SHADER_RESOURCE) != bind::RENDER_TARGET | bind::SHADER_RESOURCE
    {
        return bad;
    }
    if d.misc & TEXTURECUBE != 0 && (!d.array.is_multiple_of(6) || d.bind & bind::SHADER_RESOURCE == 0) {
        return bad;
    }
    let shared = d.misc & (SHARED | SHARED_KEYEDMUTEX | SHARED_NTHANDLE);
    match shared {
        0 | SHARED | SHARED_KEYEDMUTEX => {}
        x if x == SHARED | SHARED_NTHANDLE || x == SHARED_KEYEDMUTEX | SHARED_NTHANDLE => {
            if d.misc & (GDI_COMPATIBLE | GENERATE_MIPS | RESTRICT_SHARED_RESOURCE) != 0 {
                return bad;
            }
        }
        _ => return bad,
    }
    if d.misc & RESTRICT_SHARED_RESOURCE != 0 && shared == 0 {
        return bad;
    }
    Ok(())
}

/// `D3D11_RESOURCE_DIMENSION` (d3d11.h).
pub const DIMENSION_BUFFER: u32 = 1;
pub const DIMENSION_TEXTURE2D: u32 = 3;

/// The eviction priority of a new resource: `DXGI_RESOURCE_PRIORITY_NORMAL` (dxgi.h).
pub const PRIORITY_NORMAL: u32 = 0x7800_0000;

#[cfg(test)]
mod tests {
    use super::*;
    use bind::*;

    fn buf(byte_width: u32, usage: u32, bind: u32, cpu: u32, misc: u32, stride: u32) -> Result<BufferDesc, HRESULT> {
        check_buffer(&BufferDesc { byte_width, usage, bind, cpu, misc, stride }, None)
    }

    #[test]
    fn buffer_sizes_and_constant_buffers() {
        assert_eq!(buf(0, 0, VERTEX_BUFFER, 0, 0, 0), Err(E_INVALIDARG));
        assert!(buf(3, 0, VERTEX_BUFFER, 0, 0, 0).is_ok());
        assert!(buf(0x8000_0000, 0, 0, 0, 0, 0).is_ok());
        assert_eq!(buf(17, 0, CONSTANT_BUFFER, 0, 0, 0), Err(E_INVALIDARG));
        assert!(buf(65552, 0, CONSTANT_BUFFER, 0, 0, 0).is_ok());
        assert_eq!(buf(64, 0, CONSTANT_BUFFER | VERTEX_BUFFER, 0, 0, 0), Err(E_INVALIDARG));
        assert_eq!(buf(64, 0, DEPTH_STENCIL, 0, 0, 0), Err(E_INVALIDARG));
        assert_eq!(buf(64, 0, 0xFF, 0, 0, 0), Err(E_INVALIDARG));
        assert_eq!(buf(64, 0, 0x100, 0, 0, 0).unwrap().bind, 0x100);
    }

    #[test]
    fn buffer_usages() {
        assert_eq!(buf(64, 0, VERTEX_BUFFER, cpu::WRITE, 0, 0), Err(E_INVALIDARG));
        assert!(buf(64, 0, SHADER_RESOURCE, cpu::WRITE, 0, 0).is_ok());
        assert_eq!(buf(64, usage::IMMUTABLE, VERTEX_BUFFER, 0, 0, 0), Err(E_INVALIDARG));
        assert!(
            check_buffer(&BufferDesc { byte_width: 64, usage: 1, bind: 1, ..Default::default() }, Some(true)).is_ok()
        );
        assert!(buf(64, usage::DYNAMIC, VERTEX_BUFFER, cpu::WRITE, 0, 0).is_ok());
        assert_eq!(buf(64, usage::DYNAMIC, VERTEX_BUFFER, cpu::READ | cpu::WRITE, 0, 0), Err(E_INVALIDARG));
        assert_eq!(buf(64, usage::DYNAMIC, UNORDERED_ACCESS, cpu::WRITE, 0, 0), Err(E_INVALIDARG));
        assert!(buf(64, usage::STAGING, 0, cpu::READ, 0, 0).is_ok());
        assert_eq!(buf(64, usage::STAGING, 0, 0, 0, 0), Err(E_INVALIDARG));
        assert_eq!(buf(64, 4, 0, 0, 0, 0), Err(E_INVALIDARG));
    }

    #[test]
    fn structured_raw_indirect_shared() {
        assert_eq!(buf(60, 0, SHADER_RESOURCE, 0, misc::BUFFER_STRUCTURED, 12).unwrap().stride, 12);
        assert_eq!(buf(64, 0, SHADER_RESOURCE, 0, misc::BUFFER_STRUCTURED, 12), Err(E_INVALIDARG));
        assert_eq!(buf(4104, 0, SHADER_RESOURCE, 0, misc::BUFFER_STRUCTURED, 2052), Err(E_INVALIDARG));
        assert_eq!(buf(64, 0, VERTEX_BUFFER, 0, misc::BUFFER_STRUCTURED, 16), Err(E_INVALIDARG));
        assert_eq!(buf(64, 0, VERTEX_BUFFER, 0, 0, 16).unwrap().stride, 0);
        assert!(buf(62, 0, SHADER_RESOURCE, 0, misc::BUFFER_ALLOW_RAW_VIEWS, 0).is_ok());
        assert_eq!(buf(64, 0, VERTEX_BUFFER, 0, misc::BUFFER_ALLOW_RAW_VIEWS, 0), Err(E_INVALIDARG));
        assert!(buf(64, 0, 9, 0, misc::BUFFER_ALLOW_RAW_VIEWS, 0).is_ok());
        assert_eq!(buf(3, 0, 0, 0, misc::DRAWINDIRECT_ARGS, 0), Err(E_INVALIDARG));
        assert_eq!(buf(64, 0, UNORDERED_ACCESS, 0, 0x30, 0).unwrap().misc, 0x30);
        assert!(buf(64, 0, 0, 0, misc::SHARED_KEYEDMUTEX, 0).is_ok());
        assert_eq!(buf(64, 0, 0, 0, 0x102, 0), Err(E_INVALIDARG));
        assert_eq!(buf(64, 0, 0, 0, 0x800, 0), Err(E_INVALIDARG));
        assert_eq!(buf(64, 0, 0x28, 0, misc::GENERATE_MIPS, 0), Err(E_INVALIDARG));
        assert!(buf(64, 0, 0x28, 0, misc::TILED, 0).is_ok());
    }

    fn tex(format: u32, usage: u32, bind: u32, cpu: u32, misc: u32) -> Result<Tex2dDesc, HRESULT> {
        check_tex2d(
            &Tex2dDesc {
                width: 64,
                height: 32,
                mips: 1,
                array: 1,
                format,
                samples: 1,
                quality: 0,
                usage,
                bind,
                cpu,
                misc,
            },
            None,
        )
    }

    #[test]
    fn texture_mips_and_sizes() {
        let t = |w, h, mips| {
            check_tex2d(
                &Tex2dDesc {
                    width: w,
                    height: h,
                    mips,
                    array: 1,
                    format: 87,
                    samples: 1,
                    bind: 8,
                    ..Default::default()
                },
                None,
            )
        };
        assert_eq!(t(64, 64, 0).unwrap().mips, 7);
        assert_eq!(t(100, 30, 0).unwrap().mips, 7);
        assert_eq!(t(5, 1, 0).unwrap().mips, 3);
        assert_eq!(t(1, 1, 0).unwrap().mips, 1);
        assert_eq!(t(64, 64, 8), Err(E_INVALIDARG));
        assert_eq!(t(0, 1, 1), Err(E_INVALIDARG));
        assert!(t(32768, 1, 1).is_ok());
    }

    #[test]
    fn texture_formats_and_usages() {
        assert!(tex(87, 0, SHADER_RESOURCE | RENDER_TARGET, 0, 0).is_ok());
        assert!(tex(27, 0, UNORDERED_ACCESS, 0, 0).is_ok());
        assert_eq!(tex(88, 0, UNORDERED_ACCESS, 0, 0), Err(E_INVALIDARG));
        assert!(tex(45, 0, DEPTH_STENCIL, 0, 0).is_ok());
        assert_eq!(tex(45, 0, SHADER_RESOURCE, 0, 0), Err(E_INVALIDARG));
        assert!(tex(44, 0, SHADER_RESOURCE, 0, 0).is_ok() && tex(44, 0, DEPTH_STENCIL, 0, 0).is_ok());
        assert_eq!(tex(44, 0, RENDER_TARGET, 0, 0), Err(E_INVALIDARG));
        assert_eq!(tex(87, 0, DEPTH_STENCIL, 0, 0), Err(E_INVALIDARG));
        assert_eq!(tex(71, 0, RENDER_TARGET, 0, 0), Err(E_INVALIDARG));
        assert!(tex(107, 0, UNORDERED_ACCESS, 0, 0).is_ok());
        assert_eq!(tex(0, 0, SHADER_RESOURCE, 0, 0), Err(E_INVALIDARG));
        assert_eq!(tex(106, 0, SHADER_RESOURCE, 0, 0), Err(E_INVALIDARG));
        assert!(tex(87, usage::DYNAMIC, SHADER_RESOURCE, cpu::WRITE, 0).is_ok());
        assert_eq!(tex(87, usage::DYNAMIC, RENDER_TARGET, cpu::WRITE, 0), Err(E_INVALIDARG));
        assert!(tex(87, usage::STAGING, 0, cpu::READ, 0).is_ok());
        assert_eq!(tex(87, usage::STAGING, SHADER_RESOURCE, cpu::READ, 0), Err(E_INVALIDARG));
        assert_eq!(tex(87, 0, VERTEX_BUFFER, 0, 0), Err(E_INVALIDARG));
        assert_eq!(tex(87, 0, 0, 1, 0), Err(E_INVALIDARG));
        assert!(tex(87, 0, 0, cpu::READ | cpu::WRITE, 0).is_ok());
    }

    #[test]
    fn texture_misc() {
        let rs = SHADER_RESOURCE | RENDER_TARGET;
        for ok in [0, 1, 2, 0x100, 0x200, 0x900, 0x802, 0x1000, 0x2002, 0x202] {
            assert!(tex(87, 0, rs, 0, ok).is_ok(), "{ok:#x}");
        }
        for bad in [4, 0x800, 0x102, 0x2000, 0x8000_0000, 0x902, 0xA00, 0x801 | 0x100, 0x2800 | 0x100] {
            assert_eq!(tex(87, 0, rs, 0, bad), Err(E_INVALIDARG), "{bad:#x}");
        }
        assert_eq!(tex(87, 0, SHADER_RESOURCE, 0, 1), Err(E_INVALIDARG));
    }
}
