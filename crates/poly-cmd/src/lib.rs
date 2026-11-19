//! The logic of Poly's `cmd.exe` (ADR 0240), as host-tested code; `cmd.exe` itself
//! (`personalities/nt/win32/cmd`) only puts Windows' calls behind [`shell::Host`].
//!
//! - [`args`]: `cmd.exe`'s own command line: its switches and the `/c` / `/k` string with
//!   the documented quote rule.
//! - [`expand`]: `%VAR%` expansion (substrings and substitutions included), the batch
//!   parameters `%0`..`%9`, `%*`, `%~…`, and `!VAR!` with delayed expansion.
//! - [`parse`]: a command line into a tree: `&`, `&&`, `||`, `|`, `( … )`, `if … else …`,
//!   redirections and the `^` escape.
//! - [`arith`]: the expressions of `set /a`.
//! - [`shell`]: the interpreter: the built-in commands, program search and batch files.
//!
//! Sources (clean room, CLEAN_ROOM.md): Microsoft Learn's "cmd" page and the pages of the
//! built-in commands (`echo`, `set`, `if`, `exit`, `call`, `goto`, `shift`, `setlocal`,
//! `endlocal`, `cd`, `pushd`, `popd`, `type`, `md`, `del`, `rem`), "Command shell overview",
//! "Using command redirection operators", "Using batch parameters"; and the spec sheets that
//! start `cmd.exe` (`specs/windows/kernel32.dll/CreateProcessElevation.yaml`,
//! `specs/windows/advapi32.dll/CreateProcessAsUserW.yaml`, `specs/windows/shlwapi.dll/AssocQueryStringW.yaml`)
//! with their visible tests. What is supported is listed in ADR 0240.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod args;
pub mod arith;
pub mod expand;
pub mod parse;
pub mod shell;

/// ASCII case-insensitive equality.
pub(crate) fn eq_ic(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}
