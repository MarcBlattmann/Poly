# 0201: ANSI windows and the form of messages; carets; the common control classes

**Status:** Accepted. Windows applications, batch 3 (track `impl-win-apps-3`):
the carets, the `...A` window, dialog and resource functions, `MessageBoxIndirectW`,
`GetMenuBarInfo`, the image list drawing functions and the common control
classes that PuTTY, 7-Zip File Manager and their kind stop on. Inputs: the spec
sheets and visible tests of `spec-win-apps-3`
(`specs/windows/{user32,comctl32}.dll/`), Microsoft Learn's reference pages and
`winuser.h` / `commctrl.h` for numbers and layouts. No third-party source.

## What was there

- The `A` functions of user32 were fail-only stubs, or, for text, thin copies of
  the `W` ones (`SendMessageA` called `SendMessageW`; `GetWindowTextA` cut every
  character to 8 bits).
- A window knew whether its class was `unicode` and `IsWindowUnicode` said so, but
  nothing converted anything: a message was delivered as it was sent.
- There was no caret; `comctl32.dll` had image lists and no window classes.

## Decisions

### 1. A window has a form, a message has a form, the difference is converted once

A window procedure is either *ANSI* (the window's class was registered by
`RegisterClassA/ExA`, or the window was made by `CreateWindowExA` or by an `A`
dialog function from a system class) or *wide*. The window manager keeps the flag
(`CreateSpec.ansi` makes a window of a system class an ANSI window; `WinInfo.unicode`
is what `IsWindowUnicode` reports). Our own classes (`Button`, `Edit`, `#32770`, ...)
work in UTF-16 whatever they report, except that the dialog class hands its
dialog procedure the form of the dialog window.

A message is wide unless its sender says otherwise: `SendMessageA`,
`CallWindowProcA` and the other `A` senders set the thread's *next message form* to
ANSI (`ansi::with_form`), and the window procedure hub (`Env::call_proc`) takes
it. When the form of the message and that of the procedure differ,
`ansi::enter` converts the parameters of the messages that carry text
(`WM_SETTEXT`, `WM_GETTEXT`, `WM_CREATE` / `WM_NCCREATE`, the character messages,
`EM_REPLACESEL`, `EM_GETLINE`, the list and combo box string messages) and
`Conv::finish` converts the result and the text the receiver wrote. Rules the
sheets fixed: the conversion goes through the process's ANSI code page (Windows-1252);
a character the code page lacks becomes `?`; a buffer for text written to a wide
caller gets twice the size when it is passed to an ANSI procedure (`WM_GETTEXT`
shows 400 for `GetWindowTextW(.., 200)`); a message sent from another thread is
converted to wide by the sender (the receiver knows nothing of a form).

`DefWindowProcA` and `DefDlgProcA` do the opposite for the default handling: the
message becomes wide, `u::def_window_proc` (or the dialog manager) runs, the
result and the text written go back. An ANSI dialog gets a text converted *once*
(a `SendMessageW(WM_SETTEXT)` on an ANSI dialog leaves the text that went through
the code page and back).

### 2. A caret is state of the input queue

One caret and one caret position per input queue (`poly_win32k::caret`):
`CreateCaret` makes a hidden caret (hide count 1) at the queue's position, the
position survives the caret, `ShowCaret` lowers the count and never below 0,
`DestroyWindow` takes the caret of the window away. `GetGUIThreadInfo` reports
`hwndCaret`, `rcCaret` (`x, y, x + width, y + height`, wrapping) and
`GUI_CARETBLINKING`. The blink time is 500 ms; `SetCaretBlinkTime` works in the
interactive session only (1459 elsewhere), as on the oracle.

On the interactive desktop a shown caret is a rectangle of the owner window's
client area inverted with `DSTINVERT` and inverted again by a timer of the
owner's thread every blink time (`caretdraw.rs`): the look of Windows' caret
and of Alpine Shell's text cursor, which is the text colour inverted too.

### 3. The common control classes are one window procedure and host-tested models

`InitCommonControls` and a valid `InitCommonControlsEx` register the 19 classes
of comctl32 5.82 (not `SysLink`, which is version 6) for the process;
`poly_comctl` holds what a window of each class starts as (the toolbar's
`CCS_TOP`, the tooltip's `WS_POPUP | WS_BORDER`, the tab control's
`WS_CLIPSIBLINGS`, the texts the IP address control and the date controls show,
the sizes toolbar, rebar and status bar give themselves) and the state and the
basic messages of the status bar, progress bar, trackbar, up-down control, tab
control, list view and tree view. Painting and the rest of the messages of these
controls are not here yet.

### 4. Image lists keep their pixels in the layout of Windows' bitmaps

`ImageList_GetImageInfo` hands out a bitmap: four images wide, rows added
`grow` ahead of the count, shrinking only through `ImageList_SetImageCount`
(`ili_changes`). Drawing follows the recorded results: `ILD_MASK` ANDs the mask
(black where the image is) without a background and copies it with one,
`ILD_IMAGE` draws the colour bitmap as stored (masked pixels black), the blend
styles dither (every second pixel takes the blend colour, the highlight for
`CLR_DEFAULT`, the image alone at those pixels for `CLR_NONE`), `ImageList_Copy`
works inside one list with the flags 0 and 1 only.

## Not done

- Other code pages than 1252 (the `A` sheets name which results depend on it).
- `A` forms outside windows, dialogs, text, menus and resources (`GetObjectA`,
  `FindWindowA`, `SystemParametersInfoA`, ...).
- The painting of the common controls and every message beyond the sheets' list;
  `CreateToolbarEx`, `CreateStatusWindowW`, `PropertySheetW`.
- `SetCaretBlinkTime` values on a desktop beyond storing them.
