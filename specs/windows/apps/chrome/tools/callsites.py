#!/usr/bin/env python3
"""Reduce call-site traces of chrome_elf.dll / chrome.exe to the stable facts (spec-chrome-win-3).

Input: the `polytrace.jsonl` files `tools/wintrace/polytrace-sync` wrote (POLYTRACE_SYNC=1, so every record
has `ra`, `rm`, `ro`, `ac`, `rc` and `out`), one per run of S1 (`drivers/run_s1_callsites.ps1`), browser
process only. Output: `traces/s1-chrome-elf-init.trace.json` (`poly-callsite-trace/1`), a call-by-call
sequence that a run of the same kit on Poly can be compared against, keyed by the caller as module+offset.

    callsites.py reduce <out.json> <run1.jsonl> <run2.jsonl> [<run3.jsonl> ...] --kit <polytrace-kit.json>
                        [--machine "<text>"] [--date YYYY-MM-DD] [--state cold|warm] [--to AddVectoredExceptionHandler]
    callsites.py flow <out.json>          print the producers with consumers (markdown)
    callsites.py show <out.json> <first> <last>      print calls n=first..last
    callsites.py compare <out.json> <poly polytrace.jsonl> [--limit N]
                                      Poly's run of the same kit against the reference, call by call

What is normalised: values that depend on the run or the machine (pointers, handles, process ids, the
user's SID, the profile and application directories, the random part of the crashpad pipe name, the
environment). Pointers and handles become a class (`null`, `-1`, `stack`, `private`, `mapped`,
`image:<module>[+0x<offset>]`, `handle`, `int:<n>`); a class gets `<-n` when the value is what call `n` of
the sequence returned or wrote to an out parameter, so a NULL where Windows has `private<-597` stands out.
Anything that differs between two runs becomes `"<var>"`. The lock calls (`TryAcquireSRWLockExclusive`,
`AcquireSRWLockExclusive`, `ReleaseSRWLockExclusive`) and the allocator's `VirtualAlloc`/`VirtualFree` are not in
`calls`: how many chrome_elf makes varies by a dozen from run to run and where the allocator calls fall between the
others varies from machine to machine; they are summarised per call site in `noise_sites`.
"""
import hashlib
import json
import re
import sys
from collections import Counter, OrderedDict

APP = {"chrome_elf.dll", "chrome.exe", "chrome.dll"}
# Not in `calls`: how many chrome_elf makes varies by a dozen from run to run (the locks), and where the
# allocator's VirtualAlloc/VirtualFree fall between the other calls varies from run to run and machine to machine.
LOCKS = {"TryAcquireSRWLockExclusive", "AcquireSRWLockExclusive", "ReleaseSRWLockExclusive",
         "VirtualAlloc", "VirtualFree"}
VAR = "<var>"

BS = chr(92)
STATES = {
    "cold": (
        f"cold: before every run HKCU{BS}Software{BS}Chrome for Testing and HKCU{BS}Software{BS}Google{BS}Chrome for Testing "
        "did not exist (the state of a machine that never ran Chrome for Testing, and of Poly's registry at start); "
        "Chrome made them during the run"
    ),
    "warm": (
        "warm: both keys existed, written by earlier runs of Chrome for Testing (the blocklist beacon's state, the "
        f"ThirdParty status codes, Software{BS}Chrome for Testing{BS}usagestats); this is the state of a second run"
    ),
}

# Facts about the calls that matter to the Poly bring-up (the implementer's write-up
# `chrome-win-b4.md`, "What the real call chain says").
LANDMARKS = {
    ("VerifyVersionInfoW", "chrome_elf.dll+0x8e1cc"): (
        "Poly's S1 stops after this call: access violation reading [0x10] in the code that follows (leaf +0x8e9b4, "
        "called from +0x8e2a1, rcx = 0). On Windows the first thing after it is AcquireSRWLockExclusive at "
        "+0x1580a6, then GetProcAddress(advapi32 proxy, ConvertStringSecurityDescriptorToSecurityDescriptorW)."
    ),
    ("VerifyVersionInfoW", "chrome_elf.dll+0x1d83e"): "the first of three version checks; followed by the NtMapViewOfSection-hook installation (CreateMutexW, ReadProcessMemory, WriteProcessMemory) and the browser's crashpad set-up",
    ("CreateMutexW", "chrome_elf.dll+0x1d5e4"): "the first CreateMutexW of the process",
    ("CreateNamedPipeW", "chrome_elf.dll+0x8e243"): "the crashpad pipe, created after the SDDL string was converted",
    ("LoadLibraryExA", "chrome_elf.dll+0x158d2d"): "the delay-load helper loads the advapi32 proxy; GetProcAddress at +0x158e0a resolves each delay-loaded name",
}


# ----- normalising values -------------------------------------------------------------------

def ncls(c, links=None, value=None):
    """A class string without the parts that depend on the Windows build."""
    if c is None:
        return None
    m = re.match(r"image:([^+]+)(\+0x[0-9a-f]+)?$", c)
    if m:
        name, off = m.group(1), m.group(2) or ""
        return f"image:{name}{off}" if name in APP else f"image:{name}"
    return c


SID = re.compile(r"S-1-(?:5-21|12-1)-\d+(?:-\d+)+")


def nstr(s):
    """A string with the machine-specific parts replaced."""
    s = SID.sub("<user-sid>", s)
    s = re.sub(r"[A-Za-z]:[\\/]Users[\\/][^\\/\"]*[\\/]pc3", "<work>", s)
    s = re.sub(r"\\Device\\HarddiskVolume\d+\\Users\\[^\\]*\\pc3", "\\\\Device\\\\HarddiskVolume<n>\\\\<work>", s)
    s = re.sub(r"\\Device\\HarddiskVolume\d+", r"\\Device\\HarddiskVolume<n>", s)
    s = re.sub(r"<work>[\\/]appk", "<app>", s)
    s = re.sub(r"<work>[\\/]ud[\\/][^\\/ \"]+", "<profile>", s)
    s = re.sub(r"<work>[\\/]pages", "<pages>", s)
    s = re.sub(r"crashpad_\d+_[A-Z]{16}", "crashpad_<pid>_<rand16>", s)
    return s


def nout(o):
    """One `out` entry without raw values, normalised."""
    o = dict(o)
    o.pop("v", None)
    for k in ("w", "as", "us"):
        if k in o and isinstance(o[k], str):
            o[k] = nstr(o[k])
    if isinstance(o.get("us"), str) and "<" in o["us"]:
        o["len"] = o["max"] = VAR  # the byte length of a string that held a user SID or a path
    if "oa" in o:
        oa = dict(o["oa"])
        if isinstance(oa.get("name"), str):
            oa["name"] = nstr(oa["name"])
        o["oa"] = oa
    if "bc" in o:
        o["bc"] = ncls(o["bc"])
    if "c" in o:
        o["c"] = ncls(o["c"])
    if "env" in o:
        o["env"] = {"vars": VAR, "chars": VAR}  # the session's environment: not a fact about Chrome
    if "mbi" in o:
        o["mbi"] = {k: (ncls(v) if k in ("base", "allocbase") else v) for k, v in o["mbi"].items()}
    if "cs" in o:
        o["cs"] = {k: (ncls(v) if k in ("dbg", "owner", "sem") else v) for k, v in o["cs"].items()}
    if "kv" in o and isinstance(o["kv"].get("data"), str) and o["kv"].get("type") in (1, 2):
        o["kv"] = dict(o["kv"], data=nstr(o["kv"]["data"]))
    if "module" in o and o["module"] == "?":
        o["module"] = "<unnamed>"
    return o


def hexval(x):
    return int(x, 16) if isinstance(x, str) and x.startswith("0x") else None


class Run:
    """One run: its records up to the window's end, with producers resolved."""

    def __init__(self, path, to):
        rows = []
        with open(path, encoding="utf-8") as f:
            for line in f:
                if line.startswith('{"seq"'):
                    rows.append(json.loads(line))
        end = next((i for i, r in enumerate(rows) if r["fn"] == to and r["t"] == 0), len(rows) - 1)
        self.path = path
        self.reached_end = any(r["fn"] == to and r["t"] == 0 for r in rows)
        self.rows = rows[: end + 1]
        self.all_rows = len(rows)
        self.locks = [r for r in self.rows if r["fn"] in LOCKS]
        # The allocator reserves address space at a random hint and asks GetLastError when that fails
        # (ERROR_INVALID_ADDRESS): whether it fails differs from run to run; that GetLastError is noise too.
        def alloc_noise(i, r):
            if r["fn"] != "GetLastError":
                return False
            return any(
                q["fn"] == "VirtualAlloc" and q.get("ret") in (0, "0x0")
                for q in self.rows[max(0, i - 6) : i]
            )

        self.calls = [r for i, r in enumerate(self.rows) if r["fn"] not in LOCKS and not alloc_noise(i, r)]
        self.norm = self.normalise()

    def normalise(self):
        produced = {}  # value -> index of the call that returned it / wrote it to an out parameter
        out = []
        for n, r in enumerate(self.calls):
            ac = r.get("ac") or []
            args = []
            for j, a in enumerate(r["args"]):
                if isinstance(a, dict):  # a string argument
                    k, v = next(iter(a.items()))
                    args.append({k: nstr(v)})
                elif isinstance(a, str):  # a pointer or handle
                    cls = ncls(ac[j]) if j < len(ac) and ac[j] is not None else ("null" if hexval(a) == 0 else "?")
                    val = hexval(a)
                    if val in produced and cls not in ("null", "-1", "-2", "stack") and not cls.startswith("int:"):
                        cls += f"<-{produced[val]}"
                    args.append(cls)
                else:
                    args.append(a)
            ret = r.get("ret")
            rc = r.get("rc")
            if r["fn"] in ("GetCurrentProcessId", "GetCurrentThreadId"):
                ret = "<id>"
            if rc is not None:
                rcn = ncls(rc)
                if hexval(ret) not in (0, None) and rcn not in ("-1", "-2", "stack"):
                    produced[hexval(ret)] = n
                ret = rcn
            elif isinstance(ret, int) and r["dll"] == "ntdll.dll" and ret >= 0x80000000:
                ret = f"0x{ret:08x}"
            outs = []
            for o in r.get("out", []):
                v = hexval(o.get("v"))
                if v not in (None, 0) and not str(o.get("c", "")).startswith("stack"):
                    produced[v] = n
                no = nout(o)
                if r["fn"] in ("ReadProcessMemory", "WriteProcessMemory"):
                    # the bytes moved are a system DLL's code: only how many, not what
                    no.pop("b", None)
                outs.append(no)
            rec = OrderedDict()
            rec["fn"] = r["fn"]
            rec["dll"] = r["dll"]
            rec["at"] = f'{r["rm"]}+{r["ro"]}' if "rm" in r else f'?+ra:{r["ra"]}'
            rec["args"] = args
            rec["ret"] = ret
            if "err" in r:
                rec["err"] = r["err"]
            if outs:
                rec["out"] = outs
            out.append(rec)
        return out


def merge(a, b):
    """The fields equal in `a` and `b`; `<var>` for the others."""
    if isinstance(a, dict) and isinstance(b, dict):
        # a key one run has and another has not (`err`: only written when the call changed the last error,
        # which depends on what the call before left) differs, but the record stays a record
        keys = list(a) + [k for k in b if k not in a]
        merged = OrderedDict((k, merge(a.get(k, "<absent>"), b.get(k, "<absent>"))) for k in keys)
        return OrderedDict((k, v) for k, v in merged.items() if v != "<absent>")
    if isinstance(a, list) and isinstance(b, list) and len(a) == len(b):
        return [merge(x, y) for x, y in zip(a, b)]
    if a == b:
        return a
    # a link `<-n` to a different producer is a different fact; same class with other links too
    return VAR


def count_vars(x):
    if x == VAR:
        return 1
    if isinstance(x, dict):
        return sum(count_vars(v) for v in x.values())
    if isinstance(x, list):
        return sum(count_vars(v) for v in x)
    return 0


# ----- commands ---------------------------------------------------------------------------------

def sha256(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        h.update(f.read())
    return h.hexdigest()


def reduce_cmd(argv):
    out_path = argv[0]
    runs, kit, machine, date, to, state = [], None, "", "", "AddVectoredExceptionHandler", "cold"
    it = iter(argv[1:])
    for a in it:
        if a == "--kit":
            kit = next(it)
        elif a == "--machine":
            machine = next(it)
        elif a == "--date":
            date = next(it)
        elif a == "--to":
            to = next(it)
        elif a == "--state":
            state = next(it)
        else:
            runs.append(a)
    R = [Run(p, to) for p in runs]
    keys = [[(c["fn"], c.get("rm"), c.get("ro")) for c in r.calls] for r in R]
    if any(k != keys[0] for k in keys[1:]):
        for r, k in zip(R, keys):
            print(r.path, len(k), file=sys.stderr)
        sys.exit("the runs do not make the same calls in the same order")
    merged = R[0].norm
    for r in R[1:]:
        merged = [merge(x, y) for x, y in zip(merged, r.norm)]
    nvar = count_vars(merged)

    # lock call sites
    sites = {}
    for r in R:
        c = Counter((x["fn"], f'{x["rm"]}+{x["ro"]}') for x in r.locks)
        for k, v in c.items():
            sites.setdefault(k, []).append(v)
    lock_addr = {}
    for x in R[0].locks:
        ac = x.get("ac") or [None]
        lock_addr[(x["fn"], f'{x["rm"]}+{x["ro"]}')] = ncls(ac[0])
    noise_sites = [
        [fn, at, min(v + [0] * (len(R) - len(v))), max(v), lock_addr.get((fn, at))]
        for (fn, at), v in sorted(sites.items(), key=lambda kv: (kv[0][1], kv[0][0]))
    ]
    markers = {}
    for n, c in enumerate(merged):
        if c["fn"] in ("CreateMutexW", "ReadProcessMemory", "WriteProcessMemory", "SetUnhandledExceptionFilter",
                       "QueryPerformanceFrequency", "VerifyVersionInfoW", "CreateNamedPipeW", "CreateEventW",
                       "LoadLibraryExA", "ConvertStringSecurityDescriptorToSecurityDescriptorW",
                       "AddVectoredExceptionHandler", "NtCreateKey", "GetFileVersionInfoSizeW", "DuplicateHandle",
                       "GetCurrentProcessId", "BuildSecurityDescriptorW"):
            markers.setdefault(c["fn"], []).append(n)

    kitj = json.load(open(kit, encoding="utf-8")) if kit else {}
    doc = OrderedDict()
    doc["format"] = "poly-callsite-trace/1"
    doc["stage"] = "s1"
    doc["process"] = "browser"
    doc["recorded"] = OrderedDict(
        machine=machine,
        date=date,
        runs=len(R),
        command="chrome.exe --headless --no-sandbox --disable-gpu --user-data-dir=<profile> --dump-dom file:///<pages>/s1.html",
        env="POLYTRACE_SYNC=1 (dumps on), POLYTRACE_OUT=<file>; no other variable",
        logger="tools/wintrace/polytrace-sync (spec-chrome-win-3 version: caller module+offset, argument classes, out-buffer dumps)",
        kit_programs=kitj.get("programs"),
        kit_files={k: v for k, v in (kitj.get("files") or {}).items() if k in ("chrome.exe", "chrome_elf.dll", "polytrace.dll")},
        registry_state=STATES[state],
        stdout_sha256_every_run="7b79f8b475542cce0f029ef2459918c36404fe1986144f2fd1f2bf8aea13807b",
    )
    by_module = Counter(c["at"].split("+")[0] for c in merged)
    doc["window"] = OrderedDict(
        [
            ("from", "process start (the first call chrome_elf.dll makes through its imports)"),
            ("to", f"the first {to} (the end of chrome_elf's crash-reporting initialisation)"),
            ("calls", len(merged)),
            ("calls_by_caller_module", dict(by_module)),
            ("calls_per_run", [len(r.calls) for r in R]),
            ("ignored_calls_per_run", [len(r.locks) for r in R]),
            ("same_functions_and_sites_in_every_run", True),
            ("fields_that_differ_between_runs", nvar),
        ]
    )
    doc["how_to_compare"] = [
        "Run the same kit on Poly (chrome-win run --kit chrome_elf.dll,chrome.exe) and read polytrace.jsonl as this file's `calls`:",
        "call n of Poly's sequence (after dropping `ignored_functions`) must be the same function (`fn`) from the same caller (`at`,",
        "module+offset of the return address); `args`/`ret`/`out` are then compared field by field after the same normalisation",
        "(classes instead of values, `<-n` = the value call n returned or wrote). `<var>` fields differ between Windows runs and are not compared.",
        "The first call where Poly's `fn`/`at` differs, or where a class/link/status/out value differs, is the first divergence.",
    ]
    doc["machine_facts"] = [
        "out sysinfo (cpus, mask, revision), QueryPerformanceFrequency's u64, GetCommandLine* (paths normalised)",
        "are this machine's; they are in the file because Chrome reads them, not because Poly must equal them",
    ]
    doc["ignored_functions"] = sorted(LOCKS)
    doc["landmarks"] = [
        {"n": n, "what": what}
        for n, c in enumerate(merged)
        for (fn, at), what in LANDMARKS.items()
        if c["fn"] == fn and c["at"] == at
    ]
    doc["markers"] = markers
    doc["noise_sites"] = {"columns": ["fn", "at", "min_per_run", "max_per_run", "first_arg"], "rows": noise_sites}
    doc["calls"] = [OrderedDict([("n", n)] + list(c.items())) for n, c in enumerate(merged)]

    # one call per line, keeping the file readable and diffable
    head = {k: v for k, v in doc.items() if k != "calls"}
    text = json.dumps(head, indent=1, ensure_ascii=False)
    text = text[:-2] + ',\n "calls": [\n' + ",\n".join("  " + json.dumps(c, ensure_ascii=False, separators=(",", ":")) for c in doc["calls"]) + "\n ]\n}\n"
    with open(out_path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"{out_path}: {len(merged)} calls, {len(text)} bytes, {nvar} varying fields, {len(noise_sites)} noise sites")


def load(path):
    return json.load(open(path, encoding="utf-8"))


def flow_cmd(argv):
    doc = load(argv[0])
    calls = doc["calls"]
    cons = {}
    link = re.compile(r"<-(\d+)")

    def scan(n, v):
        if isinstance(v, str):
            for m in link.finditer(v):
                cons.setdefault(int(m.group(1)), []).append(n)
        elif isinstance(v, dict):
            for x in v.values():
                scan(n, x)
        elif isinstance(v, list):
            for x in v:
                scan(n, x)

    for c in calls:
        scan(c["n"], c["args"])
    print("| produced by (n, call at) | what | used later by |")
    print("|---|---|---|")
    for n in sorted(cons):
        c = calls[n]
        what = c["ret"] if isinstance(c["ret"], str) else ""
        outs = [o for o in c.get("out", []) if "a" in o and o["a"] != "ret" and ("c" in o or "h" in o)]
        w = f'ret {what}' if what else ""
        if outs:
            w += (" " if w else "") + "; ".join(f'arg{o["a"]} {o.get("c") or o.get("h")}' for o in outs)
        users = cons[n]
        cs = ", ".join(f'{u} {calls[u]["fn"]}' for u in users[:4]) + (f" (+{len(users) - 4})" if len(users) > 4 else "")
        print(f'| {n} {c["fn"]} `{c["at"]}` | {w} | {cs} |')


SKIP_KEYS = {"sysinfo", "u64", "env"}


def diff_fields(ref, got, path=""):
    """Differences between a reference value and Poly's: (path, windows, poly)."""
    if ref == VAR:
        return []
    if isinstance(ref, dict) and isinstance(got, dict):
        d = []
        for k in ref:
            if k in SKIP_KEYS:
                continue
            if k not in got:
                d.append((f"{path}.{k}", ref[k], "<absent>"))
            else:
                d += diff_fields(ref[k], got[k], f"{path}.{k}")
        for k in got:
            if k not in ref and k not in SKIP_KEYS:
                d.append((f"{path}.{k}", "<absent>", got[k]))
        return d
    if isinstance(ref, list) and isinstance(got, list):
        if len(ref) != len(got):
            return [(path + "[len]", len(ref), len(got))]
        d = []
        for i, (x, y) in enumerate(zip(ref, got)):
            d += diff_fields(x, y, f"{path}[{i}]")
        return d
    return [] if ref == got else [(path, ref, got)]


def compare_cmd(argv):
    """Compare a Poly run of the same kit with the reference, call by call."""
    ref = load(argv[0])
    poly = Run(argv[1], ref["window"]["to"].split()[2] if False else "AddVectoredExceptionHandler")
    limit = int(argv[argv.index("--limit") + 1]) if "--limit" in argv else 12
    calls = ref["calls"]
    got = poly.norm
    print(f"reference: {len(calls)} calls ({ref['recorded']['machine']}); poly: {len(got)} calls"
          f"{'' if poly.reached_end else ' (the run ends before AddVectoredExceptionHandler)'}")
    found = 0
    for n, g in enumerate(got):
        if n >= len(calls):
            print(f"call {n}: poly goes on after the reference's window: {g['fn']} at {g['at']}")
            break
        c = calls[n]
        if (g["fn"], g["at"]) != (c["fn"], c["at"]):
            where = "?+ra" in g["at"]
            print(f"call {n}: DIFFERENT CALL. windows {c['fn']} at {c['at']}; poly {g['fn']} at {g['at']}"
                  + ("  (poly's logger gave no module+offset: resolve ra with the crash report's module list)" if where else ""))
            print("  windows next:", ", ".join(f"{x['fn']}@{x['at']}" for x in calls[n : n + 4]))
            print("  poly    next:", ", ".join(f"{x['fn']}@{x['at']}" for x in got[n : n + 4]))
            found += 1
            break
        for field in ("ret", "err"):
            if field in c or field in g:
                for p, w, y in diff_fields(c.get(field), g.get(field), field):
                    print(f"call {n} {c['fn']} at {c['at']}: {p}: windows {json.dumps(w)} poly {json.dumps(y)}")
                    found += 1
        for field in ("args", "out"):
            for p, w, y in diff_fields(c.get(field, []), g.get(field, []), field):
                print(f"call {n} {c['fn']} at {c['at']}: {p}: windows {json.dumps(w, ensure_ascii=False)} poly {json.dumps(y, ensure_ascii=False)}")
                found += 1
        if found >= limit:
            print(f"(stopped after {found} differences)")
            return
    if not poly.reached_end and len(got) < len(calls):
        c = calls[len(got)]
        print(f"poly stops after call {len(got) - 1} ({got[-1]['fn']} at {got[-1]['at']}); windows goes on with "
              f"call {len(got)}: {c['fn']} at {c['at']}")
    if not found:
        print("no differences in the compared calls")


def show_cmd(argv):
    doc = load(argv[0])
    lo, hi = int(argv[1]), int(argv[2])
    for c in doc["calls"][lo : hi + 1]:
        print(json.dumps(c, ensure_ascii=False, separators=(",", ":")))


if __name__ == "__main__":
    cmds = {"reduce": reduce_cmd, "flow": flow_cmd, "show": show_cmd, "compare": compare_cmd}
    if len(sys.argv) < 3 or sys.argv[1] not in cmds:
        sys.exit(__doc__)
    cmds[sys.argv[1]](sys.argv[2:])
