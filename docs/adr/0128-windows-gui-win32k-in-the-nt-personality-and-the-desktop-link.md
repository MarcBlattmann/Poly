# 0128: Windows GUI apps on Poly: win32k in the NT personality, Poly's own win32k system calls behind our `user32`, `gdi32` and `win32u`, GDI drawing in process, each top-level window one surface of the native compositor through the desktop link, Poly's frame as the non-client area, and the Phase 7 plan toward Chrome

**Status:** Accepted (architecture; behaviour of individual APIs follows the
spec sheets). Roadmap Phase 7, the groundwork of its first three items
("win32k services", "`user32`", "`gdi32`"; none ticked here). Builds on
[ADR 0063](0063-nt-processes-object-manager-and-memory.md) (Poly's own NT
service numbers, `table << 12 | index`, table 1 kept for win32k),
[ADR 0065](0065-nt-threads-synchronisation-and-apcs.md) and
[ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md) ("the
dispatcher decides, the caller calls out"), [ADR 0079](0079-our-kernelbase-and-kernel32.md)
(our Win32 modules: one workspace of real PE DLLs from Rust, cached by
content hash), [ADR 0080](0080-console-host.md) (the NT personality is the
console host; no `conhost`), [ADR 0087](0087-7-zip-command-line-on-poly.md)
(the first `user32` functions), [ADR 0105](0105-fonts-poly-font-font-store-and-fontconfig-discovery.md)
(`poly-font` and the font store), the desktop ADRs
[0106](0106-wayland-server-core-and-software-compositor.md) (one neutral
frame drawn by Poly for every app; "the same core can sit behind another
transport ... for the NT personality's win32k handing surfaces to the
compositor"), [0107](0107-desktop-fbdev-input-routing-and-shell.md)
(per-window menus; input routing), [0116](0116-compositor-on-kms-vsync-toolkit-protocols-and-backdrops.md),
[0118](0118-xwayland-and-polys-x11-window-manager.md) (X windows as framed
scene windows at positions their clients choose),
[0124](0124-native-desktop-alpine-shell-look-and-builds-on-windows-and-linux.md)
(the desktop is native user space; "later Windows apps through win32k ...
bridged to the native compositor"), [0125](0125-native-system-call-interface-and-poly-rt.md)
and [0126](0126-the-desktop-on-the-native-runtime.md) (the native runtime,
device requests, memory objects, `poll`).

## Context

Phase 7 wants Windows GUI apps, ending with Chrome for Windows running with
its sandbox. A spec agent is writing the first slice's spec sheets (window
classes and windows, the message loop, timers, painting with GDI) from
real Windows. This ADR fixes the architecture those slices are implemented
in, so that the implementers of each slice fill in behaviour rather than
decide structure. It needs no Windows observation: where behaviour matters
it says "the spec sheet decides".

Sources (clean room, implementer side): Microsoft Learn — "About Messages
and Message Queues", "Using Messages and Message Queues", the reference
pages of `GetMessage`, `PeekMessage`, `SendMessage`, `SendMessageTimeout`,
`ReplyMessage`, `MsgWaitForMultipleObjectsEx`, `SetTimer`,
`SetWindowsHookEx`, `LowLevelKeyboardProc`, `CreateWindowEx`,
`WM_NCCALCSIZE`, `WM_NCHITTEST`, `WM_SYSCOMMAND`, `WM_PAINT`,
`BeginPaint`, `GdiFlush`, `CreateDIBSection`, `GetStockObject`,
`UpdateLayeredWindow`, `BLENDFUNCTION`; "Window Stations and Desktops",
"Keyboard Input Overview" (scan codes, virtual-key codes, keyboard
layouts), "About Mouse Input" (capture, double clicks), "Desktop Window
Manager" (each top-level window drawn into an off-screen surface that is
composed), "High DPI Desktop Application Development on Windows",
"Interprocess Communication Between 32-bit and 64-bit Applications" (only
the lower 32 bits of a window handle are significant), `PROCESS_MITIGATION_SYSTEM_CALL_DISABLE_POLICY`
(`DisallowWin32kSystemCalls`: the process may not make GUI system calls),
`SetProcessMitigationPolicy`, `UpdateProcThreadAttribute`; the public
headers `winuser.h` (messages, `HT*` codes, `WHEEL_DELTA`, `IDC_*`),
`wingdi.h`, `winerror.h`, `ntstatus.h`. Export *names* from the coverage map
(`coverage/windows/api/{user32,gdi32,win32u}.dll.json`,
`coverage/windows/syscall/win32k.json`). For Chrome: the Chromium project's
public sandbox design document (`docs/design/sandbox.md`: broker and
target, restricted token, job object, alternate desktop, integrity levels,
the mitigation policies it sets including win32k lockdown, interceptions
forwarded to the broker over the sandbox IPC). No Windows binary, Wine,
ReactOS or Chromium source was read.

## Decision 1: win32k lives in the NT personality, in the kernel; window procedures and drawing live in the process

Three layers, as for the console host (ADR 0080):

| Layer | Where | What |
|---|---|---|
| win32k's logic | `crates/poly-win32k` (`no_std` + `alloc`, host-tested) | the service table (the ABI), window classes and atoms, the window tree, per-thread message queues, timers, hooks, input routing and keyboard state, window stations and desktops, the surfaces of top-level windows and **the desktop link** to the compositor (its records and both ends' bookkeeping), the mapping of Poly's frame onto the non-client area. No NT or kernel types: processes and threads are numbers. |
| win32k's services | `personalities/nt/src/win32k.rs` (host-tested with the personality's fake kernel) | table 1 of the NT dispatch: decoding arguments, copying user memory, handles to sections and events, blocking through `NtKernel`, the per-process win32k mitigation |
| The kernel's side | `kernel/src/nt/` (thin) | the `/dev/win32k` device the compositor opens (the link's transport), and memory objects shared between an NT section and a native handle |

**Why the kernel, not a user-space server.** On Windows the GUI system
calls are system calls: Learn's mitigation policy speaks of "GUI system
calls" and lets a process lose them. Poly keeps that boundary where
Windows defines it, as it did for the console (ADR 0080):

- **Queues are part of waiting.** `MsgWaitForMultipleObjectsEx` waits on
  kernel objects *and* the thread's queue at once; `GetMessage` blocks
  until input, a posted or sent message, a paint or a timer is due.
  Beside the dispatcher (ADR 0065) a queue is one more thing a wait
  watches, and a timer is one more deadline. In a server process every
  wait would need a shadow event per thread and every `PeekMessage` an IPC
  round trip.
- **Sent messages block across threads and processes.** `SendMessage` to
  another thread's window waits until that thread retrieves it (Learn:
  "messages sent between threads are processed only when the receiving
  thread executes message retrieval code") while the sender still
  receives messages sent to *it*. That is scheduling state, and the kernel
  already owns it.
- **Window handles are global** (one namespace per desktop, valid in every
  process), as are `FindWindow`, `EnumWindows`, focus, capture and the
  foreground window: machine-wide state, like the object manager.
- **Chrome's win32k lockdown** is a per-process switch on exactly this
  table (Decision 2).

The weighed alternative, a native user-space "window server" process
reached over Poly IPC (Wine's model), costs two context switches per
message call, a second copy of the wait machinery and a process whose
crash takes every Windows window with it; it buys fault isolation for
code that is host-tested `no_std` Rust anyway. ADR 0124's rule ("the
desktop is never inside the kernel") is about the compositor and shell:
they stay native user space. win32k is not the desktop; it is the NT
personality's GUI system-call layer, and it reaches the desktop through
the link (Decision 3) like any other world.

**What runs in the process.** Window procedures, `DefWindowProc`,
`DispatchMessage`, `TranslateMessage`, dialog and control logic, the
A/W translation of messages, menus' drawing, GDI drawing, fonts and
resources run in `user32.dll` / `gdi32.dll` in the app's process, as on
Windows: they call app code (window procedures, hook procedures, timer
procedures) and draw into the app's memory.

**No calls from the kernel into user mode: callouts.** Windows calls window
procedures from inside win32k (a user-mode callback); Poly does not
re-enter user mode from inside a system call (16 KiB kernel stacks, one
frame per call; ADR 0071's loader made the same choice). A service that
needs user code to run returns a **callout** instead: it fills a callout
record the caller passed (which window procedure or hook, the message and
its marshalled data) and returns Poly's status `STATUS_POLY_USER_CALLOUT`;
`user32` runs the code and calls `PolyUserCalloutReturn` with the result,
which continues the operation from the state win32k kept (the thread is
"inside `SendMessage`, waiting for a reply", "inside `GetMessage`", "in a
low-level hook call"). Same-thread messages never go through the kernel at
all: `SendMessage` to a window of the calling thread calls its procedure
"immediately as a subroutine" (Learn), and the messages `CreateWindowEx`
sends to the new window during creation are the creating thread's own.
Poly's own statuses set the customer bit (bit 29, `ntstatus.h`'s
`Customer code flag`) so they can never collide with Microsoft's; the
callout record's layout is Poly's and is defined with the first slice.

**Marshalling.** Messages sent to another process carry pointers. Learn:
the system marshals only system messages (below `WM_USER`). win32k keeps
a table, per system message, of what its `wParam`/`lParam` point to
(strings, `CREATESTRUCT`, `RECT`, `WINDOWPOS`, ...) and copies it into the
receiver's callout record and the results back; the table grows with the
spec sheets, each entry citing the message's Learn page. Private messages
cross processes unmarshalled, as on Windows.

**Per-thread state.** A thread gets a queue on its first USER call (Learn:
"the system creates a thread-specific message queue only when the thread
makes its first call to one of the User or GDI functions"); win32k keys
queues by `(process, thread)` and is told by the personality when a thread
or process ends (its windows are destroyed, its hooks and timers removed).
Window handles are Poly's own 32-bit values (index and generation), the
lower 32 bits being the significant ones (Learn). Window data the app
reads constantly (`GetWindowLongPtr`, rectangles, styles) is a system call
at first; a read-only per-desktop mapping into `user32`, the obvious speed-up,
comes when a measurement asks for it, with the same values.

**Window stations and desktops** exist from the start as objects of the
object manager (`\Windows\WindowStations\WinSta0`, desktop `Default`;
Learn's names): every window belongs to a desktop, messages and handles do
not cross desktops, and only `WinSta0\Default` is shown by the
compositor. Poly has one session (ADR 0079). Chrome's renderers run on an
alternate desktop (the sandbox document), which is then a desktop the link
never shows.

## Decision 2: Poly's own win32k system calls, in `win32u.dll`; apps call `user32` and `gdi32`

- **The ABI is Poly's.** Windows' win32k service numbers change with every
  build and its `NtUser*` / `NtGdi*` entry points are undocumented; apps
  call the documented `user32` / `gdi32` functions. Poly therefore owns
  table 1 as it owns table 0 (ADR 0063): `poly_win32k::services` lists
  every service with its number (`0x1000 | index`, table 1 in bits
  12–13), its name and its arguments. Each service is named after and
  takes the parameters of the documented function it serves (or, where a
  function family shares one service, an information class like
  `NtQueryInformation*`), in the documented order.
- **Names that cannot be mistaken for Windows'.** Poly's services are
  called `PolyUser*` and `PolyGdi*`, not `NtUser*` / `NtGdi*`: a Windows
  `win32u` export of the same name takes arguments nobody has documented,
  and a program calling it directly with Windows' arguments must fail to
  bind, not run Poly's service with misread arguments.
- **`win32u.dll`** holds the stubs: generated from the table at build time
  (as `ntdll`'s `Nt*` stubs are from `services.rs`), one exported
  `mov r10, rcx; mov eax, <number>; syscall; ret` per service. `user32`
  and `gdi32` import them by name (`raw-dylib`). `xtask` checks that
  `win32u` exports exactly the table and that every stub carries its
  number.
- **Direct `win32u` callers.** Learn documents no `NtUser*`/`NtGdi*`
  function for applications (the `D3DKMT*` functions are documented as
  `gdi32` exports and come with the graphics milestone). A program that
  imports a Windows `win32u` name fails to load with
  `STATUS_ENTRYPOINT_NOT_FOUND` (or `GetProcAddress` returns NULL), which
  the loader's log and the API tracer (ADR 0090) show. When a spec sheet
  documents such a call's behaviour (a spec agent may observe it), `win32u`
  gains an export under the Windows name as an adapter onto Poly's
  services, and that entry of `coverage/windows/syscall/win32k.json` is
  claimed; until then the Windows names stay unclaimed, honestly.
- **The win32k lockdown.** `NtProcess::win32k_disabled` (set by the
  security milestone from `ProcessSystemCallDisablePolicy` /
  `PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY`) makes every table-1 call of
  that process fail before dispatch. The status is provisional
  (`STATUS_ACCESS_DENIED`) until a spec sheet says what Windows returns.
- **Not implemented yet** is explicit everywhere: a known service without
  an implementation answers `STATUS_NOT_IMPLEMENTED` and is logged by the
  kernel as `syscall: pid N nt/win32k #n not implemented`; a `user32` /
  `gdi32` export that is only a stub sets `ERROR_CALL_NOT_IMPLEMENTED`
  (`winerror.h`) and returns its documented failure value. Both are found
  with `grep -rn not_implemented`.

## Decision 3: each top-level window is one surface of the native compositor, over the desktop link

**Surfaces.** As in Learn's description of the Desktop Window Manager,
every visible top-level window draws into an off-screen surface that the
compositor composes. Child windows (`WS_CHILD`) have no surface: they draw
into their top-level's, clipped by win32k's visible regions. A surface is
32-bit pixels, top-down, `B, G, R, A` in memory (a 32-bpp DIB's layout,
read as little-endian `0xAARRGGBB`), either `Xrgb8888` (the alpha byte is
ignored: ordinary windows, whose GDI drawing leaves it undefined) or
`Argb8888` premultiplied (layered windows with per-pixel alpha; Learn's
`BLENDFUNCTION`: the source must be premultiplied). Its memory is a
memory object of the kernel that is at the same time an NT section (the
app maps a view; `gdi32` draws there) and, for the compositor, a native
handle it maps with `map_handle` (ADR 0126). A resize gets a new buffer;
the old one goes when the compositor reports it released.

**The link.** The compositor is the one process that opens `/dev/win32k`
(a second open is `Busy`) and talks with win32k in records
(`poly_win32k::link`): little-endian, each `kind: u16, length: u16`, then
its fields, versioned by a `Hello`/`Welcome` exchange. It is Poly's own
protocol, not Wayland, for the reasons X11 needed its own window manager
(ADR 0118): Windows apps place their top-level windows themselves in
screen coordinates, own popups (menus, tooltips, drop-downs) at absolute
positions with no grab, keep owned windows above their owners and expect
`GetWindowRect` to be the truth — none of which is xdg-shell's model. A
device rather than a socket because the kernel holds both ends: win32k
never blocks on user space (records queue in the kernel), the compositor
adds one handle to its `poll` loop, Windows apps run headless when no
compositor is attached (host tests, boot tests without a desktop), and a
restarted compositor gets every surface (shown or hidden) replayed on its
`Hello`.
The device and the compositor's poll-loop hook come with the first slice
(there is nothing to show before `CreateWindowEx` works); the records and
both ends' bookkeeping are built and tested now.

| win32k → compositor | Meaning |
|---|---|
| `Welcome { version }` | answer to `Hello`, followed by the replay of every surface as it is |
| `Create { surface, kind, owner, framed }` | a top-level window got a surface; `kind` is `TopLevel` (in the window list, activatable) or `Popup` (menus, tooltips, drop-downs: unframed, above its owner, never in the window list); `owner` keeps it above another surface |
| `Geometry { surface, x, y, width, height }` | the surface's rectangle on the screen, logical pixels (the window rectangle minus Poly's frame) |
| `Title { surface, text }` | the window text, UTF-8 |
| `Visible { surface, shown }` | shown or hidden |
| `Buffer { surface, buffer, width, height, stride, format }` | a new buffer; the compositor takes its handle with a device request naming `buffer` |
| `Damage { surface, x, y, width, height }` | pixels changed (buffer coordinates) |
| `Activate { surface }` | the app asks to be the foreground window (`SetForegroundWindow`); the compositor's window management decides |
| `Cursor { surface, shape }` | the cursor over the surface: a standard shape (`IDC_*` of `winuser.h`) or a cursor image buffer |
| `BeginMove { surface }`, `BeginResize { surface, edges }` | `DefWindowProc` started an interactive move or resize (`WM_SYSCOMMAND` `SC_MOVE` / `SC_SIZE`); the compositor runs the drag as it does for Wayland windows |
| `State { surface, state }` | the app maximized, minimized or restored itself |
| `Destroy { surface }` | the surface is gone |

| compositor → win32k | Meaning |
|---|---|
| `Hello { version, output, scale, frame }` | the output's logical size and scale, and Poly's frame metrics |
| `Output { width, height, scale }` | the output changed |
| `Pointer { surface, x, y, time }` | pointer position relative to the surface (negative or beyond: over the frame) |
| `Button { surface, button, pressed, time }` | evdev button codes |
| `Axis { surface, vertical, horizontal, time }` | wheel, in `WHEEL_DELTA` (120) units |
| `Leave { surface }` | the pointer left the window |
| `Key { code, pressed, time }` | evdev key code, to the focused surface |
| `Focus { surface }` | the compositor made this surface the active one (0: none of win32k's) |
| `Configure { surface, x, y, width, height }` | the compositor moved or resized it (a drag, the shell's snapping or maximizing) |
| `Request { surface, request }` | the shell asks to close, minimize, maximize or restore (its window list, shortcuts) |
| `Layout { name }` | the keyboard layout of the shell's settings |
| `Released { buffer }` | the compositor no longer reads that buffer |

On the compositor's side (`userland/poly-compositor/src/win32.rs`) a
surface is a scene window (`Scene::add_window`), framed when `framed`,
placed at `Geometry`, its content copied from the buffer at `Damage`
(ADR 0106: a committed buffer is copied, so the scene never reads app
memory while composing), popups unframed above their owner.

## Decision 4: Poly's frame is the non-client area

The owner's decision (ADR 0106): one neutral frame drawn by Poly for every
app. For Windows apps the frame **is** the window's non-client caption and
border, so Windows' geometry stays true:

- A top-level window with a caption or a sizing border (`WS_CAPTION`,
  `WS_THICKFRAME`) gets Poly's frame: its window rectangle (`GetWindowRect`)
  includes the frame, `AdjustWindowRectEx` and the frame system metrics
  report Poly's sizes (the compositor's `TITLE_HEIGHT` and `BORDER`, sent
  in `Hello`, `poly_win32k::frame::FrameMetrics`), and the surface is the
  window rectangle minus the frame. The compositor draws the frame around
  the surface exactly where Windows would put the non-client caption —
  its existing frame code (`FrameLayout::around`) with the same numbers.
- The rest of the non-client area stays the app's: the menu bar (per-window
  menus, ADR 0107), scroll bars, client edges are drawn by `user32`
  (`WM_NCPAINT`, `DefWindowProc`) into the surface, so `WM_NCCALCSIZE`
  still divides the inner rectangle between them and the client area.
- An app's own `WM_NCCALCSIZE` cannot remove Poly's frame: win32k offers it
  the rectangle inside the frame, and a larger answer is clamped to it.
  Apps that draw their own caption (Chrome's tab strip, Electron's
  frameless windows) therefore show it below Poly's title bar — the same
  visible, not broken, price ADR 0106 accepted for GTK 4.
- **Frame input goes through Windows' path.** Pointer events over the
  frame reach win32k like any other (`Pointer` with coordinates outside the
  surface), so `WM_NCHITTEST` is sent; `DefWindowProc` answers from Poly's
  frame geometry (`HTCAPTION`, `HTCLOSE`, `HTMINBUTTON`, `HTMAXBUTTON`, the
  borders and corners: `poly_win32k::frame::hit_test`, the codes of
  `winuser.h`, their meanings from Learn's `WM_NCHITTEST`), then
  `WM_NCLBUTTONDOWN` and `WM_SYSCOMMAND` follow. An app that answers
  `HTCAPTION` for part of its client area becomes draggable, one that
  handles `SC_CLOSE` or `WM_CLOSE` can ask "save changes?", and moves and
  resizes run in the compositor (`BeginMove`/`BeginResize`, then
  `Configure`). The shell's own window management (its window list,
  Alt+Tab, Super+arrows, snapping) reaches the app as `Request` records,
  which win32k turns into the `WM_SYSCOMMAND`s Windows would send. Hover
  and pressed looks of the frame's buttons are the compositor's (it knows
  the pointer); their actions are win32k's.
- Windows without caption and border (`WS_POPUP` alone, layered windows,
  menus, tooltips) get no frame (`framed: false`).

## Decision 5: GDI draws in the process

- **Pixels are made in the app's process**, by `gdi32`'s software
  rasteriser (a host-tested crate of the GDI slice: lines, fills, regions,
  ROPs, `BitBlt`/`StretchBlt`/`AlphaBlend`, DIB conversions) into memory
  the process maps: memory DCs into DIB sections and compatible bitmaps,
  window DCs into the window's surface. No pixel goes through a system
  call. Only the window DC's setup is win32k's: `GetDC`/`BeginPaint`
  return the surface's section (mapped once per process), the window's
  origin in it and its visible region (children and siblings clipped as
  the window's styles say).
- **Presenting.** Learn documents GDI batching (`GdiFlush` "flushes the
  calling thread's current batch"). Poly's batch is the damage: `gdi32`
  collects the rectangles drawn through window DCs and commits them
  (`PolyUserCommitSurface`) at `EndPaint`, `ReleaseDC`, `GdiFlush`, and
  before the thread blocks in a message wait; win32k turns them into
  `Damage` records. Drawing outside those points is shown at the next
  one.
- **Objects.** DCs, bitmaps, brushes, pens, fonts, regions and palettes are
  per-process objects in `gdi32`, in a table with handle values of Poly's
  own encoding carrying the object type (`GetObjectType`'s `OBJ_*`).
  Stock objects (`GetStockObject`) have the same values in every process.
  A DIB section created over a section handle (`CreateDIBSection`'s
  `hSection`) is how pixels are shared between processes, as documented.
- **Fonts.** Logical fonts (`LOGFONTW`) are mapped in `gdi32` onto Poly's
  font store (ADR 0105: the fonts Poly ships, `C:\Windows\Fonts`, fonts
  added with `AddFontResourceEx`) through a substitution table of
  Poly's (Windows' UI faces such as "Segoe UI", "Tahoma", "MS Shell Dlg",
  "Arial" onto shipped free fonts, metric-compatible where one exists);
  outlines are rasterised by `poly-font` in the process (grey-scale
  antialiasing first). The spec sheets decide metrics and mapping
  details.
- **Later graphics** use the same surfaces: DirectWrite (`dwrite.dll`)
  sits on the same font store and `poly-font` with shaping; a DXGI swap
  chain's buffers become surface buffers of the link (a new buffer kind:
  dma-bufs of the virgl/venus device, which the compositor already takes
  from Wayland clients, ADR 0116/0119); Direct3D 11 on Vulkan and
  DirectComposition come with the graphics milestone. The software path is
  first: Chrome's software compositing ends in a DIB section blitted to a
  window DC or an `UpdateLayeredWindow`, which is plain GDI.

## Decision 6: input from the compositor to the right thread's queue

- **Keyboard.** The compositor sends evdev key codes to the surface it
  focused. win32k turns them into Windows scan codes (set 1, Learn's
  "Keyboard Input Overview" scan code table) and virtual-key codes through
  the keyboard layout the shell's settings name (`Layout`; the same layouts
  `poly-input` serves to Wayland clients, so a German layout is German in
  both worlds), keeps the key state (`GetKeyState` per queue as of the
  last message retrieved, `GetAsyncKeyState` global), and posts
  `WM_KEYDOWN`/`WM_KEYUP`/`WM_SYSKEYDOWN`/`WM_SYSKEYUP` with the documented
  `lParam` bits to the focus window's thread. `TranslateMessage` in
  `user32` makes `WM_CHAR`/`WM_DEADCHAR` with the same layout tables (dead
  keys). Low-level keyboard hooks see the key before it is queued.
- **Pointer.** win32k converts surface coordinates to screen coordinates,
  finds the target window by walking the window tree (with `WM_NCHITTEST`
  callouts where a window must say, and `HTTRANSPARENT`), honours capture
  (`SetCapture`; the compositor's implicit grab while a button is held
  keeps motion coming outside the window, which is what Learn's capture
  rules need), synthesises double clicks for `CS_DBLCLKS` classes, sends
  the wheel to the focus window (Learn), sends `WM_SETCURSOR` and forwards
  the resulting cursor as a `Cursor` record, and reports
  `WM_MOUSELEAVE` for `TrackMouseEvent`.
- **Focus and activation.** The compositor's window management decides
  which window is active across all worlds; its `Focus` record makes win32k
  run Windows' activation sequence (the spec sheet's order of
  `WM_ACTIVATEAPP`, `WM_NCACTIVATE`, `WM_ACTIVATE`, `WM_SETFOCUS`). An app's
  `SetForegroundWindow` is an `Activate` request the compositor may grant
  (then `Focus` follows) or refuse; `SetFocus` and `SetActiveWindow` within
  the already active app are win32k's alone.
- **Hooks.** `SetWindowsHookEx` registrations live in win32k. Hooks that
  run in the installing thread (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`; Learn: the
  hook is called in the context of the thread that installed it) are
  callouts to that thread, with the documented timeout. Global hooks that
  need a DLL in other processes are loaded by `user32` in the target
  process when a callout names them (a later slice).
- **Timers** are win32k deadlines per queue: `WM_TIMER` is generated, not
  queued (Learn: low priority, after sent, posted, input and paint
  messages), and the next deadline bounds the thread's message wait.

## Decision 7: the Phase 7 plan toward Chrome

The roadmap's items stay; this is the order of slices inside them and what
each needs from the spec agents. Each slice: spec sheets and visible tests
from the oracle (tests in `tests/windows/`, ADR 0075), then implementation,
then the next.

| # | Slice (roadmap item) | Contents | Proof |
|---|---|---|---|
| A | win32k + `user32` + `gdi32` basics (spec in progress) | classes and atoms, windows (create, destroy, show, position, styles, parent/owner, text, window longs), the message loop (post, send in and across threads, get/peek/dispatch, quit, `MsgWaitForMultipleObjectsEx`), timers, painting (update regions, `WM_PAINT`/`WM_ERASEBKGND`, `BeginPaint`/`EndPaint`, `GetDC`), GDI basics (DCs, stock objects, solid brushes and pens, fills, lines, `BitBlt`, DIB sections, compatible bitmaps); the `/dev/win32k` device and the compositor's hook | a Windows program's window framed on the Poly desktop, compared pixel for pixel in its client area with the oracle's |
| B | input (win32k) | keyboard (scan codes, VKs, layouts, `WM_KEY*`/`WM_CHAR`, dead keys, key state), mouse (messages, capture, double clicks, wheel, cursors), focus and activation, the non-client path (`WM_NCHITTEST`, `WM_NCCALCSIZE`, `WM_SYSCOMMAND`, moves and resizes), hooks, `SendInput` | input tests driven through QMP on Poly and `SendInput` on the oracle |
| C | `user32` breadth, GDI text | resources (`LoadIcon`/`Cursor`/`Image`/`String`, dialog templates), dialogs and dialog units, menus (bars in the surface, popups as popup windows, accelerators), `MessageBox`, the clipboard (per window station), DPI awareness; GDI fonts and text (`CreateFontIndirect`, mapping, `TextOut`/`ExtTextOut`/`DrawText`, metrics, enumeration), regions and clipping in full | Notepad-like test programs |
| D | native look | `uxtheme`, `comctl32` v6 (needs activation contexts and manifests), common dialogs' basics | **Milestone:** Notepad++ and the 7-Zip GUI |
| E | COM and shell | `combase`/`ole32` (apartments, registration, marshalling, RPC), `oleaut32`, `shell32`/`shlwapi` (known folders, file dialogs) | |
| F | security model (Chrome sandbox) | see below | Chrome's sandbox tests' scenarios (`chrome://sandbox`) |
| G | text and graphics for browsers | DirectWrite (font collections, faces, glyph runs and their rasterisation, text analysis), then DXGI / Direct3D 11 on Vulkan, Direct2D, DirectComposition | |
| H | audio, services | WASAPI/`winmm`; service control manager, event log | **Milestones:** Paint.NET and VS Code; **Chrome with its sandbox** |

**What Chrome needs**, as far as public documents say (the sandbox design
document; Learn) — the spec agents establish the exact list by tracing
Chrome on the oracle with the API tracer (ADR 0090) and may read
Chromium's BSD-licensed source for its requirements (CLEAN_ROOM.md: a spec
agent may; an implementer may not):

- **Process model:** a broker (the browser) and targets (renderers, GPU,
  utility processes), all `chrome.exe`; children created with
  `STARTUPINFOEX` attribute lists (`UpdateProcThreadAttribute`: handle
  lists, mitigation policies, the child-process policy), suspended while
  the broker prepares them.
- **The sandbox's four mechanisms:** restricted tokens (deny-only and
  restricting SIDs, no privileges, the Untrusted integrity label;
  `SetTokenLevel(initial, lockdown)` with `LowerToken()`: the target starts
  impersonating and drops to the lockdown token), job objects (one active
  process; UI limits on desktops, clipboard, broadcasts, global hooks and
  global atoms — which win32k must enforce), an alternate desktop
  (Decision 1), integrity levels (Untrusted renderers, Low GPU process).
- **Mitigation policies** through `SetProcessMitigationPolicy` and the
  attribute list: ASLR variants, heap terminate, strict handle checks,
  **win32k lockdown** (`DisallowWin32kSystemCalls` for renderers:
  Decision 2), extension points, CFG, font loading, signature policy,
  image load policy, dynamic code. Poly records and enforces them; those
  without meaning on Poly are accepted and reported as set.
- **Interceptions:** the broker patches API calls in the target so they are
  forwarded to it over the sandbox's IPC. What exactly is patched, and what
  shape our `ntdll`'s stubs must have for the patching to succeed, is for
  the spec agent to establish; Poly owns `ntdll`'s stubs and can give them
  that shape.
- **IPC:** named pipes with overlapped I/O and I/O completion ports, shared
  memory sections duplicated into children (`DuplicateHandle` across
  processes), cross-process memory (`VirtualAllocEx`, `WriteProcessMemory`).
- **Optional for the first run:** AppContainer / LPAC (capability SIDs),
  the GPU process (Chrome falls back to software compositing when Direct3D
  11 is unavailable; Poly reports no hardware adapter until the graphics
  slice), Crashpad.
- **Elsewhere:** the browser window is a custom-caption window
  (Decision 4), its menus and bubbles layered windows with per-pixel alpha
  (`Argb8888` surfaces), text through DirectWrite (renderers get fonts from
  the browser), networking on Winsock (ADR 0095) and certificate
  verification through `crypt32` (ADR 0096).

## To be confirmed by the spec agents against real Windows

1. What a table-1 call of a process under `DisallowWin32kSystemCalls`
   returns (ours: `STATUS_ACCESS_DENIED`, provisional), and how `user32`
   functions behave in such a process.
2. Which system messages are marshalled across processes, and how
   (the marshalling table grows from the spec sheets).
3. The order of messages during creation, activation, showing, sizing
   and destruction; which are sent and which posted.
4. The frame system metrics apps read (`SM_CYCAPTION`, `SM_CXSIZEFRAME`,
   ...) are Poly's frame's on Poly by design; the spec sheets record
   Windows' values only as information, not as expected results.
5. Whether `win32u.dll` exports are called directly by the apps of the
   milestones (Chrome, Electron, .NET), from traces.

## Consequences

- New crate `crates/poly-win32k` (`no_std` + `alloc`, host-tested): the
  service table, the desktop link's records and win32k's end of it, the
  frame metrics and the non-client hit test, and the export lists the build
  checks. Classes, windows, queues, timers, hooks and input join it with
  their slices (the compositor's end of the link is in the compositor).
- `personalities/nt/src/win32k.rs`: table 1 of the dispatch with the
  win32k-lockdown check; every service answers `STATUS_NOT_IMPLEMENTED`
  (logged as unimplemented) until its slice. `NtProcess` gains
  `win32k_disabled`, `NtSystem` the win32k state (the link's surfaces).
- The Win32 workspace (ADR 0079) gains `win32u` (stubs generated from the
  table) and `gdi32`; `user32` gains the first slice's exports as stubs.
  `cargo xtask win32` builds and verifies them, the kernel embeds them and
  the layout installs them into `C:\Windows\System32` next to the others;
  `win32u.dll` joins the known DLLs.
- `userland/poly-compositor/src/win32.rs`: the compositor's end of the link
  (records into scene windows), host-tested, hooked into the poll loop with
  the device in slice A.
- The service list is the first slice's starting point, not a promise:
  until a service ships with an implementation its number may still be
  reshaped; after that, numbers are never reused (ADR 0063).
