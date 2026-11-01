# 0104: Sound on virtio-sound as ALSA card 0: a host-tested driver crate, the ALSA PCM interface in the Linux personality, one stream at a time, checked by listening to QEMU's recording

**Status:** Accepted (first slice). Part of the Phase 6 item "Audio:
virtio-sound (then Intel HDA) driver, audio server, ALSA-compatible
interface": the virtio-sound driver and the ALSA-compatible PCM playback
interface are done; Intel HDA, capture and the audio server are not, so the
item stays unticked. The owner allowed Phase 6 work to start alongside the
end of Phase 5. The follow-ups below are done in
[ADR 0109](0109-audio-server-capture-mmap-mixer-and-intel-hda.md), which
also replaces "one stream at a time" with the audio server and numbers
capture devices from 0.

## Context

Phase 6 needs sound for its milestone ("VLC plays a video with sound") and
Phase 7 builds WASAPI and `winmm` on "our audio server". The first device is
QEMU's `virtio-sound-pci` (QEMU 8.2 in the `poly-dev` image has it, and its
`wav` audio backend writes what the guest played to a file), described by the
OASIS VIRTIO 1.2 specification, §5.14. Linux programs reach sound through
ALSA's kernel/user interface: `/dev/snd/controlC<c>`, `/dev/snd/pcmC<c>D<d>p`
and the `SNDRV_*` ioctls, structures and constants of the public uapi header
`sound/asound.h`, described further by the ALSA library and kernel
documentation of the PCM interface. CLEAN_ROOM.md: implementers use these
public specifications and headers only; no Linux, ALSA or QEMU source was
read.

The `poly-dev` image has no alsa-utils (no `aplay`), only `libasound2` (as
a QEMU dependency) and the uapi header.

## Decision 1: the driver is a crate of its own, like the other virtio devices

`crates/poly-virtio-sound` (ADR 0009's split: one crate per device class,
allocating nothing) holds:

- `proto`: the messages of §5.14.6 (configuration, item queries, jack, PCM
  and channel map information, SET PARAMETERS, PCM commands, events, the
  `virtio_snd_pcm_xfer` header and `virtio_snd_pcm_status`), with the
  format and rate codes.
- `stream`: the PCM command lifecycle of §5.14.6.6.1 as a state machine
  (SET PARAMETERS from the start or after PREPARE or RELEASE; PREPARE after
  SET PARAMETERS or RELEASE; START after PREPARE or STOP; STOP after START;
  RELEASE after PREPARE or STOP). `StreamState::path_to(goal)` lists the
  commands from the current state to the goal (for example STOP, RELEASE,
  PREPARE from a started stream), so the device is never sent a command it
  must refuse; `check_params` checks a configuration against what the
  stream reported.
- `ring::PeriodRing`: period and buffer bookkeeping of an output stream in
  three byte counters (written, handed to the device, played). Messages
  never cross a period boundary or the end of the ring. Whole periods go
  out as they are written; a partial period only when nothing is in flight
  (the device would run dry) or at a drain, whose last period is padded
  with silence that is not counted as the writer's.
- `VirtioSound`: initialisation (§5.14.5: all four queues, event buffers
  posted before DRIVER_OK), control requests one at a time and polled for
  (the device answers them at once; a request that times out resets the
  device, since it might still write the response later), output messages
  of three descriptors (header and status in per-slot message memory, the
  frames pointing into the PCM ring buffer, so nothing is copied twice),
  events decoded and their buffers reposted. Interrupts are asked for on
  the event and transmit queues only.

A returned output message is the driver's "period elapsed" (§5.14.6.8: an
output message comes back once its frames are consumed); the
PERIOD_ELAPSED event and the shared-memory features are not used.

**QEMU 8.2 reports a used length shorter than the response it writes**
(the stream query came back "short" in the first boot). The response area
is zeroed before every request, so the driver judges a response by its
status: OK with a short length is taken and counted (`short_responses`), a
status of 0 was never written and is an error. The simulated device can
behave the same way, and a host test covers it.

## Decision 2: the ALSA interface lives in the Linux personality; the kernel only copies and blocks

`poly_personality_linux::sound` (host-tested, no hardware):

- `abi`: request numbers, versions (PCM 2.0.17, control 2.0.9), states,
  formats, and encoders for `snd_pcm_info`, `snd_ctl_card_info`,
  `snd_pcm_hw_params`, `snd_pcm_sw_params`, `snd_pcm_status`,
  `snd_pcm_sync_ptr` (the 64-bit layout) and `snd_xferi`; offsets checked
  against the header with `offsetof`.
- `hw`: masks and intervals with open and closed ends and the integer
  flag, interval arithmetic, and **refinement**: the device's formats,
  rates (a list, not a range), channels and size limits, then the rules
  `frame_bits = sample_bits * channels`, `period/buffer_bytes =
  size * frame_bits / 8`, `buffer_size = period_size * periods`,
  `buffer_bytes = period_bytes * periods`, `time = size * 1000000 / rate`,
  applied until nothing changes (they only narrow, so this ends).
  `HW_PARAMS` refines, then decides whatever the caller left open, one
  parameter at a time: interleaved read/write access, S16_LE if allowed,
  stereo if allowed, 48 kHz (else 44.1 kHz, else the lowest rate), 4
  periods, periods of about 20 ms. Only `RW_INTERLEAVED` access is offered:
  no mmap of the ring buffer.
- `pcm::PcmRuntime`: the playback states (OPEN, SETUP, PREPARED, RUNNING,
  XRUN, DRAINING), application and hardware pointers in frames since
  PREPARE (reported modulo the boundary, the largest power-of-two multiple
  of the buffer size that leaves room for one more buffer below
  `i64::MAX`), software parameters (defaults after `HW_PARAMS`: start at
  the first frame, stop when the buffer is empty, wake per period), auto
  start at `start_threshold`, an underrun when `avail` reaches
  `stop_threshold` (the stream stops; writes fail with `EPIPE` until
  PREPARE), drain, drop, status, `SYNC_PTR` (the kernel moves the
  application pointer for read/write access, so a different one from the
  caller is not taken; `avail_min` is), and `poll` readiness. The device
  side is a `PcmBackend` trait.

The kernel (`kernel/src/sound/`) implements `PcmBackend` over the driver
(`Glue`: SET PARAMETERS on `HW_PARAMS`; STOP and RELEASE, then waiting for
every message to come back before the ring is reused, on PREPARE, DROP and
underruns), copies structures between user memory and the ALSA layer, and
blocks where ALSA blocks: `WRITEI_FRAMES` and `write(2)` until every frame
is in the ring buffer, `DRAIN` until everything played (not with
`O_NONBLOCK`, which is taken from the open flags). Both wait on the card's
wait queue, which the audio thread wakes. `EBADFD` joins the personality's
error numbers.

## Decision 3: nodes, numbers and what is offered

- Major 116 (Linux `devices.txt`). Poly's minors: the control device at
  `32c`, playback device `d` at `32c + 16 + d`, capture at `32c + 24 + d`.
  Nodes mode 0666 in `/dev/snd` (there is one user of the machine's
  sound; Linux systems use 0660 and an `audio` group).
- An output stream's ALSA device number is its virtio stream id (QEMU's
  stream 0 is `/dev/snd/pcmC0D0p`, `hw:0,0`); up to 8 per card. Input
  streams are listed in the log but get no nodes (no capture yet).
- The control device answers `PVERSION`, `CARD_INFO` (id `Virtio`, driver
  `poly-virtio-snd`), `PCM_NEXT_DEVICE`, `PCM_INFO`, `PCM_PREFER_SUBDEVICE`,
  an empty `ELEM_LIST` (no mixer controls) and `SUBSCRIBE_EVENTS`.
- A PCM device opens once at a time (`EBUSY`, one subdevice); closing it
  stops the stream and releases it on the device.
- Capabilities per stream: the formats the device offers that ALSA has a
  little-endian code for, its rates and channels; periods of 64 bytes to
  half the PCM memory, 2 to (transmit slots - 1) periods (up to 32: a
  buffer of n periods may need n + 1 messages in flight), buffers up to the
  256 KiB of PCM memory.
- `PAUSE`/`RESUME` are `ENOSYS` (the stream info has no PAUSE flag),
  `REWIND`/`FORWARD` move 0 frames, `READI_FRAMES` is `EBADFD`.

**One stream at a time.** The transmit queue is shared by all output
streams and `Glue` takes back returned messages while stopping a stream
without sorting them by stream; QEMU has one output stream, so the card
drives one at a time. Mixing several streams belongs to the audio server.

## Decision 4: the audio thread, interrupts and polling

MSI-X entry 0 carries the event and transmit queues and configuration
changes; the handler only counts and wakes the audio thread (as for
virtio-net, ADR 0039). The thread locks the card, collects events and
returned messages, advances each running stream's ring and runtime (which
may end a drain or detect an underrun), hands the next periods to the
device, re-arms the interrupts (polling again if work arrived), wakes the
card's waiters and sleeps until the next interrupt (a 50 ms safety poll
while a stream plays). Without MSI-X it polls every 2 ms while a stream
plays; starting a stream wakes it.

## Decision 5: a C test program, and a boot test that listens

Without `aplay`, `tests/linux/audio/pcm_tone.c` (static glibc, built by
xtask with the host's `cc`; in a subdirectory, so not part of the diff
corpus) does what `aplay -D hw:0,0` does through the ioctls alone: the
control device's queries, `HW_REFINE` of everything, `HW_PARAMS` (S16_LE,
U8, S32_LE or FLOAT_LE, stereo, 1024-frame periods x 4), `SW_PARAMS`,
`PREPARE`, frames through `WRITEI_FRAMES` or `write(2)` under `poll(2)`,
`STATUS`, `SYNC_PTR`, `DRAIN` (and that playing took about as long as the
sound lasts), then an underrun (`XRUN`, `EPIPE`, `POLLERR`, recovery by
`PREPARE`), `DROP`, `START` outside PREPARED (`EBADFD`), `HW_FREE`, a
second open (`EBUSY`).

The `sound` boot group (CI shard `devices`) boots it as init with
`-device virtio-sound-pci,audiodev=snd0 -audiodev wav,id=snd0,path=...`
and analyses the recording on the host, not its bytes: 100 ms windows whose
frequency (upward zero crossings, interpolated) is within 2 % of a tone's
and whose RMS on both channels is within 25 % of the sine's. The wav
backend records at its fixed 44.1 kHz stereo S16, so QEMU resamples and
converts the other streams. One boot is interrupt-driven (440 Hz at 48 kHz
through `WRITEI_FRAMES`, 1000 Hz at 44.1 kHz through `write(2)`, 1.5 s
each: at least 10 windows each), one polled with `vectors=0` (660 Hz in U8
at 22.05 kHz, 880 Hz in FLOAT at 48 kHz: at least 6 windows each). Both
pass on TCG. It is skipped where QEMU lacks `virtio-sound-pci` or there is
no static C library.

A kernel test (`sound::tests::card_matches_the_device`) checks that there
is a card exactly when there is a virtio-sound device, with its nodes and
usable capabilities: without a device in every self-test boot, with one in
the first boot of the `sound` group (`poly.test=sound::`).

## Follow-ups

- **Intel HDA** (`intel-hda`/`ich9-intel-hda` with an `hda-output` codec in
  QEMU): controller, CORB/RIRB, codec widget discovery and stream
  descriptors behind the same `PcmBackend`, so the ALSA layer is shared.
- **Capture**: input streams on the receive queue, `READI_FRAMES`,
  `pcmC0D<n>c` nodes.
- **The audio server**: mixing several clients and streams, resampling,
  volume; what Phase 7's WASAPI and `winmm` sit on.
- **mmap access** (`SNDRV_PCM_MMAP_OFFSET_*`, status and control pages) and
  mixer controls on the control device, which alsa-lib's plugins and
  PulseAudio/PipeWire want.
- `aplay` itself on the persistent root disk once alsa-utils is available
  to the image.
