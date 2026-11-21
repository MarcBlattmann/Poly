# 0259: WASAPI: an engine per client, on 10 ms device periods

**Status:** Accepted. Batch W2 of [the audio plan](../windows/audio-plan.md), track `impl-win-audio-w2`. Builds on
[ADR 0244](0244-the-windows-side-reaches-the-mixer-through-a-poly-audio-device.md) (`\Device\PolyAudio`) and
[ADR 0109](0109-audio-server-capture-mmap-mixer-and-intel-hda.md) (the kernel's mixer).

## Context

Chrome for Windows plays sound through WASAPI shared mode only. W1 showed that it does not open `waveOut`. The W2
sheets (`specs/windows/mmdevapi.dll/`) and the oracle trace (`docs/windows/wasapi-trace.md`) fix Chrome's path:

1. enumerator, default render endpoint, `Activate(IAudioClient)`, `GetMixFormat` (48000 Hz, 2 channels, float);
2. `Initialize(SHARED, EVENTCALLBACK)`, a 1056-frame buffer;
3. `SetEventHandle`, then the render client, clock and session;
4. a 1056-frame prefill, `Start`, then 480 frames per event, one event per 10 ms engine period.

The sheets are strict about time. The padding at *every* wake-up is the buffer less exactly one period. The clock
counts the bytes the engine took, so it equals the bytes written as soon as the padding is 0.

Poly's device side did not have a 10 ms grain. The device ran 20 ms periods, and a stream counts as played period
by period (ADR 0244). A program waiting for each next 10 ms, a 48 kHz stream kept fed and timed with
`QueryPerformanceCounter` over 300 waits, measured:

| Device periods | Mean | Within 2 ms of 10 ms | Standard deviation | Shape |
|---|---|---|---|---|
| 4 × 20 ms | 10.0 ms | 5 of 300 | 10.1 ms | about 0 and 20 ms alternating |
| 6 × 10 ms | 9.96 ms | 293 of 300 | 1.4 ms | 10 ms; 6 near 0 (two periods finished at once) |

## Decision

- **10 ms device periods, six in flight**, on virtio-sound and Intel HDA alike. The device buffer drops from 80 ms to
  60 ms. ALSA clients see the same server; the `sound` boot tests (both drivers, both directions) pass unchanged.
- **`mmdevapi.dll` is Poly's own** (it replaces the generated fail-only stub; ordinal 17 and the other imported
  names stay fail-only). `CLSID_MMDeviceEnumerator` is served through `poly_com::inproc`, as the other classes of
  Poly's DLLs are. One render endpoint exists when the machine has a sound card; there is no capture endpoint.
  `audioses.dll` exists as an empty server for the session classes; no sheet asks for more yet.
- **The rules live in `poly_kernelbase::wasapi`, host-tested:** formats, buffer sizes, the client's state, the
  render client's `GetBuffer`/`ReleaseBuffer` order, start, stop, reset, the event handle, session registrations and
  the device id. The DLL is the COM shell around them.
- **An engine thread per initialised client.** It owns a PolyAudio stream in the client's format; the kernel converts
  and mixes. Its two jobs are kept apart, because the application can see the difference:
  - **Sending ahead.** Released frames go to the device stream so that it always holds two periods. They can no
    longer change, so sending them early is safe. The second period is a lead over the mixer. When the device
    finished two periods in one round (a Windows host's audio timer does that), the mixer takes two at once and
    still finds frames. Silence goes in only when the application has written nothing more.
  - **Consuming, on the engine's own clock.** Every 10 ms, on an absolute schedule of the performance counter, one
    period leaves the client's buffer and the event is set. The padding at each wake-up is exactly the buffer less
    one period, as the sheets require. `Start` consumes the first period at once. The device's audio runs on the
    same host clock, so the two keep pace; the device stream's level absorbs the difference in between. If the device
    falls behind (more than four periods queued), a tick does not consume, so no frame is ever dropped. If the
    machine was too busy to tick for more than two periods, the schedule starts again instead of catching up in a
    burst.
  - Session state notifications go out from this thread, as on Windows.
  - Two designs that followed the device were dropped; the visible tests and Chrome showed why. **Consuming per
    period played** lagged by the device's six periods in flight, so the engine ran out of frames it could send
    (Chrome: silence and 5–8 % underruns). **Consuming per period the mixer took** inherited the device's bursts
    and stalls: one or two events lost, or two close together, in about one run of the event test in six.
- **The clock counts consumed bytes**, so it matches the sheet's numbers (8448 bytes after a 1056-frame play-out).
- **A running stream holds a reference on its client**, as on Windows (`IAudioClient.Stream.yaml`: released while
  it runs, the client still counts one). `Stop` gives it back.
- **One diagnostic line per stream** in the kernel log, at each stop and when the client goes: periods fed,
  silence put in for the application, and the device stream's underruns.
- **Methods no sheet specifies** answer `E_NOTIMPL` with their exact argument count, so a 32-bit `stdcall` caller's
  stack stays right. Examples: `IMMDevice::OpenPropertyStore`, `IAudioClient2`'s three methods,
  `ISimpleAudioVolume`, the session's name, icon and grouping methods.
- **The sandbox.** Chrome's audio process runs with a restricted token at Low integrity. Its restricting SIDs are
  Users, Everyone, INTERACTIVE, Authenticated Users, RESTRICTED, the user and the logon SID; it is in a job
  (limits 0x2508, no child processes). The engine opens `\Device\PolyAudio` asking only for
  `FILE_READ_ATTRIBUTES | SYNCHRONIZE`. Its requests need no access (`FILE_ANY_ACCESS`), and a Low token would be
  refused write access to an unlabelled device on Windows' rules. Today the device has no security descriptor, so
  any token opens it. What Windows grants a sandboxed audio client is a question for the spec agent.

## Consequences

- Chrome for Windows plays the 440 Hz page and YouTube on Poly with its default sandbox. The tone is clean apart from
  a few short gaps. Video sound underruns in about 11 % of periods while the guest's CPU is saturated: Poly's scheduler
  has no thread priorities, so the audio threads wait beside the renderer. Numbers are in the plan doc.
- The 24 visible tests pass on Poly, repeated.
- Latency from `ReleaseBuffer` to the speaker is about the client's buffer (up to 22 ms), the lead (up to 20 ms) and
  the device (60 ms), plus the host's audio backend. Chrome reports an output latency of 24 ms from its own view.
- Exclusive mode, capture, loopback, device changes, `ISimpleAudioVolume` and the endpoint volume are not done. W3
  and W4 add them with their sheets.

## Addendum (2026-11-20): long buffers, `IAudioClock2`, the enumerator's count

Found by the integrator's held-out run (`int-round-e`):

- **A long buffer panicked the kernel.** The engine opened its PolyAudio stream with the client's whole buffer, so
  `Initialize` with a 20 s duration (960,000 frames, which the sheet accepts: no upper limit was seen on Windows)
  asked the kernel for 7.68 MB in one allocation, and the kernel panicked when that failed. Now the engine's
  PolyAudio stream is 100 ms (ten periods; the engine never queues more than four), whatever the client's buffer.
  The kernel reserves a stream's buffer fallibly and fails the open with `STATUS_INSUFFICIENT_RESOURCES`
  ([ADR 0250](0250-out-of-memory-never-panics-the-kernel.md)); the client's own buffer, in the application's
  process, is reserved fallibly too and gives `E_OUTOFMEMORY`.
- **`IAudioClock2`.** `QueryInterface` on the clock answers it (Learn; audioclient.h) with a tear-off: its slot 3,
  `GetDevicePosition`, is not the clock's. The tear-off holds a reference on the clock and answers every other
  interface as the clock does, so both have the clock's identity. No sheet specifies `GetDevicePosition`: it gives
  the clock's position in frames and the performance counter.
- **The enumerator's count.** On Windows an application's `AddRef` on a freshly created enumerator counts one more
  than its own references. Each creation is still an object of its own, and now carries one reference the module
  keeps, so the counts match. That reference is never given back; an enumerator is a few words.
