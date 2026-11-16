# 0209: Windows programs type with the desktop's keyboard layout

**Status:** Accepted. Chrome for Windows browsing in its window (track
`chrome-win-browse`, ADR 0155 addendum "browsing"). Inputs: Microsoft Learn
("Virtual-Key Codes", "Keyboard Input Overview" with its scan code table,
"Keyboard identifiers and input method editors", "GetKeyboardLayout",
"ToUnicode", "MapVirtualKeyW"), the printed legends of the US keyboard, the spec
sheets `MapVirtualKeyW.yaml`, `ToUnicode.yaml`, `SendInput.yaml`. No third-party source.

## What was there

win32k knew one keyboard layout, the German one the oracle uses (ADR 0136,
`poly_win32k::kbd`). Poly's desktop sends keys as their places on the keyboard
(evdev codes, which for the main block are set 1 scan codes) and reads them
with the layout of its settings (`us` by default, `de-ch`, `de`;
`poly-input`). So a key gave Poly's own apps one character and a Windows
program another: on the default desktop, typing `http://example.com/` into
Chrome's address field gave `httpÖ--example.com-` (`:` is Shift and the US `;`
key, which is `Ö` in the German layout; `/` is the German `-`).

## Decisions

1. **The desktop's layout is the one Windows programs type with.** The
   compositor tells win32k its layout's name with the desktop link's existing
   `Layout` record, at the start and whenever the settings change it. win32k
   (`Wm::set_layout`) reads keys into virtual keys with it, and `user32` asks
   for it (`Request::KeyboardLayout`) before it turns keys into characters
   (`TranslateMessage`, `ToUnicode(Ex)`, `MapVirtualKey(Ex)W`) and when it
   names it (`GetKeyboardLayout`, `GetKeyboardLayoutNameW`,
   `GetKeyboardLayoutList`'s first entry). One question per call; the answer
   is a number.
2. **Two layouts: German and US.** `kbd::Layout::Us` takes its places from
   Learn: the `VK_OEM_*` keys Learn names for the US keyboard (`VK_OEM_1` the
   `;:` key, `VK_OEM_2` `/?`, ...) at their places in Learn's scan code table,
   Y and Z in their QWERTY places; its characters are the US legends; there
   is no AltGr and there are no dead keys, Caps Lock acts on letters only.
   Its handle is `0x04090409`, its KLID `00000409` (Learn). Swiss German has
   no table of its own yet and gets the German one (the same letters,
   digits, `.`, `,`, `-`, and `:` and `/` on Shift); its own table is the next
   step when someone needs `ü`/`è` and the other differences.
3. **Without a desktop nothing changes.** A program that runs before a
   compositor greets win32k, every headless run and every spec sheet's test
   keeps the German layout and the oracle's handle `0x04072000`.

## Consequences

- A URL typed on Poly's desktop reaches Chrome as typed.
- The layout lives in the window manager, so all processes see the same one
  at the same time; a program that cached `GetKeyboardLayout` before the
  layout changed sees the change at its next keyboard call, not through
  `WM_INPUTLANGCHANGE` (not sent yet).
- The Control characters of the US punctuation keys follow the German table's
  places (`Ctrl+[` is `0x1B`, `Ctrl+]` `0x1D`, `Ctrl+\` `0x1C`); the others
  give nothing. A spec sheet on a US oracle would pin them down.
