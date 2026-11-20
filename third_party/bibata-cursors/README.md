# Bibata cursor theme (Bibata-Modern-Ice)

Poly's pointer uses the **Bibata** cursor theme, variant **Bibata-Modern-Ice** (white, rounded
edges). It is the theme the owner's Alpine Shell uses (ADR 0252).

- **Author:** Abdulkaiz Khatri ([@ful1e5](https://github.com/ful1e5)), Copyright (C) Abdulkaiz Khatri.
- **Project:** https://github.com/ful1e5/Bibata_Cursor
- **Licence:** GNU General Public License, version 3: the text is in [`LICENSE`](LICENSE). It is upstream's
  `LICENSE` at the release tag (SHA-256 `fe92c7ac268ce17193b98298336d3793a843a94eba9efc7ad112f9f48ad9b214`).
- **Release:** v2.0.7 (2024-06-18), tag commit `35ccfe209a808e40d6c2ca60a46cbe4faf68b690`.
- **Package:** the release's X11 package `Bibata-Modern-Ice.tar.xz`,
  https://github.com/ful1e5/Bibata_Cursor/releases/download/v2.0.7/Bibata-Modern-Ice.tar.xz,
  1,701,468 bytes, SHA-256 `a68cae60c4dc706350e194ebc91c5fe48bc7bc9d59e119555834a2a7ee5078ef`.

## Source

The theme's preferred form for modification is in the upstream repository at the tag above:
- the SVG drawings in `svg/` (the Modern variant's in `svg/modern/`);
- the build configuration in `configs/`, `build.sh` and `render.json`.

Download: https://github.com/ful1e5/Bibata_Cursor/archive/35ccfe209a808e40d6c2ca60a46cbe4faf68b690.tar.gz

## What Poly does with it

- **Not committed to Poly's repository.** Only this README and the licence are here.
- **Fetched.** `cargo xtask cursors fetch` (`tools/xtask/src/cursors.rs`) downloads the package
  above and checks its size and SHA-256.
- **Twelve cursors are used**, one for each look Poly draws: `left_ptr`, `left_ptr_watch`,
  `hand2`, `xterm`, `wait`, `crosshair`, `move`, `circle`, `sb_h_double_arrow`,
  `sb_v_double_arrow`, `bd_double_arrow`, `fd_double_arrow`.
- **Modified:** each XCursor file keeps only its images of nominal sizes 24, 32 and 48 (Poly's
  scales from 100 % to 200 %). The others and the files' comment chunks are left out; no picture
  is changed.
- **Shipped as separate data files.** Poly's desktop images carry the files in
  `/usr/share/icons/Bibata-Modern-Ice/cursors/`, with `index.theme`, this README and the licence
  beside them.
- **Read at start.** Poly's compositor reads the files at start and draws the pointer from them.
  Nothing of the theme is compiled into Poly's programs; without the files the compositor draws
  its own pointer.
