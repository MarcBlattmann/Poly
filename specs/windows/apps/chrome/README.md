# Chrome for Windows on Poly: scenarios, recorded traces, gap map

Spec side (agent `spec-chrome-win`) of the roadmap goal "Chrome for Windows runs on Poly with its sandbox on".
Everything here is **observed behaviour** of the real application on real Windows through Poly's API tracer
(ADR 0090); no Microsoft or Google binary was decompiled or disassembled (CLEAN_ROOM.md). The map that tells an
implementer what to build is [`gap-map.md`](gap-map.md) in this directory (the same text is the hand-off
`spec-chrome-win.md`).

## The pinned application

| | |
|---|---|
| Product | Chrome for Testing, Stable channel |
| Version | 157.0.8103.38, win64 |
| URL | `https://storage.googleapis.com/chrome-for-testing-public/157.0.8103.38/win64/chrome-win64.zip` |
| SHA-256 of the zip | `ae0deef27d449e1d7529df7705d609bab102de48464da13567ed3dd2ee1a0586` (210,600,182 bytes) |
| Layout | flat: `chrome.exe` (4.1 MB), `chrome.dll` (311 MB), `chrome_elf.dll`, `chrome_proxy.exe`, `libEGL.dll`, `libGLESv2.dll`, `D3DCompiler_47.dll`, `dxcompiler.dll`, `vk_swiftshader.dll`, `vulkan-1.dll`, `*.pak`, `locales/` ... |

It is downloaded at build time and checked against the SHA-256; it is never committed or redistributed.

## The ladder

Each stage is a scenario with observable results. The pages are in `pages/` (SHA-256 in `windows.json`).

| Stage | Command (after unpacking the zip) | Observable result on Windows |
|---|---|---|
| **S1** | `chrome.exe --headless --no-sandbox --disable-gpu --user-data-dir=<fresh> --dump-dom file:///<dir>/pages/s1.html` | stdout is the serialised DOM, 440 bytes, SHA-256 `7b79f8b4...813b` (below), exit code 0, in 3-5 s. Stderr has two `external_registry_loader_win.cc` lines (a missing registry value), nothing else. |
| **S2** | S1 without `--no-sandbox` | the same stdout (same bytes), exit code 0, in 2-4 s |
| **S3** | `chrome.exe --disable-gpu --no-first-run --user-data-dir=<fresh> --window-position=40,40 --window-size=900,500 file:///<dir>/pages/s3.html`, driven by `drivers/s3_driver.ps1` | a top-level `Chrome_WidgetWin_1` window titled `poly-s3 clicks=0 keys= moved=0 - Google Chrome for Testing`; after a mouse move, one click and the keys p-o-l-y: `poly-s3 clicks=1 keys=poly moved=1 - ...`; the screenshot shows the page's text (Georgia, Arial bold, Consolas, German umlauts); `WM_CLOSE` to the window ends the browser with exit code 0 |
| **S4** | S3 with `http://example.com/`, then `https://www.iana.org/` through the address bar (`Ctrl+L`); headless: `pages/s4.html` frames both | titles `Example Domain - ...` then `Internet Assigned Numbers Authority - ...`; headless DOM `<p id="r">http=1 https=1</p>` (831 bytes) |
| **S5** | headless without `--disable-gpu` (default GPU flags) | the same DOM as S1; only the GPU process differs (see the gap map) |

S1's DOM (CRLF line ends, as Windows' console/pipe writes them):

```
<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Poly Chrome Test</title></head>
<body>
<h1 id="h">Hello Chrome</h1>
<p id="p">script-ran 42</p>
<div id="b" style="width:200px;height:50px">box 200</div>
<script> ... </script>

</body></html>
```

## What was traced, and how

`cargo xtask win trace-kit <chrome.exe> <chrome.dll> <chrome_elf.dll> --agg --out <dir>` (this branch: ADR 0090's
kit extended, `tools/xtask/src/wintrace.rs`, `tools/wintrace/polytrace-agg`):

* the three programs get the import names of their 63 system DLLs changed to proxies (`pt_<dll>.dll`, 1,303
  functions: every static **and delay-loaded** import of Chrome's three binaries); only `chrome_elf.dll` (Chrome's
  own), `bthprops.cpl` and `winspool.drv` are left alone;
* the logger is the *counting* form: Chrome makes millions of calls and starts about 15 processes, so a call is
  written in full only for a function's first four calls and for every new string it is given (paths, key names,
  DLL names, pipe names, command lines); everything else is a counter per function (calls, first call, results
  zero/not zero/-1, the first distinct last errors and results); one `polytrace-<pid>.jsonl` per process, the
  trace lives in a memory-mapped view (no file handle for the sandbox's handle closer);
* proxies of delay-loaded DLLs are loaded at start-up of child processes (`polytrace-preload.txt`), because Chrome
  narrows the DLL search path to System32 in them;
* `tools/chrometrace.py run` reduces a run to its processes (type from `--type=`), `merge` keeps **only what two
  runs share** (a function called in both, with the minimum and maximum of its counts; the objects named in
  both), `canon` writes the committed form below. Run each stage at least twice; the stable facts only are kept.

**What this cannot see**, honestly: calls made through pointers (`GetProcAddress` results, vtables, COM) are
invisible, but the *names* resolved with `GetProcAddress` are recorded (the gap map lists them); direct system
calls (`syscall` instructions in Chrome's own code) are not seen; the NT syscalls under `kernel32` are Windows'
business, not Chrome's, and are not interesting for Poly (Poly provides `kernel32` itself).

**The sandbox and the tracer.** With the sandbox on, the traced kit's *renderers* die at start (the browser
launches them with `CreateProcessAsUserW`, they never reach the logger, Chrome retries and stalls): the tracer's
unsigned proxy DLLs are loaded into a process Chrome confines (restricted token, job, mitigations: which of them
refuses the image is not known; an ACL for `ALL (RESTRICTED) APPLICATION PACKAGES` and `RESTRICTED` on the kit
directory did not help). So **the sandboxed stage S2 has the browser (the sandbox's broker: every token, job,
desktop, mitigation and `CreateProcessAsUser` call), the GPU process, the network service and crashpad traced,
but no renderer**; S2's renderer API set is S1's (same code, same DLL) plus the sandbox target services
(token lowering, handle closing), whose functions are named by the strings the children pass to
`GetProcAddress`. The scenario results of S2 (stdout, exit code) are from the untraced program.

## Files

| Path | What |
|---|---|
| `gap-map.md` | **the gap map** (also the hand-off `spec-chrome-win.md`): APIs by area and DLL against `coverage/windows`, batches |
| `chrome.yaml` | the spec sheet (`windows/api/apps/chrome`): oracle, scenarios, expected results |
| `pages/` | `s1.html` (DOM + script + layout), `s3.html` (text, mouse, keys), `s4.html` (http + https frames) |
| `drivers/run_stage.ps1` | S1, S2, S4 (headless), S5: one run, traced or not |
| `drivers/s3_driver.ps1`, `interactive_task.ps1` | S3, S4 (visible): the window, `SendInput`, screenshot, `WM_CLOSE`; the second runs it in the interactive session through a scheduled task, as `poly-oracle.ps1 -Interactive` does |
| `drivers/ablate.ps1` | which delay-loaded DLLs Chrome can do without (S1) |
| `drivers/remote_run.ps1` | runs one S3/S4 round on homeserver2 (after `scp` of the kit and the zip; see below) |
| `tools/chrometrace.py`, `covmap.py`, `gapmap.py`, `gapmap_md.py` | reduce, merge, join with `coverage/windows`, write the tables |
| `traces/s<N>.trace.json` | the committed reduced traces (`poly-chrometrace/1`, below) |
| `windows.json` | the recorded scenario results: exit codes, output digests, titles, machine facts |

**Window titles** have an EN DASH (U+2013) before `Google Chrome for Testing`; `windows.json` writes it as `-`.
**Where each stage was recorded**: S1, S2, S4 (headless), S5 on the owner's PC (Windows 11 Enterprise 25H2, 26200.9711);
S3 and S4 (window) on homeserver2 (Windows 11 Home 25H2, 26200.9711), interactive session through a scheduled task, as ADR 0075/0097 do.
The traces of S3 and S4 (window) were taken with `--no-sandbox` (the traced renderers do not run sandboxed); their untraced
runs (sandbox on) give the same titles and exit codes (`windows.json`).
The kit's file hashes in each trace are those of the recorded build (the logger source was reformatted afterwards).

## `poly-chrometrace/1`

The `wintrace` boot group compares a call-by-call sequence (ADR 0110). Chrome's sequences are millions of calls
per process, interleaved over dozens of threads and processes: not comparable. The committed form keeps what is
stable, per stage and per process type (`browser`, `renderer`, `renderer:extension`, `renderer:webui`,
`gpu-process`, `utility:network.mojom.NetworkService`, `utility:storage.mojom.StorageService`,
`crashpad-handler`):

```json
{"format":"poly-chrometrace/1","stage":"s1","recorded":{...},"runs":2,
 "process_types":{"browser":{"processes_per_run":[1,1],
   "calls":[["kernel32.dll","CreateFileW",n_min,n_max,"ok|mixed|never-ok",[last errors seen],rank], ...],
   "objects":[["CreateFileW","<profile>\\Default\\History"], ...],
   "unstable_calls":0,"unstable_objects":174}}}
```

`calls` are the functions called in both runs ordered by first call (`rank`: position among the process's
functions, not a position in a sequence); `n_min`/`n_max` the call counts over the two runs; `ok` = every result
was non-zero and non-(-1), `never-ok` = never, `mixed`; `objects` are the strings given to file, registry,
named-object, module and process functions, normalised (`<profile>`, `<app>`, `<n>`, `<hex>`).
A comparison on Poly (once Chrome runs there) is: every `calls` entry is called at least once
(`n >= 1`), a function that is `ok` on Windows returns non-failure on Poly, a process type that exists on Windows
exists on Poly, every `objects` entry that names an object Poly must provide (not `<profile>` files Chrome
creates) is opened. `tools/chrometrace.py` reads both sides' raw runs; porting the comparison into `cargo xtask
win chrome-trace diff` is the follow-up.
