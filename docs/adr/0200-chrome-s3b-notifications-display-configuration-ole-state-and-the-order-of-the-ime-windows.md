# 0200: Chrome S3b: notification registrations, display configuration, OLE state and the order of the IME windows

**Status:** Accepted. Implements the spec round `spec-chrome-win-s3b` (35 entries, `specs/windows/apps/chrome/gap-map.md`
section 14) for [0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md)'s stage S3, on top of
[0190](0190-chrome-s3-windows-hooks-winevents-and-the-text-services-window.md). Inputs: the spec sheets and visible tests,
Microsoft Learn (display configuration, device and power notifications, OLE drag and drop, COM), `wingdi.h`, `winuser.h`,
`dbt.h`, `objidl.h`, `wtypes.h`. No Chromium, Wine or ReactOS source.

## Decisions

1. **Notification registrations are one table per process** (`poly_win32k::notify`). `RegisterDeviceNotificationW`,
   `RegisterPowerSettingNotification` and `RegisterSuspendResumeNotification` give a handle of their own for every call, never a
   number twice; each release function accepts only its own kind and answers another kind's, a released and a NULL handle with
   its own error (6 for devices, 87 for the other two; a pointer that was never a handle is 1066 for devices). The recipient is
   not checked. Nothing arrives by itself on this machine except the one initial `WM_POWERBROADCAST`
   (`PBT_POWERSETTINGCHANGE`) of a power setting that has a value now (display on, user present, AC power): it waits in the table
   and is sent when the window's thread next looks into its queue (`Env::pump_events`); a release cancels it.
   `RegisterPointerDeviceNotifications` is once per window (a window that died is forgotten, so a new window with its handle
   value registers again); `GetPointerDevices` finds no device.
2. **The display configuration describes the desktop's monitor as one path** (`poly_win32k::dispcfg`): one source (id 0,
   `\\.\DISPLAY1`) joined to one target of a generic monitor without EDID, one adapter, two modes per path (target, source),
   three with `QDC_VIRTUAL_MODE_AWARE` (the desktop image mode; the packed index words). The refresh rate is 60 Hz and the same in the
   path and in the target mode. `GetDisplayConfigBufferSizes`, `QueryDisplayConfig` and `DisplayConfigGetDeviceInfo` return the
   error code and do not set the last error (the two exceptions of the sheets are kept).
3. **OLE is a state of its own on top of the apartment** (`poly_com::apartment`). A thread's word keeps the model, the
   initialisation count, whether it is the main STA, the number of `OleInitialize` calls and whether OLE was ever used. The first
   STA of the process, while it lives, is the main STA (`APTTYPE_MAINSTA`, 3); `ole32` keeps its thread id and finds it dead when
   another thread asks. A thread without COM in a process that has an MTA thread is in the MTA implicitly (`CoGetApartmentType`:
   MTA, qualifier 1). `CoRegisterMessageFilter` keeps one AddRef'ed filter per STA thread (the MTA is `CO_E_NOT_SUPPORTED`), which
   goes with the apartment. `RegisterDragDrop` needs OLE on the thread, AddRef's the target and leaves the window property
   `OleDropTargetInterface` holding it; `DestroyWindow` does not revoke; `RevokeDragDrop` and the last `OleUninitialize` of the
   registering thread release it once. `ole32` finds `user32` when it first needs it (no GUI DLL is loaded by console programs).
4. **`CoCreateFreeThreadedMarshaler` makes the inner unknown of an object that is `IMarshal` too.** The `IMarshal` interface is a
   field of the same block; aggregated, its `QueryInterface`, `AddRef` and `Release` go to the outer unknown, otherwise they count
   on the object.
5. **The IME windows are listed last by `EnumWindows` and `EnumThreadWindows`**: every window of the classes `IME` and
   `MSCTFIME UI` after all the others, each group in z order (`poly_win32k::ime::enumeration_order`). The z order walk
   keeps them next to the active window. Once the last shown window of the thread is destroyed the default IME window has no owner,
   and the text services' window lies below it (`ime`, `ctf`, holder), where it lies above its owner as long as there is one.
6. **Smaller rules from the sheets**: raising an owned window raises the family of its root owner (activating the owned popup
   `C` of `A` puts `C A` above a hidden `B`, not `C B A`); `FindWindowExW` does not find the desktop and the stand-ins of other
   processes; a hidden child under a hidden parent gets `WM_SHOWWINDOW(0)` and no position messages when destroyed;
   `EnumThreadWindows` lists no message-only window, returns FALSE when the callback stops it, skips a window destroyed by an earlier
   callback (last error 1400) and says 87 for a thread id that names no thread; `MsgWaitForMultipleObjectsEx` takes at most 63
   handles, refuses unknown flags, honours `MWMO_ALERTABLE`, returns the abandoned and I/O completion codes and leaves the last
   error alone; `GetWindowDisplayAffinity` per window (0, 1, 0x11, none for a child); `PropVariantClear` of an undefined type is
   `STG_E_INVALIDPARAMETER` (and clears), `PropVariantCopy` is a deep copy.

## Not done

- `GetSystemMenu` needs the menu objects (`AppendMenuW`, `EnableMenuItem`, `IsMenu`, ...); they are the work of the
  `impl-win-apps-gui` track (`user32/src/menu.rs`), so the export here stays as it was.
- `GetWindowDC` draws into the window's client-sized surface only; pixels of the non-client area (`gwdc_draw`) need a backing
  for the whole window.
- Real pen and touch devices, device arrival and suspend messages, `DoDragDrop`, `IsWindowArranged` TRUE (window snapping).
