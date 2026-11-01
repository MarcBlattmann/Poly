//! `zwp_linux_dmabuf_v1` (linux-dmabuf-v1.xml): buffers a client made on
//! the GPU (Mesa's EGL and Vulkan, or a dumb buffer of its own) shared as
//! dma-buf file descriptors (PRIME), instead of shared memory.
//!
//! Poly's compositor composes in software, so a dma-buf is mapped
//! ([`Host::map_dmabuf`]: Poly's DRM device lets a dma-buf be mapped, ADR
//! 0108) and its pixels copied at commit like a `wl_shm` buffer's: the
//! formats are the 32-bit RGB ones `wl_shm` takes, one plane, linear
//! layout (`DRM_FORMAT_MOD_LINEAR`, and `DRM_FORMAT_MOD_INVALID`, the
//! "implicit" modifier, which on Poly's dumb buffers is linear too).
//!
//! Version 4's feedback (`get_default_feedback`, `get_surface_feedback`)
//! names the main device (the DRM render node's `dev_t`, from the host)
//! and one tranche with every format, through a format table file the
//! host makes; versions 3 and below get `format` and `modifier` events.
//! The global is offered only if the host can map dma-bufs.
//!
//! [`Host::map_dmabuf`]: super::Host::map_dmabuf

use super::shm::{Buffer, Pool, from_fourcc};
use super::{ClientId, Data, Res, Server, error};
use crate::protocol::{wl_buffer, zwp_linux_buffer_params_v1, zwp_linux_dmabuf_feedback_v1, zwp_linux_dmabuf_v1};
use crate::wire::{Fd, ObjectId};
use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

/// `DRM_FORMAT_MOD_LINEAR` and `DRM_FORMAT_MOD_INVALID` (drm_fourcc.h).
pub const MOD_LINEAR: u64 = 0;
pub const MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

/// The formats and modifiers offered: ARGB8888, XRGB8888, ABGR8888,
/// XBGR8888 (drm_fourcc.h), each linear and implicit.
pub fn formats() -> Vec<(u32, u64)> {
    let mut out = Vec::new();
    for f in [*b"AR24", *b"XR24", *b"AB24", *b"XB24"] {
        for m in [MOD_LINEAR, MOD_INVALID] {
            out.push((u32::from_le_bytes(f), m));
        }
    }
    out
}

/// The format table of version 4's feedback: per entry the format (32
/// bits), 4 bytes of padding and the modifier (64 bits), host byte order.
pub fn format_table() -> Vec<u8> {
    let mut t = Vec::new();
    for (f, m) in formats() {
        t.extend_from_slice(&f.to_ne_bytes());
        t.extend_from_slice(&[0; 4]);
        t.extend_from_slice(&m.to_ne_bytes());
    }
    t
}

/// A `zwp_linux_buffer_params_v1`: the planes added so far.
#[derive(Debug, Default)]
pub(crate) struct Params {
    /// Per plane: the dma-buf, offset, stride and modifier.
    pub planes: [Option<(Fd, u32, u32, u64)>; 4],
    pub used: bool,
}

impl Server {
    pub(super) fn dmabuf_bound(&mut self, c: ClientId, id: ObjectId) {
        let client = self.client(c);
        let version = client.version(id);
        if version >= 4 {
            return;
        }
        let mut seen = Vec::new();
        for (f, m) in formats() {
            if !seen.contains(&f) {
                seen.push(f);
                client.send(id, zwp_linux_dmabuf_v1::Event::Format { format: f });
            }
            client.send(
                id,
                zwp_linux_dmabuf_v1::Event::Modifier {
                    format: f,
                    modifier_hi: (m >> 32) as u32,
                    modifier_lo: m as u32,
                },
            );
        }
    }

    pub(super) fn dmabuf_request(&mut self, c: ClientId, id: ObjectId, req: zwp_linux_dmabuf_v1::Request) -> Res {
        let version = self.client(c).version(id);
        match req {
            zwp_linux_dmabuf_v1::Request::CreateParams { params_id } => {
                let iface = &zwp_linux_buffer_params_v1::INTERFACE;
                self.client(c).insert(params_id, iface, version, Data::DmabufParams(Box::default()));
            }
            zwp_linux_dmabuf_v1::Request::GetDefaultFeedback { id: fb }
            | zwp_linux_dmabuf_v1::Request::GetSurfaceFeedback { id: fb, .. } => {
                let iface = &zwp_linux_dmabuf_feedback_v1::INTERFACE;
                self.client(c).insert(fb, iface, version, Data::DmabufFeedback);
                self.send_feedback(c, fb);
            }
            zwp_linux_dmabuf_v1::Request::Destroy => {}
        }
        Ok(())
    }

    /// The feedback: the format table, the main device, one tranche of
    /// every format on it.
    fn send_feedback(&mut self, c: ClientId, fb: ObjectId) {
        let table = format_table();
        let size = table.len() as u32;
        let device = self.dmabuf_device.unwrap_or(0).to_ne_bytes().to_vec();
        let fd = self.host().dmabuf_table(&table);
        let client = self.client(c);
        if let Some(fd) = fd {
            client.send(fb, zwp_linux_dmabuf_feedback_v1::Event::FormatTable { fd, size });
        }
        client.send(fb, zwp_linux_dmabuf_feedback_v1::Event::MainDevice { device: device.clone() });
        client.send(fb, zwp_linux_dmabuf_feedback_v1::Event::TrancheTargetDevice { device });
        client.send(fb, zwp_linux_dmabuf_feedback_v1::Event::TrancheFlags { flags: 0 });
        let indices: Vec<u8> = (0..formats().len() as u16).flat_map(|i| i.to_ne_bytes()).collect();
        client.send(fb, zwp_linux_dmabuf_feedback_v1::Event::TrancheFormats { indices });
        client.send(fb, zwp_linux_dmabuf_feedback_v1::Event::TrancheDone);
        client.send(fb, zwp_linux_dmabuf_feedback_v1::Event::Done);
    }

    fn params(&mut self, c: ClientId, id: ObjectId) -> Option<&mut Params> {
        match self.client(c).data_mut(id) {
            Some(Data::DmabufParams(p)) => Some(p),
            _ => None,
        }
    }

    pub(super) fn params_request(
        &mut self,
        c: ClientId,
        id: ObjectId,
        req: zwp_linux_buffer_params_v1::Request,
    ) -> Res {
        use zwp_linux_buffer_params_v1::Error as E;
        let Some(p) = self.params(c, id) else { return Ok(()) };
        match req {
            zwp_linux_buffer_params_v1::Request::Add { fd, plane_idx, offset, stride, modifier_hi, modifier_lo } => {
                let problem = if p.used {
                    Some((E::ALREADY_USED, "the params were used".into()))
                } else if plane_idx > 3 {
                    Some((E::PLANE_IDX, format!("plane {plane_idx}")))
                } else if p.planes[plane_idx as usize].is_some() {
                    Some((E::PLANE_SET, format!("plane {plane_idx} is set already")))
                } else {
                    None
                };
                if let Some((code, msg)) = problem {
                    self.host().close_fd(fd);
                    return Err(error(id, code.0, msg));
                }
                let modifier = u64::from(modifier_hi) << 32 | u64::from(modifier_lo);
                p.planes[plane_idx as usize] = Some((fd, offset, stride, modifier));
            }
            zwp_linux_buffer_params_v1::Request::Create { width, height, format, flags } => {
                match self.import(c, id, width, height, format, flags)? {
                    Some(buffer) => {
                        let client = self.client(c);
                        let bid = client.new_server_id();
                        client.insert(bid, &wl_buffer::INTERFACE, 1, Data::Buffer(buffer));
                        client.send(id, zwp_linux_buffer_params_v1::Event::Created { buffer: bid });
                    }
                    None => self.client(c).send(id, zwp_linux_buffer_params_v1::Event::Failed),
                }
            }
            zwp_linux_buffer_params_v1::Request::CreateImmed { buffer_id, width, height, format, flags } => {
                match self.import(c, id, width, height, format, flags)? {
                    Some(buffer) => self.client(c).insert(buffer_id, &wl_buffer::INTERFACE, 1, Data::Buffer(buffer)),
                    None => {
                        let code = E::INVALID_WL_BUFFER.0;
                        return Err(error(id, code, "the dma-buf could not be imported"));
                    }
                }
            }
            zwp_linux_buffer_params_v1::Request::SetSamplingDevice { .. } => {}
            zwp_linux_buffer_params_v1::Request::Destroy => {
                let planes = core::mem::take(&mut p.planes);
                for (fd, ..) in planes.into_iter().flatten() {
                    self.host().close_fd(fd);
                }
            }
        }
        Ok(())
    }

    /// Make a buffer of the params' planes: protocol errors for what the
    /// client got wrong, `None` if the dma-buf cannot be used.
    fn import(
        &mut self,
        c: ClientId,
        id: ObjectId,
        width: i32,
        height: i32,
        format: u32,
        flags: u32,
    ) -> Res<Option<Buffer>> {
        use zwp_linux_buffer_params_v1::Error as E;
        let Some(p) = self.params(c, id) else { return Ok(None) };
        if p.used {
            return Err(error(id, E::ALREADY_USED.0, "the params were used"));
        }
        p.used = true;
        let planes = core::mem::take(&mut p.planes);
        let close_all = |srv: &mut Server, planes: &[Option<(Fd, u32, u32, u64)>]| {
            for (fd, ..) in planes.iter().flatten() {
                srv.host().close_fd(*fd);
            }
        };
        let Some((fd, offset, stride, modifier)) = planes[0] else {
            close_all(self, &planes);
            return Err(error(id, E::INCOMPLETE.0, "no plane 0"));
        };
        if planes[1..].iter().any(Option::is_some) {
            close_all(self, &planes);
            return Err(error(id, E::INCOMPLETE.0, format!("format {format:#x} has one plane")));
        }
        let Some(shm_format) = from_fourcc(format).filter(|_| formats().contains(&(format, modifier))) else {
            close_all(self, &planes);
            return Err(error(id, E::INVALID_FORMAT.0, format!("format {format:#x} modifier {modifier:#x}")));
        };
        if width <= 0 || height <= 0 {
            close_all(self, &planes);
            return Err(error(id, E::INVALID_DIMENSIONS.0, format!("{width}x{height}")));
        }
        if flags & !zwp_linux_buffer_params_v1::Flags::Y_INVERT.0 != 0 {
            close_all(self, &planes);
            return Ok(None);
        }
        // The host owns the descriptor from here on.
        let Some(map) = self.host().map_dmabuf(fd) else { return Ok(None) };
        let len = map.len();
        if !Buffer::fits(i64::from(offset), width, height, stride as i32, len) {
            let msg = format!("{width}x{height} stride {stride} at {offset} does not fit {len} bytes");
            return Err(error(id, E::OUT_OF_BOUNDS.0, msg));
        }
        let pool = Rc::new(Pool { map: RefCell::new(map) });
        // Y_INVERT: the first row is the bottom one.
        let y_invert = flags & zwp_linux_buffer_params_v1::Flags::Y_INVERT.0 != 0;
        Ok(Some(Buffer {
            pool,
            offset: offset as usize,
            width,
            height,
            stride: stride as usize,
            format: shm_format,
            y_invert,
        }))
    }
}
