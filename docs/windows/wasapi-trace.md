# Chrome for Windows and WASAPI: the COM-level oracle trace (W2)

Track `spec-win-wasapi` (2026-11-20), batch W2 of [the audio plan](audio-plan.md). Chrome for Windows does not use
`waveOut` (the W1 result): its audio service creates `CLSID_MMDeviceEnumerator` and plays through `IAudioClient`.
Poly's API tracer (ADR 0090) sees calls into imported DLLs only, so the COM calls through vtables were invisible
to it. This document says how they were seen, and what Chrome does.

## How it was observed

- **Application:** Chrome for Testing 157.0.8103.38, win64 (the repository's pinned package), run on the oracle
  homeserver2 (Windows 11 Home 26H2, build 26300.9974; one real sound card, **a tone page that plays at a level no one
  can hear**: a 440 Hz oscillator at a gain of 0.0005).
- **The page** (`tone.html`, not committed): a Web Audio oscillator for 6 s, `suspend()` for 3 s, `resume()`, `close()`
  at 13 s; the page title records the context's state, base latency and sample rate.
- **The wrapper:** a logging COM wrapper (`wspy.dll`, a `no_std` Rust DLL written for this track, not part of Poly):
  registered for the user (`HKCU\Software\Classes\CLSID\{BCDE0395-E52F-467C-8E3D-C4579291692E}\InprocServer32`,
  `ThreadingModel=Both`, removed again after every run) as the in-process server of the device enumerator. It loads
  the system's `MMDevAPI.dll` by its full path, creates the real enumerator, and hands Chrome wrappers: every
  interface that comes back from the audio interfaces (`IMMDeviceEnumerator`, `IMMDeviceCollection`, `IMMDevice`,
  `IPropertyStore`, `IAudioClient`/`2`/`3`, `IAudioRenderClient`, `IAudioClock`, `IAudioSessionControl`, ...) is
  wrapped by its interface id, and each method call is logged (arguments, result, output values) and forwarded
  unchanged. Only the public COM interfaces were used; no Microsoft or Google binary was read, disassembled or
  debugged. Creations the real implementation makes itself while it serves one of our forwarded calls get the real
  factory (a wrapper that also answered those made `IMMDevice::Activate` hang).
- **Process:** Chrome ran with `--no-sandbox` (an unsandboxed audio service can load the wrapper; the sandboxed
  audio service reads the system's registration, not ours, so it was not seen). The browser process and the audio
  utility process (`--type=utility --utility-sub-type=audio.mojom.AudioService`) each write a log.
- **What it cannot see:** calls through objects that are not wrapped (an interface id not in the wrapper's list is
  passed through and only named), the notifications Windows sends *into* Chrome's own objects
  (`IMMNotificationClient`, `IAudioSessionEvents`), what happens in a sandboxed audio process, and calls Chrome
  makes through `ActivateAudioInterfaceAsync` (mmdevapi's ordinal 17, which `chrome.dll` delay-loads). In every run
  Chrome activated its clients through `IMMDevice::Activate`; two runs of a variant of the wrapper that stood in for
  `mmdevapi.dll` under that name and exported the function at ordinal 17 saw no call to it (that variant could not
  carry a stream, so the evidence is for the start of the audio service only).

The committed form is `specs/windows/apps/chrome/traces/s6-audio.trace.json`: per process, the ordered calls with
their arguments and results, thread numbers renumbered, object numbers in creation order, the render loop collapsed
to its pattern. Only what every run shared was kept.

## What Chrome does (the shared-mode render path)

1. **Start of the audio service:** `CoCreateInstance(CLSID_MMDeviceEnumerator)`, `QueryInterface(IMMDeviceEnumerator)`,
   `RegisterEndpointNotificationCallback`, `EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)` + `GetCount`,
   `GetDefaultAudioEndpoint(eRender, eConsole)`, `GetState`, `GetId`, `GetDevice(id)`.
2. **Device parameters** (repeated for each new stream or query): `IMMDevice::Activate(IAudioClient,
   CLSCTX_INPROC_SERVER)`, `IAudioClient::GetMixFormat` (48000 Hz, 2 channels, 32-bit float, `WAVE_FORMAT_EXTENSIBLE`),
   `QueryInterface(IAudioClient3)`, `IAudioClient3::GetSharedModeEnginePeriod(mix format)` (480 frames in all four
   outputs on the oracle).
3. **Opening the stream:** another `Activate`; `IsFormatSupported(SHARED, mix format)` = `S_OK`;
   `Initialize(SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 0, 0, mix format, NULL)` = `S_OK` (the call takes about
   200 ms); `SetEventHandle`; `GetBufferSize` (1056 frames); `GetStreamLatency` (0); `GetDevicePeriod` (default
   100000, minimum 30000 in 100 ns units); `GetService(IAudioRenderClient)`, `GetService(IAudioClock)`,
   `GetService(IAudioSessionControl)`, `IAudioSessionControl::RegisterAudioSessionNotification`.
4. **Starting:** `GetBufferSize`, `GetCurrentPadding` (0), `GetBuffer(1056)`, `ReleaseBuffer(1056,
   AUDCLNT_BUFFERFLAGS_SILENT)` (the whole buffer is queued as silence), `IAudioClock::GetFrequency`, then `Start`.
5. **The render loop**, on its own thread, once per event (every 10 ms): `GetCurrentPadding` (576 = 1056 - 480),
   `IAudioClock::GetPosition`, `GetBuffer(480)`, `ReleaseBuffer(480, 0)`.
6. **`suspend()`:** `Stop`, `Reset`, `GetCurrentPadding` (0). **`resume()`:** the same client again: device queries
   as in step 2, then step 4 and `Start`. **`close()`:** `Stop`, `Reset`; the client stays until the process ends.
7. **The browser process** only creates the enumerator and queries it once.

`IAudioClient2`, `IAudioClient3::InitializeSharedAudioStream`, `IAudioClockAdjustment`, `ISimpleAudioVolume` and the
capture interfaces were not used in these runs.

The sheets under `specs/windows/mmdevapi.dll/` and their tests cover exactly this set plus the errors a caller meets.
