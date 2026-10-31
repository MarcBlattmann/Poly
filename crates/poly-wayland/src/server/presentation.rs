//! `wp_presentation`: when content was shown (presentation-time.xml), the
//! groundwork for vsync-driven clients. Feedback requested before a commit
//! belongs to that commit's content and is answered with the frame that
//! shows it (`presented`), or `discarded` if newer content replaced it
//! first or the surface is not shown.

use super::{ClientId, Data, Res, Server};
use crate::protocol::{wp_presentation, wp_presentation_feedback};
use crate::wire::ObjectId;

/// `CLOCK_MONOTONIC` from linux/time.h: the clock presentation times are on.
pub const CLOCK_MONOTONIC: u32 = 1;

impl Server {
    pub(super) fn presentation_bound(&mut self, c: ClientId, id: ObjectId) {
        self.client(c).send(id, wp_presentation::Event::ClockId { clk_id: CLOCK_MONOTONIC });
    }

    pub(super) fn presentation_request(&mut self, c: ClientId, id: ObjectId, req: wp_presentation::Request) -> Res {
        let wp_presentation::Request::Feedback { surface, callback } = req else { return Ok(()) };
        let client = self.client(c);
        let version = client.version(id);
        client.insert(callback, &wp_presentation_feedback::INTERFACE, version, Data::Feedback);
        if let Some(s) = self.surface(c, surface) {
            s.pending.feedbacks.push(callback);
        }
        Ok(())
    }
}
