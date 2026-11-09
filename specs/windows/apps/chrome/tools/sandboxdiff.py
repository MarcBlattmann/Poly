#!/usr/bin/env python3
"""What a sandbox does to a Chrome child, from two kinds of runs of the same page.

  sandboxdiff.py reduce <unsandboxed-run.json>... --sandboxed <sandboxed-run.json>... <out.json>

Inputs are `chrometrace.py run` outputs (one per run) of the counting tracer (tools/wintrace/polytrace-agg):
the unsandboxed runs are S1 (`--no-sandbox`), the sandboxed ones S2 (renderers created by the browser with the
sandbox's restricted token, job and mitigations; see the gap map, section 4a). For every process type that exists
in both, the output lists, keeping only what **every sandboxed run** shows:

* `denied`      calls the sandboxed process made that Windows refused (a sampled call whose result is a failure and
                whose last error / NTSTATUS says "access denied" or "privilege not held" and so on), with the object
                it was given, and whether the same call (same object) succeeded unsandboxed;
* `failed`      every other failing sampled call (error code, object), with the same flag;
* `gone`        functions the unsandboxed process called and the sandboxed one did not;
* `new`         functions only the sandboxed process called;
* `outcome`     functions whose result mix changed: they never succeeded sandboxed but did unsandboxed, or got an
                error code the unsandboxed runs never had.

A sampled call is one of the first four calls of a function and every call with a name the function had not seen
before (paths, key names, DLL names, named objects): that is what the tracer writes in full. A function the tracer
only counted has its results in `outcome` (zero / not-zero counters and the first distinct last errors).
"""
import collections
import json
import re
import sys

sys.path.insert(0, __file__.rsplit('/', 1)[0] if '/' in __file__ else '.')
import chrometrace as ct  # noqa: E402

# winerror.h
ERR_NAMES = {
    2: 'FILE_NOT_FOUND', 3: 'PATH_NOT_FOUND', 5: 'ACCESS_DENIED', 6: 'INVALID_HANDLE', 21: 'NOT_READY', 32: 'SHARING_VIOLATION',
    50: 'NOT_SUPPORTED', 87: 'INVALID_PARAMETER', 122: 'INSUFFICIENT_BUFFER', 126: 'MOD_NOT_FOUND', 127: 'PROC_NOT_FOUND',
    193: 'BAD_EXE_FORMAT', 232: 'NO_DATA', 259: 'NO_MORE_ITEMS', 267: 'DIRECTORY', 534: 'ARITHMETIC_OVERFLOW',
    740: 'ELEVATION_REQUIRED', 998: 'NOACCESS', 1008: 'NO_TOKEN', 1260: 'ACCESS_DISABLED_BY_POLICY', 1314: 'PRIVILEGE_NOT_HELD',
    1346: 'BAD_IMPERSONATION_LEVEL', 1347: 'CANT_OPEN_ANONYMOUS', 1349: 'BAD_INHERITANCE_ACL', 1350: 'NO_SECURITY_ON_OBJECT',
    1722: 'RPC_S_SERVER_UNAVAILABLE', 5023: 'CLUSTER_...(5023)', 1789: 'TRUSTED_DOMAIN_FAILURE', 4317: 'INVALID_OPERATION',
    0x80070005: 'E_ACCESSDENIED',
}
# status codes (ntstatus.h) for Nt* functions
NT_NAMES = {
    0xC0000022: 'ACCESS_DENIED', 0xC0000061: 'PRIVILEGE_NOT_HELD', 0xC0000034: 'OBJECT_NAME_NOT_FOUND',
    0xC000003A: 'OBJECT_PATH_NOT_FOUND', 0xC0000008: 'INVALID_HANDLE', 0xC000000D: 'INVALID_PARAMETER',
    0xC0000035: 'OBJECT_NAME_COLLISION', 0xC0000043: 'SHARING_VIOLATION', 0xC00000BB: 'NOT_SUPPORTED',
    0xC0000005: 'ACCESS_VIOLATION', 0xC0000024: 'OBJECT_TYPE_MISMATCH', 0xC0000101: 'DIRECTORY_NOT_EMPTY',
    0xC000007C: 'NO_TOKEN', 0xC0000121: 'CANNOT_DELETE', 0xC00000A2: 'MEDIA_WRITE_PROTECTED',
    0xC0000428: 'INVALID_IMAGE_HASH', 0xC0000142: 'DLL_INIT_FAILED',
}
DENY = {5, 1314, 1260, 1346, 1347, 1349, 740, 998, 0x80070005, 0xC0000022, 0xC0000061, 0xC0000428}
REG_FNS = ('RegOpenKeyExW', 'RegOpenKeyExA', 'RegCreateKeyExW', 'RegQueryValueExW', 'RegQueryValueExA', 'RegGetValueW',
           'RegSetValueExW', 'RegEnumKeyExW', 'RegEnumValueW', 'RegDeleteKeyExW', 'RegDeleteValueW', 'RegNotifyChangeKeyValue',
           'RegQueryInfoKeyW', 'RegCloseKey', 'RegConnectRegistryW')


def num(x):
    if isinstance(x, str) and x.startswith('0x'):
        return int(x, 16)
    return x if isinstance(x, int) else None


def verdict(s):
    """(failed?, code, label) for one sampled record."""
    fn, ret, err = s['fn'], num(s.get('ret')), s.get('err')
    if ret is None:
        return False, None, ''
    if fn.startswith('Nt') or fn.startswith('Zw'):
        if isinstance(ret, int) and ret >= 0xC0000000:
            return True, ret, NT_NAMES.get(ret, '0x%08X' % ret)
        return False, None, ''
    if fn in REG_FNS:
        if isinstance(ret, int) and ret != 0 and ret < 0x10000:      # ERROR_SUCCESS == 0, otherwise the error itself
            return True, ret, ERR_NAMES.get(ret, str(ret))
        return False, None, ''
    failed = ret == 0 or ret == 0xFFFFFFFF or ret == 0xFFFFFFFFFFFFFFFF
    if failed and err:
        return True, err, ERR_NAMES.get(err, str(err))
    return False, None, ''


def collect(runs):
    """Per process type, per run: functions (counters) and failing samples, objects that worked."""
    by_type = collections.defaultdict(list)
    for r in runs:
        d = json.load(open(r))
        per = collections.defaultdict(lambda: {'funcs': {}, 'fails': collections.Counter(), 'works': set(), 'procs': 0})
        for p in d['processes']:
            t = per[p['type']]
            t['procs'] += 1
            for k, f in p['funcs'].items():
                a = t['funcs'].setdefault(k, {'n': 0, 'z': 0, 'nz': 0, 'm1': 0, 'errs': set()})
                for c in ('n', 'z', 'nz', 'm1'):
                    a[c] += f[c]
                a['errs'] |= {int(c) for c, _ in f['errs']}
            for s in p['samples']:
                obj = ct.sample_object(s)
                failed, code, label = verdict(s)
                key = (s['dll'], s['fn'], obj[1] if obj else '')
                if failed:
                    t['fails'][(key, code, label)] += 1
                else:
                    t['works'].add(key)
        for typ, t in per.items():
            by_type[typ].append(t)
    return by_type


def reduce(unsandboxed, sandboxed, out):
    plain = collect(unsandboxed)
    box = collect(sandboxed)
    res = {'format': 'poly-sandboxdiff/1', 'unsandboxed_runs': len(unsandboxed), 'sandboxed_runs': len(sandboxed), 'types': {}}
    for typ in sorted(set(box) & set(plain)):
        b, u = box[typ], plain[typ]
        bfun = set.intersection(*[set(r['funcs']) for r in b])
        ufun = set.union(*[set(r['funcs']) for r in u])
        ufun_all = set.intersection(*[set(r['funcs']) for r in u])
        entry = {'processes_per_run': [r['procs'] for r in b], 'unsandboxed_processes_per_run': [r['procs'] for r in u]}
        entry['gone'] = sorted(ufun_all - set.union(*[set(r['funcs']) for r in b]))
        entry['new'] = sorted(bfun - ufun)
        # failing samples every sandboxed run shows
        common = set.intersection(*[set(r['fails']) for r in b])
        uworks = set.union(*[r['works'] for r in u])
        ufails = set.union(*[set(r['fails']) for r in u])
        denied, failed = [], []
        for (key, code, label) in sorted(common):
            row = [key[0], key[1], key[2], code, label, key in uworks]
            (denied if code in DENY else failed).append(row)
        entry['denied'] = denied
        entry['failed'] = failed
        outcome = []
        for k in sorted(bfun & ufun_all):
            bz = [r['funcs'][k] for r in b]
            uz = [r['funcs'][k] for r in u]
            b_never = all(f['nz'] == 0 and f['n'] > 0 for f in bz)
            u_ever = any(f['nz'] > 0 for f in uz)
            new_errs = set.union(*[f['errs'] for f in bz]) - set.union(*[f['errs'] for f in uz])
            if (b_never and u_ever) or new_errs:
                outcome.append([k, 'never-ok-sandboxed' if (b_never and u_ever) else 'ok-ish',
                                sorted(ERR_NAMES.get(e, str(e)) for e in new_errs),
                                [min(f['n'] for f in bz), max(f['n'] for f in bz)]])
        entry['outcome'] = outcome
        entry['functions_sandboxed'] = len(bfun)
        entry['functions_unsandboxed'] = len(ufun_all)
        res['types'][typ] = entry
    text = json.dumps(res, indent=None, separators=(',', ':'))
    for pat, rep in ((r'(?i)marc\.blattmann', '<user>'), (r'(?i)server\.homeserver2', '<user>'), (r'(?i)homeserver2', '<machine>'),
                     (r'S-1-5-21-[0-9\-]+', 'S-1-5-21-<domain>')):
        text = re.sub(pat, rep, text)
    open(out, 'w', encoding='utf-8', newline='\n').write(text)
    for typ, e in res['types'].items():
        print(typ, 'funcs', e['functions_unsandboxed'], '->', e['functions_sandboxed'], 'denied', len(e['denied']), 'failed', len(e['failed']),
              'gone', len(e['gone']), 'new', len(e['new']), 'outcome', len(e['outcome']))


if __name__ == '__main__':
    a = sys.argv[1:]
    if len(a) >= 5 and a[0] == 'reduce' and '--sandboxed' in a:
        i = a.index('--sandboxed')
        reduce(a[1:i], a[i + 1:-1], a[-1])
    else:
        print(__doc__)
        sys.exit(2)
