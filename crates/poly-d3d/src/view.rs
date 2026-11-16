//! Views (ID3D11Device.CreateRenderTargetView / CreateShaderResourceView .yaml,
//! ID3D11DeviceContext.ClearDepthStencilView.yaml): a NULL description is completed from the
//! resource; an explicit one is checked; `GetDesc` returns the completed words.

use crate::format;
use crate::hr::*;
use crate::resource::{BufferDesc, Tex2dDesc, bind, misc};

/// The resource a view is made of.
#[derive(Clone, Copy, Debug)]
pub enum Res {
    Buffer(BufferDesc),
    Tex2d(Tex2dDesc),
}

const ALL: u32 = u32::MAX;

/// `D3D11_RTV_DIMENSION` / `D3D11_SRV_DIMENSION` (d3d11.h, d3dcommon.h).
pub mod dim {
    pub const BUFFER: u32 = 1;
    pub const TEXTURE2D: u32 = 4;
    pub const TEXTURE2DARRAY: u32 = 5;
    pub const TEXTURE2DMS: u32 = 6;
    pub const TEXTURE2DMSARRAY: u32 = 7;
    pub const TEXTURECUBE: u32 = 9;
    pub const TEXTURECUBEARRAY: u32 = 10;
    pub const BUFFEREX: u32 = 11;
}

/// `D3D11_DSV_DIMENSION` (d3d11.h).
pub mod dsv_dim {
    pub const TEXTURE2D: u32 = 3;
    pub const TEXTURE2DARRAY: u32 = 4;
    pub const TEXTURE2DMS: u32 = 5;
    pub const TEXTURE2DMSARRAY: u32 = 6;
}

fn bad<T>() -> Result<T, HRESULT> {
    Err(E_INVALIDARG)
}

/// A slice range `first` / `size` (`size` -1: the rest) of `n` slices, completed.
fn range(first: u32, size: u32, n: u32) -> Result<u32, HRESULT> {
    if first >= n {
        return bad();
    }
    let size = if size == ALL { n - first } else { size };
    if size == 0 || size > n - first { bad() } else { Ok(size) }
}

/// The view format: UNKNOWN is the resource's; a typed resource takes only its own format, a
/// typeless one any typed member of its family.
fn view_format(tex: u32, asked: u32) -> Result<u32, HRESULT> {
    let f = if asked == format::UNKNOWN { tex } else { asked };
    if format::view_compatible(tex, f) { Ok(f) } else { bad() }
}

/// `D3D11_RENDER_TARGET_VIEW_DESC` as 5 words (Format, ViewDimension, the union).
pub fn rtv(res: &Res, desc: Option<[u32; 5]>) -> Result<[u32; 5], HRESULT> {
    match res {
        Res::Buffer(b) => {
            let d = desc.ok_or(E_INVALIDARG)?;
            if b.bind & bind::RENDER_TARGET == 0 || d[1] != dim::BUFFER {
                return bad();
            }
            buffer_elements(b, d[0], d[2], d[3])?;
            Ok([d[0], d[1], d[2], d[3], 0])
        }
        Res::Tex2d(t) => {
            if t.bind & bind::RENDER_TARGET == 0 {
                return bad();
            }
            let ms = t.samples > 1;
            let Some(d) = desc else {
                if format::is_typeless(t.format) {
                    return bad();
                }
                return Ok(match (ms, t.array > 1) {
                    (true, true) => [t.format, dim::TEXTURE2DMSARRAY, 0, t.array, 0],
                    (true, false) => [t.format, dim::TEXTURE2DMS, 0, 0, 0],
                    (false, true) => [t.format, dim::TEXTURE2DARRAY, 0, 0, t.array],
                    (false, false) => [t.format, dim::TEXTURE2D, 0, 0, 0],
                });
            };
            let f = view_format(t.format, d[0])?;
            match d[1] {
                dim::TEXTURE2D if !ms && d[2] < t.mips => Ok([f, d[1], d[2], 0, 0]),
                dim::TEXTURE2DARRAY if !ms && d[2] < t.mips => {
                    let n = range(d[3], d[4], t.array)?;
                    Ok([f, d[1], d[2], d[3], n])
                }
                dim::TEXTURE2DMS => Ok([f, d[1], 0, 0, 0]),
                dim::TEXTURE2DMSARRAY => {
                    let n = range(d[2], d[3], t.array)?;
                    Ok([f, d[1], d[2], n, 0])
                }
                _ => bad(),
            }
        }
    }
}

/// The element size and count of a buffer view of `format` (UNKNOWN: the structure stride).
fn buffer_elements(b: &BufferDesc, format: u32, first: u32, num: u32) -> Result<(), HRESULT> {
    let structured = b.misc & misc::BUFFER_STRUCTURED != 0;
    let size = if structured {
        if format != format::UNKNOWN {
            return bad();
        }
        b.stride
    } else {
        format::texel_bytes(format)
    };
    if size == 0 || num == 0 {
        return bad();
    }
    let total = b.byte_width / size;
    if first >= total || num > total - first { bad() } else { Ok(()) }
}

/// `D3D11_SHADER_RESOURCE_VIEW_DESC` as 6 words (Format, ViewDimension, the union).
pub fn srv(res: &Res, desc: Option<[u32; 6]>) -> Result<[u32; 6], HRESULT> {
    match res {
        Res::Buffer(b) => {
            if b.bind & bind::SHADER_RESOURCE == 0 {
                return bad();
            }
            let structured = b.misc & misc::BUFFER_STRUCTURED != 0;
            let raw = b.misc & misc::BUFFER_ALLOW_RAW_VIEWS != 0;
            let Some(d) = desc else {
                if !structured {
                    return bad();
                }
                return Ok([0, dim::BUFFER, 0, b.byte_width / b.stride, 0, 0]);
            };
            match d[1] {
                dim::BUFFER => {
                    buffer_elements(b, d[0], d[2], d[3])?;
                    Ok([d[0], d[1], d[2], d[3], 0, 0])
                }
                dim::BUFFEREX => {
                    let flags = d[4];
                    if flags & !1 != 0 {
                        return bad();
                    }
                    if flags & 1 != 0 {
                        // RAW: R32_TYPELESS elements of a buffer that allows raw views.
                        if !raw || d[0] != format::R32_TYPELESS || d[3] == 0 || d[2] + d[3] > b.byte_width / 4 {
                            return bad();
                        }
                    } else {
                        buffer_elements(b, d[0], d[2], d[3])?;
                    }
                    Ok([d[0], d[1], d[2], d[3], flags, 0])
                }
                _ => bad(),
            }
        }
        Res::Tex2d(t) => {
            if t.bind & bind::SHADER_RESOURCE == 0 {
                return bad();
            }
            let ms = t.samples > 1;
            let cube = t.misc & misc::TEXTURECUBE != 0;
            let Some(d) = desc else {
                if format::is_typeless(t.format) || matches!(format::layout(t.format), format::Layout::Planar { .. }) {
                    return bad();
                }
                return Ok(match (ms, cube, t.array) {
                    (true, _, 1) => [t.format, dim::TEXTURE2DMS, 0, 0, 0, 0],
                    (true, _, n) => [t.format, dim::TEXTURE2DMSARRAY, 0, n, 0, 0],
                    (false, true, 6) => [t.format, dim::TEXTURECUBE, 0, t.mips, 0, 0],
                    (false, true, n) => [t.format, dim::TEXTURECUBEARRAY, 0, t.mips, 0, n / 6],
                    (false, false, 1) => [t.format, dim::TEXTURE2D, 0, t.mips, 0, 0],
                    (false, false, n) => [t.format, dim::TEXTURE2DARRAY, 0, t.mips, 0, n],
                });
            };
            let f = view_format(t.format, d[0])?;
            if format::is_depth(f) {
                return bad();
            }
            let mips = |most: u32, levels: u32| -> Result<u32, HRESULT> {
                if most >= t.mips {
                    return bad();
                }
                let levels = if levels == ALL { t.mips - most } else { levels };
                if levels == 0 || levels > t.mips - most { bad() } else { Ok(levels) }
            };
            match d[1] {
                dim::TEXTURE2D if !ms => Ok([f, d[1], d[2], mips(d[2], d[3])?, 0, 0]),
                dim::TEXTURE2DARRAY if !ms => {
                    let levels = mips(d[2], d[3])?;
                    Ok([f, d[1], d[2], levels, d[4], range(d[4], d[5], t.array)?])
                }
                dim::TEXTURE2DMS => Ok([f, d[1], 0, 0, 0, 0]),
                dim::TEXTURE2DMSARRAY => Ok([f, d[1], d[2], range(d[2], d[3], t.array)?, 0, 0]),
                dim::TEXTURECUBE if cube && !ms => Ok([f, d[1], d[2], mips(d[2], d[3])?, 0, 0]),
                dim::TEXTURECUBEARRAY if cube && !ms => {
                    let levels = mips(d[2], d[3])?;
                    let first = d[4];
                    if first >= t.array {
                        return bad();
                    }
                    let n = if d[5] == ALL { (t.array - first) / 6 } else { d[5] };
                    if n == 0 || first + n * 6 > t.array {
                        return bad();
                    }
                    Ok([f, d[1], d[2], levels, first, n])
                }
                _ => bad(),
            }
        }
    }
}

/// `D3D11_DEPTH_STENCIL_VIEW_DESC` as 6 words (Format, ViewDimension, Flags, the union).
pub fn dsv(res: &Res, desc: Option<[u32; 6]>) -> Result<[u32; 6], HRESULT> {
    let Res::Tex2d(t) = res else { return bad() };
    if t.bind & bind::DEPTH_STENCIL == 0 {
        return bad();
    }
    let ms = t.samples > 1;
    let Some(d) = desc else {
        if format::is_typeless(t.format) {
            return bad();
        }
        return Ok(match (ms, t.array > 1) {
            (true, true) => [t.format, dsv_dim::TEXTURE2DMSARRAY, 0, 0, t.array, 0],
            (true, false) => [t.format, dsv_dim::TEXTURE2DMS, 0, 0, 0, 0],
            (false, true) => [t.format, dsv_dim::TEXTURE2DARRAY, 0, 0, 0, t.array],
            (false, false) => [t.format, dsv_dim::TEXTURE2D, 0, 0, 0, 0],
        });
    };
    let f = view_format(t.format, d[0])?;
    if !format::is_depth(f) || d[2] > 3 {
        return bad();
    }
    match d[1] {
        dsv_dim::TEXTURE2D if !ms && d[3] < t.mips => Ok([f, d[1], d[2], d[3], 0, 0]),
        dsv_dim::TEXTURE2DARRAY if !ms && d[3] < t.mips => Ok([f, d[1], d[2], d[3], d[4], range(d[4], d[5], t.array)?]),
        dsv_dim::TEXTURE2DMS => Ok([f, d[1], d[2], 0, 0, 0]),
        dsv_dim::TEXTURE2DMSARRAY => Ok([f, d[1], d[2], d[3], range(d[3], d[4], t.array)?, 0]),
        _ => bad(),
    }
}

/// The subresources (mip, first slice, slices) a render-target or depth view writes.
pub fn rtv_target(d: &[u32; 5]) -> (u32, u32, u32) {
    match d[1] {
        dim::TEXTURE2DARRAY => (d[2], d[3], d[4]),
        dim::TEXTURE2DMSARRAY => (0, d[2], d[3]),
        dim::TEXTURE2D => (d[2], 0, 1),
        _ => (0, 0, 1),
    }
}

pub fn dsv_target(d: &[u32; 6]) -> (u32, u32, u32) {
    match d[1] {
        dsv_dim::TEXTURE2DARRAY => (d[3], d[4], d[5]),
        dsv_dim::TEXTURE2DMSARRAY => (0, d[3], d[4]),
        dsv_dim::TEXTURE2D => (d[3], 0, 1),
        _ => (0, 0, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tex(format: u32, mips: u32, array: u32, samples: u32, bind: u32, misc: u32) -> Res {
        Res::Tex2d(Tex2dDesc { width: 16, height: 16, mips, array, format, samples, quality: 0, usage: 0, bind, cpu: 0, misc })
    }

    #[test]
    fn render_target_views() {
        let t = tex(87, 3, 1, 1, 0x28, 0);
        assert_eq!(rtv(&t, None), Ok([87, 4, 0, 0, 0]));
        assert_eq!(rtv(&tex(87, 1, 1, 4, 0x20, 0), None), Ok([87, 6, 0, 0, 0]));
        assert_eq!(rtv(&tex(87, 1, 2, 4, 0x20, 0), None), Ok([87, 7, 0, 2, 0]));
        assert_eq!(rtv(&tex(87, 2, 3, 1, 0x20, 0), None), Ok([87, 5, 0, 0, 3]));
        assert_eq!(rtv(&tex(27, 1, 1, 1, 0x20, 0), None), Err(E_INVALIDARG));
        assert_eq!(rtv(&tex(87, 1, 1, 1, 0x8, 0), None), Err(E_INVALIDARG));
        assert_eq!(rtv(&t, Some([0, 4, 0, 0, 0])), Ok([87, 4, 0, 0, 0]));
        assert_eq!(rtv(&t, Some([91, 4, 0, 0, 0])), Err(E_INVALIDARG));
        assert_eq!(rtv(&t, Some([87, 4, 3, 0, 0])), Err(E_INVALIDARG));
        assert_eq!(rtv(&t, Some([87, 5, 0, 0, ALL])), Ok([87, 5, 0, 0, 1]));
        assert_eq!(rtv(&t, Some([87, 5, 0, 0, 2])), Err(E_INVALIDARG));
        assert_eq!(rtv(&tex(27, 1, 1, 1, 0x20, 0), Some([29, 4, 0, 0, 0])), Ok([29, 4, 0, 0, 0]));
        let a = tex(87, 4, 3, 1, 0x20, 0);
        assert_eq!(rtv(&a, Some([87, 5, 0, 1, ALL])), Ok([87, 5, 0, 1, 2]));
        assert_eq!(rtv(&a, Some([87, 5, 0, 2, 2])), Err(E_INVALIDARG));
        let b = Res::Buffer(BufferDesc { byte_width: 256, bind: 0x20, ..Default::default() });
        assert_eq!(rtv(&b, Some([41, 1, 0, 16, 0])), Ok([41, 1, 0, 16, 0]));
        assert_eq!(rtv(&b, Some([41, 1, 0, 65, 0])), Err(E_INVALIDARG));
        assert_eq!(rtv(&b, None), Err(E_INVALIDARG));
    }

    #[test]
    fn shader_resource_views() {
        assert_eq!(srv(&tex(87, 4, 1, 1, 8, 0), None), Ok([87, 4, 0, 4, 0, 0]));
        assert_eq!(srv(&tex(87, 1, 6, 1, 8, 4), None), Ok([87, 9, 0, 1, 0, 0]));
        assert_eq!(srv(&tex(87, 2, 12, 1, 8, 4), None), Ok([87, 10, 0, 2, 0, 2]));
        assert_eq!(srv(&tex(44, 1, 1, 1, 0x48, 0), None), Err(E_INVALIDARG));
        assert_eq!(srv(&tex(44, 1, 1, 1, 0x48, 0), Some([46, 4, 0, 1, 0, 0])), Ok([46, 4, 0, 1, 0, 0]));
        assert_eq!(srv(&tex(44, 1, 1, 1, 0x48, 0), Some([45, 4, 0, 1, 0, 0])), Err(E_INVALIDARG));
        let t = tex(87, 4, 1, 1, 8, 0);
        assert_eq!(srv(&t, Some([87, 4, 1, ALL, 0, 0])), Ok([87, 4, 1, 3, 0, 0]));
        assert_eq!(srv(&t, Some([87, 4, 2, 3, 0, 0])), Err(E_INVALIDARG));
        assert_eq!(srv(&t, Some([87, 4, 0, 0, 0, 0])), Err(E_INVALIDARG));
        let c2 = tex(87, 1, 12, 1, 8, 4);
        assert!(srv(&c2, Some([87, 10, 0, 1, 6, 1])).is_ok());
        assert_eq!(srv(&c2, Some([87, 10, 0, 1, 0, 3])), Err(E_INVALIDARG));
        let s = Res::Buffer(BufferDesc { byte_width: 256, bind: 8, misc: 0x40, stride: 16, ..Default::default() });
        assert_eq!(srv(&s, None), Ok([0, 1, 0, 16, 0, 0]));
        let raw = Res::Buffer(BufferDesc { byte_width: 256, bind: 8, misc: 0x20, ..Default::default() });
        assert!(srv(&raw, Some([39, 11, 0, 64, 1, 0])).is_ok());
        assert_eq!(srv(&raw, Some([39, 11, 0, 65, 1, 0])), Err(E_INVALIDARG));
        let typed = Res::Buffer(BufferDesc { byte_width: 256, bind: 8, ..Default::default() });
        assert!(srv(&typed, Some([41, 1, 8, 16, 0, 0])).is_ok());
        assert_eq!(srv(&typed, Some([41, 11, 0, 64, 1, 0])), Err(E_INVALIDARG));
        assert_eq!(srv(&typed, None), Err(E_INVALIDARG));
    }

    #[test]
    fn depth_stencil_views() {
        assert_eq!(dsv(&tex(45, 1, 1, 1, 0x40, 0), None), Ok([45, 3, 0, 0, 0, 0]));
        assert_eq!(dsv(&tex(40, 1, 3, 1, 0x40, 0), None), Ok([40, 4, 0, 0, 0, 3]));
        assert_eq!(dsv(&tex(44, 1, 1, 1, 0x40, 0), None), Err(E_INVALIDARG));
        assert_eq!(dsv(&tex(44, 1, 1, 1, 0x40, 0), Some([45, 3, 0, 0, 0, 0])), Ok([45, 3, 0, 0, 0, 0]));
        assert_eq!(dsv(&tex(44, 1, 1, 1, 0x40, 0), Some([40, 3, 0, 0, 0, 0])), Err(E_INVALIDARG));
        assert_eq!(dsv(&tex(45, 1, 1, 1, 0x40, 0), Some([0, 3, 4, 0, 0, 0])), Err(E_INVALIDARG));
        assert_eq!(dsv(&tex(45, 1, 1, 1, 0x40, 0), Some([0, 5, 0, 0, 0, 0])), Ok([45, 5, 0, 0, 0, 0]));
        assert_eq!(dsv(&tex(87, 1, 1, 1, 0x20, 0), None), Err(E_INVALIDARG));
    }
}
