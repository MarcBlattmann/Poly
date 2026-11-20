# 0252: The Bibata cursor theme

**Status:** Accepted. Track `cursors-bibata`.
- **Builds on:** ADR 0234 (the pointer's named looks), ADR 0124 and ADR 0127 (Poly looks like the owner's Alpine Shell).
- **Inputs:** the official Bibata v2.0.7 release, and the XCursor file format as the `Xcursor` library documents it.

## Context

The owner wants Poly's pointer to be Alpine Shell's.
- **What Alpine Shell uses:** the **Bibata** cursor theme, variant **Bibata-Modern-Ice** (its `CURSOR_VARIANT`; white, rounded edges), from upstream release v2.0.7.
- **What Poly drew until now:** since ADR 0234, its own small black-and-white pictures for the arrow, the busy arrow, the hand, the I-beam, the hourglass, the crosshair, the four-way arrow, "not allowed" and the four resize arrows.

Bibata is under the **GPL-3.0**. `THIRD_PARTY.md` admitted only permissive licences, because GPL code compiled into Poly would put Poly under the GPL (CLEAN_ROOM.md).

## Decision

1. **Data, not code.** The theme is never committed and never compiled into Poly.
   - **Fetching.** `cargo xtask cursors fetch` downloads the release's X11 package (`Bibata-Modern-Ice.tar.xz`, pinned by size and SHA-256) into `target/cursors/` and unpacks the twelve cursors Poly draws.
   - **Trimming.** Each file keeps only its images of nominal sizes 24, 32 and 48. That is a modified copy, and the README beside the files says so.
   - **The images.** The desktop's images (the desktop initramfs, the Chromium disk) carry the files in `/usr/share/icons/Bibata-Modern-Ice/cursors/`, with the GPL-3.0 text, `index.theme` and `third_party/bibata-cursors/README.md` beside them. The README names the author, the release, the hashes and the source (the SVGs and build files at upstream's tag commit `35ccfe2`).
   - **The policy.** `THIRD_PARTY.md` now admits GPL **data** that is fetched and installed as separate files beside its licence (an aggregate). The Bibata cursors are the only case.
2. **Read at start.** The compositor reads the files and keeps a `CursorTheme` (`poly_wayland::compose::xcursor`).
   - **The format.** The reader is `poly-wayland`'s own, host-tested: XCursor's table of contents and image chunks of premultiplied ARGB.
   - **Which picture.** For the output's scale it takes the nominal size nearest to 24 × scale (24 at 100 %, 32 at 125 and 150 %, 48 at 175 and 200 %). The picture is drawn pixel for pixel, blended over the screen, with the theme's hot spot at the pointer.
   - **The busy arrow** steps through `left_ptr_watch`'s 54 frames by its phase. The named "wait" look shows its first frame.
   - **Damage.** The rectangle the scene damages for the pointer holds the whole picture at every scale.
3. **The fallback.** A look without a file, or any look when the theme is not on the disk, is drawn as before (ADR 0234). The compositor logs which it found: `cursor theme Bibata-Modern-Ice: 12 of 12 looks`, or `no cursor theme`.

The looks map to the theme's files:

| Look | File |
|---|---|
| Arrow | `left_ptr` |
| Busy arrow | `left_ptr_watch` |
| Hand | `hand2` |
| I-beam | `xterm` |
| Wait | `wait` |
| Crosshair | `crosshair` |
| Move | `move` |
| Not allowed | `circle` |
| Resize left-right | `sb_h_double_arrow` |
| Resize up-down | `sb_v_double_arrow` |
| Resize NW-SE | `bd_double_arrow` |
| Resize NE-SW | `fd_double_arrow` |

## Consequences

- **The same pointer as Alpine Shell**, at its size: 24 logical pixels, where Poly's arrow was 12x19. Windows apps (through `IDC_*`) and Wayland apps (through the cursor-shape protocol) get it alike.
- **Size.** The images grow by about 1.8 MB: twelve files, 16 KB each for the static ones and 851 KB for the two animated ones.
- **CI** fetches the theme for the boot jobs, next to the fonts. Without it, the desktop still works with Poly's own pointer.
- **Not done:**
  - `CursorLook::Image`: the cursors apps draw themselves are untouched.
  - The named "wait" look does not animate (it has no phase).
  - The busy arrow's own spinner over Poly's arrow (ADR 0167) is replaced by the theme's animation when the theme is there.
