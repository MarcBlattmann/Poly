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

## Spleen

Copyright (c) 2018-2024, Frederic Cambus. All rights reserved.

The BDF files are unmodified copies of the 2.1.0 release (tag `2.1.0` of
the repository above). The generated glyph tables hold the same bitmaps in
another form (every glyph placed in its cell, private-use and control
characters left out). The BSD 2-Clause licence asks that binary
redistributions reproduce the copyright notice, the conditions and the
disclaimer in their documentation: anything that ships a kernel image must
include [third_party/spleen/LICENSE](third_party/spleen/LICENSE).
