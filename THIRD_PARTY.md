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
| [Geist and Geist Mono](https://github.com/vercel/geist-font) fonts by Vercel: `Geist-Regular.ttf`, `Geist-Medium.ttf`, `Geist-SemiBold.ttf`, `GeistMono-Regular.ttf` | 1.7.2 | SIL Open Font License 1.1 ([text](third_party/geist/OFL.txt)) | **not committed**: `cargo xtask fonts fetch` takes them from the pinned release archive (`geist-font-v1.7.2.zip`) into `target/fonts/`; only the licence is in `third_party/geist/` | the shell's text, as Alpine Shell's ([ADR 0126](docs/adr/0126-the-alpine-shell-look.md)): `/usr/share/poly/fonts/` on the desktop's images |
| [Averia Serif Libre](https://github.com/google/fonts/tree/main/ofl/averiaseriflibre) font by Dan Sayers: `AveriaSerifLibre-Regular.ttf`, `AveriaSerifLibre-Bold.ttf` | Google Fonts commit `6f3c56c7` | SIL Open Font License 1.1, Reserved Font Names "Averia" and "Averia Libre" ([text](third_party/averia-serif-libre/OFL.txt)) | **not committed**: fetched like Geist; only the licence is in `third_party/averia-serif-libre/` | the settings window's titles, as Alpine Shell's (ADR 0126) |
| [Lucide](https://lucide.dev) icons (lucide-react): the path data of the 35 icons in `crates/poly-shell/src/icons.rs` | 0.575.0 | ISC; parts from Feather, MIT ([text](third_party/lucide/LICENSE)) | `crates/poly-shell/src/icons.rs` (the data), `third_party/lucide/` (the licence) | the shell's icons, the set Alpine Shell draws with (ADR 0126) |
| [Wayland](https://gitlab.freedesktop.org/wayland/wayland) core protocol: `wayland.xml` | 1.26.0 | MIT ([text](third_party/wayland/COPYING)) | `third_party/wayland/` | the protocol's specification: `cargo xtask wayland` generates `crates/poly-wayland/src/protocol/wayland.rs` from it (ADR 0106) |
| [wayland-protocols](https://gitlab.freedesktop.org/wayland/wayland-protocols): `xdg-shell.xml`, `xdg-decoration-unstable-v1.xml`, `viewporter.xml`, `presentation-time.xml`, `linux-dmabuf-v1.xml`, `fractional-scale-v1.xml`, `xdg-activation-v1.xml`, `cursor-shape-v1.xml`, `xwayland-shell-v1.xml` (staging, ADR 0118) | 1.50 | MIT ([text](third_party/wayland-protocols/COPYING)) | `third_party/wayland-protocols/` | the same, into the other modules of `crates/poly-wayland/src/protocol/` |

## Geist, Averia Serif Libre and Lucide

Geist (c) 2024 The Geist Project Authors and Averia Serif Libre (c) 2011
Dan Sayers are under the SIL Open Font License 1.1: they may be bundled
with software and redistributed, not sold by themselves, and the licence
and copyright notice go with every copy, so the desktop's images install
`OFL-Geist.txt` and `OFL-AveriaSerifLibre.txt` (the texts in
`third_party/geist/OFL.txt` and `third_party/averia-serif-libre/OFL.txt`)
beside the fonts. Poly ships them unmodified (a modified Averia could not
keep its Reserved Font Name). `tools/xtask/src/fonts.rs` pins the archive,
each file and each licence by size and SHA-256. The Lucide icons' path
data is copied into `crates/poly-shell/src/icons.rs` with the ISC notice
in `third_party/lucide/LICENSE`; the owner, who owns Alpine Shell, allows
Poly to reuse its choice of icons, tokens and measurements (ADR 0124).

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
