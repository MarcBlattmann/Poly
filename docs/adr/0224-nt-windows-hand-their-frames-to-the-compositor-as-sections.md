# 0224: NT windows hand their frames to the compositor as sections

**Status:** Accepted. Track `chrome-paint-2`. Builds on ADR 0128 (each top-level window one
surface of the compositor over the desktop link; "a DXGI swap chain's buffers become surface
buffers of the link"), ADR 0211 (the software Direct3D 11 device, whose swap chains presented
through GDI) and ADR 0219 (commits carry only what a process drew; Windows apps in output pixels
at 200 %). Inputs: Microsoft Learn ("DXGI flip model", `IDXGISwapChain1::Present1`,
`DXGI_PRESENT_PARAMETERS`, `DXGI_SWAP_EFFECT`, `CreateFileMappingW`, `MapViewOfFile`); the
public SDK headers (`dxgi1_2.h`, `dxgiformat.h`, `winnt.h`) for constants and layouts; Poly's own
traces of Chrome for Windows running on Poly. No third-party source.

## What was there

Chrome for Windows at 2560x1600 and 200 % (WHPX) showed whole-window changes at about 20–30
frames a second. Measured on Poly (compositor `--stats`, a temporary counter in `d3d11`):

- Chrome's GPU process presents a **flip-model swap chain** (`DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL`,
  two buffers, `B8G8R8A8_UNORM`, 2074x1499) with **`Present1` and one dirty rectangle** each
  frame. While typing, the rectangle is a few hundred thousand pixels; the window is 3.1 million.
- Poly ignored the rectangle. Every present copied the **whole** frame about six times:
  `SetDIBitsToDevice` into the child window's bitmap in the GPU process; the commit read it from
  user memory and wrote it into win32k's buffer; the compositor's `READ_BUFFER` copied rows into a
  kernel vector and then into its own memory; a per-pixel loop put them into the scene. Then the
  compositor composed and flipped the whole window: 76 % of the screen was damaged on every
  keystroke and every caret blink.
- Per compositor frame: about 6–9 ms taking the pixels, 6 ms composing, 8–9 ms for the KMS
  flip; per present 3–6 ms in the GPU process (peaks of 20–90 ms).

A Wayland client (Linux Chromium) hands the compositor shared memory or a dma-buf, and the
compositor copies only the damaged rectangles of it into the scene: one copy.

## Decisions

1. **A swap chain's buffers are sections.** A flip-model swap chain (`FLIP_SEQUENTIAL`,
   `FLIP_DISCARD`) of `B8G8R8A8` pixels on a window, or bound to one by a DirectComposition
   commit, makes two pagefile sections of its back buffer's size (`CreateFileMappingW`,
   `MapViewOfFile`) on its first present. The back buffer texture's bytes are a view of one of
   them; the device draws there. Other swap chains (blt model, other formats) keep presenting
   through GDI.
2. **`Present` hands the section over.** The new win32k service
   **`PolyUserPresentBuffer(hwnd, section, width, height, stride, rects, count)`** (`0x1003`,
   WOW64 `[V, H, V, V, V, P, V]`) says that the frame in `section` is `hwnd`'s picture from the
   calling process now, and which rectangles changed (`Present1`'s dirty rectangles plus the
   scrolled rectangle; none, or more than 64, for the whole buffer). Section 0 removes it. No
   pixel is copied by the app or by win32k.
3. **The back buffer keeps the last frame.** After a present the texture switches to the other
   section, which first gets the presented rectangles of the frame (rows copied in the process).
   So the back buffer always holds the frame last presented, as with one buffer, whatever the
   app assumes about the flip model's buffer contents; the copy is the dirty rectangles only.
   The compositor reads one section while the app draws into the other.
4. **Layers on the desktop link.** win32k keeps, for each (window, process), a *layer* of the
   surface the window draws into: its top-level window's surface, at the window's client area,
   clipped to it and to the surface (`poly_win32k::layers`). Each section presented becomes a
   buffer of the layer (at most four; a resized swap chain's old buffers go); win32k holds a
   reference on the section object while the layer has it, and drops it when the buffer goes, the
   layer is removed or the process ends. New records tell the compositor: `LayerBuffer`,
   `LayerPlace` (where and clipped to what, sent when it changes), `LayerDamage` (the buffer shown
   now and its changed rectangle), `LayerDrop`, `LayerGone`. A reconnecting compositor gets
   every layer replayed with its buffers, place and a whole-buffer damage.
5. **The compositor maps a layer buffer from the device.** `map_handle` of `/dev/win32k` at
   offset `buffer << 32` (`poly_win32k::link::layer_buffer_offset`) maps the section's pages,
   read-only and shared, the way a DRM dumb buffer is mapped at its fake offset (ADR 0108).
   The kernel finds the pages through win32k's layer and the NT section store. A mapping keeps
   its pages after the section is gone.
6. **The compositor copies only damage, over the surface's own picture.** A `LayerDamage`
   rectangle is copied from the mapped section into the window's content and damaged there; the
   surface's own damage (GDI commits of the window's process) is copied and then the layers are
   put over that part again; a layer that moves or goes shows the surface's picture where it was.
   The scene still never reads app memory while composing (ADR 0106): this is the one copy a
   Wayland client's buffer gets too.
7. **Rows, not pixels.** The compositor's copy of surface and layer pixels into the scene is one
   `memcpy` a row (then the alpha byte set for `Xrgb8888`) instead of a bounds-checked loop per
   pixel; the GDI path's surfaces get it too.
8. **Measurement stays.** `poly-compositor --stats` also reports the time spent taking clients'
   pixels (`take_avg_ms`: Wayland commits and the desktop link) and, for key presses, the time to
   the next drawn frame (`key_to_frame_avg_ms`, `key_to_frame_max_ms`), the typist's wait.

## Consequences

- A present costs the app a system call and a copy of its dirty rectangles; win32k copies nothing;
  the compositor copies, composes and flips only the dirty rectangles. Measurements before and
  after, next to Linux Chromium on Poly, are in the track's hand-back.
- win32k's own buffer of a window no longer holds the pixels of a layer. What reads it (the screen
  pixel of `GetPixel` on a screen DC, the hit test of layered windows) sees the surface's own
  picture under the layer.
- A torn frame is not possible within a section the compositor is copying from, except when the
  app presents twice before the compositor took the first present (it then shows a newer frame's
  pixels early, never stale or half-drawn ones outside a later present's damage).
- Not done: DirectComposition surfaces (`IDCompositionSurface::BeginDraw`), visuals with offsets
  and clips of their own, and the newer DComp interfaces Chrome asks for (spec sheets first);
  scanning out a layer without composing it; formats other than `B8G8R8A8`.
