//! Kernel tests of the sound card. Most boots have no sound device: then
//! there is no card and no `/dev/snd`. With one (the `sound` boot test
//! attaches QEMU's virtio-sound), the card, its nodes and the playback
//! devices' capabilities must be there; the boot test itself plays sound
//! through them from user space.

use super::*;
use crate::kernel_test;
use poly_vfs::{At, Cred};

fn node_exists(path: &[u8]) -> bool {
    let vfs = crate::fs::vfs();
    let Ok(ctx) = vfs.root_context() else { return false };
    vfs.stat(&ctx, &Cred::root(), At::Cwd, path, true).is_ok()
}

kernel_test! {
    /// A card exactly when there is a virtio-sound device, with its nodes.
    fn card_matches_the_device(_env: &TestEnv) -> TestResult {
        let has_device = crate::pci::devices().iter().any(virtio::is_virtio_sound);
        let Some(card) = card() else {
            if node_exists(b"/dev/snd/controlC0") {
                return Err("/dev/snd/controlC0 without a card");
            }
            return if has_device { Err("a virtio-sound device but no card") } else { Ok(()) };
        };
        if !has_device {
            return Err("a card without a device");
        }
        if !node_exists(b"/dev/snd/controlC0") {
            return Err("no /dev/snd/controlC0");
        }
        for p in &card.pcms {
            let path = alloc::format!("/dev/snd/pcmC{CARD}D{}p", p.device);
            if !node_exists(path.as_bytes()) {
                return Err("a playback device without its node");
            }
            if p.caps.rate_count == 0 || p.caps.formats.is_empty() || p.caps.periods_max < 2 {
                return Err("a playback device without usable capabilities");
            }
        }
        if card.inner.lock().driver.is_failed() {
            return Err("the device broke its protocol");
        }
        Ok(())
    }
}
