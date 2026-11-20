# 0244: The Windows side reaches the mixer through a Poly audio device

**Status:** Accepted. Batch W0 of [the audio plan](../windows/audio-plan.md), track `windows-audio-path`. Builds on
[ADR 0109](0109-audio-server-capture-mmap-mixer-and-intel-hda.md) (the kernel's audio server), [ADR 0067](0067-nt-file-io.md) (NT file I/O
and its asynchronous requests), [ADR 0080](0080-console-host.md) and [ADR 0095](0095-winsock-on-polys-sockets.md) (Poly's own
device protocols for the console and for Winsock).

## Context

Chrome for Windows plays no sound on Poly: `mmdevapi`'s enumerator is not registered and `winmm`'s
`waveOutGetNumDevs` answers 0 (the survey, `docs/windows/audio-plan.md`). W1 (`winmm` wave output) and W2 (WASAPI
shared mode) will give those APIs their behaviour, sheet by sheet. Both need the same thing underneath, which no
Windows API defines: a way from a Windows process to the kernel's audio server, the one ALSA clients already play
through (ADR 0109). That way is Poly's own, like the console's and AFD's: Windows' interface below its sound APIs
(the audio engine service, kernel streaming) is not documented.

What W1 and W2 need of it, from Microsoft Learn's descriptions of the two APIs:

- whether there is an output device, and its format, period and buffering (`waveOutGetNumDevs`, `GetMixFormat`,
  `GetDevicePeriod`, `GetStreamLatency`);
- a stream in the program's format, converted and mixed by the server (`waveOutOpen`, `Initialize` with
  `AUTOCONVERTPCM`);
- writing frames without blocking, and how many the stream still holds (`waveOutWrite`, `GetBuffer` /
  `ReleaseBuffer`, `GetCurrentPadding`);
- start, pause, reset, a volume per stream (`waveOutRestart`/`Pause`/`Reset`, `Start`/`Stop`/`Reset`,
  `waveOutSetVolume`, `ISimpleAudioVolume`);
- the position, and being told when the stream got as far as a given frame: a `WAVEHDR` is done when its last frame
  played (`WOM_DONE`), an event-driven client is woken once a period (`SetEventHandle`).

The plan sketched Poly services, a ring buffer shared as a section and an event the kernel sets each period. This
ADR does it with less new machinery.

## Decision

**A device, `\Device\PolyAudio`, with Poly's I/O control codes.** A stream is a file object opened on it with
`NtCreateFile` and closed with `NtClose`; everything else is `NtDeviceIoControlFile` with
`CTL_CODE(FILE_DEVICE_SOUND, 0x800 + n, METHOD_NEITHER, FILE_ANY_ACCESS)`, the vendor range of Learn's "Defining I/O
Control Codes". The protocol lives in `poly_audio::nt` (no dependencies, no allocation), so the DLLs of W1 and W2 and
the personality share one definition:

| Code | Input | Output | Does |
|---|---|---|---|
| `DEVICE` | | format, period, periods | the card's output stream; `STATUS_NO_SUCH_DEVICE` without a card |
| `OPEN` | format, buffer frames | | makes the handle's stream, once |
| `WRITE` | whole frames | frames taken | as many as there is room for; never waits |
| `START`, `STOP` | | | the mixer takes the stream's frames, or holds it |
| `RESET` | | | stops, drops the frames, counts from 0, cancels the waits |
| `SET_GAIN` | 1–8 gains (16.16) | | per channel; one value is all channels |
| `QUERY` | | written, played, silence, underruns, running, buffer, queued, device rate/period/periods | |
| `WAIT` | a played-frame count | the count reached | finishes when the device has played that many frames |

- **`WAIT` instead of an event set by the kernel.** It is an ordinary NT request: it completes at once when the
  count is reached, else it is pending and completes later through its event, its APC, the handle's completion port
  or, on a synchronous handle, in the call. So `WOM_DONE` is a wait for a header's last frame, and a period event
  is a wait for the next period's frame count, set by the DLL's own thread; the kernel never holds a reference to
  a program's event, and cancellation, `CancelIo` and the end of a thread work as for every other request. The
  personality parks the request and hands it to the kernel's `nt-audio` worker (`NtKernel::audio_pend`), which
  completes it under the NT lock when the audio thread says frames were played, as the `nt-afd` worker does
  for sockets.
- **The frames are copied, not shared.** `WRITE` carries them in its input buffer. A shared section would save one
  copy of at most a few hundred KB a second and needs a mapping per process, its own lifetime and a second
  position for what the program wrote; W2's `GetBuffer`/`ReleaseBuffer` can be built on a buffer in the DLL's own
  memory that `ReleaseBuffer` writes. If a trace shows the copy matters, a mapped buffer can be added as another
  request.
- **The kernel's stream** (`kernel/src/sound/ntstream.rs`) is a client of the same server as an ALSA substream:
  its ring buffer (the accounting is `poly_audio::nt::Ring`, host-tested), a `Converter` to the device's format,
  its gains. Every output period the server mixes the running streams beside the substreams; a running stream
  that ran out plays silence for the rest of its share, counted in `silence`, with one `underrun` per dry spell.
  The device stream runs while any substream or stream runs (ADR 0109's idle rule). The card keeps weak
  references: the stream ends with its file object.
- **The personality** (`personalities/nt/src/audio.rs`) keeps a handle's stream in its file record
  (`Target::Audio`) and gets streams from the kernel through `NtKernel::audio_device`, `audio_open` and
  `audio_pend`, whose defaults answer as a machine without a card, so host tests use a fake.
- **Playback only.** Capture (W4) adds requests when it has its sheets.
- **No Windows API behaviour here.** Which formats `waveOutOpen` accepts, what `GetCurrentPadding` counts, the
  latency a client is told: the DLLs decide with their sheets, on these facts.
- **Who may open it.** The device carries no security descriptor of its own, as `\Device\Afd` and
  `\Device\PolyConsole` do not, so any token opens it, a restricted or AppContainer one too. Whether Chrome's
  sandbox lets its audio process call `NtCreateFile` on it is for W1 to observe.

## Consequences

- W1 and W2 build on one tested path: a Windows program (a scratch probe, not committed) played 2 s of a 441 Hz
  tone through it on Poly with 30 synchronous `WAIT`s and no underrun, and QEMU's `wav` backend recorded the tone.
- A program sees only what Poly's DLLs give it; the device is not a Windows interface and no Windows program
  should open it itself.
- One more client kind in the server: its lock order is the card's lock, then a stream's.
- **Played means played by the device.** The mixer runs several device periods ahead of the speaker (four of 20 ms
  on virtio-sound). A stream's frames count as played when the device finished the period they went out in: each
  stream notes its frames per period in flight, and the server credits them as the device frees periods (all of
  them when it stops). `queued` stays the ring's occupancy (written and not yet taken), so a writer sees room as
  soon as the mixer took frames. A first version counted frames when the mixer took them; `waveOut`'s sheets
  (W1) showed buffers reported done 70 ms early, so this ADR was amended with W1.
