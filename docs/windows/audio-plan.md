# Sound for Chrome on Poly (Windows and Linux): survey and plan

Track `survey-audio` (2026-11-20). Question: what does it take for Chrome for Windows on Poly to play sound
(YouTube, web audio, notification sounds), and for Linux Chromium? The QA agent could not test sound: there is
no sound device under `polyd`.

The answer comes in two parts:
- **Linux Chromium already plays sound** through Poly's ALSA device, as soon as the machine has a sound card.
  The desktop's QEMU line has none today.
- **Chrome for Windows asks for WASAPI first, then falls back to `winmm`'s `waveOut`.** Poly has neither, so
  Chrome plays into a silent fake device without any error.

Inputs (clean room):
- Poly's own code and ADRs.
- QEMU's `-device help` and `-audiodev help` on this PC.
- Import *names* of `chrome.dll` (157.0.8103.38), read from its PE tables.
- The recorded oracle traces under `specs/windows/apps/chrome` (recorded calls only).
- Two runs of the browsers **on Poly**, observed through Poly's kernel log and a recording of what the guest
  played.
- No Wine, ReactOS, Chromium, PulseAudio or PipeWire source was read.

## 1. What Poly has today

| Layer | State | Where |
|---|---|---|
| **QEMU device** | `virtio-sound-pci` and `intel-hda` + `hda-duplex` are driven. Only the boot tests attach one (`wav` or `none` backend: `cargo xtask boot-tests sound`, `toolkits`). **`cargo xtask desktop` (also `--chrome-win`, `--chromium`) attaches no sound device**, so on the desktop there is no card and the compositor logs "no mixer (/dev/snd/controlC0: not found): the ridge shows no volume". The Windows QEMU on this PC (11.1) has the host backends `dsound` and `sdl` (also `wav`, `none`) and the devices `virtio-sound-pci`, `intel-hda`/`ich9-intel-hda` + `hda-*`, `AC97`, `ES1370`, `sb16` | `tools/xtask/src/sound.rs`, `desktop.rs` |
| **Kernel driver** | virtio-sound (`crates/poly-virtio-sound`, ADR 0104) and Intel HDA output (`crates/poly-hda`, ADR 0109), host-tested. Observed in this survey: "card 0 … virtio-sound, 2 streams", device stream S16, stereo, 48 kHz, periods of 960 frames × 4 | `kernel/src/sound/` |
| **Audio server** | **in the kernel** (ADR 0109): every client gets its own ring buffer in its own format; the audio thread mixes every playing client into the device's stream each period (conversion, linear resampling, master gain) and hands captured periods to recording clients. 8 playback and 8 capture clients per card; controls (master volume, mute, capture). ADR 0109 already names WASAPI and `winmm` as its next clients | `kernel/src/sound/mod.rs`, `crates/poly-audio` |
| **Linux API** | ALSA's kernel interface (`/dev/snd/controlC0`, `pcmC0D0p`, `pcmC0D0c`; read/write and mmap access, `SNDRV_*` ioctls, subdevices). alsa-lib programs work unchanged (VLC's sound plays in the `toolkits` boot group). **No PulseAudio or PipeWire server**: Chromium's Debian root disk carries `libasound2t64` and its data and the `libpulse0` client library, but no server | `kernel/src/sound/alsa.rs`, `tools/xtask/chromium.lock` |
| **Windows API** | `winmm.dll`: only `timeGetTime`, `timeBeginPeriod`, `timeEndPeriod` (coverage: 64 `stub`, 125 `missing`; every `waveOut*`/`midi*` binds to a fail stub). `mmdevapi.dll`, `avrt.dll` and `dsound.dll` exist only as generated stub DLLs (ADR 0155, step 10): `CLSID_MMDeviceEnumerator` is not registered. No `audioses.dll`, `ksuser.dll` or audio service | `personalities/nt/win32/winmm`, `crates/poly-stubs`, `coverage/windows/api/{winmm,mmdevapi,avrt,dsound}.dll.json` |

## 2. What the browsers do (observed on Poly)

Setup:
- WHPX, 2560x1600 at 200 %.
- A local switch (not committed) added `-audiodev wav,id=snd0,path=… -device virtio-sound-pci,audiodev=snd0` to
  the desktop's QEMU line.
- A driver typed a `data:` page whose full-window button starts a 440 Hz WebAudio oscillator at half volume for
  4 s, then clicked it. A click satisfies the autoplay policy.
- What the guest played was recorded on the host.

### Linux Chromium: plays

The recording holds the 440 Hz tone at the expected level (RMS ≈ 11 600 of 32 767, i.e. half scale) for 4.1 s:
the oscillator's whole life. No PulseAudio server ran: Chromium went to ALSA's default device, i.e. Poly's
`/dev/snd`, and the kernel mixer. Nothing else is needed for Chromium; the missing piece is the QEMU device.

### Chrome for Windows: silent, without an error

Calls in Poly's log (`stub:` lines), in this order:

1. **The browser process at start-up:** `CoCreateInstance(CLSID_MMDeviceEnumerator
   {BCDE0395-E52F-467C-8E3D-C4579291692E}, IID_IMMDeviceEnumerator {A95664D2-9614-4F35-A746-DE8DB63617E6})`:
   not registered.
2. **The click starts a new Chrome process** (by its timing, Chrome's audio service). It makes the same
   `CoCreateInstance` (not registered), then **falls back to `winmm`**: `waveOutGetNumDevs` (fail stub, 0
   devices), twice.
3. The page still reports `AudioContext` state `running` at 44 100 Hz: Chrome plays into a fake output. The
   recording is empty (header only).

Chrome's import tables (names only) agree:
- `winmm`: `waveOutGetNumDevs`, `waveOutOpen`, `waveOutPrepareHeader`, `waveOutWrite`, `waveOutPause`,
  `waveOutRestart`, `waveOutReset`, `waveOutUnprepareHeader`, `waveOutClose`, `waveInGetNumDevs`, the `midiIn*`
  and `midiOut*` families, and the three timer functions. All of them are static imports.
- `mmdevapi`: one ordinal, `#17`, delay-loaded. The coverage map does not know it: it lists `#2`–`#16` and the
  named exports.
- No imports from `avrt`, `audioses` or `dsound`: WASAPI comes through COM (and `LoadLibrary`).
- `mf`, `mfplat`, `mfreadwrite` (delay-loaded) are media and device helpers. Chrome decodes YouTube's formats
  itself, so **sound needs an output API, not Media Foundation**.

The recorded oracle traces (`specs/windows/apps/chrome/traces`) show `mmdevapi.dll` and `mf*.dll` being loaded
and no audio calls: none of the recorded stages played sound. An oracle trace of a page that plays sound is the
first job of the spec agent in W2.

## 3. The plan, in batches

Each Windows batch = spec sheets on the oracle, implementer, integrator, as in
[chrome-installer-plan.md](chrome-installer-plan.md). Sizes: S < M < L < XL.

| Batch | Content | Oracle spec agent? | Size | What the owner hears afterwards |
|---|---|---|---|---|
| **A0 A sound card on the desktop** | `cargo xtask desktop` attaches `virtio-sound-pci` with the host's backend by default: `dsound` on Windows (QEMU 11.1 here), `pa`/`pipewire`/`alsa` on Linux hosts when QEMU has them, `none` otherwise. `--no-sound` turns it off. `--record-sound FILE` uses `wav` instead, so QA and agents under `polyd` can check sound without speakers, as the `sound` boot test does. A boot check that the desktop's mixer control appears. | no (implementer only) | **S** | **Linux Chromium plays sound** (YouTube, web audio), and so do VLC and every ALSA program. The desktop's volume control shows and works. |
| **A1 Linux check** | QA: Linux Chromium on YouTube and notification sounds, two at once (mixing), volume, and a long video (drift, underruns). Linux apps that need a PulseAudio server (e.g. Firefox's) are listed, not fixed. | no | S | confirms A0 |
| **W0 The NT path to the mixer** | The kernel's audio server gets a client kind for the NT personality: Poly NT services to open a playback (later capture) stream in a format, a ring buffer shared as a section, the position, an event set every period, the stream's volume, start/stop/reset. Host-tested in `poly-audio` and the NT personality. Needs an ADR. No new process: the server is the one ALSA uses (ADR 0109). | no (Poly-internal) | **M** | nothing yet |
| **W1 `winmm` wave output** | `waveOutGetNumDevs`, `waveOutGetDevCapsW/A`, `waveOutOpen` (`WAVE_FORMAT_PCM`, `IEEE_FLOAT`, `EXTENSIBLE`, `WAVE_FORMAT_QUERY`, callbacks `NULL`/event/function/window/thread with `WOM_OPEN`/`WOM_DONE`/`WOM_CLOSE`), `waveOutPrepareHeader`/`UnprepareHeader` (`WHDR_PREPARED`), `waveOutWrite` (`WHDR_INQUEUE`/`WHDR_DONE`, loops), `waveOutPause`/`Restart`/`Reset`/`BreakLoop`, `waveOutGetPosition` (MMTIME types), `waveOutGet/SetVolume`, `waveOutClose` (`WAVERR_STILLPLAYING`), `waveOutGetErrorTextW`, `waveOutMessage`, on W0. About 16 entries. | **yes**: callback order and threads, header flags, positions, errors, what a closed or reset device does | **M** | **Chrome for Windows plays sound** through its fallback (observed: it asks `waveOutGetNumDevs` when WASAPI is missing): YouTube, web audio, notification sounds. Older Windows programs that use `waveOut` too. |
| **W2 WASAPI shared-mode output** | `CLSID_MMDeviceEnumerator` registered in-process (Poly's own `mmdevapi.dll` replaces the stub DLL). `IMMDeviceEnumerator` (`EnumAudioEndpoints`, `GetDefaultAudioEndpoint`, `GetDevice`, endpoint notification registration), `IMMDeviceCollection`, `IMMDevice` (`Activate`, `GetId`, `GetState`, `OpenPropertyStore`: friendly name, form factor, engine format), `IMMEndpoint`. `IAudioClient`/`2`/`3` in shared mode (`GetMixFormat`, `IsFormatSupported`, `Initialize` with event callback and `AUTOCONVERTPCM`, `GetDevicePeriod`, `GetBufferSize`, `GetStreamLatency`, `GetCurrentPadding`, `SetEventHandle`, `Start`/`Stop`/`Reset`, `GetService`). `IAudioRenderClient` (`GetBuffer`/`ReleaseBuffer`, silent flag), `IAudioClock`. `avrt`: `AvSetMmThreadCharacteristicsW`, `AvRevertMmThreadCharacteristics`. `mmdevapi` ordinal `#17` (Chrome delay-loads it; its identity from the export table). All on W0. About 30 methods. | **yes**: first an oracle trace of Chrome playing sound, to fix which methods and flags it uses; then the sheets (formats offered, periods and buffer sizes, padding and position arithmetic, event timing, errors such as `AUDCLNT_E_NOT_INITIALIZED`, `AUDCLNT_E_ALREADY_INITIALIZED`, `AUDCLNT_E_UNSUPPORTED_FORMAT`) | **L** | Chrome for Windows on its primary path: lower latency and better audio/video sync; Chrome's settings name the output device. Chrome stops using `waveOut` (W1 stays for other programs). |
| **W3 Volume, sessions, device changes** | `IAudioSessionControl`/`2`, `IAudioSessionManager`/`2`, `ISimpleAudioVolume`, `IAudioEndpointVolume` (`Activate` on the device), `IAudioSessionEvents` and `IMMNotificationClient` callbacks. The endpoint volume is the desktop mixer's master volume, so both show the same level. | **yes** | **M** | Windows apps follow the desktop's volume and mute; a per-app volume in Settings becomes possible. |
| **W4 Capture** | `IAudioCaptureClient` and `waveIn*` on the server's capture clients (ADR 0109 has capture); QEMU input from the host's microphone (`dsound` input). | **yes** | **M** | microphones for WebRTC (Meet), later |
| **W5 DirectSound** | `DirectSoundCreate8`, `IDirectSound8`, `IDirectSoundBuffer8` on W0. Not used by Chrome. | **yes** | **M** | older games and programs; defer |
| **L2 PulseAudio-protocol clients** | For Linux apps that require a PulseAudio server (Chromium does not). Two options: (a) Debian's PipeWire with `pipewire-pulse`, run as packaged programs on the root disk over Poly's ALSA device, as Chromium is run; (b) a Poly-native server speaking the PulseAudio protocol (little public documentation: large). Recommend (a) when an app needs it. | no | M (a) / XL (b) | Firefox and other PulseAudio-only Linux apps |

**Order:** A0 first (small, and Linux Chromium sounds at once), then W0 → W1 (Chrome for Windows sounds), then W2
→ W3; W4, W5 and L2 when an app needs them.

**Why W1 before W2:**
- `waveOut` is a quarter of the size of WASAPI.
- Chrome was seen falling back to it.
- It gives the owner sound in Chrome for Windows one batch earlier.

**The risk:** Chrome's `waveOut` path was observed only up to `waveOutGetNumDevs` returning 0. Whether it plays
well with `waveOut` (its latency, its buffer sizes) is unknown until W1 runs. If it does not, W2 is the path and W1
still serves other programs.

### Status

- **A0 done** (`desktop-sound`). `cargo xtask desktop` gives the guest virtio-sound (else Intel HDA) on the host's
  backend: `dsound`, `pa`/`pipewire`/`alsa` or `coreaudio`. `--no-sound` and `--record-sound=FILE` are there too.
  A host whose output cannot start keeps a card that plays into nothing, after a short probe and with one line in
  the log. Checked on WHPX: the plain desktop boots on `dsound` with the mixer control present, and Linux
  Chromium's 440 Hz tone was recorded through `--record-sound`. In the `polyd` container, `alsa` fails and the
  probe falls back quietly.
- **W0 done** (`windows-audio-path`, [ADR 0244](../adr/0244-the-windows-side-reaches-the-mixer-through-a-poly-audio-device.md)).
  It differs from the sketch above: a device, `\Device\PolyAudio`, with Poly's I/O control codes
  (`poly_audio::nt`). The frames are copied in by a `WRITE` request, not shared in a section. A `WAIT` request that
  finishes at a played-frame count replaces a kernel-set event, so `WOM_DONE` and a WASAPI period event are each a
  pending request. Playback only. A scratch Windows probe played 2 s of 441 Hz through it on Poly, and QEMU recorded
  it. W1 and W2 build on it.

## 4. Where this plan is least certain

- **What Chrome does with WASAPI when it works.** Not traced yet (no oracle trace plays sound). The method list
  in W2 is from Microsoft Learn's description of shared-mode rendering, not from a trace; the first W2 job fixes
  it.
- **`mmdevapi` ordinal `#17`.** Chrome delay-loads it; which export it is, and when Chrome calls it, is open.
- **Timing under WHPX and dsound.** The guest's 48 kHz stream with 20 ms periods and 4 in flight went through
  QEMU's `wav` backend here. With `dsound` the host's own latency adds; underruns under load are A1's question.
- **The audio process's sandbox.** The process that asked for audio is a fresh Chrome process. If it runs
  sandboxed, W0's services must be reachable from a restricted token: an ADR question for W0.

## 5. Commands and evidence (not committed)

- `chrome.dll` imports: a 40-line PE import reader (names only) over
  `target/chrome-win/157.0.8103.38/chrome-win64/chrome.dll`.
- Two desktop runs (`cargo xtask desktop --release --chrome-win|--chromium --size=2560x1600 --scale=2`) with the
  local sound switch and a QMP driver. Their logs, screenshots and the recordings are kept in the survey's
  worktree scratch (`.b2a/au-win*`, `.b2a/au-linux*`).
- The switch itself is A0's first line.

## 6. W1 status: the `waveOut` sheets (spec-win-waveout, 2026-11-20)

Observed on the oracle (homeserver2 has a real sound card, so `waveOut` works from the SSH session; only silence was
played). 19 entries under `specs/windows/winmm.dll/` (`waveOutGetNumDevs`, `GetDevCapsW/A`, `Open`, `Close`,
`PrepareHeader`, `UnprepareHeader`, `Write`, `Pause`, `Restart`, `Reset`, `BreakLoop`, `GetPosition`, `GetVolume`,
`SetVolume`, `GetErrorTextW/A`, `GetID`, `Message`), 74 recorded tests (about 70 % visible). Chrome imports ten of them
(`GetNumDevs`, `Open`, `PrepareHeader`, `Write`, `Pause`, `Restart`, `Reset`, `UnprepareHeader`, `Close`, and the
`waveIn` count); the rest are for older programs. What the sheets fix, in short:

- **Callbacks.** `WOM_OPEN` and `WOM_CLOSE` are delivered synchronously, on the calling thread, before `waveOutOpen` /
  `waveOutClose` return; `WOM_DONE` comes on another thread, once per buffer, in the order written, with
  `WHDR_DONE` already set and `WHDR_INQUEUE` already cleared. `waveOutReset` delivers every pending `WOM_DONE` before it
  returns. `CALLBACK_EVENT` sets the event at open, at each `WOM_DONE`, at reset and at close. Window and thread callbacks
  are posted messages.
- **Headers.** `PREPARED` set by prepare (clears `DONE`, keeps the loop flags); `INQUEUE` set by write; `DONE|PREPARED` when
  played; unprepare clears only `PREPARED`; `WAVERR_UNPREPARED` / `WAVERR_STILLPLAYING` / `MMSYSERR_INVALPARAM` rules.
- **Position** is in bytes and samples of the opened format, 0 after reset, frozen while paused; every other
  `MMTIME` type is answered in bytes.
- **Formats:** PCM 16 bit at any common rate, 24/32 bit, float 32 bit and `WAVE_FORMAT_EXTENSIBLE` (float and int16) are
  accepted; the query and the real open differ for a few bad formats (`WAVERR_BADFORMAT` against `MMSYSERR_INVALPARAM`).
- **Volume** is per process (0xFFFFFFFF at start), read back exactly; the system volume is not touched.
- **No device** (the oracle's card disabled for the recording): `waveOutGetNumDevs` is 0 and every call that names a device,
  the mapper included, is `MMSYSERR_BADDEVICEID` (not `MMSYSERR_NODRIVER`). This is what Chrome meets today on the desktop
  without a sound card.

To run the tests on Poly the boot needs a sound device (virtio-sound) for every test but four; those four (their first line
says `NO SOUND DEVICE`: one each in `waveOutGetNumDevs`, `GetDevCapsW`, `Open`, `GetVolume`) need a boot **without** one.
`win poly` and `polycheck` boot one configuration per run, so the integrator has to give them both (the two sets can be
told apart by that first line).

### W1 implemented (impl-win-audio, 2026-11-20)

- **`winmm`'s `waveOut*`** on `\Device\PolyAudio` (`personalities/nt/win32/winmm/src/wave.rs`, the decisions in
  `poly_kernelbase::waveout`). The 48 visible tests pass on Poly. `win poly` and `polycheck` now boot each test the
  way it needs (`sound` in the manifest, `docs/AGENT_PIPELINE.md`): a card for the tests that import a `winmm` sound
  function, none for the four `NO SOUND DEVICE` tests.
- **ADR 0244 amended.** A stream's frames count as played when the device finished their period, not when the mixer
  took them: the sheets caught buffers reported done about 70 ms early.
- **Chrome for Windows does not play through `waveOut`.** Chrome 157 on the desktop with a sound card was observed
  with a trace line on every `waveOut*` entry (not committed). Its audio process:
  - asks for `CLSID_MMDeviceEnumerator`, which is not registered;
  - calls `waveOutGetNumDevs` two or three times and gets 1;
  - never calls `waveOutOpen`, `waveOutGetDevCaps*` or any other `winmm` function.

  The tone page's `AudioContext` runs (44.1 kHz, output latency 32 ms) into nothing: QEMU's recording stays empty.
  Starting Chrome with `--force-wave-audio` (it reached the browser, per `chrome://version`) changes nothing. **Chrome
  needs W2 (WASAPI shared mode)**; W1 serves programs that use `waveOut` themselves.

## 7. W2 status: the WASAPI shared-mode render path (spec-win-wasapi, 2026-11-20)

The oracle trace first ([wasapi-trace.md](wasapi-trace.md), `specs/windows/apps/chrome/traces/s6-audio.trace.json`): Chrome's
audio service was **not seen calling `ActivateAudioInterfaceAsync`** (and `chrome.dll` has no `avrt` import): it creates `CLSID_MMDeviceEnumerator`, takes
the default render endpoint, `Activate`s an `IAudioClient` (`CLSCTX_INPROC_SERVER`), asks the mix format (48000 Hz, 2 channels,
32-bit float, extensible) and `IAudioClient3::GetSharedModeEnginePeriod` (480 frames), then
`Initialize(SHARED, EVENTCALLBACK, 0, 0, mix format, NULL)`, `SetEventHandle`, `GetService` for `IAudioRenderClient`,
`IAudioClock` and `IAudioSessionControl` (+ `RegisterAudioSessionNotification`), prefills the 1056-frame buffer as silence,
`Start`, and renders one 480-frame buffer per event (`GetCurrentPadding`, `IAudioClock::GetPosition`, `GetBuffer`,
`ReleaseBuffer`). `suspend()` is `Stop` + `Reset`, `resume()` the same client again.

12 entries under `specs/windows/mmdevapi.dll/` (`CLSID_MMDeviceEnumerator`, `IMMDeviceEnumerator`, `IMMDeviceCollection`, `IMMDevice`,
`IMMNotificationClient`, `IAudioClient.Format`, `.Initialize`, `.Stream`, `.Events`, `IAudioRenderClient`, `IAudioClock`,
`IAudioSessionControl`) and their recorded tests. What the sheets fix beyond Chrome's calls: the result codes of every misuse
(not initialised, already initialised, wrong format, out of order, buffer too large, wrong flags, event handle not set or not
expected), the buffer-size and period arithmetic (`2 * rate / 100 + 96` frames at the minimum, the duration at the rate above
it), padding, the clock counting bytes (frequency 384000, position equal to the bytes written when played out), the event rate
(one per 10 ms) and the session state notifications. Device removal under a running stream was observed once, by restarting
the Windows Audio service: `AUDCLNT_E_DEVICE_INVALIDATED` everywhere (in the sheet of `IAudioClient.Stream`, no test).
