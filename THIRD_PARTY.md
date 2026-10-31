# Third-party material

Poly is licensed under Apache-2.0 ([ADR 0005](docs/adr/0005-license.md)).
These parts of the repository come from elsewhere, under the licences
listed. Each is kept under `third_party/<name>/` with its licence text;
anything derived from it says so in its header. Adding to this list needs a
permissive licence (no GPL or LGPL, see [CLEAN_ROOM.md](CLEAN_ROOM.md)) and
an entry here.

| What | Version | Licence | Where | Used for |
|---|---|---|---|---|
| [Spleen](https://github.com/fcambus/spleen) bitmap fonts by Frederic Cambus: `spleen-8x16.bdf`, `spleen-12x24.bdf` | 2.1.0 | BSD 2-Clause ([text](third_party/spleen/LICENSE)) | `third_party/spleen/` | the kernel's display fonts: `cargo xtask font` converts them to `crates/poly-gfx/src/font/spleen_8x16.rs` and `spleen_12x24.rs`, which are compiled into the kernel |
| [DejaVu](https://dejavu-fonts.github.io/) fonts: Sans, Serif and Sans Mono in regular, bold, oblique/italic and bold oblique/italic (12 `.ttf` files) | 2.37 | Bitstream Vera Fonts licence (the Vera glyphs), public domain (the DejaVu changes), Arev Fonts licence (the glyphs from Arev); all permissive ([text](third_party/dejavu/LICENSE)) | **not committed**: `cargo xtask fonts fetch` downloads the pinned release into `target/fonts/`; only the licence is in `third_party/dejavu/` | the font store: `/usr/share/fonts/truetype/dejavu/` (and `C:\Windows\Fonts`) on the root disk, `poly-font`'s tests on real fonts ([ADR 0105](docs/adr/0105-fonts-poly-font-font-store-and-fontconfig-discovery.md)) |

## DejaVu

Fonts are (c) Bitstream; DejaVu changes are in the public domain; glyphs
imported from the Arev fonts are (c) Tavmjong Bah (the licence file has the
full texts). The font files are unmodified copies from the 2.37 release
(`dejavu-fonts-ttf-2.37.tar.bz2` of the project's GitHub releases), never
committed: `tools/xtask/src/fonts.rs` pins the archive's and each file's
size and SHA-256. Both licences ask that the copyright and permission
notices go with every copy, so the root disk installs the release's
`LICENSE` (unchanged, the same text as `third_party/dejavu/LICENSE`) beside
the fonts. Neither allows selling the fonts by themselves, and modified
fonts must be renamed (no "Bitstream", "Vera", "Tavmjong Bah" or "Arev" in
their names):
Poly ships them unchanged and as part of a larger system. The golden images
in `crates/poly-font/tests/golden/dejavu-*` are renderings of a few strings,
made by Poly's own rasteriser.

## Spleen

Copyright (c) 2018-2024, Frederic Cambus. All rights reserved.

The BDF files are unmodified copies of the 2.1.0 release (tag `2.1.0` of
the repository above). The generated glyph tables hold the same bitmaps in
another form (every glyph placed in its cell, private-use and control
characters left out). The BSD 2-Clause licence asks that binary
redistributions reproduce the copyright notice, the conditions and the
disclaimer in their documentation: anything that ships a kernel image must
include [third_party/spleen/LICENSE](third_party/spleen/LICENSE).
