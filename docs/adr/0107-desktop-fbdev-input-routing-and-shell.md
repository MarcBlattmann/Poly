# 0107: The first desktop: `/dev/fb0` over the kernel display, the compositor on the screen with evdev input routed to clients, and a shell in the look of Alpine Shell (ridge on top, launcher in the middle) as compositor layers

**Status:** Accepted. Roadmap Phase 6. It builds on the virtio-gpu
display ([ADR 0102](0102-virtio-gpu-2d-display.md)), the event layer and
`/dev/input/event*` ([ADR 0103](0103-input-event-layer-evdev-ps2-mouse-virtio-input.md)),
`poly-font` and the font store ([ADR 0105](0105-fonts-poly-font-font-store-and-fontconfig-discovery.md)),
and the Wayland server core with its software compositor
([ADR 0106](0106-wayland-server-core-and-software-compositor.md)).

It covers more of "Compositor: Wayland server, surface composition, HiDPI
scaling, vsync", but that item stays unticked: frames are paced by a
60 Hz timer, and real vsync needs page flips, which come with DRM/KMS.
It starts "Shell: launcher, window management, notifications, settings",
which also stays unticked: notifications and settings are still missing.
The open decision on menus is not touched. The ridge is a status strip
with no menus hanging from it, and there is no global menu bar.

## Context

ADR 0106 left the compositor composing into memory, with no input. To get
a desktop on the screen, the compositor (a Linux-personality program)
needs three things:

- a way to put pixels on the display from user space;
- the keyboards and pointers, routed to the right client;
- a shell.

The owner's direction for the shell's look follows his own Windows shell,
Alpine Shell: "just a ridge at the top, but menus/panels don't extend from
it. The input box / launcher sits in the middle of the screen."

Another agent is building the kernel's DRM/KMS interface at the same time.
The compositor moves to KMS when that is ready, so the display path here
stays small.

## Decision

### `/dev/fb0`: Linux fbdev over the kernel display

`crates/poly-fbdev` is new. It is `no_std` and tested on the host, and
holds the ABI:

- `struct fb_var_screeninfo` (160 bytes) and `struct fb_fix_screeninfo`
  (80 bytes). The offsets were checked against the public uapi header
  `linux/fb.h` with `offsetof` on x86-64.
- The requests: `FBIOGET_VSCREENINFO`, `FBIOPUT_VSCREENINFO` (only the
  current mode), `FBIOGET_FSCREENINFO`, `FBIOPAN_DISPLAY` (offset 0 only),
  `FBIOBLANK`.
- The screen information of a 32-bit `XRGB8888` true-colour mode with one
  page.
- The byte spans a rectangle covers, cut at page boundaries.

Sources: `linux/fb.h`, `asm-generic/ioctl.h`, the kernel's framebuffer
documentation, and fbdev(4) for the numbers. No Linux source was read.

`kernel/src/fbdev.rs` connects it to the kernel:

- **Device node.** Character device 29, 0, at `/dev/fb0`, mode 0660. It
  exists when there is a display.
- **Memory.** The memory a program maps is a *shadow* frame buffer: an
  `AnonShared` object, the memory object behind shared anonymous mappings.
  `mmap` of the device maps its pages through the ordinary fault path;
  `linux/services.rs`'s `mmap` asks `fbdev::page_source`, which is the only
  change there. The pages are made at the first open and kept, so showing
  pixels never allocates.
- **Showing pixels.** Nothing reaches the screen until the program says
  so. Each of these copies a rectangle from the shadow pages into the
  display's back buffer and flushes it (`display::draw`):
  - `FBIOPAN_DISPLAY` shows the whole frame;
  - `write` shows the rows it wrote;
  - **`FBIO_POLY_DAMAGE`** (`_IOW('F', 0x80, struct { __u32 x, y, width, height; })`,
    a number `linux/fb.h` does not use) shows one rectangle.

  On a virtio GPU that is one `TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH` of
  the rectangle; on Bochs VBE it is a copy into the linear frame buffer.

**Why a shadow and a copy, not the back buffer mapped directly.** Mapping
the display's own pages into a process would need a new kind of mapping:
frames the address space does not own. That would mean changing the
fault, unmap and fork paths that the DRM work will also touch. The shadow
reuses `AnonShared` unchanged. It works on both display devices. Because
the program decides when to show a frame, it never shows a half-drawn
one. The copy costs one pass over the damaged pixels, which is small next
to the compositor drawing them.

**Why a Poly-specific damage request.** fbdev has no standard way to say
which part changed. Showing the whole 1280×800 frame for every cursor
move would mean a 4 MB copy and transfer per frame under TCG.
`poly-compositor` falls back to `FBIOPAN_DISPLAY` when the request fails
(on Linux).

### The compositor on the screen (`poly-compositor --desktop`)

- **`FbFramebuffer`** opens `/dev/fb0`, reads both screen informations,
  maps the frame buffer shared, and implements `poly-wayland`'s
  `Framebuffer`. Rows are slices of the mapping, and
  `present(damage)` sends one `FBIO_POLY_DAMAGE` per rectangle. The scene
  already draws only its damage, merged into at most 16 rectangles.
- **Frame clock.** Frames come at most every 1/60 s, when something
  changed or a client waits. The `wp_presentation` time is taken when the
  damage requests return, which is when the device has the frame (the
  flush is synchronous, ADR 0102). Frame callbacks get the same time.
  There is no page flip or vblank to wait for; that is why the roadmap's
  vsync stays open.

### Input routing

`userland/poly-compositor/src/evdev.rs` opens every `/dev/input/event*`
and sorts each device:

- absolute X/Y: tablet (with its axis ranges);
- relative X/Y: mouse;
- `KEY_A`: keyboard.

It grabs each device (`EVIOCGRAB`) so the kernel console stops reading
the keys too. Its `Translator`, tested on the host, collects events until
`SYN_REPORT` and then emits one motion event (relative, or absolute scaled
onto the output) followed by the frame's buttons, keys and wheel steps in
order. A wheel step is 15 logical pixels; `REL_WHEEL` up scrolls up. The
device's own key repeats are dropped, because clients repeat by
`repeat_info`. `SYN_DROPPED` discards the frame. The whole thing is
`poly-wayland`'s `seat::InputSource`.

`poly-wayland`'s `server/input.rs` follows wayland.xml's description of the
seat.

**Pointer**
- The focus is the surface under the pointer. When it changes, the old
  surface gets `leave` and the new one `enter` with surface-local
  coordinates. While it stays, `motion`. Each sequence ends with `frame`
  (version 5).
- While a button is held the focus stays put (an implicit grab).
- Buttons go to the focus with serials. The wheel arrives as
  `axis_source` (wheel), `axis_discrete`, `axis`, `frame`.

**Keyboard**
- The focus follows the active toplevel: `leave`, then `enter` with the
  keys down, then `modifiers`.
- `key` events carry serials; `modifiers` is sent whenever it changes.
- A new `wl_keyboard` first gets `keymap` and `repeat_info` (30/s after
  250 ms, the event layer's rates).
- While the launcher shows, the compositor takes the keyboard
  (`set_keyboard_suspended`) and the focused client gets `leave`.

**Window management**
- A left press on the close button closes the window.
- A press on the title bar or border focuses and raises the window, and
  starts moving it until release. `xdg_toplevel.move` while a button is
  held does the same.
- A press on content focuses the window and also goes to the client.
- Windows cannot be dragged above the work area: the output less the rows
  the shell reserves (`Scene::set_reserved_top`). Maximized windows and
  new windows respect it too.

**Placement**
- `Placement::Centered`: the first window is centred in the work area, the
  next ones step down and right. The old cascade stays the default for
  existing users.

**Cursor**
- A software cursor (an arrow of our own, 12×19 logical pixels, black
  outline, white fill) drawn last by the scene, with only its old and new
  rectangles damaged when it moves.
- `wl_pointer.set_cursor` surfaces are not shown yet.

**Keymap.** `poly_input::xkb` generates an XKB keymap in the text format
(`xkb_v1`) from Poly's own layout tables. It was written from the public
descriptions of the format: the XKB protocol specification's key types and
compat map, xkbcommon's "keymap text format, V1" documentation, and
wayland.xml. No keymap database or xkbcommon source was read.

- Key codes are `<Knnn>` at evdev + 8.
- Types are `ONE_LEVEL`, `TWO_LEVEL`, `ALPHABETIC`, `KEYPAD`, and
  `POLY_SIX_LEVEL`, whose levels are exactly the six of a layout key: base,
  Shift, Caps Lock, Caps Lock + Shift, AltGr, AltGr + Shift.
- Modifiers are real ones only: Shift, Lock, Control, Mod1 (left Alt), Mod2
  (Num Lock), Mod4 (Windows keys), Mod5 (right Alt, always
  `ISO_Level3_Shift`).
- Keysyms are written as hexadecimal numbers.

The seat computes `wl_keyboard.modifiers` with the same bit assignment
(`xkb::modifier_bit`). The keymap goes into a file in `$XDG_RUNTIME_DIR`
that is unlinked once open, because Poly has no `memfd_create` yet. Each
`wl_keyboard` gets a duplicate of the descriptor through the new
`Host::keymap` (a default method, so existing hosts are unchanged).
`--keymap us|de-ch|de` picks the layout.

### The shell: `crates/poly-shell`, as compositor layers

**Where it lives.** Inside `poly-compositor`, as two layers of the scene
(`compose::Layer`, `Scene::render_with`):

- **below** the windows: the wallpaper and the ridge;
- **above** them: the launcher. The cursor goes on top of both.

The alternative was a separate privileged Wayland client with a
layer-shell-like protocol. That needs a protocol Poly does not have, a
second process, and buffers copied through SHM for every change. As a
layer, the shell is drawn directly into the damaged rectangles, sees input
before clients, and changes only what changed. The logic is in a
`no_std`, host-tested crate, so it can move into a client later if the
shell grows (settings, notifications).

**The ridge.** A slim status strip with no menus.
- Left: a ring glyph where Alpine has its power menu (inert).
- Right: a status pill with network and volume placeholders, then the
  clock and date as Alpine formats them (`21:41 Sep 12`). It is redrawn
  when the minute changes.

**The launcher.**
- **Position.** An input box in the middle of the screen. It sits 10 % of
  the screen's height above the exact centre, so the list has room below.
  This is the owner's direction; Alpine's launcher hangs from its bar.
- **Search.** Typing filters the apps: built-ins ("Poly Demo", "Wayland
  Test Client") plus every `.desktop` file in `/usr/share/applications`
  (the Desktop Entry Specification 1.5: `Type`, `Name`, `Comment` or
  `GenericName`, `Keywords`, `Exec` with its quoting and field codes,
  `NoDisplay`/`Hidden`; `Terminal=true` entries are skipped because there
  is no terminal yet).
- **Matching.** Each word of the query must match the name, a keyword or
  the program's file name. Tiers, best first: exact, prefix, word start,
  substring, letters in order. Keyword and program matches count one tier
  lower, and a name match wins ties; then shorter names, then the
  alphabet. It shows at most 8 rows (Alpine's default), fewer if the
  screen has no room.
- **Keys.**
  - Up/Down or Ctrl+P/N move the selection; hovering a row selects it.
  - Enter or a click on a row starts the app (fork + exec with
    `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`).
  - Escape or a click outside the panel hides the launcher.
  - Backspace deletes a character; Ctrl+Backspace or Ctrl+W a word;
    Ctrl+U everything.
- **When it shows.** On an empty desktop: at start, and again when the
  last window closes.
- **Hotkeys.** The Windows key tapped alone (Alpine's default) or
  Ctrl+Space toggles it.
- **Text.** DejaVu Sans through `poly-font`, with a glyph cache. Window
  titles use it too (`TitleFont`, the frame's `TextRenderer`, replacing
  the Spleen bitmap font). Without fetched fonts everything falls back to
  Spleen.

**The wallpaper.** Alpine uses the Windows wallpaper, and its bundled
images are not ours to copy, so Poly gets a generated one: a vertical
gradient from `#2c3e50` to `#10161d` with a soft glow, ordered-dithered so
the gentle gradient shows no steps.

#### Alpine Shell measurements

Read from the owner's Alpine Shell repository
(`apps/desktop/src/components/topbar/…`). Only values were taken; its code
is a separate product and was not copied. Tailwind units are converted at
1 unit = 4 px.

| Element | Alpine Shell | Poly | Source in Alpine Shell |
|---|---|---|---|
| Ridge height | 24 px, full width, flat | 24 px | `topBar.tsx` (`h-6`), `electron/main.js` `TOP_BAR_HEIGHT` |
| Ridge colour | rgb(32,32,32) opaque; no blur, shadow or gradient | same | `electron/modules/topbarColorDefaults.js` |
| Ridge text | system UI font, 12 px, weight 400, white | DejaVu Sans 12 px white | `index.css`, `faces/clockFace.tsx` |
| Clock | `HH:mm MMM d`, e.g. `21:41 Sep 12`; pill 18 px, 8 px padding | same | `calendar/calendarLayout.ts`, `faces/clockFace.tsx` |
| Right group | 4 px from the edge, items 4 px apart | same | `topBar.tsx` (`mr-1`, `gap-1`) |
| Status pill | 18 px, white at 10 %, 8 px padding, 14 px icons | same (network/volume placeholders) | `faces/statusFace.tsx` |
| Left glyph | ring icon 14 px, 8 px from the edge | same, inert | `faces/systemMenuFace.tsx` |
| Launcher width | 720 px | 720 px (16 px margins on small screens) | `launcher/launcherPanel.tsx` |
| Launcher position | hangs from the bar (top at 24 px) | **centred**, raised by 10 % of the height (owner's direction) | `launcherPanel.tsx` |
| Launcher surface | bar colour, opaque; 14 px radius (bottom), 1 px outline white at 12 % | same colour and outline, 14 px radius on every corner, plus Alpine's floating shadow token `0 10px 15px -3px rgba(0,0,0,.5)` approximated (it floats now) | `chrome/ChromeCanvas.tsx`, `chrome/chromeShape.ts`, `@alpine/ui` `tokens.css` |
| Search row | 48 px high, 16 px padding, 8 px gap, search icon 16 px at 60 % | same | `launcher/searchBar.tsx`, `launcherSizing.ts` |
| Input text | 15 px, white; placeholder white at 40 % | same; placeholder "Search apps" (Alpine: "Search apps, windows and settings") | `searchBar.tsx`, `launcherColors.ts` |
| Divider | 1 px white at 10 % | same | `launcherPanel.tsx` |
| Section title | 11 px, weight 500, white at 50 %, 10 px padding, ~28 px | 11 px white at 50 %, 28 px | `resultList.tsx` |
| Rows | 48 px, 10 px padding, 12 px gap, radius 6, 2 px apart; list padding 6 px | same | `resultRow.tsx`, `resultList.tsx` |
| Selected / hovered row | white at 10 % / 5 % | same | `launcherColors.ts` |
| Row icon | 28×28 px | 28 px tile in a colour of the app with its initial (no icon themes yet) | `resultIcons.tsx` |
| Title / subtitle | 13 px white / 11 px white at 50 % | same | `resultRow.tsx` |
| Footer | 36 px, right-aligned, 11 px white at 50 %, key chips 10 px on white at 10 % | same ("Open ↵", "Close Esc") | `launcherFooter.tsx` |
| Max rows | 8 by default | 8, or what fits | `electron/modules/launcherPrefs.js` |
| Empty query | search row only | same | `lib/launcher/launcherQuery.ts` |
| Hotkey | Windows key tapped alone | same, plus Ctrl+Space | `electron/modules/launcherHotkey.js` |
| Animation | opens 460 ms easeOutExpo, closes 300 ms easeInCubic | none yet (every frame costs a lot under TCG) | `ChromeCanvas.tsx` |
| Window frames | (Windows' own) | Poly's neutral frame (ADR 0106), titles now in DejaVu 13 px | n/a |

### Booting it

`cargo xtask desktop` boots Poly with a virtio GPU (1280×800), a virtio
keyboard and a virtio tablet straight into `poly-compositor --desktop` as
init, with `poly.terminal=off`. It uses QEMU's window, or VNC on port 5900
without a desktop. The image holds:

- the compositor, `wl-demo` and `wl-client`;
- a desktop file ("Demo Window", which starts `wl-demo`);
- DejaVu Sans from the font store, when fetched.

`tests/linux/wayland/wl-demo.c` is a small client of our own (no
libwayland). It opens a 360×240 window and reports what the seat sends:
the keymap (mapped and checked to be XKB text), keyboard enter, keys,
modifiers, pointer enter, motion, buttons and the wheel. A click swaps its
colour, and Q or the close button ends it.

Boot group `desktop` (shard `userland`) drives all of it through QMP and
saves a PNG per step:

1. a `screendump` shows the ridge, the launcher's panel and the wallpaper;
2. `input-send-event` types `demo`; the list appears, and Enter starts
   "Demo Window" from the desktop file;
3. its window appears centred below the ridge, in its colour;
4. a tablet move and a click reach it (it logs enter and the button and
   recolours, which the next screenshot shows), and a key reaches it (with
   a valid keymap);
5. a drag of the title bar moves it;
6. Q ends it, and the launcher is back on the empty desktop.

It takes about 25 s on TCG. The kernel test `fbdev::*` checks that
written and damaged pixels reach the display's back buffer and puts the
screen back afterwards.

## Consequences

- Poly shows a desktop: Linux GUI programs that speak Wayland over SHM
  can run on it with keyboard and mouse.
- **DRM/KMS.** When it lands, `FbFramebuffer` is replaced by a dumb buffer
  plus page flips. That brings real vsync and lets the roadmap's
  compositor item be ticked. The `Framebuffer` trait, damage tracking and
  input routing stay. fbdev stays for simple programs.
- **Not done yet:**
  - **Compositor:** `wl_pointer.set_cursor` surfaces; interactive resize;
    popup grabs; `wl_data_device` (clipboard, drag and drop);
    `wl_subcompositor`; `wp_viewporter`.
  - **Input:** keyboard repeat for the launcher; touch.
  - **Shell:** animations; icon themes and `.desktop` `Icon=`; a real
    status (network, volume) behind the ridge's placeholders; time zones
    (the clock shows UTC); notifications and settings.
  - **Windows:** windows menu, minimize, workspaces.
- The menu decision is still open. Nothing here assumes either answer.
