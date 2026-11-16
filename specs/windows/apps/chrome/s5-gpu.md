# Chrome stage S5 (default GPU flags): what Windows does, what Poly does, what to build

Spec agent `spec-chrome-win-s5`, 2026-11-16. Chrome for Testing 157.0.8103.38 win64 (the pinned zip). Everything here is observed
behaviour (our own test programs, Chrome's DevTools `SystemInfo.getInfo`, the API tracer's counts, Poly's own serial log); no
Microsoft or Google binary was decompiled and nothing of Chrome is committed. The tests of the D3D11 / DXGI / DirectComposition entries
that this PR adds are the specification; this file says why those calls and what is still missing.

## 1. The machines

| Machine | OS | Hardware adapter | Session |
|---|---|---|---|
| owner's PC | Windows 11 Enterprise 25H2, build 26200.9711 | AMD Radeon 8060S, driver 32.0.22018.5 | interactive (S5 traced, DevTools probe, local looks of the tests) |
| homeserver2 | Windows 11 Home 25H2, build 26200.9974 | Intel UHD Graphics 630, driver 31.0.101.2135 | SSH, session 0 (the oracle of every test; DevTools probe) |

Both have the "Microsoft Basic Render Driver" (WARP, vendor 0x1414, device 0x8C) as a second adapter. The tests use WARP (deterministic);
The tests were run on both: of the 76 test programs all but three gave the same bytes on the PC and on homeserver2 (the exceptions are creation flags that need the SDK layers, `CreateDXGIFactory2(DEBUG)` and `D3D11CreateDevice` flags 0x4, 0x10, 0xFFFFFFFF; the oracle has no Graphics Tools). All 196 sheets are recorded in homeserver2's **interactive** session: in the SSH session 0 `DCompositionCreateDevice*` fails with E_ACCESSDENIED, `CreateSwapChainForHwnd` and `D3D11CreateDeviceAndSwapChain` fail (0x887A0022), `IDXGIAdapter::EnumOutputs` gives 0x887A0022 instead of NOT_FOUND on WARP and 28 test programs crash on the missing objects (see section 2: this is also why Chrome composes in software in session 0).

## 2. Which path Chrome takes (DevTools `SystemInfo.getInfo`, headless, `--no-sandbox`)

| Machine | Switches | GL renderer (ANGLE) | gpu_compositing | DirectComposition | WebGL | Graphite |
|---|---|---|---|---|---|---|
| PC | none | Direct3D11 on the AMD GPU (vs_5_0 ps_5_0) | enabled | yes | enabled | enabled |
| PC | `--use-angle=d3d11-warp` | Direct3D11 on Microsoft Basic Render Driver (0x8C) | enabled | yes | enabled | unavailable |
| PC | `--use-angle=swiftshader` | Vulkan on SwiftShader (vk_swiftshader.dll, SwANGLE) | enabled | no | enabled | enabled |
| PC | `--disable-gpu` | Disabled | disabled_software | no | disabled | disabled |
| homeserver2 | none | Direct3D11 on the Intel GPU | **disabled_software** | **no** | enabled_readback | enabled |
| homeserver2 | `--use-angle=d3d11-warp` | none (GL disabled) | disabled_software | no | disabled | disabled |
| homeserver2 | `--use-angle=swiftshader` | Vulkan on SwiftShader | enabled | no | enabled | enabled |
| homeserver2 | `--disable-gpu` | Disabled | disabled_software | no | disabled | disabled |

So: on an interactive desktop Chrome renders its page with ANGLE's D3D11 backend (shaders compiled at run time by `D3DCompiler_47.dll`
from the app's directory), presents with DXGI flip swap chains made for composition and attaches them to a DirectComposition visual tree;
in a session without a desktop (session 0) it keeps GPU work for WebGL and composites in software; with WARP forced it is the same
D3D11 backend on the software adapter when a desktop exists and software compositing when not; `--use-angle=swiftshader` runs the whole
GL stack on the CPU in Chrome's own DLL and needs from the system only a swap-chain present path.

## 3. What the GPU process calls (S5 headless, PC, two runs, tracer kit with `libEGL`, `libGLESv2`, `D3DCompiler_47`, `dxcompiler`, `vk_swiftshader` also proxied)

195 distinct API functions in the GPU process (S1: 159). Direct3D, DXGI, DirectComposition and display related:

| Kind | Names (counts per run) | Poly |
|---|---|---|
| import | `dxgi.dll!CreateDXGIFactory1` (5 to 6; returns S_OK, leaves last error 2) | has (software factory) |
| `GetProcAddress` | `D3D11CreateDevice`, `CreateDXGIFactory1`, `CreateDXGIFactory2`, `DXGIGetDebugInterface1`, `DCompositionCreateDevice3`, `DXCoreCreateAdapterFactory`, `D3DCompile`, `D3DDisassemble` | has D3D11CreateDevice, CreateDXGIFactory1; lacks `CreateDXGIFactory2` (see the sheet), `DCompositionCreateDevice3` (stub), `DXCore.dll` (missing: `LdrLoadDll(DXCore.dll) failed 0xC0000135`), `DXGIGetDebugInterface1` |
| `LoadLibrary` | `d3d11.dll`, `dxgi.dll`, `dcomp.dll`, `dwmapi.dll`, `DXCore.dll`, `D3DCompiler_47.dll`, `dxcompiler.dll` (the last two from the app directory) | all but `DXCore.dll` load |
| user32 display | `QueryDisplayConfig`, `GetDisplayConfigBufferSizes`, `DisplayConfigGetDeviceInfo` (12 to 25 each; they return 0, ERROR_SUCCESS, every time), `GetMonitorInfoW` (12 to 25), `MonitorFromWindow` (24 to 52), `GetDC` (1), `WindowFromDC` (2) | claimed entries with specs (s3b); `WindowFromDC` is specified here |
| COM (not seen by the tracer) | the interface methods of the D3D11 device and context, the DXGI factory, adapter, swap chain, the DirectComposition device, target and visuals | the entries of this PR |

The interface methods are not visible to the API tracer. The list of methods Chrome calls was therefore taken from Learn's description of
what ANGLE's D3D11 backend and the compositor need, and from what Poly's software `d3d11.dll` is asked for and does not have (section 4).

## 4. S5 on Poly (this tree, Chrome with default GPU flags, 4 GiB, 4 CPUs; stage 5 headless, stage 6 window)

| Run | Result |
|---|---|
| stage 5 (headless `--dump-dom`) | **passes**: 440 bytes, SHA-256 `7b79f8b4...813b` like S1, 33 to 43 s, exit 0 |
| stage 6 (window `--window-size=900,500`, 120 s) | Chrome stays up; the page is painted by the software path only after the 9th to 12th GPU process (the first `d3d11: present` lines come from the last one) |
| GPU processes started in one run | 9 to 13 (headless), 9 to 18 (window; `--use-angle=swiftshader` 11, `d3d11-warp` 9, `--disable-gpu-compositing` 12, `--disable-gpu --use-angle=swiftshader-webgl` 18) |
| what each GPU process asks and does not get | `QueryInterface` of the device for `ID3DUserDefinedAnnotation` ({b2daad8b-03d4-4dbf-95eb-32ab4b63d0ab}), `ID3D11Device1` ({a04bfb29-08ef-43d6-a49c-a9bdbdcbe686}) and `IDXGIDevice2` ({05008617-fbfd-4051-a790-144884b4f6a9}), twice each, then the process ends; no `CreateBuffer`, view, shader or state stub is ever hit, so it ends before any rendering call |
| browser's stderr | `components\viz\host\persistent_cache_sandboxed_file_factory.cc:153 Failed to open persistent cache files in directory "...\GPUPersistentCache\GPUCache\..."` and `...\DawnGraphiteCache\...`, error 0x20 (ERROR_SHARING_VIOLATION), once per GPU process start |
| other stubs hit | `ole32!CoCreateInstance` for classes that are not registered (the GPU info and the audio enumerator), `kernel32!ReadDirectoryChangesW`, `RegisterApplicationRestart`, `CreateToolhelp32Snapshot`, `advapi32!LogonUserW`, `uxtheme` ordinals 133 and 135 |
| user-mode exceptions | none (`poly.nt.exceptions`): each GPU process leaves with an exit code, not a breakpoint |

Not isolated: why each GPU process ends (Poly's trace of its calls was too short to show it). The first suspects are the QueryInterface
misses and the persistent-cache sharing violation, both of which are file/interface behaviour that these specs pin down. Whatever the
reason, no route (`--use-angle=swiftshader`, `d3d11-warp`, `--disable-gpu-compositing`) changes the number of restarts, so it is not
about which GL backend is chosen: it happens in the common GPU process start-up.

## 5. Recommendation (the simplest route for Poly)

1. **Software device, not a shader machine.** A WARP-like device that has the whole non-shader surface (device, textures, buffers,
   views, queries, Map, copies, clears, shared textures, DXGI factory/adapter/swap chains, DirectComposition) is enough for the
   first goal: Chrome should settle on its software-compositing path (what homeserver2's session 0 does, `gpu_compositing:
   disabled_software`) at the **first** GPU process instead of the 10th. Poly's S3 path already paints the page from there.
2. The D3D11 shader entry points (`CreateVertexShader`, `CreatePixelShader`, ...) take DXBC produced by `D3DCompiler_47.dll`, which Chrome
   brings. Running ANGLE's D3D11 backend for real would need a DXBC interpreter or JIT in Poly's `d3d11.dll`; that is a project of its own
   and nothing in S5 requires it while Chrome can fall back.
3. **If Chrome insists on a GL backend**, `--use-angle=swiftshader` (also reachable through `--enable-unsafe-swiftshader` /
   the fallback Chrome takes by itself) runs Vulkan on the CPU inside `vk_swiftshader.dll`; it needs from Poly only the swap-chain
   present path, which the S3 hunt already built, plus DXGI answering every `QueryInterface` the GPU process makes.
4. Order of work: (a) the sheets of `CreateDXGIFactory1/2`, `D3D11CreateDevice`, the `IDXGIDevice` / `ID3D11Device` interface chain
   (this PR) so the GPU process stops ending after its QueryInterface calls; (b) the persistent-cache file behaviour (sharing violation 0x20);
   (c) `DXCore.dll` with `DXCoreCreateAdapterFactory` returning an error (Windows 10 behaviour) or a software adapter list; (d) textures,
   buffers, views, queries, Map and copies exactly as specified; (e) swap chains and DirectComposition; (f) only then shaders.

## 6. Limits

* The tracer sees API calls, not COM methods; the COM list is inferred (section 3). A vtable-hooking spy for the GPU process would give it exactly.
* S5 with a window was not traced on Windows (the interactive session of homeserver2 belongs to other work); the committed
  `traces/s5-window.trace.json` has the counts of an earlier traced window run.
* No machine without any GPU was available, so "Chrome on a machine whose only adapter is WARP" is inferred from homeserver2's session 0
  and from `--use-angle=d3d11-warp`.
