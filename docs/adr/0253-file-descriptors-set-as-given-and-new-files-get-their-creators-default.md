# 0253: File descriptors set as given, and new files get their creator's default

**Status:** Accepted. Track `chrome-feel` (round 4); sheets `specs/windows/advapi32.dll/SetFileSecurityW.yaml`,
`SetFileSecurityA.yaml`, `GetFileSecurityW.yaml` and `GetFileSecurityA.yaml` (PR #399). Builds on
[ADR 0171](0171-windows-security-model-tokens-descriptors-and-access-checks.md) (descriptors per path, inheritance down the path) and
[ADR 0185](0185-windows-names-for-volume-devices-and-tests-on-c.md) (tests run in `C:\PolyTests`). Inputs: the sheets, Microsoft Learn
("SetFileSecurityW", "SetSecurityInfo", "Security Descriptors for New Objects", "ACE Inheritance",
"Automatic Propagation of Inheritable ACEs").

## Context

Poly keeps a file's or registry key's descriptor by path: what was set on a path, and what an object shows is that
plus the inheritable ACEs of its parents, computed down the path (ADR 0171). That is what `SetNamedSecurityInfoW` and
`SetSecurityInfo` do on Windows: they ask for automatic inheritance and propagate it.

The sheets of `SetFileSecurityW` and `GetFileSecurityW` show the other half of Windows:

- `SetFileSecurityW` (and `SetKernelObjectSecurity`) store a DACL **exactly as given**:
  - ACEs in the given order, `ID` flags kept;
  - no inherited ACEs added and no `AI`;
  - the protection is the descriptor's own (the information flags change nothing visible).
- A directory's new DACL is **not pushed down** to the files already in it.
- A **new file** gets its creator's owner and group. Where its directory passes nothing down, it gets the DACL
  SYSTEM, Administrators and the creating user with full access, for the elevated as for the standard token.
- **The SACL information** replaces the audit ACEs and leaves the label; the label changes only with the label
  information.

Poly showed `AI` and the parents' ACEs after every set, propagated a directory's DACL to existing files, gave new
files the volume's flat default (owner Administrators), and lost the label on a SACL set. kernelbase opened the
file with `SYNCHRONIZE`, so a DACL denying `FILE_GENERIC_WRITE` (which includes `SYNCHRONIZE`) locked even its owner
out.

## Decisions

1. **A stored descriptor may be kept verbatim** (`inherit::Stored::verbatim`): its DACL is shown exactly as stored,
   without inherited ACEs and without `SE_DACL_AUTO_INHERITED`.
2. **`NtSetSecurityObject` on a file decides by the descriptor's `SE_DACL_AUTO_INHERIT_REQ`.**
   - Without it, the DACL is stored verbatim, protected only if the descriptor is.
   - With it, it is stored as explicit ACEs that inherit as before. `SetSecurityInfo` (and so
     `SetNamedSecurityInfoW`) sets the flag.
   - Owner, group and label sets keep the mode the path had. Registry keys and other objects are unchanged.
3. **A new file or directory gets its creator's descriptor**
   (`ObjectManager::created_path_security`, called when the NT create makes a name):
   - owner and group from the creator's token;
   - where its directory passes nothing down, the DACL SYSTEM, Administrators and the creating user with full
     access, stored verbatim;
   - otherwise it keeps inheriting, and only an owner other than the default is stored.
4. **A file laid out in a verbatim directory that passes nothing down shows that directory's DACL, owner and
   group**: it got the same default when it was put there.
   - `C:\PolyTests` is seeded so: the user owns it; SYSTEM, Administrators and the user have full access;
     nothing is inherited.
   - This is the oracle's scratch directory: a test that changes its own image's DACL (`rmt_sb_appcontainer`)
     keeps access to it through its owner.
5. **The SACL information replaces audit ACEs only**; the label is kept unless the label information is given too
   (`sdops::merge`).
6. **`GetFileSecurityW` and `SetFileSecurityW` open the file for exactly the rights they need**, without
   `SYNCHRONIZE` (`file::open_for_security`).
   - The sheets' argument errors: `ERROR_INVALID_NAME` for a NULL or empty name, `ERROR_NOACCESS` for NULL
     pointers, `ERROR_UNKNOWN_REVISION` for a zeroed descriptor, `ERROR_ACCESS_DENIED` for no known part.
   - The ANSI forms convert the path.

## Consequences

- Visible tests: all of `SetFileSecurityW/A` and `GetFileSecurityW/A` pass, and so does `rmt_sb_appcontainer`.
- A directory's DACL set with `SetFileSecurityW` no longer changes the files in it; one set with
  `SetNamedSecurityInfoW` still does (the files keep inheriting unless they were given a verbatim DACL).
- A file made where the volume has no descriptor of its own (most of `C:`) now carries a stored descriptor: one
  entry per such file in the path table, also after the file is deleted (as for every path set before).
- Programs in other sandboxes do not read files a program made outside its profile unless the DACL lets them, as on
  Windows. Before, the flat default let every AppContainer read them.
