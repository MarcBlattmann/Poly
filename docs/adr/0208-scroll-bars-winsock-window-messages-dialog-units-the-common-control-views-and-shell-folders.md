# 0208: Scroll bars, Winsock window messages, dialog units, the common control views and the shell folders

**Status:** Accepted. Windows applications, batch 5 (track `impl-win-apps-5`): what
PuTTY and 7-Zip File Manager stop on after ADR 0201. Inputs: the spec sheets and
visible tests of `spec-win-apps-5` (`specs/windows/{user32,ws2_32,gdi32,shell32,comctl32}.dll/`),
Microsoft Learn's reference pages and `winuser.h`, `commctrl.h`, `shlobj_core.h`,
`shellapi.h`, `winsock2.h` for numbers and layouts. No third-party source.

## Decisions

### 1. A scroll bar is a model plus a style bit; the frame change is a plain `SetWindowPos`

`poly_win32k::scroll::Bar` holds one bar (range, page, position, disabled arrows) and
the rules of the sheets as host-tested code: the position stays in
`[min, max - max(page - 1, 0)]`, a page larger than the range is stored as the range,
`min > max` becomes `min..min`, a bar with nothing to scroll (`max(page, 1) >= max - min + 1`)
goes unless `SIF_DISABLENOSCROLL` keeps it with both arrows disabled, arrows accumulate
(`EnableScrollBar(.., 0)` enables both). The state of a window's two bars and of a
`ScrollBar` control lives in `user32` (`scroll.rs`), in a table keyed by the window
handle that goes with `WM_NCDESTROY` (the hub `Env::call_proc` drops it, whoever handles
the message). A bar exists when the window was made with the style or a call gave it one;
`GetScrollInfo` and `SetScrollPos` fail with 1447 for a window that has none.

A bar appearing or going changes the window's style (`Request::SetStyle`, which sends no
`WM_STYLECHANGING`) and then calls `SetWindowPos(SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE
| SWP_NOZORDER | SWP_NOACTIVATE)`: `WM_WINDOWPOSCHANGING`, `WM_NCCALCSIZE`,
`WM_WINDOWPOSCHANGED`, `WM_SIZE` come out in the order the sheets observed, and
`metrics::client_insets` already took the bar's width off. `SB_BOTH` acts on both bars for
`SetScrollInfo` and `SetScrollPos`, on the vertical one only for `SetScrollRange`.
The `SBM_*` messages are answered by the control's window procedure on the same model.
Not done: nothing paints the non-client scroll bars yet, and no mouse or key reaches them.

### 2. `WSAAsyncSelect` rides on the `WSAEventSelect` machinery

`poly_winsock::events::Registration` gets a window and a message number
(`post_to`); the helper thread that polls selected sockets (ADR 0155) feeds the same state
machine, and `signal` queues one window message per recorded event instead of setting an
event object: wParam the socket, the low word of lParam the event, the high word its
error, posted in the order accept, connect, read, write, out-of-band, close (a refused
connection posts `FD_CONNECT` before the data events). The queue is flushed after the
table lock is released, so `PostMessageW` is never called with it held. A window message is
one event: data arriving while earlier data is unread does not make another `FD_READ`
(the event-object mode keeps recording it). `accept` hands the listener's registration on
to the socket it made and makes it non-blocking. `ws2_32` does not import `user32`: it looks
up `PostMessageW` and `IsWindow` with `LoadLibraryW`/`GetProcAddress` at the first call, so
programs that only use sockets do not load the GUI. Events above `0x3FF` are `WSAEINVAL`;
`WSAAsyncSelect(s, hwnd, 0, 0)` cancels. Ordinal 101 is pinned to the name.

### 3. Dialog units: base units from the dialog's font, not a table

`MapDialogRect` and `GetDialogBaseUnits` use the width of "a..zA..Z" in the dialog's font
(`(W / 26 + 1) / 2`) and its height (the sheet's `dialog_units`), already in the dialog
manager; this batch fixed the errors (`ERROR_WINDOW_NOT_DIALOG` 1420 for a child, 1400
for any other window) and made the system font measure what the oracle's stock font
measures: seven letter advances of Poly's bitmap system font grew by a pixel (`z J L N P S V`,
none of them measured by an earlier sheet) so that the 52 letters add up to 429.
That is an approximation fitted to one observation; the glyph rows are unchanged.

### 4. The list view, tree view, header, toolbar and status bar are modules of their own

`comctl32` keeps one window procedure per class family, but these five classes no longer
live in the models-under-one-lock table of ADR 0201: each has a module (`listview.rs`,
`treeview.rs`, `header.rs`, `toolbar.rs`, `statusbar.rs`) and a table of its own state,
and `controls.rs` only routes (`Model::Ext`). **No state table is locked while a message is
sent**: a handler works on the state in short steps and sends its notifications between
them (a program answers `LVN_GETDISPINFOW` with `LVM_GETITEM`, or `TVN_SELCHANGING` with
`TVM_GETNEXTITEM`). Pure rules go to `poly_comctl` (the toolbar's button layout); the
rest is observed behaviour kept next to the glue. Structures are `#[repr(C)]` with the
native pointer size, so the 32-bit build has `commctrl.h`'s layouts too.

Rules the sheets fixed that are easy to get wrong: list view notifications carry
the bits the change is about (`new & mask`, `old & mask`), `LVN_ITEMCHANGING` comes before
the changes and `LVN_ITEMCHANGED` after all of them in reverse order, an owner data list
is told (`LVN_ITEMCHANGED` only) and asks for the indent afterwards, one `LVN_GETDISPINFOW`
asks for text and image together, a destroyed list sends `LVN_DELETEALLITEMS` and then
`LVN_DELETEITEM` for each item; the tree view tells its parent when an item is expanded
(by `TVM_EXPAND(TVE_EXPAND)`, the keys, the mouse, a selection below it), not when it is
collapsed or toggled with the message, a collapse with `TVE_COLLAPSERESET` removes the
children and leaves the item asking for them, and `I_CHILDRENCALLBACK` is answered by the
parent; the toolbar's default bitmap is 16 x 16 with a 7 x 6 padding, a flat toolbar has
no margin and no divider, a negative insert index appends; the status bar is 22 high,
aligns to the parent's bottom (top with `CCS_TOP`) on `WM_SIZE` whatever the size sent, and
draws sunken parts in the colours of `COLOR_BTNSHADOW`/`COLOR_BTNHIGHLIGHT`. The report
view's header is a real `SysHeader32` child; a click on a heading comes back as
`LVN_COLUMNCLICK`. Controls paint in `WM_PAINT` and `WM_PRINT`/`WM_PRINTCLIENT` with the
system colours and the GUI font. Not done: label editing, drag and drop, tooltips, column
dragging, groups, custom draw of sub items, the scroll bars of the views.

One line of `lv_report` cannot match: the width of the item text in the oracle's GUI
font is what `LVIR_SELECTBOUNDS` reports, and Poly's GUI font measures it differently.

### 5. Shell folders and item lists are Poly's own layout behind Learn's shapes

`poly_kernelbase::csidl` holds the table of the `CSIDL` values (environment variable,
suffix and the two spellings the sheets saw for `System32` and `Resources`), the errors
(`E_INVALIDARG` for a value that is no folder, `E_FAIL` with last error 0 for the virtual
folders, `E_HANDLE` for a made-up token, `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)` unless
`CSIDL_FLAG_DONT_VERIFY`) and the **item identifier list**: items with a 2-byte size,
terminated by a size of 0; the empty list is the desktop; a virtual folder is one item with
its CLSID; a path is a drive item and one item per name carrying a serial number from a
counter (so two lists for the same path differ, as the real ones that hold times do, and a
`ILClone` is equal). `ILCreateFromPathW` walks the path with `FindFirstFileW` to take the file
system's spelling and returns NULL for a missing file or a path with forward slashes.
The layout of ADR 0073 now makes the profile folders these calls verify. `SHGetFileInfoW`
(`poly_kernelbase::fileinfo`): type names by extension, display names without the extensions
the shell hides, the `SFGAO` attribute values, `SHGFI_EXETYPE` from the file's first bytes,
and a system image list per size (16 and 32) made at the first call from icons Poly draws
itself (`poly_kernelbase::shellicon`: generic file, folder, application, library, drive,
text, archive; no Windows icon is reproduced).

### 6. `GetMonitorInfoA`

PuTTY clamps its window to the monitor's work area and, with the function a stub that
returned 0 and left the structure as it was, placed the window at a garbage position (off
screen). `GetMonitorInfoA` is `GetMonitorInfoW` with the ANSI device name
(`MONITORINFO` 40 bytes or `MONITORINFOEXA` 72 bytes).

## Consequences

PuTTY connects to a TCP listener and shows its terminal; 7-Zip File Manager shows its
toolbar, report view with header and icons, and status bar. The next stoppers are
non-client scroll bar painting and input, `GetObjectA`/`TranslateCharsetInfo`/
`GetLocaleInfoA`/`SetClassLongPtrA`, `MakeDragList`/`LBItemFromPt`/`DrawInsert`, the drive
display names in 7-Zip's root list, and `LsaAddAccountRights`.
