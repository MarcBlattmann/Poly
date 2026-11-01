//! Kernel tests of the sound card. Most boots have no sound device: then
//! there is no card and no `/dev/snd`. With one (the `sound` boot tests
//! attach QEMU's virtio-sound or an Intel HDA controller), the card, its
//! nodes and the PCM devices' capabilities must be there, and the server
//! must play: a substream started with frames in its buffer has them
//! taken by the mixer. The boot tests themselves play and record through
//! them from user space.

use super::*;
use crate::kernel_test;
use poly_personality_linux::sound::pcm::State;
use poly_vfs::{At, Cred};

fn node_exists(path: &[u8]) -> bool {
    let vfs = crate::fs::vfs();
    let Ok(ctx) = vfs.root_context() else { return false };
    vfs.stat(&ctx, &Cred::root(), At::Cwd, path, true).is_ok()
}

fn has_device() -> bool {
    crate::pci::devices()
        .iter()
        .any(|d| virtio::is_virtio_sound(d) || (d.header.class.class == 0x04 && d.header.class.subclass == 0x03))
}

kernel_test! {
    /// A card exactly when there is a sound device, with its nodes.
    fn card_matches_the_device(_env: &TestEnv) -> TestResult {
        let Some(card) = card() else {
            if node_exists(b"/dev/snd/controlC0") {
                return Err("/dev/snd/controlC0 without a card");
            }
            return if has_device() { Err("a sound device but no card") } else { Ok(()) };
        };
        if !has_device() {
            return Err("a card without a device");
        }
        if !node_exists(b"/dev/snd/controlC0") {
            return Err("no /dev/snd/controlC0");
        }
        for p in &card.pcms {
            let path = alloc::format!("/dev/snd/{}", alsa_node(p));
            if !node_exists(path.as_bytes()) {
                return Err("a PCM device without its node");
            }
            if p.caps.rate_count == 0 || p.caps.formats.is_empty() || p.hw.period_frames == 0 {
                return Err("a PCM device without usable capabilities");
            }
        }
        let g = card.inner.lock();
        if g.hw.is_failed() {
            return Err("the device broke its protocol");
        }
        if g.subs.len() != card.pcms.len() * SUBDEVICES || g.controls.is_empty() {
            return Err("substreams or controls missing");
        }
        Ok(())
    }
}

kernel_test! {
    /// The server takes the frames of a started playback substream: its
    /// hardware pointer moves while the device plays them.
    fn the_server_mixes_a_started_substream(_env: &TestEnv) -> TestResult {
        let Some(card) = card() else { return Ok(()) };
        let Some(i) = card.pcms.iter().position(|p| p.dir == Dir::Playback) else { return Ok(()) };
        let index = i * SUBDEVICES + SUBDEVICES - 1;
        {
            let mut g = card.inner.lock();
            if g.subs[index].open {
                return Err("the last subdevice is in use");
            }
            g.subs[index].open = true;
        }
        let cfg = HwConfig {
            access: abi::access::RW_INTERLEAVED,
            format: abi::format::S16_LE,
            channels: 2,
            rate: 44100,
            period_frames: 441,
            periods: 4,
            buffer_frames: 1764,
            frame_bytes: 4,
        };
        let frames = vec![0u8; 1764 * 4];
        let r = card.with_sub(index, |rt, g| -> Result<(), Errno> {
            rt.hw_params(cfg, g)?;
            rt.prepare(g)?;
            rt.write(&frames, g, monotonic())?;
            Ok(())
        });
        let mut result = r.map_err(|_| "the substream did not start");
        if result.is_ok() {
            let deadline = time::now_ns() + 3 * time::NANOS_PER_SEC;
            loop {
                let (hw, state) = card.with_sub(index, |rt, _| (rt.hw(), rt.state()));
                if hw > 0 && hw <= 1764 && matches!(state, State::Running | State::Xrun) {
                    break;
                }
                if time::now_ns() > deadline {
                    result = Err("the server took no frames in 3 s");
                    break;
                }
                thread::yield_now();
            }
        }
        card.with_sub(index, |rt, g| rt.close(g));
        card.inner.lock().subs[index].open = false;
        result
    }
}

fn alsa_node(p: &PcmDesc) -> String {
    alloc::format!("pcmC{CARD}D{}{}", p.device, if p.dir == Dir::Playback { 'p' } else { 'c' })
}
