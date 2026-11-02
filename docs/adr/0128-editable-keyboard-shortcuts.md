# 0128: Editable keyboard shortcuts, as Alpine Shell records them

**Status:** Accepted. The owner's request after ADR 0127: the Keyboard
shortcuts page (Desktop tab) becomes editable, in Alpine Shell's style.
It replaces the compositor's fixed hotkeys of ADR 0107 and ADR 0117
(`desktop::Hotkeys`, `desktop::wm_key`) with one keymap in `poly-shell`.

## Context

Until now every shortcut was a constant in the compositor: the Windows key
tapped alone and Ctrl+Space for the launcher, Super+I, Super+N, Alt+Tab
(Shift backwards), Alt+F4 and Super+arrows. The settings page listed them
read-only. Alpine Shell lets the user record a new combination for its
shortcuts (sources, values only: `electron/modules/shellShortcutBindings.js`
and `shellShortcuts.js`, `src/components/settings/shortcutsPage.tsx`,
`hotkeyRecorder.tsx`, `keyCaps.tsx`, `src/settings/shortcutCatalogue.ts`).

## Decision

### One keymap (`poly-shell/src/shortcuts.rs`)

- **Actions**, each with Poly's default: the Search hotkey (Win tapped
  alone), the second Search hotkey (Ctrl+Space), Settings (Win+I),
  Notifications (Win+N), Switch windows (Alt+Tab; with Shift added it goes
  backwards), Snap window (Win+arrows), Close window (Alt+F4). Poly has no
  shortcut for Log Out or a screenshot, so there is none to edit. Unlike
  Alpine, Alt+Tab and Alt+F4 are editable: on Poly the compositor, not
  Windows, owns them.
- **Combinations**: Win, Ctrl, Alt, Shift and a key; a modifier alone means
  tapping it; snapping takes the modifiers with any arrow. Keys are named
  as the keyboard layout types them (the character, unshifted, upper case;
  Alpine's labels for the rest: Space, Tab, Esc, F1..F12, Page Up, ←), so
  a combination follows the layout as Windows' virtual keys do: Win+Z is
  the key that types Z, which Swiss German puts where US has Y.
- **The keymap** keeps only the changes from the defaults (Alpine's
  overrides). The compositor matches every key against it
  (`shortcuts::Matcher`, with the seat's modifiers, exact: Ctrl+Shift+Space
  is not Ctrl+Space); the switcher, while open, steps with its key again
  and Left/Right, cancels with Escape, switches on Enter or when one of its
  modifiers is released.

### Recording (Alpine's recorder)

Set on a row starts recording: the caps give way to "Press a shortcut"
(22 px high, radius 5, a quiet ring, 11 px) and Set to Cancel. The
compositor hands every key to the recorder first, so neither the clients
nor the shortcuts see them (nor the releases of the keys it took): the
modifiers pressed are collected, the first other key ends it, Escape alone
cancels, 15 seconds without a key give up, a press elsewhere in the window,
another page or Cancel stops it. The combination is then checked as Alpine
checks it (`validateChord`), in its order, with its messages under the row
in Alpine's danger red (`#f87171` dark, `#dc2626` light):

| Refusal | Message |
|---|---|
| Snapping without an arrow | Press the modifiers with an arrow key. |
| No Win, Ctrl or Alt | Hold Win, Ctrl or Alt with the key. |
| Reserved (Ctrl+Alt+Delete) | Poly keeps that one for itself. (Alpine: "Windows keeps that one for itself.") |
| Already another action's (any of its arrows, the switcher's Shift too) | Already used by *action*. |
| A key without a character (a dead key) | Couldn't save that shortcut. |

A combination already taken is refused, not swapped, as Alpine does. An
accepted one applies at once (the compositor matches against the new
keymap on the next key) and is saved. A changed row shows Alpine's reset
button (RotateCcw, 28 × 28) before Set; once anything is changed, "Restore
all defaults" (a small ghost button on the title's line; Alpine has no
reset-all, the owner asked for one) puts every shortcut back. The Search
page's hotkey row is the same recordable row.

### Persistence

The settings already persist (ADR 0117): `key=value` lines in
`$XDG_CONFIG_HOME/poly/shell.conf` (else `$HOME/.config/poly/shell.conf`)
or `--settings FILE`, written to a temporary file, synced, renamed,
`sync(2)`. Shortcuts join them as `shortcut.<action>=<combination>` lines
(`shortcut.launcher=Ctrl+Alt+K`), only the changed ones; a line that does
not parse, names no action or clashes with what is already there is
dropped. They survive a reboot where that file is on a disk: the
`desktop-shell` boot test's ext2 disk on `/mnt`, and now `cargo xtask
desktop`, which keeps a small ext2 settings disk
(`target/poly-desktop-state.img`, mounted on `/mnt`, `--fresh-state` makes
it again). A boot from the initramfs alone keeps `/root` in memory and
forgets them.

## Tests

- `shortcuts`: reading and writing combinations, key names on US and Swiss
  German, defaults, changes and resets, every refusal, the settings file's
  lines, the recorder (swallowed releases, Escape, the time-out), matching
  (tap, exact modifiers, the switcher, rebound combinations, the layout).
- `settings_ui`: recording from Set, a refusal in red, an accepted
  combination with its caps and reset, Restore all defaults, Cancel and a
  click elsewhere; the boot test's clicks land.
- `shell`: keys through `Shell::shortcut_key`, the time-out.
- The compositor: evdev keys to the shell's matches, rebound.
- Boot group `desktop-shell`: the Search hotkey rebound to Ctrl+Alt+K with
  the pointer and keys in the settings window; the Windows key no longer
  opens the launcher, Ctrl+Alt+K opens and closes it; Restore all defaults
  brings the Windows key back (screenshots of each).

## Consequences

- New public API: `poly_shell::shortcuts` (`Action`, `Chord`, `Keymap`,
  `Mods`, `Recorder`, `Matcher`, `Hit`, `Refusal`, `key_name`),
  `Settings::shortcuts`, `Change::{Shortcut, ResetShortcut,
  ResetShortcuts}`, `Shell::{shortcut_key, shortcuts}`,
  `SettingsWindow::{recorder, recorded, recording}`, `Palette::danger`;
  `poly-shell` depends on `poly-input` (the layouts).
- The compositor: `desktop::Shortcuts` replaces `Hotkeys` and `wm_key`;
  keys go to the recorder first; it logs the launcher shown or hidden by a
  shortcut and every shortcut change.
