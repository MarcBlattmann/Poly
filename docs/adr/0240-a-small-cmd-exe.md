# 0240: A small `cmd.exe` in `C:\Windows\System32`

**Status:** Accepted. Track `impl-win-cmd`. Inputs: the spec sheets that start `cmd.exe`
(`specs/windows/kernel32.dll/CreateProcessElevation.yaml`,
`specs/windows/advapi32.dll/CreateProcessAsUserW.yaml`,
`specs/windows/shlwapi.dll/AssocQueryStringW.yaml`) and the visible tests of
`CreateProcessElevation`, `K32QueryWorkingSetEx`, `SetProcessWorkingSetSize`,
`WTSGetActiveConsoleSessionId`, `AssocQueryStringW` and `CreateProcessAsUserW`; Microsoft Learn
("cmd" and the pages of the built-in commands it lists, "Command shell overview", "Using command
redirection operators", "Using batch parameters", "VS_FIXEDFILEINFO", "Version Information").
No third-party source; nobody's `cmd` was read.

## What was there

Poly had no `cmd.exe`. The visible tests of six coverage entries (`CreateProcessElevation`,
`K32QueryWorkingSetEx`, `SetProcessWorkingSetSize`, `WTSGetActiveConsoleSessionId`,
`AssocQueryStringW`, `CreateProcessAsUserW`) start `C:\Windows\System32\cmd.exe /c exit`
(suspended, to have a child process) or `cmd.exe /c exit 3` (and wait for the exit code);
`CreateProcessAsUserW.yaml` also has `cmd /c exit 5` found by its name and a variable of the
child's environment expanded by `cmd`; `AssocQueryStringW`'s
`ASSOCSTR_FRIENDLYAPPNAME` reads the program's version resource. Installers run
`cmd.exe /c …` lines now and then.

## Decisions

1. **The logic is a host-tested crate, `crates/poly-cmd`; the program is thin.**
   `personalities/nt/win32/cmd` (a console program, entry `cmd_main`, imports only
   `kernel32.dll`) answers the crate's `shell::Host` with `kernel32` calls: files, the
   environment, the current directory, `CreateProcessW` and waiting. It is built and installed
   like `schedsvc.exe` and `services.exe`: an output of `cargo xtask win32`, embedded by the
   kernel, written to `C:\Windows\System32` at boot.
2. **A version resource**, built in `build.rs` like `kernel32.dll`'s (`verinfo::res_file`):
   `VFT_APP`, Poly's system version, `FileDescription` "Windows Command Processor" (the value
   `AssocQueryStringW.yaml` records as the program's friendly name), `CompanyName` and
   `ProductName` "Poly" as on Poly's other system files. No other strings of Windows' copy.
3. **Messages are Poly's own.** Where Learn shows no text, errors are `cmd: …` lines on standard
   error; only `ECHO is on.` / `ECHO is off.` (Learn: "echo") follow Windows. A name that is no
   built-in and no program sets `ERRORLEVEL` 9009.

## What is supported

- **Switches:** `/c`, `/k` (then commands from standard input), `/s`, `/q`, `/d`, `/a`, `/u`
  (UTF-16 output to files and pipes), `/v:on|off`, `/e:on|off`, `/t:` and `/f:` (accepted,
  nothing to do). The `/c` and `/k` string follows Learn's quote rule (two quotes around an
  existing program with a space kept; otherwise a leading quote and the last one removed).
  Without `/c` or `/k`, commands are read from standard input with the `C:\path>` prompt (no
  banner).
- **Exit code:** `exit n`, `exit` (the current `ERRORLEVEL`), or the `ERRORLEVEL` of the last
  command. `exit /b n` ends a batch file and sets `ERRORLEVEL`; outside one it ends `cmd.exe`.
- **Operators:** `&`, `&&`, `||`, `|` (the left side runs to its end into a temporary file in
  `%TEMP%`, then the right side reads it), `( … )` groups over several lines, `^` escapes,
  quotes.
- **Redirection:** `<`, `>`, `>>`, with a handle number (`2>`), `>&n` (`2>&1`), `nul`; on
  commands and groups; files are opened inheritable and given to programs with
  `STARTF_USESTDHANDLES`.
- **Expansion:** `%VAR%` (left as written when not defined on the command line, empty in a
  batch file), `%VAR:~s,l%`, `%VAR:old=new%`, `%VAR:*old=new%`, the dynamic `%CD%`,
  `%ERRORLEVEL%`, `%CMDCMDLINE%`, `%CMDEXTVERSION%` (2), `%RANDOM%`; `!VAR!` with `/v:on` or
  `setlocal enabledelayedexpansion`.
- **Built-ins:** `echo` (`echo.`, `echo on|off`), `set` (`name=value`, `name=`, `"name=value"`,
  a prefix lists, `/a` with `+ - * / % << >> & ^ | ! ~ ( ) ,` and the assignment operators, `/p`),
  `if [/i] [not] errorlevel n | exist path | defined name | cmdextversion n | a==b | a op b`
  (`equ neq lss leq gtr geq`, numbers compared as numbers) with `else` after a group, `exit`,
  `rem`, `cd`/`chdir` (`/d`, `d:`), `pushd`, `popd`, `type`, `md`/`mkdir` (with the directories on
  the way), `del`/`erase` (no wildcards), `setlocal`, `endlocal`, `call`, `goto`, `shift`, and
  `cls`, `title`, `color` (nothing to do).
- **Programs:** the current directory first, then each `PATH` directory, each `PATHEXT`
  extension when the name has none; started with the command's text as its command line, the
  environment `cmd.exe` has, and waited for (GUI programs too); the exit code is the new
  `ERRORLEVEL`.
- **Batch files** (`.bat`, `.cmd`): run line by line in this shell; `@`, echo of commands with the
  prompt, `%0`–`%9`, `%*`, `%~1`, `%~f0`, `%~d0`, `%~p0`, `%~n0`, `%~x0` (and combinations such as
  `%~dp0`), `%%`, labels, `goto label`, `goto :eof`, `call file.bat`, `call :label args`,
  `shift`, `setlocal`/`endlocal` (undone at the end of the file); a batch file started without
  `call` from another ends that one.

Not supported: `for`, `start`, `copy`, `move`, `dir`, `ren`, wildcards, `call`'s second
expansion, `shift /n`, `%DATE%`/`%TIME%`, AutoRun, command history and editing, a real pipe
(both sides at once).

## Consequences

- The 15 visible tests of the six entries pass on Poly: an integrator can set them green.
- `cmd /c` lines of installers that use only the above work; anything else prints a `cmd:` line
  and fails with a non-zero `ERRORLEVEL` instead of the program not being there.
