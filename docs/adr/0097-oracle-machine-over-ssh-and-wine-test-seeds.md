# 0097: The Windows oracle is a dedicated machine reached over SSH (`homeserver2`), not a VM; `win remote` builds, runs there, fetches, records and compares in one step; Wine's conformance tests become name-only spec seeds in the coverage map

**Status:** Accepted. Two of the three parts of the Phase 5 item "Windows
test oracle VM, Windows API tracer, Wine conformance tests imported as spec
seeds"; the API tracer is ADR 0090's (branch `win-api-tracer`), so the
item stays unticked until that is merged too. Builds on
[ADR 0075](0075-windows-test-oracle.md) (test programs, `poly-oracle.ps1`,
bundles, `poly-winrun/1`, `win diff`) and
[ADR 0006](0006-coverage-map-one-file-per-component.md) (the coverage map); compatible with
the runner's data files and per-test timeouts of
[ADR 0088](0088-7-zip-scenario-driver.md).

## Context

ADR 0075 made the owner's own Windows PC the oracle, driven through a
Claude Code session on it, with bundles pasted as text. Two things changed:

- The owner's corporate PC (where the coordinating sessions run) now
  refuses to start unsigned test executables, so tests must not run there.
- The owner named a dedicated Windows machine as the oracle:
  **`homeserver2`**, Windows 11 Home 25H2 (build 26200.9711), x64, 6 CPUs,
  8 GB, Defender only, user `server`, ANSI code page 1252 / OEM 850, W.
  Europe Standard Time. It is reachable from the corporate PC as
  `ssh homeserver2` (OpenSSH server, PowerShell 5.1 as the login shell,
  key authentication). Work happens only in
  `C:\Users\server.HOMESERVER2\poly-oracle-work\`; other agents use their
  own subfolders there (`cxxeh\`, `tracer\`), this flow uses `remote\`.

The roadmap says "oracle **VM**". The builds run in the `poly-dev` Docker
container on the corporate PC (`polyd`), which has the toolchain but no
SSH keys; SSH and `scp` work on the Windows host (Git Bash). And the third
part, Wine's conformance tests (`dlls/<dll>/tests/*.c`, LGPL), is spec-agent
oracle material (CLEAN_ROOM.md): a spec agent should know, per coverage
entry, where Windows behaviour for it is tested.

## Decision 1: a dedicated machine over SSH satisfies "oracle VM"

What the roadmap needs from the oracle is a real, licensed Windows that
runs our test programs unattended and reproducibly, observed only from
outside (CLEAN_ROOM.md). A dedicated machine gives that today; a VM would
add isolation and resets, at costs that need the owner:

| | Dedicated machine (`homeserver2`) | Hyper-V VM | QEMU/KVM VM on a Linux host |
|---|---|---|---|
| Available now | yes | no: Windows 11 **Home** has no Hyper-V (needs Pro) | no: needs a host with KVM and disk space |
| Licence | the machine's own | another Windows licence per VM (evaluation images expire after 90 days and must be rebuilt) | the same |
| Clean state | a fresh work directory per run, removed afterwards; nothing else touched | checkpoints: revert after every run | snapshots: the same |
| Several Windows builds | one (its build is in every result file) | one VM per build | one image per build |
| Speed | native | near native | near native with KVM; minutes per boot under TCG |
| Resources | 6 CPUs, 8 GB | a 4 GB guest leaves the 8 GB host little | depends on the host |
| Defender | the owner's settings, never changed by us | ours to configure (still an owner decision) | the same |
| Who decides | done (owner named it) | owner: Pro upgrade or licence, RAM | owner: host, licence |

**Decision:** `homeserver2` is the oracle; the roadmap's "VM" is read as "a
Windows machine used only as the oracle, reachable by the tools". A VM
stays an option for later, when several Windows builds or reverting to a
snapshot are needed (for example tests that change machine-wide state);
creating one is the owner's decision and needs a licence. Nothing in the
tools depends on the machine being physical: any host that `ssh <host>`
reaches with Windows PowerShell 5.1 works, a VM included.

The machine's facts differ from the PC that made ADR 0088's recording
(Enterprise, 32 processors), so tests must not depend on edition, processor
count, code pages or time zone, as ADR 0075 already asks (no addresses,
handles or times in observations).

## Decision 2: `win remote`, split between the container and the host

```sh
# On the owner's PC (builds in the container, SSH on the host), from the repository root:
POLY_RUN=~/poly-dev/polyd tools/pipeline/windows/poly-remote.sh --host homeserver2 [--tests a,b] [--record] [--diff]
# Where cargo and ssh are on the same machine:
cargo xtask pipeline win remote --host homeserver2 [--tests a,b] [--record] [--diff]
```

Three steps; the one-step form runs all three, the script runs 1 and 3
through `POLY_RUN` (the container) and 2 itself:

1. **`win remote prepare --host H`** builds the tests (`win build`, with
   `--tests`, `--tests-dir`; Rust and, per ADR 0098, C++ tests) and bundles
   them (`win bundle`, with `--data-dir` for ADR 0088's data files) into a **stage
   directory**, `.oracle-remote/<host>/`: `bundle-<id>.txt`,
   `manifest.json`, a copy of `poly-oracle.ps1` (so runner and tools always
   match; the machine needs no clone of the repository), `plan.json`
   (`poly-winremote/1`: host, remote directory, bundle id, runner arguments,
   tests) and `transfer.sh`. The stage is inside the repository (ignored by
   git) and not under `target/`, which `polyd` keeps in a Docker volume the
   host cannot see; so the container writes it and the host reads it.
2. **`sh .oracle-remote/<host>/transfer.sh`**, generated by step 1, is
   everything SSH does: create `poly-oracle-work/remote/<bundle id>/` on the
   machine, `scp` the bundle and the runner there, run the runner (`-Work`
   inside that directory, so `%TEMP%` is not used), `scp` `results.json`
   back, remove the remote directory. Remote commands are PowerShell sent as
   `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass
   -EncodedCommand <base64 UTF-16LE>`, so the remote login shell (PowerShell
   or cmd) passes them on unchanged, and the plain text of each is a comment
   in the script. The execution policy is bypassed for that process only.
   Progress is off, the runner's host messages go to standard output and an
   error is one line of text with exit code 1 (a PowerShell whose output is
   not a console otherwise writes CLIXML). `ssh`/`scp` use `BatchMode`, so a
   missing key fails instead of prompting.
3. **`win remote finish`** reads the stage: the result file must be of the
   staged bundle (its id), from `windows`, with a result for every test, of
   the staged executables (SHA-256); otherwise it is refused. It prints each
   test's status and exit code, keeps the file as
   `target/pipeline/windows/results/windows-<id>.json` (where ADR 0075's
   manual round puts it), and:
   - **`--record`** writes the references ADR 0075 describes: a corpus
     test's to `tests/windows/oracle/<test>.windows.json` (one file per
     test, as ADR 0088 records the 7-Zip driver, so a subset can be
     re-recorded without touching the others; this replaces ADR 0075's
     `windows-<build>.json`), a spec entry's tests' to `windows.json` beside
     them (merged per test; the held-out store's the same way, outside the
     repository). The runner's whole result is kept (edition, display
     version, timeouts). A test that did not run, timed out or was not
     stable is not recorded.
   - **`--diff`** runs Poly on exactly the bundle's executables (unpacked
     from the stage, hashes checked) and compares, as `win diff` (same
     report, `summary.json`, exit status; `--kernel`, `--build-kernel`,
     `--jobs`, `--timeout`, `--strict`). Tests with data files are scenario
     drivers and are left to `win scenario diff` (ADR 0088), which compares
     them step by step; `finish` names the command.

Host names, the remote directory and runner arguments are checked before
they reach a command line (letters, digits and a few punctuation marks; no
`..`). `--runner-arg` passes arguments to `poly-oracle.ps1` (`--runner-arg
-Runs --runner-arg 3`; for ADR 0088's driver `--data-dir DIR --runner-arg
-TestTimeouts --runner-arg sevenzip_scenario=300`: its data files travel
inside the bundle as `data` lines). `win remote` and `win remote finish` read Windows'
observations, so they refuse `POLY_ROLE=implementer` like `win diff`;
`prepare` does not observe anything.

Nothing is installed on the machine and nothing outside
`poly-oracle-work\remote\` is changed; Defender is not touched. If Defender
takes an executable away, the runner reports that test as `error` (`could
not copy it` / `could not start`), `finish` shows it, exits 1 and does not
record it; an exclusion stays the owner's decision (ADR 0075).

**First runs** (2026-10-30), on `homeserver2` (build 26200.9711), no
Defender interference:

- The three NT corpus tests (`nt_file_rw`, `nt_query_system_time`,
  `nt_virtual_memory`; bundle `66486cf140c5`, built after ADR 0088's
  `poly-wtest` changes) ran with exit code 0, stable over two runs;
  `--diff` against Poly passed all three (`nt_query_system_time` included:
  our `ntdll` exports `NtQuerySystemTime` since ADR 0075 was written). They
  are recorded as `tests/windows/oracle/<test>.windows.json`.
- Two of ADR 0098's C++ tests (`_CxxThrowException.tests/cxxthrow_record`,
  `cxxthrow_handmade`; bundle `ddc97e24042b`) ran with exit code 0 and are
  byte for byte the references recorded on the other PC (`win diff
  --target` of the two result files: 2 pass), so the machine's differences
  (edition, processors) do not show in them.
- `sevenzip_scenario` was not re-recorded: its committed reference is ADR
  0088's.

A test program's reference names its executable's hash, and the build is
reproducible, so a change to a test or to its support library needs a new
recording (`--record` again).

## Decision 3: Wine's conformance tests as name-only seeds in the coverage map

ADR 0075 said nothing of Wine's tests enters the repository and listed the
files a spec agent should look at first. This refines it: **names** enter,
nothing else. A seed is a locator, the same kind of fact as a `.spec` export
name or a bibliography entry:

```json
{"symbol":"NtCreateFile","src":["wine-spec"],"seeds":["wine:ntdll/tests/file.c:test_NtCreateFile", "..."]}
```

`wine:<dll>/tests/<file>.c:<function>`: the test file's path below `dlls/`
and the name of a function in it that checks something, for each export it
calls. No code, comments, string literals, arguments or expectations are
stored or printed, and none is copied. The Wine checkout stays outside the
repository (here `~/poly-generated/wine`, a sparse checkout of
`dlls/*/*.spec` and `dlls/*/tests/*.c`); its revision is recorded in
`coverage/index.json` under `seeds.wine` and checked against the checkout's
`HEAD`. Following a seed (reading the Wine test) remains a spec agent's
step only (CLEAN_ROOM.md: Wine source for behaviour, spec agent ✅,
implementer ❌); `cargo xtask coverage show` hides seeds under
`POLY_ROLE=implementer` and prints only their number. The owner reviews
this reading of CLEAN_ROOM.md with this change; CLEAN_ROOM.md itself is not
edited.

**How seeds are found** (`tools/coverage/src/seeds.rs`, host-tested): each
`dlls/<dll>/tests/*.c` is read lexically (comments, literals and
preprocessor lines skipped, only the first branch of a conditional kept),
functions are found by their `name(...) {` shape at file level, a function
**checks** if it calls `ok(` or an `ok_*(` macro, and its calls are the
`name(` in its body; a call through Wine's function-pointer convention
`pName` / `p_name` counts as a call of `Name`, and a function the file
defines itself is not an export. A call seeds the entries of that name in
the test's own DLL (`kernel32` tests: `kernel32.dll`) and in the base system
every test uses (`ntdll.dll`, `kernel32.dll`, `kernelbase.dll`, and with
`ntdll`'s the `ntoskrnl` system-call entry of the same name); other DLLs'
exports are not seeded from a test that only uses them. Per entry at most
**8** seeds, best first: a test named after the export (`test_CreateFileW`,
`test_CreateFile` for `CreateFileW`) before one about it
(`test_CreateFile_share`) before the rest; then tests of the entry's own DLL
(for `kernelbase.dll` also `kernel32`'s, for `ntoskrnl` `ntdll`'s); then
`test*` functions before checking helpers; then the function that calls the
fewest exports (the most focused); then the one calling it most often.

**Format** (backwards compatible): `seeds` is an optional list on an entry,
after `src`, absent when empty, so files without seeds are unchanged;
`index.json` gains an optional `seeds` map (prefix to description and
revision), absent until the first seed import. `coverage check` requires
every seed to be `<known prefix>:<locator>` and no seed twice. `coverage
import` keeps seeds; `seed-wine` replaces every `wine:` seed and keeps other
prefixes. The README progress section does not change.

```sh
cargo xtask coverage seed-wine <wine checkout> --revision <wine commit>
cargo xtask coverage show windows/api/ntdll.dll/NtCreateFile
```

**The import** at Wine `840a939ee526137639665ff5f587224d6a36403f` (the
revision of the coverage map's `.spec` import): 826 test files, 29 950
functions of which 18 356 check something, 75 125 entry/test pairs, **8 521
entries seeded**: `kernel32.dll` 826 of 1559 entries, `kernelbase.dll` 883
of 1440, `ntdll.dll` 684 of 1523, `ntoskrnl` (system calls) 170 of 271,
`advapi32.dll` 262 of 589, `user32.dll` 486 of 992, `msvcrt.dll` 321 of
1393, `ucrtbase.dll` 178 of 2602 (about 1.6 MB in 195 component files).

**Limits:** a lexical reading misses tests generated by macros and calls
made only in helpers a test calls (helpers that check are seeds of their
own); exports with the same function under two DLLs outside the base system
(`msvcrt` and `ucrtbase`) are seeded only from each DLL's own tests. A
coverage file changed by a claim and by a seed import at the same line
conflicts in git; take the other side and run `seed-wine` again.

## Not built here

- The Windows API tracer (ADR 0090, another agent); the roadmap item stays
  open until it is merged.
- A `win polycheck` in CI against the recorded references, and `tests
  split` / `handoff` for `.rs` tests (ADR 0075's next steps).
- Uploading bundles through a release for an unattended loop (ADR 0075);
  SSH makes it unnecessary while a machine is reachable.

## Consequences

- One command runs the Windows tests on the oracle and compares Poly with
  it; no session on the Windows machine and no copy-paste are needed, and no
  test executable runs on the corporate PC.
- New: `tools/pipeline/src/remote.rs` (`poly-winremote/1`, the generated
  `transfer.sh`, result checks, recording), `tools/pipeline/windows/poly-remote.sh`,
  `.oracle-remote/` (ignored), `tests/windows/oracle/*.windows.json`.
- The coverage map gains seeds for 8 521 Windows entries; `coverage show`
  lists them for spec agents. Re-run `seed-wine` when the Wine revision of
  the map moves.
