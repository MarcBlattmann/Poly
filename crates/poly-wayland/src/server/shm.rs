//! `wl_shm`, `wl_shm_pool` and `wl_buffer`: shared memory buffers in
//! `argb8888` and `xrgb8888`, the two formats every renderer supports.

use super::{ClientId, Data, Res, Server, ShmMap, error};
use crate::compose::Rect;
use crate::protocol::{wl_buffer, wl_shm, wl_shm_pool};
use crate::wire::ObjectId;
use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::vec;
use core::cell::RefCell;

/// The formats the server takes, as `wl_shm.format` events announce.
pub const FORMATS: [wl_shm::Format; 2] = [wl_shm::Format::ARGB8888, wl_shm::Format::XRGB8888];

/// A pool: the client's memory, mapped. Buffers keep it alive.
pub(crate) struct Pool {
    pub map: RefCell<Box<dyn ShmMap>>,
}

impl core::fmt::Debug for Pool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pool").field("len", &self.map.borrow().len()).finish()
    }
}

/// A buffer: where its pixels are in a pool.
#[derive(Debug)]
pub(crate) struct Buffer {
    pub pool: Rc<Pool>,
    pub offset: usize,
    pub width: i32,
    pub height: i32,
    pub stride: usize,
    pub format: wl_shm::Format,
}

impl Buffer {
    /// Copy `r` (buffer pixels, inside the buffer) into `dst`, rows of
    /// `dst_width` premultiplied pixels: `argb8888` as it is (it is
    /// premultiplied already), `xrgb8888` made opaque. Rows the mapping no
    /// longer covers (the client shrank its file) are left as they were.
    pub fn copy_rect(&self, r: Rect, dst: &mut [u32], dst_width: usize) {
        let opaque = if self.format == wl_shm::Format::XRGB8888 { 0xff00_0000 } else { 0 };
        let mut bytes = vec![0u8; r.width as usize * 4];
        let map = self.pool.map.borrow();
        for y in r.y..r.bottom() {
            let at = self.offset + y as usize * self.stride + r.x as usize * 4;
            if !map.read(at, &mut bytes) {
                continue;
            }
            let row = &mut dst[y as usize * dst_width + r.x as usize..][..r.width as usize];
            for (px, b) in row.iter_mut().zip(bytes.chunks_exact(4)) {
                *px = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) | opaque;
            }
        }
    }
}

impl Server {
    pub(super) fn shm_bound(&mut self, c: ClientId, id: ObjectId) {
        let client = self.client(c);
        for f in FORMATS {
            client.send(id, wl_shm::Event::Format { format: f.0 });
        }
    }

    pub(super) fn shm_request(&mut self, c: ClientId, shm: ObjectId, req: wl_shm::Request) -> Res {
        let wl_shm::Request::CreatePool { id, fd, size } = req else { return Ok(()) };
        if size <= 0 {
            self.host().close_fd(fd);
            return Err(error(shm, wl_shm::Error::INVALID_STRIDE.0, format!("pool size {size}")));
        }
        let Some(map) = self.host().map_shm(fd, size as usize) else {
            return Err(error(shm, wl_shm::Error::INVALID_FD.0, "cannot map the pool's file"));
        };
        let client = self.client(c);
        let version = client.version(shm);
        client.insert(id, &wl_shm_pool::INTERFACE, version, Data::Pool(Rc::new(Pool { map: RefCell::new(map) })));
        Ok(())
    }

    pub(super) fn pool_request(&mut self, c: ClientId, pool_id: ObjectId, req: wl_shm_pool::Request) -> Res {
        let client = self.client(c);
        let Some(Data::Pool(pool)) = client.data(pool_id) else { return Ok(()) };
        let pool = Rc::clone(pool);
        match req {
            wl_shm_pool::Request::CreateBuffer { id, offset, width, height, stride, format } => {
                let format = wl_shm::Format(format);
                if !FORMATS.contains(&format) {
                    let msg = format!("format {:#x} is not supported", format.0);
                    return Err(error(pool_id, wl_shm_pool::Error::INVALID_FORMAT.0, msg));
                }
                let len = pool.map.borrow().len();
                let fits = offset >= 0
                    && width > 0
                    && height > 0
                    && stride >= width.saturating_mul(4)
                    && (offset as u64) + (stride as u64) * (height as u64 - 1) + (width as u64) * 4 <= len as u64;
                if !fits {
                    let msg = format!("{width}x{height} stride {stride} at {offset} does not fit a pool of {len}");
                    return Err(error(pool_id, wl_shm_pool::Error::INVALID_STRIDE.0, msg));
                }
                let buffer = Buffer { pool, offset: offset as usize, width, height, stride: stride as usize, format };
                client.insert(id, &wl_buffer::INTERFACE, 1, Data::Buffer(buffer));
            }
            wl_shm_pool::Request::Resize { size } => {
                let len = pool.map.borrow().len();
                if size < 0 || (size as usize) < len {
                    let msg = format!("a pool may only grow ({len} to {size})");
                    return Err(error(pool_id, wl_shm_pool::Error::INVALID_STRIDE.0, msg));
                }
                if !pool.map.borrow_mut().resize(size as usize) {
                    return Err(error(pool_id, wl_shm::Error::INVALID_FD.0, "cannot map the pool's file again"));
                }
            }
            wl_shm_pool::Request::Destroy => {}
        }
        Ok(())
    }
}
