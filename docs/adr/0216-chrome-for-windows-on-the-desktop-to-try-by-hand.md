# 0216: Chrome for Windows on the desktop, to try by hand

**Status:** Accepted. Track `chrome-try`. Builds on ADR 0155 (Chrome for Windows and its
stages), ADR 0209 (the desktop's keyboard layout), ADR 0124 (builds on Windows and Linux) and
ADR 0144 (the desktop's launcher and its desktop files).

## What was there

Chrome for Testing ran only in scripted checks: a boot in Docker, a script that clicks through
QMP, screenshots. Nobody could start it on a PC and browse. The desktop with a browser was
Chromium, a Linux program on a Debian disk (ADR 0147).

## Decisions

1. **A menu entry of its own**: *Desktop with Chrome for Windows* in `cargo xtask start`, the
   command `cargo xtask desktop --chrome-win`. It is not the "With Chromium" switch: the disk, the
   program and the way it starts are other ones. The two switches exclude each other.
2. **A disk of its own**, `target/poly-chrome-try-disk.img`: Chrome under
   `C:\Program Files\Chrome`, `polystub.dll` and Poly's fonts (`chromewin::app_tree`), without the
   test launcher, the stage files and the test pages that `chrome-win`'s disk has. Its own file,
   because a person's profile must not meet a boot check's runs. It is written by the
   repository's formatter (pure Rust, `rust-lld`: no host C toolchain), so the same command works
   on a Windows and a Linux host and nothing goes through Docker. The disk is made again only
   when what it carries changes (Chrome's version, `polystub.dll`, the fonts) or with
   `--fresh-disk`.
3. **The profile is on that disk**, in `C:\poly-chrome-profile` (`--user-data-dir`). The NT
   personality keeps `C:` on the first writable disk with a `/poly` directory (ADR 0073), so a
   reboot finds bookmarks, history and logins. The cost: a disk made again starts with an empty
   profile (said in RUNNING.md and in the menu's option text).
4. **The launcher entry is a desktop file in the initramfs**, not on the disk, so it follows the
   checkout while the disk is kept (as Chromium's, ADR 0147). Its `Exec` quotes the program's path
   (`/winc/poly/nt/c/Program Files/Chrome/chrome.exe`); its icon is Chrome's own largest PNG
   from the icon group of `chrome.exe`'s resources (the picture only; the shell draws its own glyph
   when there is none).
5. **Mounts**: the disk is `virtio0` on `/winc`, the desktop's settings disk `virtio1` on `/mnt`
   (layout, scale, shortcuts; ADR 0209's layout follows them). The root disk comes first so `C:` is
   on it.
6. **Resources**: 4 GB and 4 CPUs (`--memory=`, `--cpus=` change them), user networking, QEMU's
   window with the fastest accelerator the host has (`accel.rs`: WHPX, KVM, else TCG).
7. **Chrome's switches live in `chrometry::FLAGS`**, nowhere else (the stage 3 check reads the
   same list). On `main` it is `--no-sandbox --disable-gpu`; when the sandbox (S2) and the GPU
   (S5) work are on `main` it is empty. Besides it: `--no-first-run`, `--no-default-browser-check`
   and the profile.
8. **A boot check, the group `chrome-try`** (`chrometry::group_boot_test`, run in the CI job of the
   window check, ADR 0214): the same
   disk and entry, started from the launcher by its name, the page painted after 125 s, a URL typed
   into the address field. It skips where Chrome is not fetched and on a host without Unix
   sockets (QMP); CI's window job fetches Chrome, so it runs there.

## Consequences

- One command, on the owner's PC, to browse on Poly.
- The disk is about 760 MB plus room for the profile (the formatter's 25% and spare space); the
  profile's cache grows there.
- `winapps` has a second scripted-run entry (`desktop_entry_check`) for a disk and an entry that
  are not `win-app`'s; its `launch` step types a query the caller gives.

## Addendum (2026-11-17): Chrome starts with its defaults, the sandbox and the GPU on

Track `chrome-defaults`. With the sandbox (S2, ADR 0206/0217) and the software Direct3D 11
device (S5, ADR 0211) both on `main`, `chrometry::FLAGS` is **empty**. The launcher entry starts
Chrome with only `--no-first-run --no-default-browser-check --user-data-dir=C:\poly-chrome-profile`:
nothing that changes security or the GPU, so Chrome shows no "unsupported command-line flag" bar.
Its own "Chrome for Testing is only for automated testing" bar stays; Windows shows it too.

- **The two together had never run.** With both on, the GPU process runs in its own sandbox
  and opens `d3d11.dll`/`dxgi.dll` there. It did not stop on anything new: no exception, no
  access denied, one rendering GPU process. Chrome's GPU-information process still runs
  beside it, without the GPU sandbox by Chrome's own choice (`--disable-gpu-sandbox`,
  observed). Poly needed no fix.
- **Stage 7** (`chrome-win run --stage 7`): stage 2 without `--disable-gpu`, i.e. headless
  `--dump-dom` with Chrome's defaults. Its output is the spec's `s2` (the same page and DOM).
- **Boot group `chrome-win-defaults`**, in the `chrome-win-sandbox` CI shard (the
  `chrome (s2-sandbox)` job), required (`S7_REQUIRED`). It needs the DOM, plus exactly one
  rendering GPU process whose switches name neither `no-sandbox` nor `disable-gpu-sandbox`
  (`chromewin::defaults_gpu_note`). A second rendering GPU process would mean the first one
  ended.
- **`chrome-try` and stage 3's window check** read `FLAGS`, so they now run with the defaults.
  `chrome-try` also applies `defaults_gpu_note` to its run. Stage 5 and the S5 window keep
  `--no-sandbox`: they are S5's own check, separate from S2.
- Measured (release kernel, 4 GB, 4 CPUs): stage 7 takes about 30 s on KVM. On the owner's PC
  on WHPX, Chrome's window showed Google's new-tab page less than 45 s after Enter in
  the launcher (the first screenshot, at 45 s). `http://example.com/` loads. A bookmark and the history survive closing QEMU's
  window and starting again.
- **A first-start race, fixed.** On a checkout whose `ntdll.dll` (or Win32/WOW64 modules)
  had to be built, `desktop --chrome-win` built it twice at once. The kernel's build and the
  generation of `polystub.dll` (`winimports::load_ours`) each called `ntdll::ensure` into the
  same `target/ntdll`. The first to finish removed that directory under the other, which then
  failed with "The system cannot find the path specified" (seen on the PC on a fresh worktree).
  `ntdll::ensure`, `win32::ensure` and `wow64::ensure` now run one at a time in a process. The
  second caller waits and takes the first one's cached result. Checked on the PC: with an
  ntdll input changed, one build, and the desktop came up.
