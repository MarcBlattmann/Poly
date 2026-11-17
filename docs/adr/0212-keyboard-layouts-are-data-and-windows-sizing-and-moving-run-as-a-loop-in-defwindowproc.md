# 0212: Keyboard layouts are data, and Windows' sizing and moving run as a loop in DefWindowProc

**Status:** Accepted. Track `impl-win-input`. Inputs: the spec sheets and visible
tests of `ToUnicodeEx`, `MapVirtualKeyExW`, `VkKeyScanExW`, `LoadKeyboardLayoutW`,
`UnloadKeyboardLayout`, `ActivateKeyboardLayout`, `WM_NCHITTEST`,
`WM_ENTERSIZEMOVE`, `WM_MOUSEHWHEEL` and `SC_MAXIMIZE` (`specs/windows/user32.dll/`,
hand-off `spec-win-input`); Microsoft Learn ("Keyboard identifiers and input method
editors", "ToUnicodeEx", "MapVirtualKeyExW", "VkKeyScanExW", "LoadKeyboardLayoutW",
"ActivateKeyboardLayout", "WM_NCHITTEST", "WM_ENTERSIZEMOVE", "WM_SIZING",
"WM_MOUSEHWHEEL", "WM_SYSCOMMAND"); `winuser.h` for constants. No third-party source.
Builds on ADR 0209 (Windows programs type with the desktop's layout).

## What was there

ADR 0209 gave win32k two layouts (the oracle's German and US English), written as code
with one `match` per character key, and said that Swiss German "has no table of its own
yet and gets the German one". `LoadKeyboardLayoutW`, `UnloadKeyboardLayout`,
`ActivateKeyboardLayout` and `VkKeyScanExW` were stubs that failed; `GetKeyboardLayoutList`
answered a fixed list; `MapVirtualKeyExW` and `ToUnicodeEx` ignored their layout argument.
`DefWindowProc` answered `WM_NCHITTEST` with a frame as thick as the border everywhere (no
corner zones, no caption buttons); `WM_SYSCOMMAND` with `SC_SIZE` / `SC_MOVE` only asked the
compositor to drag the surface, so a program in a session without a compositor never saw a
resize and none of `WM_ENTERSIZEMOVE`, `WM_SIZING`, `WM_MOVING`, `WM_EXITSIZEMOVE`; the
horizontal wheel was dropped; Win+Up did not maximize.

## Decisions

1. **A layout is a table** (`poly_win32k::kbdtab`): per key its virtual key, its scan code in
   the layout (0: no such key there) and ten cells, one per shift state (none, Shift, Caps
   Lock, Shift+Caps Lock, Ctrl, Ctrl+Shift, AltGr, AltGr+Shift, Alt, AltGr+Caps Lock); a cell
   is a character, "none" or a dead key's spacing character. `kbd.rs` has no per-layout code
   left: `ToUnicode(Ex)`, `MapVirtualKey(Ex)W`, `VkKeyScan(Ex)W` and `TranslateMessage` read
   the tables. Four layouts: German (the oracle's, KLID `00000407`), US English
   (`00000409`), Swiss German (`00000807`) and French (`0000040C`); a loaded layout of any
   other KLID types as US English, as `LoadKeyboardLayoutW` answers an unknown KLID with
   the US layout. The cells are the spec sheet's tables (`ToUnicodeEx.yaml`), written out.
2. **The desktop's Swiss German is its own layout.** `Layout::for_desktop("de-ch")` is
   `SwissGerman` (HKL `0x08070807`), so a Windows program on a desktop set to Swiss German
   types `z`, `y`, `ü`, `ä`, `é` and AltGr+2 `@` with the keys of the keyboard.
3. **Dead keys.** The accents are `^`, `´`, `` ` ``, `¨` and `~`; a key after a dead key
   gives the combined character, or the accent and the key's character when they do not
   combine (Space gives the accent alone). A control character (Ctrl+letter) replaces the
   accent, and a letter, digit or punctuation key that makes no character in this state
   (AltGr+a) drops the pending accent and gives nothing (`ToUnicodeEx.yaml` `dead_keys`).
4. **The layout list is per process** (`poly_win32k::kbdlist::LayoutSet`, host-tested; the
   state is `user32`'s `kbdlayout`): the oracle's three layouts (Swiss German and German
   under English (UK) around the desktop's own, `0x08070809, <desktop>, 0x04070809`), what
   the process loaded (appended, once), a layout per thread that activated one (the process's
   otherwise), `KLF_SETFORPROCESS`, `KLF_REORDER`, `HKL_NEXT` / `HKL_PREV` with wrap-around.
   Unloading a layout the thread has falls back to the first of the list. An HKL is
   `(device identifier << 16) | language identifier` of the KLID; the layout it types with
   follows the device identifier. `WM_INPUTLANGCHANGE` goes to the focus window after a
   change, and a `WM_KEYUP` of Ctrl follows it on every layout but Swiss German, as observed.
   `ToUnicodeEx`, `MapVirtualKeyExW` and `VkKeyScanExW` use the layout of their `HKL`
   argument (NULL: the thread's); an `HKL` that is not in the list gives nothing.
5. **`WM_NCHITTEST`** (`poly_win32k::nchit`): a sizable captioned window has corner zones of
   19 / 18 pixels at the top and 18 / 17 at the bottom whatever its size (9 and 7 rows
   along the sides), the system menu and the three caption buttons on the caption's rows, a
   tool window's smaller top corners; a sizable window without caption has corners of the
   frame plus 22 pixels on every side. Windows that cannot be sized keep their caption and border.
6. **The sizing and moving loop** (`user::sizemove`) runs inside `DefWindowProc`'s
   `WM_SYSCOMMAND(SC_SIZE | SC_MOVE)` when no compositor drags the surface: `WM_GETMINMAXINFO`,
   `WM_ENTERSIZEMOVE`, the mouse captured; each move is `WM_SIZING` / `WM_MOVING` (the handler
   may change the rectangle; an edge never goes past the track limits) and a `SetWindowPos`
   with the flags the oracle's calls carry (`0x80214`); the release (or Escape, which gives the
   first rectangle back) ends the capture, makes the closing `SetWindowPos` (`0x204`, `0x205`
   for a move) and sends `WM_EXITSIZEMOVE`. With a compositor the compositor still drags
   (ADR 0128): `WM_ENTERSIZEMOVE` is sent when it starts and `WM_EXITSIZEMOVE` is posted when
   the left button comes up.
7. **Small input rules the tests pinned:** the horizontal wheel (`MOUSEEVENTF_HWHEEL`, the
   desktop link's horizontal axis) reaches the window under the pointer as `WM_MOUSEHWHEEL`
   after a `WM_NCHITTEST`; Win+Up and Win+Down act on the active window as
   `WM_SYSCOMMAND` `SC_MAXIMIZE` (position -1,-1) / `SC_RESTORE` / `SC_MINIMIZE` and the Up or
   Down key-down is not delivered; a top-level window that comes under the cursor by a move
   or a resize gets a mouse move; a mouse move that a capture release makes asks the window
   `WM_NCHITTEST` except over the default caption.

## Consequences

- Swiss German works in every Windows program on a desktop set to it; adding a layout is
  adding a table and a row in `Layout`.
- A thread that activates another layout types with it (`TranslateMessage`, `ToUnicode`),
  but win32k still turns keys into virtual keys with the desktop's layout: a program that
  activates a different layout than the desktop's sees the desktop's key places.
- A layout loaded by a KLID this crate has no table for types as US English.
- Not done: `KLF_REPLACELANG`, per-window input languages, IME switching, the keyboard
  layout changing the list of a program that is already running (its middle entry follows the
  desktop at its next keyboard call).
