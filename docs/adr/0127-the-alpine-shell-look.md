# 0127: The Alpine Shell look: surfaces that hang from the ridge, Alpine's fonts, icons and tokens, anti-aliased shapes, CSS shadows and blur drawn in software

**Status:** Accepted. Roadmap Phase 6, "The shell looks 1:1 like Alpine
Shell…" (decision 2 of [ADR 0124](0124-native-desktop-alpine-shell-look-and-builds-on-windows-and-linux.md)).
The milestone stays unticked: see "What still differs". It changes the
shell of [ADR 0107](0107-desktop-fbdev-input-routing-and-shell.md) and
[ADR 0117](0117-shell-window-management-notifications-and-settings.md)
and the frame's colours of [ADR 0106](0106-wayland-server-core-and-software-compositor.md);
window management, the session bus, the settings file and the frame's
geometry are unchanged.

## Context

The owner wants Poly's shell to look like his Alpine Shell
(`alpine-shell`, Electron, React and Tailwind; its shell is in
`apps/desktop/src`), checked against its screenshots at the same size.
The first shell was an approximation: DejaVu Sans, hand-drawn glyphs,
nothing hanging from the ridge, a launcher in the middle of the screen,
toasts as floating cards. Another agent moves the desktop onto Poly's own
runtime at the same time, so this work stays in the host-tested drawing
crates and touches the compositor's platform layer only where it must
(the fonts it loads, two new press outcomes, the panels' changes).

Sources: Alpine Shell's source (values only; its TypeScript was not
copied, ADR 0124), its `@alpine/ui` 0.10.0 tokens and components, its
`.agent-screenshots`, and the public specifications named below. The live
copy on `homeserver2` was first not usable (see "The live reference" for how it was
captured later): Alpine Shell was not installed or
running there (only its old settings folder is), and installing it is not
allowed, so every comparison is against the repository's screenshots.

## Decision

### The surfaces hang from the ridge (`poly-shell/src/chrome.rs`)

Alpine draws one surface at a time in the ridge's colour under the ridge
(`chrome/chromeShape.ts`): a body with its bottom corners rounded (14 px)
and, on each side not against the screen's edge, a concave "shoulder", a
quarter circle of 14 px from the ridge's edge into the body's side.
Placement (`shared/popupPlacement.ts`): centred under the item that opened
it, unless a shoulder would come within 6 px of a screen edge, then against
that edge without the shoulder there. Motion (`ChromeCanvas.tsx`): opening
grows the body from the ridge with its contents sliding down inside it,
460 ms easeOutExpo; closing shrinks it, 300 ms easeInCubic; one surface
turns into another by moving and resizing, 400 ms easeOutExpo.

`Motion<K>` keeps that state on the shell's millisecond clock; while
anything moves the shell asks for a frame every 16 ms through
`next_notification_deadline` (so the compositor needs no change for it).
Priority, as Alpine's superseding: the launcher over the switcher over a
panel over the notices. `Shell::set_animations(false)` moves at once (the
tests use it).

The surfaces:

| Surface | Opened by | Alpine source | Poly |
|---|---|---|---|
| Launcher | the Windows key, Ctrl+Space, an empty desktop | `launcher/*` | hangs from the ridge's middle (was centred on the screen, ADR 0107): 720 wide, a 48 px search row (Lucide search 16 px at 60 %, 15 px text, placeholder at 40 %), a divider, the section title (11 px medium at 50 %), 48 px rows 2 px apart with a 28 px icon box (Lucide glyphs at 40 % for apps without icons), 13/11 px text, selected white 10 %, hovered 5 %, the footer (36 px, "Open" and a ↵ chip on white 10 %); the list at most 560 px or 70 % of the screen; nothing typed or nothing found shows the search row only |
| System menu | the dashed circle (Lucide `circle-dashed`, 14 px) | `systemMenu.tsx` | Restart and Shut Down, 12 px at 80 % with 12 px icons, each confirmed by a second click ("Confirm restart?"); `Press::Power` makes the compositor `sync` and `reboot(2)` |
| Control centre | the status pill | `status/controlCenter.tsx`, `controlSlider.tsx`, `soundMenu.tsx` | 260 wide, `p-3 gap-3`: Ethernet (the interface under it) and Sound (the card) rows with 17 px icons and 15 px chevrons, each opening a list (Output/Input; the interface) with a back arrow; the volume slider on `#3F3F3F` at 20 % with a 10 px white track over white 20 %, its speaker muting; the gear (24 px) opening the settings. Its changes reach the compositor through `Shell::take_changes` |
| Calendar | the clock | `calendar/calendarFlyout.tsx` | one day (Alpine's default): a 46 px gutter and a 264 px column, the date heading (14 px medium) with day arrows and "Today", weekday in 9 px capitals, today's date in a white circle, the all-day strip, 44 px hours in a 416 px window scrolled to the time now, the red now line |
| Notices | a notification | `noticeStrip.tsx` | against the right edge under the ridge (the owner's top right), newest first with dividers: 14 × 10 px padding, an 8 px dot (rose when critical), 12 px medium title at 95 %, 12 px message at 70 % in 16 px lines, the × at 60 %, action buttons on white 15 % |
| History | the unread count in the ridge's middle, Super+N | (Alpine keeps none; its notice face `noticeFace.tsx` is the trigger) | 360 wide, "Notifications" and Clear, rows of title, time and message |
| Switcher | Alt+Tab | `windowSwitcherPanel.tsx` | the cards of ADR 0117, now hanging from the ridge |

Panels open when the pointer comes onto their item on the ridge (Alpine's
`onMouseEnter`) or on a click, and close when it leaves the item and the
panel; a click elsewhere closes the panel and goes to the windows. While a
window is dragged the compositor sends the shell no motion, so the ridge
opens nothing then; while the launcher shows the ridge opens nothing (it is
under Alpine's launcher overlay).

### Tokens

Read from `@alpine/ui` and the components; the settings window's colours
are those of Alpine's screenshots (sRGB, measured), which are a little
darker than the current tokens' `#202020` ground:

| Token | Value |
|---|---|
| Ridge and surfaces | `#202020` (Alpine's default bar colour, `topbarColorDefaults.js`), 24 px |
| Settings surfaces 0/1/2/3 | `#1a1a19`, `#212120`, `#292928`, `#333332`; strong line `#3b3b39` |
| Inks | `#ebebe7`, quiet `#9a9a95`, faint `#6f6f6b`; white at 10/5/50/40 % on the ridge's surfaces |
| Radii | surfaces 14, cards 12, fields 8, rows 6, chips 4 |
| Settings layout | header 56, sidebar 224 with 36 px rows (radius 8, 15 px icons, 13 px text, the active one medium on surface 3), section labels 11 px semibold at 5 % spacing; the page on surface 1 with a rounded top-left corner and a hairline, 20 px gutter, the title in Averia Serif Libre 24 px (`tracking-tight`), headings 14 px semibold, rows of a 14 px label over a 12 px description 62 px apart, outline segmented choices (`rounded-full`, 13 px), a 176 × 8 px slider with its value, the 38 × 22 px toggle (knob 16 px with `0 1px 2px rgba(0,0,0,.25)`), cards on surface 2 with a hairline |
| Window frame | title bar `#1a1a19` (inactive `#242423`), text `#ebebe7`/`#9a9a95`, border `#3b3b39`, top corners 8 px (Windows 11's, the windows Alpine runs among), shadow `0 8px 32px rgba(0,0,0,.37)` (inactive `0 4px 20px rgba(0,0,0,.25)`) kept off the ridge's rows; geometry unchanged (24 px title, 1 px border) |

The settings window is 900 × 620 (was 760 × 520) so the longer pages fit.
Its sidebar has Alpine's sections with Poly's pages (Personal: Wallpaper;
Devices: Displays, Keyboard, Sound; System: Date & time, About), the
header Alpine's back and forward arrows (a page history) and the page's
name underlined.

### The settings window in Alpine's current layout

Alpine's live settings app (measured on the window captures, content
1200 × 800) puts the sections in tabs in a 40-pixel header — back and
forward arrows (28 px, `pl-3`), then Home, Personal, Desktop, Devices,
System (`h-7 px-3`, 13 px, the active one medium on surface 3) and the
settings search on the right (256 × 32, radius 8, a hairline, a 14-pixel
search glyph, "Search settings", "Ctrl K" in Geist Mono) — and lists only
the active section's pages in the 224-pixel sidebar (36-pixel rows from
y 44). The page title is Geist 22 px medium (`tracking-tight`, a 32-pixel
line), sections are an 11-pixel medium label over a group card (white at
6 %, radius 12, rows at least 48 pixels with `px-3.5 py-2.5`, hairlines
between), 32 pixels apart. Poly does the same with its pages: Home;
Personal: Wallpaper; Devices: Displays, Sound, Keyboard; System: Date &
time, About (Desktop has no Poly page yet and, as in Alpine, a section
without pages has no tab). Home shows the wallpaper (236 × 132), "Poly",
the system and processor, the memory in use as a bar (`MemAvailable`), and
a card of rows to the pages with their values. The search finds pages by
their names and words (volume, resolution, time zone, ...); typing in the
window searches, Enter opens the first result, Escape clears. Dark theme
with Alpine's dark tokens; Alpine's light theme is not added.

### The launcher lists open windows

As Alpine, the launcher finds open windows as well as apps: a "Windows"
section first, then "Apps", each with its 11-pixel title; Enter or a click
on a window switches to it (`apps::window_entry`, `Shell::set_windows`; the
compositor keeps the list current from its window manager).

### Fonts

Alpine's: Geist (regular, medium, semibold) for everything, Averia Serif
Libre for display titles (bold for the settings window's name), Geist Mono
for key hints. `cargo xtask fonts
fetch` takes Geist 1.7.2 from Vercel's release archive (pinned by size and
SHA-256, unpacked by a pure-Rust ZIP and DEFLATE reader, `tools/xtask/src/unzip.rs`,
from PKWARE's APPNOTE and RFC 1951, so it works on Windows and Linux) and
Averia Serif Libre from the Google Fonts repository at commit `6f3c56c7`,
with both OFL licences; the desktop's images carry them in
`/usr/share/poly/fonts/` (outside fontconfig's directories, so the
toolkits' fonts do not change) and the compositor loads them
(`desktop::shell_fonts`), falling back to DejaVu Sans.

`poly-shell`'s `Text` sets text by `Style` (face, size, letter spacing)
with `poly-font`'s quarter-pixel positioning, a light vertical hinting —
the outline is scaled so the face's x-height lands on a whole pixel
(`poly_font::render_glyph_scaled`, new) — and a 1.2 gamma on coverage, so
light text on the dark surfaces keeps the weight DirectWrite gives it.

### Icons

Alpine draws Lucide icons (lucide-react 0.575.0, ISC). `poly-shell/src/icons.rs`
carries the path data of the 35 it uses and strokes it as an SVG renderer
would: a parser for SVG 1.1 path data (`M L H V C S Q T A Z`, implicit
repeats, packed numbers and flags), curves flattened, arcs converted from
endpoint to centre form (SVG 1.1 F.6.5), and each pixel covered by its
distance to the nearest segment for Lucide's 2-unit round-capped stroke.

### Shapes, shadows and blur (`poly-wayland/src/compose/paint.rs`)

In `poly-wayland` because the frame needs them and the shell depends on it
(`poly-gfx` does not allocate):

- **Anti-aliased shapes**: signed distance functions at pixel centres,
  coverage `0.5 − distance`: rounded rectangles with a radius per corner,
  CSS-style inside borders, discs and rings, and `Chrome`, the hanging
  surface with its shoulders.
- **Shadows** as CSS Backgrounds and Borders 3 §7.1 defines `box-shadow`:
  the shape offset and spread, blurred by a Gaussian of standard deviation
  half the blur radius, approximated by three box blurs (within about 3 %)
  over a coverage mask of only the damaged region plus the blur's reach, so
  drawing in pieces gives the same pixels as drawing whole; an occluder (the
  window itself) is skipped. Used for the frame and the toggle's knob —
  Alpine itself uses `box-shadow` only there and on popovers Poly does not
  have.
- **Backdrop blur** (`blur_backdrop`, CSS `backdrop-filter`'s standard
  deviation), with the same boxes per channel, normalised at the edges.
  Alpine's shell uses no backdrop blur (only a blurred album cover behind
  its media face, which Poly does not have), so nothing draws with it yet.

### The wallpaper

Alpine shows the Windows wallpaper and bundles two pictures,
`Grainy Valley.jpeg` and `Quiet Moment In Nature.jpg`
(`apps/desktop/wallpapers`). The owner, whose they are or who may use them
freely, lets Poly ship them (THIRD_PARTY.md). They are in
`third_party/alpine-shell/wallpapers/`; `cargo xtask` decodes them on the
host with the `jpeg-decoder` crate (pure Rust, so on Windows too), scales
them to cover 1600 × 900 by averaging, and puts them in the desktop's
images as `/usr/share/poly/wallpapers/<preset>.pwal`, a plain picture
format (`PWAL`, width, height, RGB; `poly_shell::picture`) so the native
compositor needs no JPEG decoder. The compositor hands them to the shell
(`Shell::add_wallpaper`), which draws them to cover the screen, sampled
bilinearly, once per size into its cache. Grainy Valley is the default;
Quiet Moment and four gradients follow. Without the pictures (tests,
images built without them) Grainy Valley falls back to a drawn valley in
the same greys and the others to gradients.

### The live reference (homeserver2)

A fresh build of Alpine Shell (its repository at `3557227`, in a separate
worktree, `bun install`, the native add-on, `vite build`, electron-builder
`--dir`) packaged shows only its first-run window ("Welcome to a faster,
cleaner Windows experience", Install, a gear, a flask). Install registers
it as the Windows shell; the flask's test session retires Explorer. Its
development mode — the stock Electron runtime running the unpacked app, the
pages served from `dist/` on 127.0.0.1:5173 as its dev server would — runs
the shell beside Explorer without registering anything (its
`planShellActivation` keeps an unpackaged run idle) and without a licence
check. It ran on homeserver2 in the logged-on session (a scheduled task,
`/IT`) at 1920 × 1080 and scale 1, driven over the Chrome DevTools Protocol
(Alpine's keyboard hook ignores injected keys): the ridge, the launcher
empty and typed, the system menu, the control centre, a notice (its own
"plugin not enabled" notice through an `alpine-shell://open-app` link) and
the settings pages were captured; then it was stopped, the task deleted,
the build removed. The calendar needs a connected Google calendar and the
switcher a real Alt+Tab, so those have no live picture. From the live
launcher the section title's height became 31 (with the list's gap).

## Tests

- `paint`: edges exact and corners smooth, per-corner radii, borders,
  rings, the chrome's shoulders and edges, the Gaussian boxes' variance, a
  shadow's darkness, extent and symmetry, drawing in pieces equals drawing
  whole, occluders, the backdrop blur.
- `chrome`: Alpine's placement, its curves, opening, turning, closing and
  no animation.
- `icons`: trigonometry, path data with implicit commands, packed numbers
  and flags, arcs on their circle, strokes where the path runs, clipping,
  every icon parses.
- `shell`: the launcher hanging from the ridge (pixels of its body,
  shoulder and corner), typing and launching, the animation's heights over
  time and the frames it asks for, the notices at the top right and giving
  way to the launcher, panels opening on hover and closing when left, the
  system menu's confirmation, the control centre's slider, mute, lists and
  gear, the calendar's dates and days, the switcher, HiDPI.
- `settings_ui`: every page fits and draws, controls in Alpine's colours,
  the arrows' history, every change.
- The compositor's frame: rounded corners, the shadow fading out and
  damaged with the window, kept off a shell's rows.
- `poly-font`: the vertically scaled rendering. `xtask`: CRC-32, stored,
  fixed and dynamic DEFLATE blocks, a ZIP archive and a corrupt member.
- `tests/preview.rs` renders every part in Alpine's fonts at 1920 × 1080
  (`POLY_SHELL_FONT_DIR`, `POLY_SHELL_PNG_DIR`): the comparison sheets'
  Poly side.
- Boot groups `desktop` and `desktop-shell` check the new places (the
  launcher under the ridge, the switcher hanging from it, the settings
  window's controls at their new positions).

## What still differs (why the milestone stays unticked)

- **Typeface of the ridge and the launcher.** Alpine's top bar and launcher
  inherit `system-ui`, which is Segoe UI on Windows (only the settings use
  Geist); Segoe UI cannot be shipped, so Poly uses Geist there too.
- **Text rendering.** Alpine's Chromium draws ClearType (subpixel) text on
  opaque surfaces; Poly draws grayscale with light hinting.
- **No live calendar or switcher.** The calendar needs a Google account
  connected and the switcher a real Alt+Tab; both follow Alpine's source.
- **Settings content and theme.** The layout is Alpine's current one, but
  homeserver2 runs it in the light theme (Poly: dark, no light theme yet);
  Poly has no Desktop tab, no disk bar, no Network, Bluetooth, Battery,
  Appearance or Ridge pages, and its search finds pages, not single
  settings.
- **The notices** hang from the ridge's middle in Alpine and at the top
  right in Poly (the owner's direction).
- **The screenshots' ridge colour** is navy (the user picked the wallpaper's
  colour); Poly uses Alpine's default `#202020`.
- **Content.** Poly's pages are its own (Displays has no arrangement
  card, there is no Home, Appearance or search field in the sidebar); the
  launcher has no settings, calculator or web rows, and no app icons; the control
  centre has no Wi-Fi or Bluetooth; the system menu has no Log Out or
  Hibernate; the clock is 24-hour.
- The 1 px border Alpine draws along the ridge and around surfaces where
  windows lie under them, and hover colours of the frame's buttons, are not
  drawn.

## Consequences

- The shell has Alpine's structure: everything opens from the ridge, one
  surface at a time, animated as in Alpine.
- New public API: `poly_wayland::compose::paint`, `Theme::{FLAT, shadow,
  shadow_extent}`, `Scene::drawn_extent`, `CORNER_RADIUS`;
  `poly_shell::{chrome, icons}`, `draw::{Face, Style, Fonts}`,
  `Text::{with_fonts, draw_style, measure, centre}`, `Shell::{open_panel,
  close_panel, panel, surface, release, take_changes, set_sound_card,
  set_animations}`, `Press::Power`; `poly_font::render_glyph_scaled`.
- The compositor's changes: the fonts it loads, `Press::Power`, the
  release of a shell press, the panels' changes applied, no ridge hover
  while a window is dragged, the sound card's name for the control centre,
  the wallpapers' pictures (`desktop::load_wallpapers`).
