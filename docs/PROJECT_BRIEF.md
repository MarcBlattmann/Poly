# Project brief: Universal-app OS on a Rust kernel

Handoff document for the next agent. It captures the goals, architecture, process and constraints agreed so far. Nothing has been built yet.

## 1. Vision

Build a new operating system on a custom Rust kernel that runs **Linux, Windows and macOS apps side by side, in one unified UI** rendered by our own compositor.

**Design rule:** the OS shell itself does **not** use a glass design. Instead, every app should look exactly as it does on its native OS. In particular, **macOS apps render Apple's Liquid Glass design** (materials, blur, refraction, animations) just like on a real Mac, because our macOS framework reimplementation draws it faithfully. Windows apps look like Windows apps, Linux apps like Linux apps.

Reimplementation work is split into small units that AI agents pick up, implement, verify against the real OS, and merge. Progress is tracked publicly as percentages in the repo root.

## 2. Hard constraints (non-negotiable)

- **Clean-room only.** No decompiling, disassembling or copying code from Windows or macOS. Agents may only observe *behavior* (inputs, outputs, error codes, side effects, ordering). Decompiled-derived code is a derivative work and could not be distributed.
- **Strict role separation.** The spec agent (observes the real OS) and the implementer agent (writes our code) run in separate contexts. The implementer only ever receives specs and visible tests. Log every handoff so the separation can be proven.
- **Linux source:** reading it is fine, but copying GPL code into the kernel makes the kernel GPL. Keep the implementer side behavior-only here too.
- **Licensed copies only** of Windows and macOS are used as test oracles in VMs.
- Exported API *names* (from DLL export tables, SDK headers, macOS SDK `.tbd` files) are facts and may be listed.

## 3. Architecture

**Three native personalities on one Rust kernel. No Wine, no Linux underneath.** Every OS world runs directly on our kernel.

```
              Unified compositor + shell (own non-glass design)
     ┌────────────────────────┬────────────────────────┬────────────────────────┐
  Linux apps               Windows apps              macOS apps
  (glibc, Mesa, GTK/Qt     (our ntdll, kernel32,     (our libSystem, Foundation,
   run unchanged)           user32, gdi32, DirectX,   AppKit, Core Animation,
                            COM, ... reimplemented)   Metal, SwiftUI reimpl)
     │                        │                        │
  Linux personality        NT personality           XNU personality
  ~350 syscalls (ELF)      ~500 NT syscalls (PE)    BSD syscalls + Mach traps (Mach-O)
     └────────────────────────┼────────────────────────┘
              Shared Rust core: scheduler, memory, VFS, IPC,
              networking, drivers (virtio first, then one real laptop)
```

- **Personality selection by binary format** at process start: ELF → Linux, PE → NT, Mach-O → XNU. Each process is tagged with its personality and its syscalls are routed to that personality's syscall table. Precedent: Windows NT's original subsystems (Win32, OS/2, POSIX) and WSL1 pico processes.
- **Shared core** serves all three: scheduler, memory management, file system, IPC, networking, drivers, compositor.
- **Linux personality:** smallest and best documented. Open-source Linux userland (glibc, Mesa, GTK, Qt) runs unchanged on a correct syscall layer.
- **NT personality:** kernel side implements NT objects, handles, waits, events, case-insensitive file lookups natively (fast, no translation). The **user-mode DLL layer** (ntdll, kernel32, user32, gdi32, DirectX stack, COM, hundreds more) is written by us from scratch. This is the largest Windows work item.
- **XNU personality:** BSD syscalls + Mach traps (XNU is open source, so behavior is well documented). On top: our clean-room reimplementation of the macOS frameworks (libSystem, Foundation, AppKit, Quartz/Core Animation, Metal, SwiftUI). This is the largest and hardest layer of the whole project.
- **The kernel layer is the small part.** The user-mode libraries for Windows and macOS are where most of the effort goes.
- **One compositor owns the screen.** Linux apps via Wayland (X11 via XWayland); Windows apps via our own user32/gdi32 windowing layer; macOS apps via our AppKit/Quartz layer. All hand surfaces directly to the compositor.

### Using Wine as a reference (not a dependency)
- **Wine's conformance test suite** was written by running against real Windows: use it as a ready-made oracle and seed for the diff runner.
- **Wine's `.spec` files** list every export per DLL: use them (plus real Windows export tables) to build the initial Windows coverage map.
- Wine is LGPL. Reading it for behavior is fine; keep our own code independently written to keep our own license.

### Shared integration bridges (one per personality)
Clipboard, drag and drop across app worlds, one file picker, notifications, fonts, HiDPI scaling, keyboard/input methods, app launcher.

### Native look per app
The goal is visual fidelity, not a unified theme: each app should look the same as on its original OS. App contents are drawn by the apps themselves; Windows apps should render native-looking Windows controls (our common-controls implementation must match real Windows), Linux apps use their GTK/Qt themes. **Liquid Glass in macOS apps** is a fidelity target for the macOS framework layer: the spec agent should capture its look on a real Mac (screenshots, animations, how it reacts to content behind it), and the diff runner compares rendering against the real Mac using the visual fidelity rules in section 4. This requires a strong GPU path (blur and refraction need shader effects through our Metal reimplementation) and the compositor must expose what is behind a window so glass materials can sample it.

**Open question:** window frames. Should macOS apps get real macOS-style title bars and traffic-light buttons, and Windows apps Windows-style frames (each drawn native), or does the shell draw one neutral frame for everyone? Native-per-app frames match the fidelity goal best.

### Open design decision
**Global menu bar or per-window menus?** Mac apps expect a global top menu bar; Windows/Linux apps use in-window menus. Decide early; it shapes much of the UI work.

## 4. Agent pipeline

1. **Target apps** → **call tracer** logs every API they call → entries ranked by usage.
2. **Spec agent** probes the real OS in a VM and writes a spec sheet plus tests. Split tests ~70% visible / ~30% held out.
3. **Implementer agent** sees only the spec and visible tests, writes the implementation.
4. **Diff runner** executes the same tests on the real OS VM and on our build, compares results (visual output per the visual fidelity rules below).
5. **Green** → merge, rerun all app-level tests. **Red** → diff goes back to implementer.
6. Any app-level failure becomes a new entry.

### Spec sheet format (per function)

```yaml
function: FindFirstFileW / FindNextFileW
returns:
  order: "NTFS: case-insensitive name order; FAT32: creation order"
  includes: [".", ".."]
errors:
  path_not_found: ERROR_PATH_NOT_FOUND (3)
  empty_dir: returns "." and "..", then ERROR_NO_MORE_FILES (18)
side_effects: none
threading: handle not safe to share across threads
timing:
  typical: "< 50µs per entry, local SSD"
  guarantees: none
tests: 214 visible, 92 held-out
```

Specs should capture return values, ordering, error codes, side effects, threading rules, event ordering and a **timing envelope** (ranges and ordering, not exact microseconds — real OSes are not deterministic either).

### Visual fidelity rules (1:1 look)

The goal is that every app looks **1:1 the same** as on its native OS. "1:1" is defined as: **as close as the real OS is to itself.**

**Why not byte-identical pixels:** the real OS does not reproduce its own pixels exactly. Two screenshots of the same window on two Macs, or twice on the same Mac, differ slightly due to GPU rounding in blur/refraction math, anti-aliasing, font smoothing, dithering, and animation frames captured milliseconds apart. A byte-exact test would fail even real Mac vs real Mac, so it would be useless.

**Noise-floor calibration:**
1. Render the same screen on the real OS several times, and on at least two different machines where possible.
2. Compare those captures against each other and record the difference. This is the **noise floor** for that screen.
3. Our output must stay within that same noise floor. Anything above it is a real visual bug and the test goes red.
4. Recalibrate when the real OS version or test hardware changes.

**Split exact checks from perceptual checks:**

| Checked exactly (zero tolerance, compared as data) | Checked perceptually (noise-floor tolerance, e.g. SSIM) |
|---|---|
| Layout positions and sizes, window/control frames | Blur, Liquid Glass refraction and materials |
| Text content and line breaks | Shadows |
| Colors of flat areas | Text anti-aliasing and font smoothing |
| Which elements are visible, z-order | Gradients and dithering |
| Animation curves, durations, keyframe timing | Individual animation frames |

So a control that is 1 px off or a wrong shade of a flat color fails immediately, while a glass blur that differs only by GPU rounding does not cause false alarms.

### Trace diffing (key technique)
Record real app sessions on the real OS: every API call, arguments, results, order. Replay the same session on our OS and diff the logs. The first divergence is the bug. Every recorded session also becomes new tests and spec details. This covers state combinations that per-function tests cannot.

## 5. Coverage map (repo root)

- Generate the full symbol list from SDKs and export tables into a single `coverage.json` (source of truth). Expect tens of thousands of entries per OS.
- One generated doc page per entry.
- Status per entry: `missing` → `claimed` (with agent ID, prevents double work) → `stub` → `green`. Also: `blocked-by-design` for things that cannot be made to work (see section 7).
- CI regenerates the README on every merge.
- One row per personality (Linux, Windows, macOS), each covering syscalls and user-mode APIs separately.
- Show **three numbers per OS**: raw API coverage %, **app-weighted coverage %** (weighted by how often target apps use each function — this is the one that tracks real progress), and **trace sessions passing**.
- Track test depth per entry (green with 3 tests ≠ green with 300).

```
Linux     ██████████░░░░░░  64.0%  (app-weighted: 82.5%)
Windows   ██████░░░░░░░░░░  38.2%  (app-weighted: 61.4%)
macOS     ██░░░░░░░░░░░░░░  11.7%  (app-weighted: 19.0%)
kernel32  ████████████░░░░  74%   1,204 / 1,627
```

Long-term goal is all APIs covered. Weighting only decides order, not scope.

## 6. Target apps (difficulty ladder)

Start with 20 apps:
- **Easy:** 7-Zip, Notepad++, VLC, other simple tools.
- **Medium:** Paint.NET, an IDE, one Electron app (e.g. VS Code — stepping stone to Chrome).
- **Hard:** Microsoft Office, a Photoshop-class creative app, a couple of games, **Chrome with the sandbox enabled** (stresses multi-process IPC, security tokens/job objects/integrity levels, Direct3D 11 via ANGLE, DirectWrite). Pin one exact Chrome version for traces; bump deliberately.

Per-app milestones: launches → main window draws correctly → core workflow (open/edit/save) → full recorded trace passes.

## 7. Known limits (be honest in the README)

- 100% API coverage ≠ every app works: undocumented quirks and compat-bugs only show up via traces.
- Kernel-mode anti-cheat, DRM and driver-installing apps need an NT driver model and may deliberately refuse non-Windows systems.
- Apple services (iCloud, App Store, iMessage, Apple Pay) can be blocked server-side by Apple.
- Microsoft and Apple add APIs every year — the map is a treadmill.
- GPU behavior is hard to match pixel-exact, especially Metal on Apple Silicon.
- **Drivers are the biggest risk for the kernel.** Develop entirely in QEMU with virtio devices (disk, network, virtio-gpu) first, then support exactly one real laptop.

## 8. Roadmap

1. Rust kernel boots in QEMU; personality routing by binary format (ELF/PE/Mach-O).
2. Shell + basic Linux binaries via the Linux personality (agent pipeline validated against real Linux).
3. NT personality + first user-mode DLLs; simple Windows test apps work natively on our kernel.
4. virtio-gpu + our own compositor and shell; Linux and Windows apps in one UI, each with its native look.
5. Integration bridges (clipboard, drag and drop, file picker, ...).
6. Mach/BSD personality and macOS framework reimplementation, including faithful Liquid Glass rendering compared against a real Mac.
7. Hardware support for one real laptop.

**First proof-of-concept milestones:** (a) a simple Linux app passes its recorded trace on our Linux personality; (b) one simple Windows app (e.g. 7-Zip) passes its full recorded trace on our NT personality, with no Wine involved.

**Good first agent task:** parse Wine's `.spec` files and real Windows export tables into `coverage.json` to seed the Windows coverage map.

## 9. References to study (verify before relying on them)

- **Asterinas** — Rust kernel with Linux ABI compatibility (closest reference).
- **Redox OS** — Rust OS, shows scope and timeline.
- **gVisor** — Linux kernel interface reimplemented in Go.
- **WSL1**, FreeBSD Linux emulation, Windows NT's original subsystem design — multiple personalities on one kernel in practice.
- **Wine / Proton** — reference only: conformance tests (oracle), `.spec` export lists, behavior documentation. Not used as a runtime dependency.
- **Darling** — macOS compatibility on Linux.
- **ReactOS** — clean-room Windows clone.
- **XNU source** (`syscalls.master`), published NT syscall tables, Windows SDK headers, macOS SDK `.tbd` files.
- **CrossOver, WSLg** — precedents for mixing app worlds in one desktop.
