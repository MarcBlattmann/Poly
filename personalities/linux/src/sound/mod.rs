//! The ALSA kernel/user interface for sound cards (`/dev/snd/*`), the
//! parts that do not touch hardware (ADR 0104):
//!
//! - [`abi`]: request numbers, versions, constants and structure layouts
//!   from the public uapi header `sound/asound.h`.
//! - [`hw`]: hardware parameters (`struct snd_pcm_hw_params`), their
//!   refinement (`HW_REFINE`) against what the device can do, and the
//!   choice of one configuration (`HW_PARAMS`).
//! - [`pcm`]: the runtime of a playback substream: states, pointers,
//!   thresholds, underruns, drains, over a [`pcm::PcmBackend`] the kernel
//!   implements for its sound device.
//!
//! The kernel's sound module puts these behind character devices
//! (`/dev/snd/controlC0`, `/dev/snd/pcmC0D<n>p`), copies the structures
//! to and from user memory and blocks between attempts. Sources: the
//! uapi header and the ALSA library and kernel documentation of the PCM
//! interface ("PCM (digital audio) interface", "hardware and software
//! parameters", "PCM states").

pub mod abi;
pub mod hw;
pub mod pcm;

#[cfg(test)]
mod tests;

/// `SNDRV_CTL_IOCTL_PCM_NEXT_DEVICE`: the first PCM device number above
/// `current` among `devices` (`-1` starts the walk), or `-1` when there is
/// none.
pub fn next_device(devices: &[u32], current: i32) -> i32 {
    devices.iter().filter(|&&d| (d as i64) > current as i64).min().map_or(-1, |&d| d as i32)
}
