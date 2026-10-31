//! Modesetting policy: how big the framebuffer is and which scanouts show
//! it (VIRTIO 1.2 §5.7.6.1, §5.7.6.4).
//!
//! The kernel console uses one framebuffer resource for every scanout
//! ("single framebuffer" multihead, §5.7.6.4): the first enabled scanout is
//! the primary one, and its preferred size (from `GET_DISPLAY_INFO`) is the
//! framebuffer's unless the command line asks for a size. Every other
//! enabled scanout mirrors the framebuffer's top-left corner, as much of it
//! as fits its own preferred size. Disabled scanouts show nothing.
//!
//! A display change (§5.7.4.2) is planned the same way from the new display
//! info; [`Plan::scanouts_to_disable`] lists the scanouts the old plan used
//! and the new one does not.

use crate::protocol::{DisplayInfo, MAX_SCANOUTS, Rect};

/// What the caller wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// A size the command line asked for, which wins over the device's.
    pub size: Option<(u32, u32)>,
    /// The size when no scanout reports one.
    pub fallback: (u32, u32),
    /// The most pixels the framebuffer may have (memory); a larger size is
    /// replaced by `fallback`.
    pub max_pixels: u64,
}

/// What to show where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub width: u32,
    pub height: u32,
    /// The scanout whose preferred mode decided the size.
    pub primary: u32,
    /// For each scanout: the rectangle of the framebuffer it shows, or
    /// `None` if it shows nothing.
    pub scanouts: [Option<Rect>; MAX_SCANOUTS],
    /// Where the size came from.
    pub source: SizeSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeSource {
    /// The primary scanout's preferred mode.
    Preferred,
    /// The command line.
    Requested,
    /// No scanout reported a size (or it was too large).
    Fallback,
}

impl Plan {
    /// Decide from `info` (the first `num_scanouts` entries count).
    pub fn new(info: &DisplayInfo, num_scanouts: u32, req: &Request) -> Plan {
        let primary = info.enabled(num_scanouts).next();
        let fits = |(w, h): (u32, u32)| w > 0 && h > 0 && (w as u64) * (h as u64) <= req.max_pixels;
        let (size, source) = match (req.size, primary) {
            (Some(s), _) if fits(s) => (s, SizeSource::Requested),
            (_, Some((_, s))) if fits((s.rect.width, s.rect.height)) => {
                ((s.rect.width, s.rect.height), SizeSource::Preferred)
            }
            _ => (req.fallback, SizeSource::Fallback),
        };
        let (width, height) = size;
        let primary = primary.map_or(0, |(i, _)| i);
        let mut scanouts = [None; MAX_SCANOUTS];
        // The primary scanout always shows the framebuffer, enabled or not
        // (scanout 0 when none is enabled: the device may still have a
        // display it did not report as connected).
        scanouts[primary as usize] = Some(Rect::sized(width, height));
        for (i, s) in info.enabled(num_scanouts).filter(|(i, _)| *i != primary) {
            scanouts[i as usize] = Some(Rect::sized(s.rect.width.min(width), s.rect.height.min(height)));
        }
        Plan { width, height, primary, scanouts, source }
    }

    /// The scanouts that show the framebuffer, with the rectangle each
    /// shows.
    pub fn active(&self) -> impl Iterator<Item = (u32, Rect)> + '_ {
        self.scanouts.iter().enumerate().filter_map(|(i, r)| r.map(|r| (i as u32, r)))
    }

    /// Whether the framebuffer must be made again (another size).
    pub fn resized(&self, old: &Plan) -> bool {
        (self.width, self.height) != (old.width, old.height)
    }

    /// The scanouts `old` showed the framebuffer on and `self` does not.
    pub fn scanouts_to_disable<'a>(&'a self, old: &'a Plan) -> impl Iterator<Item = u32> + 'a {
        (0..MAX_SCANOUTS).filter(|&i| old.scanouts[i].is_some() && self.scanouts[i].is_none()).map(|i| i as u32)
    }
}
