# Working in this repo (instructions for AI agents)

Read `CLEAN_ROOM.md` before anything else. Breaking it can make the whole
project undistributable. If you are an implementer agent, you may only use the
spec sheet and visible tests you were handed, plus public docs and export names.

## Layout

- `kernel/`: the bare-metal kernel (its own Cargo workspace, target `x86_64-unknown-none`)
- `crates/`: `no_std` libraries shared by the kernel, unit-tested on the host
- `personalities/{linux,nt,xnu}/`: per-OS system call layers
- `tools/xtask/`: build/run/test driver; `tools/coverage/`: coverage map tooling;
  `tools/pipeline/`: agent pipeline tools (tracer, diff runner, specs, hand-offs; see docs/AGENT_PIPELINE.md)
- `tests/linux/`: test programs run on real Linux and on Poly by the diff runner
- `tests/windows/`: Windows test programs (Rust, cross-built here) run on the owner's
  Windows PC and on Poly (ADR 0075, `cargo xtask pipeline win ...`)
- `coverage/`: the coverage map, source of truth for implementation status
  (`index.json` + one file per component: `coverage/<os>/<layer>/<component>.json`)
- `specs/`: spec sheets and visible tests, one directory per OS
- `docs/`: architecture, decision records (`docs/adr/`) and the ordered plan (`docs/ROADMAP.md`)
- `docs/progress/`: generated SVG badges and charts shown in the README

## Commands

```sh
cargo xtask test        # host unit tests + kernel boot tests in QEMU
cargo xtask ci          # everything CI runs (fmt, clippy, tests, coverage checks)
cargo xtask run         # boot the kernel interactively (Ctrl-A X quits QEMU)
cargo xtask coverage claim <id> --agent <your-agent-id>
```

Run `cargo xtask ci` before every commit. It must pass.

## Rules

- **Put logic in host-testable crates.** Anything that does not touch
  hardware (parsers, allocators, tables, dispatch) goes in `crates/` or
  `personalities/` with unit tests. The kernel crate stays thin.
- **No third-party crates in the kernel.** Host tools may use well-known crates.
- **Every `unsafe` block gets a `// SAFETY:` comment** explaining why it is sound.
- **Claim before you work.** `cargo xtask coverage claim` marks an entry so no
  other agent duplicates it. Release it if you stop.
- **Never hand-edit the README progress section or `docs/progress/`.** Run
  `cargo xtask coverage readme` after changing `coverage/` or `docs/ROADMAP.md`.
- **Work in roadmap order.** Pick the next unticked milestone in `docs/ROADMAP.md`;
  tick it (`- [x]`) in the same change that delivers it with tests. Do not start a
  milestone marked ⚖️ until the owner has made that decision.
- **Constants from public SDK headers** (error codes, struct layouts) cite the header.
- Architecture-specific code lives under `kernel/src/arch/<arch>/` only.
- Significant design decisions get an ADR in `docs/adr/`.
- **Build on Windows and Linux.** Everything that ships in Poly (kernel, native
  programs, the desktop) must build and boot with the same `cargo xtask` commands
  on a Windows host and a Linux host. Use pure Rust and `rust-lld`; no host C
  toolchain, no Unix-only std APIs outside `tools/xtask/src/hostfs.rs`. Only tooling
  that needs Linux by nature (the diff runner, distro-package root disks, QEMU-socket
  boot tests) may be Linux-only, and it reports that instead of failing to build.
  `ci lint` checks xtask for `x86_64-pc-windows-msvc`. (ADR 0124)
- **The desktop is native Poly user space** (compositor, shell, panels,
  notifications, settings): pure Rust on Poly's own system calls, in user-space
  processes, never a Linux, Windows or macOS program and never inside the kernel.
  Apps of every world reach it through their own protocols. (ADR 0124)
- **The UI looks 1:1 like Alpine Shell**, the owner's shell
  (`C:\Users\Marc.Blattmann\Documents\GitHub\alpine-shell`; a running copy is on
  `ssh homeserver2` for reference screenshots). Its assets (icons, colour and spacing
  tokens, wallpaper) may be reused; its OFL fonts (Geist, Geist Mono, Averia Serif
  Libre) come from upstream with their licences. Check changes by comparing
  screenshots with Alpine Shell's at the same size. (ADR 0124)
