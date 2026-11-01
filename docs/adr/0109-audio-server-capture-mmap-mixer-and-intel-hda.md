# 0109: The audio server in the kernel behind ALSA's subdevices; capture, mmap access and mixer controls; Intel HDA output

**Status:** Accepted. Completes the Phase 6 item "Audio: virtio-sound (then
Intel HDA) driver, audio server, ALSA-compatible interface" that
[ADR 0104](0104-virtio-sound-and-alsa-pcm.md) started (virtio-sound and
ALSA playback). Its follow-ups are done here: capture, mmap access and
mixer controls, Intel HDA, and the audio server.

## Context

ADR 0104 played one stream at a time straight into the device: the ALSA
runtime of `/dev/snd/pcmC0D0p` drove virtio-sound's period ring, a second
open was `EBUSY`, only read/write access, no capture, no controls. alsa-lib
plugins, PulseAudio and PipeWire want mmap access and mixer controls; two
programs must be able to play at once; Phase 7's WASAPI and `winmm` will
sit on "our audio server"; and real machines mostly have Intel HDA.

Sources (CLEAN_ROOM.md, implementer): the OASIS VIRTIO 1.2 specification
§5.14 (input messages), Intel's *High Definition Audio Specification* rev
1.0a (controller registers, CORB/RIRB, stream descriptors, BDL, verbs,
parameters, widget types, configuration default), the ALSA uapi header
`sound/asound.h` (structures checked with `offsetof` in the build image)
and the ALSA library and kernel documentation (PCM states, mmap transfers,
control names), QEMU's documentation of its audio options. No Linux, ALSA,
QEMU or PipeWire source was read.

## Decision 1: the server mixes in the kernel, behind ALSA subdevices

The two candidates were a userland daemon (a PulseAudio-like socket
protocol and a client library) or mixing in the kernel behind the ALSA
device. We mix in the kernel:

- Every ALSA program, with or without alsa-lib, works unchanged: opening
  `hw:0,0` gives a stream of its own. ALSA has the model for it, a PCM
  device with several **subdevices** (cards with hardware mixing): each
  open of `pcmC0D0p` takes the first free one of 8 (`EBUSY` beyond), and
  `PCM_INFO` reports `subdevices_count` 8 and how many are free.
- No daemon has to run before sound works, and Phase 7's WASAPI and
  `winmm` can sit on the same server from the NT personality without a
  second protocol.
- The arithmetic is host-tested anyway: `crates/poly-audio` (`no_std`, no
  allocation) holds it; the kernel only glues ring buffers to it.

Each substream has its own ALSA runtime (`PcmRuntime`, unchanged in
spirit) and its **own ring buffer** in shared memory, in the format the
client chose: any of U8, S8, S16, U16, S24, U24, S24_3, U24_3, S32, U32,
FLOAT, FLOAT64 (little-endian), mono or stereo, 8 to 96 kHz, periods of 64
bytes to 256 KiB, 2 to 64 of them, buffers up to 512 KiB. Its backend
(`SoftGlue`) only fills or reads that buffer and marks the client active.

The **device** runs one stream in one fixed format, chosen at boot: S16
(else S32, FLOAT, S24, U8), stereo where allowed, 48 kHz (else 44.1 kHz,
else its fastest), periods of 20 ms, 4 of them in flight. The audio
thread, whenever the device has room for a period, mixes one: for every
active playback client `poly_audio::server::mix_source` peeks at its
frames from the hardware pointer on, converts them (`Converter`: channels
mapped, mono to both, many to mono by averaging; linear-interpolation
resampling with a 32.32 phase; equal rates pass through), sums them into
an `i32` accumulator (24 significant bits per stream, headroom for the
sum) and reports how many it used, which moves that client's hardware
pointer (`on_played`). A client short of frames gives what it has; its
runtime sees the underrun as before (`avail` reaching `stop_threshold`),
and the end of a drain when everything was taken. The accumulator goes
out clipped after the master gain (`render`). The device stream starts
with the first active client and stops 300 ms after the last (playing
silence meanwhile), so back-to-back streams do not restart it. Sample
conversion is integer-only: IEEE floats are taken apart and built from
their fields, so the kernel needs no FPU state.

Capture runs the other way: each period the device captured goes to every
active capture client (`deliver`), converted to its format after the
capture gain, written at its hardware pointer (`on_captured`); a client
whose buffer is full overruns (XRUN), as ALSA defines it.

Because the server takes a client's frames up to four device periods
before they sound, a client's hardware pointer and the end of its drain
run up to 80 ms ahead of what is audible; `DELAY` does not include it.
Linear interpolation leaves images above the passband; a better filter
can replace it inside `Converter` without touching anything else.

## Decision 2: capture on virtio-sound

`poly-virtio-sound` drives the receive queue: an input message is the
`virtio_snd_pcm_xfer` header (device-readable), a buffer in the PCM memory
and the `virtio_snd_pcm_status` (both device-writable); the device returns
it filled (`submit_rx`, `poll_rx`; per-slot header and status memory like
the transmit side, the buffer's offset kept beside the header where the
device never looks). The used length counts data and status; one that does
not even cover the status, with an OK status, is taken as a full buffer
and counted (`rx_short`), as for QEMU's short control responses. The
simulated device implements the receive queue from §5.14.6.8 and RELEASE
returns the stream's pending input messages empty.

The kernel lends the input stream's four periods (upper half of the 256
KiB PCM memory), copies each filled one out to the server and lends it
again. Input streams become capture devices: `/dev/snd/pcmC0D0c` (minor
`32c + 24 + d`). **Device numbers now count each direction's streams from
0** (ADR 0104 used the virtio stream id): QEMU's output stream 0 stays
`pcmC0D0p`, its input stream 1 is `pcmC0D0c`, where `arecord -D hw:0,0`
looks.

The ALSA runtime gained the capture side (host-tested): `avail` is
captured-and-unread, `READI_FRAMES` and `read(2)` (blocking until all
frames came, like writes), a read of at least `start_threshold` frames
starts a prepared stream, `START` needs nothing written, overruns, poll
readable, `DRAIN` stops the device and leaves DRAINING until the captured
frames are read (the ALSA documentation: "stop PCM permitting to retrieve
residual frames").

QEMU's `wav` backend only records what is played and offers no input
(QEMU documentation, "Audio options"), so the capture boot test uses the
`none` backend, whose input is silence in real time: it checks the reads,
that capturing takes real time, the level (silence), the overrun and its
recovery. Real captured bytes are checked by the host tests (the simulated
device's input, and `deliver` with a tone).

## Decision 3: mmap access and mixer controls

**mmap.** Hardware parameters offer `MMAP_INTERLEAVED` beside
`RW_INTERLEAVED` (`SNDRV_PCM_INFO_MMAP | MMAP_VALID`). A substream's ring
buffer, its status page and its control page are shared anonymous memory
(`AnonShared`, `kernel/src/sound/buffer.rs`: pages made at once, reached by
the kernel through the direct map), so `mmap(2)` of the PCM device maps
the frames the server reads: offset 0 the buffer (mmap access only), the
`SNDRV_PCM_MMAP_OFFSET_STATUS` and `_CONTROL` offsets (old and new: the
same layout on 64-bit) one page each, `MAP_SHARED` only. Character devices
gained one additive hook for this, `CharDevice::as_any`, which the Linux
`mmap` service uses to find an open PCM device (`sound::alsa::map`).

The status page (`struct snd_pcm_mmap_status`: state, hardware pointer,
timestamps) is written after every operation and every server round. The
control page (`struct snd_pcm_mmap_control`) is read before them: a new
application pointer is taken through `PcmRuntime::set_appl` (unwrapped
from the boundary, at most a buffer ahead of the hardware pointer for
playback, at most up to it for capture; anything else is refused and
overwritten), and the kernel writes its own moves back. `SYNC_PTR` without
`SYNC_PTR_APPL` takes the caller's pointer the same way with mmap access
(with read/write access the kernel still moves it). Moving the pointer
does not start a stream: mmap programs call `START`, as ALSA's
documentation of mmap transfers has them do. `REWIND` and `FORWARD` now
move the pointer (over frames the server has not taken, or over free
room) and return how far.

**Controls.** `controlC0` has the controls ALSA's control-name
conventions give a simple card: "Master Playback Volume" (integer 0-100,
two channels), "Master Playback Switch" (boolean, two channels), and with
capture "Capture Volume" and "Capture Switch"; `ELEM_LIST`, `ELEM_INFO`,
`ELEM_READ`, `ELEM_WRITE` (values out of range are `EINVAL` and change
nothing), lookup by `numid` or by interface, name and index
(`poly_personality_linux::sound::ctl`). A change queues a
`SNDRV_CTL_EVENT_ELEM` / `MASK_VALUE` event for every open control file
that subscribed (`SUBSCRIBE_EVENTS`), read with `read(2)` and `poll(2)`.
Locks always succeed, there is no TLV (`ENXIO`), the power state is D0.
The volume is linear in amplitude and applied by the server in software
on both devices: VIRTIO 1.2 has no volume, and the HDA path is set to 0
dB once.

## Decision 4: Intel HDA output

`crates/poly-hda` (`no_std`, no allocation, host-tested against a
simulated controller and codec written from the specification):

- controller reset (GCTL.CRST, the codecs' 521 µs, STATESTS), CORB and
  RIRB set up with the largest rings that fit, verbs sent one at a time
  and polled for, unsolicited responses and answers from other codecs
  skipped;
- every codec's audio function group walked (bounded: 64 widgets, 32
  connections each, paths of at most 8 nodes, no revisits): an output
  path from an output-capable, connected pin (line out preferred, then
  speaker, then headphones) back through mixers and selectors to a DAC;
  powered to D0, output amplifiers unmuted at 0 dB, the path's input
  amplifiers unmuted, connection selects set, pin OUT_EN (+HP_EN), EAPD;
- the first output stream descriptor: a BDL of periods with IOC each over
  a cyclic buffer, the stream format in SDnFMT and on the DAC (stream tag
  1, channel 0), RUN, LPIB as the position.

**QEMU's controller stopped taking commands after the first response.**
RIRBCTL had the response interrupt control off; it turned out (observed:
CORBWP 2, CORBRP 1, RIRBWP 1) that the controller waits for RINTFL to be
cleared once RINTCNT responses are in, and sets RINTFL only with RINTCTL
on. The driver now turns RINTCTL on (INTCTL.CIE stays off, so nothing
interrupts) and clears RINTFL with every response it takes; the simulated
controller behaves the same way, and a host test sends 600 verbs through
it.

The kernel (`kernel/src/sound/hda.rs`) finds the first function of class
04h/03h, maps BAR0, turns on memory decoding and bus mastering, lends four
DMA buffers, connects plain MSI (polled without it) and plays a ring of
four 20 ms periods (a multiple of 128 bytes, the BDL's alignment) round
and round: the position says which periods the DMA engine has left
behind, and those take the next mixed periods; the interrupt per period
only wakes the audio thread, which clears the stream status. HDA is used
when there is no virtio-sound device. Capture on HDA is not done.

## Decision 5: tests

- Host: `poly-audio` (formats, float bit work, channel maps, resampling
  checked by the frequency and level of converted tones, piecewise equals
  whole conversion, two clients mixed into one period, capture delivery);
  `poly-virtio-sound` (input messages lent and returned filled in order,
  release, ranges, short used lengths); `poly-hda` (format words, graph
  search incl. selectors, loops and hostile codecs, configuration, BDL,
  DMA and interrupts, the RINTFL behaviour); the personality (capture
  runtime, mmap pointers and wrapping, status/control encodings, every
  control request); xtask (two tones told apart by a single-bin DFT).
- Kernel (`poly.test=sound::`, with a virtio-sound and with an HDA
  device): the card, nodes, substreams and controls exist, and the server
  takes the frames of a started substream.
- Boot (`sound` group, `/bin/pcm-tone` as init, QEMU's recording analysed
  on the host): ADR 0104's two virtio boots; the master volume set to 50
  then 523 Hz through mmap with status and control pages and `SYNC_PTR`,
  heard at half the level; two processes (440 Hz at 48 kHz, 660 Hz at
  44.1 kHz) heard together at their levels; capture on the `none` backend;
  the default tones on `intel-hda` + `hda-duplex`. All pass on TCG.
  `POLY_SOUND_ONLY=virtio|hda` runs one device's boots.

## Consequences

- `pcm-tone`'s "second open is `EBUSY`" became "8 opens, then `EBUSY`".
- Follow-ups: capture on HDA (an input stream descriptor and an ADC path);
  a better resampler; PAUSE; `aplay`/`arecord` and alsa-lib/PipeWire on the
  persistent root disk once the image has them (mmap and controls are
  there for them); Phase 7's WASAPI and `winmm` on `poly-audio`'s server;
  dB ranges (TLV) for the volume.
