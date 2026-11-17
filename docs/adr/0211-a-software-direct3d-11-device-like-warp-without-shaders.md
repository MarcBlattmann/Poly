# 0211: A software Direct3D 11 device like WARP, without shaders

**Status:** Accepted. Chrome for Windows with its default GPU flags (stage S5, track
`impl-chrome-s5`). Inputs: the spec sheets and visible tests of `specs/windows/{d3d11,dxgi,dcomp,dxcore}.dll`
(spec agent `spec-chrome-win-s5`, PR #270) and `specs/windows/apps/chrome/s5-gpu.md`; Microsoft
Learn's Direct3D 11, DXGI and DirectComposition pages; the public SDK headers (`d3d11*.h`,
`d3dcommon.h`, `dxgi*.h`, `dxgiformat.h`, `dcomp.h`, `winerror.h`) for constants, layouts and the
order of the methods in the vtables. Learn "IRP_MJ_CLEANUP" / "IoRemoveShareAccess" for the
sharing fix. No third-party source.

## What was there

The S3 hunt (ADR 0155) gave Poly a `d3d11.dll` for Chrome's `--disable-gpu` path: a device of
2D textures in memory with copies, maps and clears and swap chains that present through GDI.
With the default flags Chrome's GPU process asked the device and its context for
`ID3D11Device1`, `IDXGIDevice2` and `ID3DUserDefinedAnnotation`, got `E_NOINTERFACE`, and ended;
the browser started another one, 9 to 18 times, before it fell back to software compositing
(s5-gpu.md section 4). The browser also logged a sharing violation (0x20) on its GPU cache files
at every GPU start.

## Decisions

1. **One software device that behaves like Windows' WARP device for everything that is not a
   shader.** `D3D11CreateDevice` makes it for the WARP, HARDWARE and NULL driver types (Poly's only
   adapter is the software one) and for UNKNOWN with an adapter, at the first level of the caller's
   list (9_1 to 12_1; 11_0 without a list), with the argument rules the sheet observed. The
   device answers `ID3D11Device` .. `5`, `IDXGIDevice` .. `4`, `ID3D11Multithread` and its own
   `IUnknown`; its immediate context answers `ID3D11DeviceContext` .. `4`,
   `ID3DUserDefinedAnnotation` (no profiling tool: events are not recorded) and
   `ID3D11Multithread`. The vtables follow the SDK headers' C `...Vtbl` structs; a method with no
   behaviour yet logs `stub: d3d11: Interface::Method` once.
2. **Resources live in memory, allocated on first use.** Buffers and 2D textures (mips, arrays,
   cubes, sample counts) keep their subresources one after the other with unpadded rows (the
   pitches of Map.yaml). Views, state objects (equal normalised descriptions are one object, at
   most 4095/4096 alive), queries (work is done when a call returns, so an ended query has its
   data at once), maps, copies, updates, clears, mip generation and resolves are implemented as
   the sheets observed; an invalid copy box removes the device as WARP does. Views and pipeline
   bindings hold *hidden* references, which keep an object alive without showing in the count
   that `AddRef` / `Release` return (the sheets observe that a view does not change its resource's
   public count).
3. **No shaders.** `Create*Shader` and `CreateInputLayout` refuse every bytecode with
   `E_INVALIDARG`, draws and dispatches change nothing (Draw.yaml observes exactly this with no
   shaders bound). Chrome's GPU process then settles on software compositing at its first start,
   which is what Windows does on a machine without a usable GPU path; ANGLE's D3D11 backend would
   need a DXBC interpreter, which is a project of its own (s5-gpu.md section 5).
4. **The rules are host-tested in `crates/poly-d3d`:** creation arguments, the format table
   (`CheckFormatSupport`'s observed masks per feature level; a format only the held-out tests show
   takes the mask of the observed format of its family and kind), buffer / texture / view
   descriptions, state normalisation, queries, the bytes a clear stores (single-precision
   products rounded half to even), private data. The DLLs hold the COM objects.
5. **DXGI lists one software adapter.** `EnumAdapters1(0)` is the machine's adapter ("Microsoft
   Basic Render Driver", vendor 0x1414, device 0x8C, flag SOFTWARE) with the screen as its output;
   `EnumWarpAdapter` gives the same adapter (same LUID) without outputs, as WARP has none.
   Factories answer `IDXGIFactory` .. `7`, adapters `IDXGIAdapter` .. `4`; objects keep private
   data per COM object.
6. **Share access ends at a file's last handle.** A file object's share access is counted out
   when its last handle closes (the cleanup point, Learn "IoRemoveShareAccess"), not when its last
   reference goes: a section that maps the file keeps the object but not its sharing. The object
   manager reports the cleanup point (`pop_cleaned`) before the deletion (`pop_deleted`).
7. **`DXCore.dll` stays absent**, as on Windows 10 before it: Chrome's probe of it fails with
   `STATUS_DLL_NOT_FOUND` and goes on.

## Consequences

- Chrome with its default GPU flags starts one GPU process and paints its window from it
  (S5 stages 5 and 6 of `cargo xtask chrome-win`, the S5 checks of `boot-tests chrome-win`,
  reported but not required until they are stable).
- `d3d11.dll` logs once per process which kind of process loaded it
  (`d3d11: loaded (process type gpu-process)`), so a run's log counts GPU processes.
- What is not done: shaders, input layouts, UAV clears, 1D/3D textures, deferred contexts, NT
  shared handles (`CreateSharedHandle`, `OpenSharedResource1`), fences, DXCore.
