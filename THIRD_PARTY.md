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
| [Wayland](https://gitlab.freedesktop.org/wayland/wayland) core protocol: `wayland.xml` | 1.26.0 | MIT ([text](third_party/wayland/COPYING)) | `third_party/wayland/` | the protocol's specification: `cargo xtask wayland` generates `crates/poly-wayland/src/protocol/wayland.rs` from it (ADR 0106) |
| [wayland-protocols](https://gitlab.freedesktop.org/wayland/wayland-protocols): `xdg-shell.xml`, `xdg-decoration-unstable-v1.xml`, `viewporter.xml`, `presentation-time.xml`, `linux-dmabuf-v1.xml`, `fractional-scale-v1.xml`, `xdg-activation-v1.xml`, `cursor-shape-v1.xml`, `xwayland-shell-v1.xml` (staging, ADR 0118) | 1.50 | MIT ([text](third_party/wayland-protocols/COPYING)) | `third_party/wayland-protocols/` | the same, into the other modules of `crates/poly-wayland/src/protocol/` |

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

## Wayland and wayland-protocols

Copyright © 2008-2012 Kristian Høgsberg, © 2010-2013 Intel Corporation and
the other holders their `COPYING` files list.

The XML files are unmodified copies from the release tarballs
`wayland-1.26.0.tar.xz` (SHA-256
`64176eaa46e4969903e286f8e5ef8331affc17fdf03ac9b58381d2b23162b7a3`) and
`wayland-protocols-1.50.tar.xz` (SHA-256
`4460c20cfcf68b994a5ee206ed15fc19ad312f16c96075186e83b0e3f4aa48a5`), as
published with their `.sha256sum` files on gitlab.freedesktop.org; each
file's own SHA-256 is pinned in `tools/xtask/src/wayland.rs`, which
refuses to generate from a changed copy. Only the protocol descriptions
were taken, nothing of libwayland. The generated modules keep the
interfaces, messages, arguments, enum entries and their `summary` texts in
another form. The MIT licence asks that its notice be included with
copies or substantial portions: anything that ships these modules must
include [third_party/wayland/COPYING](third_party/wayland/COPYING) and
[third_party/wayland-protocols/COPYING](third_party/wayland-protocols/COPYING).
