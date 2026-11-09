#!/usr/bin/env python3
"""Reduce the per-process traces of the counting API tracer (tools/wintrace/polytrace-agg) of one
Chrome run to a JSON summary, and combine runs into stable facts.

  chrometrace.py run   <trace-dir> <out.json>            one run -> processes with their called functions
  chrometrace.py merge <out.json> <run.json> <run.json>  two or more runs of one stage -> stable facts per process type
  chrometrace.py canon <out.json> <stage> <merged.json> <meta.json>   the committed form (poly-chrometrace/1)

Only calls Chrome makes through its import tables are seen; names resolved with GetProcAddress show
up as the strings given to it (in `samples`), not as calls.
"""
import json, os, re, sys, glob, collections


def classify(cmd):
    if not cmd:
        return 'unknown'
    m = re.search(r'--type=([a-z\-]+)', cmd)
    if not m:
        return 'browser'
    t = m.group(1)
    if t == 'utility':
        sub = re.search(r'--utility-sub-type=([A-Za-z0-9_.\-]+)', cmd)
        return 'utility:' + (sub.group(1) if sub else '?')
    if t == 'renderer':
        if '--extension-process' in cmd:
            return 'renderer:extension'
        if '--top-chrome-webui' in cmd:
            return 'renderer:webui'
        return 'renderer'
    return t


HOME = re.compile(r'[A-Za-z]:\\Users\\[^\\]+', re.I)


def norm_str(x):
    """Machine- and run-independent form of a path / name a call was given."""
    x = re.sub(r'[A-Za-z]:\\Users\\[^\\]+\\pc\\ud\\[^\\]+', '<profile>', x, flags=re.I)
    x = re.sub(r'[A-Za-z]:\\Users\\[^\\]+\\pc\\(chrome|orig|kit)', '<app>', x, flags=re.I)
    x = re.sub(r'[A-Za-z]:\\Users\\[^\\]+\\pc\\tr\\[^\\]+', '<trace>', x, flags=re.I)
    x = re.sub(r'[A-Za-z]:\\Users\\[^\\]+\\pc\\pages', '<pages>', x, flags=re.I)
    x = re.sub(r'[A-Za-z]:\\Users\\[^\\]+\\pc', '<work>', x, flags=re.I)
    x = re.sub(r'[A-Za-z]:\\Users\\MARC~1\.BLA', '<home>', x)
    x = HOME.sub('<home>', x)
    x = re.sub(r'\{[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\}', '{<guid>}', x)
    x = re.sub(r'S-1-5-21-[0-9\-]+', 'S-1-5-21-<domain>', x)
    x = re.sub(r'(?i)[0-9a-f]{12,}', '<hex>', x)
    x = re.sub(r'(?i)(?<![0-9a-z])[0-9a-f]{8,}(?![0-9a-z])', '<hex>', x)
    x = re.sub(r'\d{3,}', '<n>', x)
    return x


def norm_cmd(x):
    """A command line: the role of the process and the switch names (no values)."""
    flags = re.findall(r'(?:^|\s)(--?[A-Za-z][A-Za-z0-9\-]*)(?:=|\s|$)', x)
    typ = re.search(r'--type=([a-z\-]+)', x)
    sub = re.search(r'--utility-sub-type=([A-Za-z0-9_.\-]+)', x)
    return {'type': typ.group(1) if typ else 'browser', 'sub': sub.group(1) if sub else '', 'switches': sorted(set(flags))}


# function -> index of the argument that names the object it works on (None: not a string)
STRING_FNS = {
    'CreateFileW': 0, 'CreateFileA': 0, 'FindFirstFileW': 0, 'FindFirstFileExW': 0, 'FindFirstFileExA': 0,
    'GetFileAttributesW': 0, 'GetFileAttributesA': 0, 'GetFileAttributesExW': 0, 'CreateDirectoryW': 0,
    'DeleteFileW': 0, 'DeleteFileA': 0, 'RemoveDirectoryW': 0, 'MoveFileExW': 0, 'CopyFileW': 0,
    'ReplaceFileW': 0, 'GetFullPathNameW': 0, 'GetLongPathNameW': 0, 'GetShortPathNameW': 0,
    'SetCurrentDirectoryW': 0, 'CreateNamedPipeW': 0, 'WaitNamedPipeW': 0,
    'CreateEventW': 3, 'CreateEventA': 3, 'CreateMutexW': 2, 'CreateMutexA': 2, 'CreateSemaphoreW': 3,
    'CreateSemaphoreA': 3, 'OpenEventW': 2, 'OpenFileMappingW': 2, 'CreateFileMappingW': 5,
    'CreateFileMappingA': 5, 'CreateJobObjectW': 1, 'CreateWaitableTimerExW': 1,
    'RegOpenKeyExW': 1, 'RegOpenKeyExA': 1, 'RegCreateKeyExW': 1, 'RegQueryValueExW': 1,
    'RegQueryValueExA': 1, 'RegGetValueW': 1, 'RegSetValueExW': 1, 'RegDeleteValueW': 1,
    'RegDeleteKeyExW': 1, 'GetPrivateProfileStringW': 0,
    'LoadLibraryW': 0, 'LoadLibraryA': 0, 'LoadLibraryExW': 0, 'LoadLibraryExA': 0,
    'GetModuleHandleW': 0, 'GetModuleHandleA': 0, 'GetProcAddress': 1,
    'GetEnvironmentVariableW': 0, 'GetEnvironmentVariableA': 0, 'SetEnvironmentVariableW': 0,
    'CreateWindowExW': 1, 'RegisterWindowMessageW': 0, 'FindWindowW': 0, 'FindWindowExW': 2,
    'RegisterClipboardFormatW': 0, 'LoadCursorW': 1, 'CreateProcessW': 1, 'CreateProcessAsUserW': 2,
    'CreateProcessWithTokenW': 3, 'OpenServiceW': 1, 'LookupPrivilegeValueW': 1, 'LookupAccountNameW': 1,
    'GetNamedSecurityInfoW': 0, 'SetNamedSecurityInfoW': 0, 'ConvertStringSidToSidW': 0,
    'ConvertStringSecurityDescriptorToSecurityDescriptorW': 0, 'GetAddrInfoW': 0, 'getaddrinfo': 0,
    'GetLocaleInfoEx': 0,
}
ROOTS = {'0xffffffff80000000': 'HKCR', '0xffffffff80000001': 'HKCU', '0xffffffff80000002': 'HKLM',
         '0xffffffff80000003': 'HKU', '0xffffffff80000005': 'HKCC'}


def sample_object(s):
    """(function, normalised name) for a sampled record that names something, else None."""
    fn = s['fn']
    i = STRING_FNS.get(fn)
    if i is None:
        return None
    args = s.get('args', [])
    if i >= len(args):
        return None
    a = args[i]
    if isinstance(a, dict):
        v = a.get('w', a.get('a'))
    elif isinstance(a, int):
        v = '#%d' % a
    else:
        return None
    if v is None:
        return None
    if fn.startswith('CreateProcess'):
        c = norm_cmd(v)
        return (fn, c['type'] + ((':' + c['sub']) if c['sub'] else ''))
    if fn.startswith('Reg') and args and isinstance(args[0], str):
        return (fn, ROOTS.get(args[0], 'key') + '\\' + norm_str(v))
    return (fn, norm_str(v))


def read_process(path):
    proc = {'file': os.path.basename(path), 'funcs': {}, 'samples': [], 'cmd': '', 'pid': None, 'calls': 0, 'threads': 0, 'msgs': {}}
    stats = {}
    with open(path, encoding='utf-8', errors='replace') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                o = json.loads(line)
            except Exception:
                continue  # a cut-off last line
            if 'format' in o:
                proc['pid'] = o.get('pid')
                c = o.get('cmd')
                proc['cmd'] = c.get('w', '') if isinstance(c, dict) else (c or '')
            elif 'stat' in o:
                stats[(o['dll'], o['fn'])] = o
            elif 'msg' in o:
                proc['msgs'][o['id']] = o   # the last snapshot's counters
            elif 'snapshot' in o:
                proc['calls'] = o.get('calls', 0)
                proc['threads'] = o.get('threads', 0)
            elif 'seq' in o:
                proc['samples'].append(o)
    for (dll, fn), o in stats.items():
        proc['funcs'][dll + '!' + fn] = {
            'n': o['n'], 'first': o['first'], 'z': o['z'], 'nz': o['nz'], 'm1': o['m1'],
            'errs': o['errs'], 'rets': o['rets'],
        }
    # a function that was sampled but whose stat line was lost (killed before a snapshot)
    for s in proc['samples']:
        k = s['dll'] + '!' + s['fn']
        if k not in proc['funcs']:
            proc['funcs'][k] = {'n': 1, 'first': s['seq'], 'z': 0, 'nz': 0, 'm1': 0, 'errs': [], 'rets': [], 'partial': True}
    proc['type'] = classify(proc['cmd'])
    return proc


def run(tracedir, out):
    procs = []
    for p in sorted(glob.glob(os.path.join(tracedir, 'polytrace-*.jsonl'))):
        pr = read_process(p)
        if pr['pid'] is None:
            continue
        procs.append(pr)
    meta = {}
    for name in ('stdout.txt', 'stderr.txt', 'result.json'):
        fp = os.path.join(tracedir, name)
        if os.path.exists(fp):
            with open(fp, encoding='utf-8', errors='replace') as f:
                meta[name] = f.read()[:4000]
    json.dump({'dir': os.path.basename(tracedir.rstrip('/\\')), 'processes': procs, 'meta': meta}, open(out, 'w'))
    by = collections.Counter(p['type'] for p in procs)
    print(os.path.basename(tracedir), dict(by), 'functions:', len({k for p in procs for k in p['funcs']}))


def merge(out, runs):
    data = [json.load(open(r)) for r in runs]
    types = sorted({p['type'] for d in data for p in d['processes']})
    res = {'runs': len(data), 'types': {}}
    for t in types:
        per_run = []
        for d in data:
            procs = [p for p in d['processes'] if p['type'] == t]
            agg = {}
            for p in procs:
                order = sorted(p['funcs'].items(), key=lambda kv: kv[1]['first'])
                for rank, (k, v) in enumerate(order):
                    a = agg.setdefault(k, {'n': 0, 'z': 0, 'nz': 0, 'm1': 0, 'errs': {}, 'rank': rank, 'procs': 0})
                    a['n'] += v['n']; a['z'] += v['z']; a['nz'] += v['nz']; a['m1'] += v['m1']
                    a['rank'] = min(a['rank'], rank)
                    a['procs'] += 1
                    for code, c in v['errs']:
                        a['errs'][code] = a['errs'].get(code, 0) + c
            objs = {}
            for p in procs:
                for smp in p['samples']:
                    o = sample_object(smp)
                    if o:
                        objs[o] = objs.get(o, 0) + 1
            msgs = {}
            for p in procs:
                for mid, mo in p['msgs'].items():
                    a = msgs.setdefault(mid, {'sent': 0, 'posted': 0, 'first': mo['first']})
                    a['sent'] += mo['sent']; a['posted'] += mo['posted']; a['first'] = min(a['first'], mo['first'])
            per_run.append({'procs': len(procs), 'funcs': agg, 'objects': objs, 'msgs': msgs})
        stable = {}
        keys = set(per_run[0]['funcs'])
        for pr in per_run[1:]:
            keys &= set(pr['funcs'])
        union = set().union(*[set(pr['funcs']) for pr in per_run])
        for k in sorted(keys):
            vs = [pr['funcs'][k] for pr in per_run]
            stable[k] = {
                'n_min': min(v['n'] for v in vs), 'n_max': max(v['n'] for v in vs),
                'z_max': max(v['z'] for v in vs), 'nz_min': min(v['nz'] for v in vs), 'm1_max': max(v['m1'] for v in vs),
                'zero_always': all(v['z'] == 0 and v['m1'] == 0 for v in vs),
                'ok_never': all(v['nz'] == 0 for v in vs),
                'errs': sorted({int(c) for v in vs for c in v['errs']}),
                'rank': int(sum(v['rank'] for v in vs) / len(vs)),
            }
        okeys = set(per_run[0]['objects'])
        for pr in per_run[1:]:
            okeys &= set(pr['objects'])
        mkeys = set(per_run[0]['msgs'])
        for pr in per_run[1:]:
            mkeys &= set(pr['msgs'])
        messages = []
        for mid in sorted(mkeys, key=lambda k: min(pr['msgs'][k]['first'] for pr in per_run)):
            vs = [pr['msgs'][mid] for pr in per_run]
            messages.append([mid, min(v['sent'] for v in vs), max(v['sent'] for v in vs), min(v['posted'] for v in vs), max(v['posted'] for v in vs)])
        res['types'][t] = {
            'messages': messages,
            'processes_per_run': [pr['procs'] for pr in per_run],
            'stable': stable,
            'unstable': sorted(union - keys),
            'objects': sorted([list(k) for k in okeys]),
            'objects_unstable': len(set().union(*[set(pr['objects']) for pr in per_run]) - okeys),
        }
    json.dump(res, open(out, 'w'), indent=1, sort_keys=True)
    for t, v in res['types'].items():
        print(t, 'procs/run', v['processes_per_run'], 'stable funcs', len(v['stable']), 'unstable', len(v['unstable']))


WM = {0x1: 'WM_CREATE', 0x2: 'WM_DESTROY', 0x3: 'WM_MOVE', 0x5: 'WM_SIZE', 0x6: 'WM_ACTIVATE', 0x7: 'WM_SETFOCUS',
      0x8: 'WM_KILLFOCUS', 0xA: 'WM_ENABLE', 0xB: 'WM_SETREDRAW', 0xC: 'WM_SETTEXT', 0xD: 'WM_GETTEXT', 0xE: 'WM_GETTEXTLENGTH',
      0xF: 'WM_PAINT', 0x10: 'WM_CLOSE', 0x12: 'WM_QUIT', 0x14: 'WM_ERASEBKGND', 0x18: 'WM_SHOWWINDOW', 0x1C: 'WM_ACTIVATEAPP',
      0x1F: 'WM_CANCELMODE', 0x20: 'WM_SETCURSOR', 0x22: 'WM_CHILDACTIVATE', 0x3D: 'WM_GETOBJECT', 0x127: 'WM_CHANGEUISTATE', 0x21: 'WM_MOUSEACTIVATE', 0x24: 'WM_GETMINMAXINFO', 0x30: 'WM_SETFONT', 0x31: 'WM_GETFONT',
      0x46: 'WM_WINDOWPOSCHANGING', 0x47: 'WM_WINDOWPOSCHANGED', 0x4A: 'WM_COPYDATA', 0x4E: 'WM_NOTIFY', 0x7B: 'WM_CONTEXTMENU',
      0x7C: 'WM_STYLECHANGING', 0x7D: 'WM_STYLECHANGED', 0x7E: 'WM_DISPLAYCHANGE', 0x7F: 'WM_GETICON', 0x80: 'WM_SETICON',
      0x81: 'WM_NCCREATE', 0x82: 'WM_NCDESTROY', 0x83: 'WM_NCCALCSIZE', 0x84: 'WM_NCHITTEST', 0x85: 'WM_NCPAINT',
      0x86: 'WM_NCACTIVATE', 0x87: 'WM_GETDLGCODE', 0xA0: 'WM_NCMOUSEMOVE', 0xA1: 'WM_NCLBUTTONDOWN', 0xA2: 'WM_NCLBUTTONUP',
      0xFF: 'WM_INPUT', 0x100: 'WM_KEYDOWN', 0x101: 'WM_KEYUP', 0x102: 'WM_CHAR', 0x104: 'WM_SYSKEYDOWN', 0x105: 'WM_SYSKEYUP',
      0x106: 'WM_SYSCHAR', 0x109: 'WM_UNICHAR', 0x111: 'WM_COMMAND', 0x112: 'WM_SYSCOMMAND', 0x113: 'WM_TIMER',
      0x200: 'WM_MOUSEMOVE', 0x201: 'WM_LBUTTONDOWN', 0x202: 'WM_LBUTTONUP', 0x203: 'WM_LBUTTONDBLCLK', 0x204: 'WM_RBUTTONDOWN',
      0x205: 'WM_RBUTTONUP', 0x207: 'WM_MBUTTONDOWN', 0x208: 'WM_MBUTTONUP', 0x20A: 'WM_MOUSEWHEEL', 0x20E: 'WM_MOUSEHWHEEL',
      0x210: 'WM_PARENTNOTIFY', 0x214: 'WM_SIZING', 0x215: 'WM_CAPTURECHANGED', 0x216: 'WM_MOVING', 0x218: 'WM_POWERBROADCAST',
      0x219: 'WM_DEVICECHANGE', 0x281: 'WM_IME_SETCONTEXT', 0x282: 'WM_IME_NOTIFY', 0x2A3: 'WM_MOUSELEAVE', 0x2E0: 'WM_DPICHANGED',
      0x31A: 'WM_THEMECHANGED', 0x31F: 'WM_DWMNCRENDERINGCHANGED', 0x320: 'WM_DWMCOLORIZATIONCOLORCHANGED'}


def wm_name(i):
    i = int(i)
    if i in WM:
        return WM[i]
    if 0x400 <= i < 0x8000:
        return 'WM_USER+%d' % (i - 0x400)
    if 0xC000 <= i <= 0xFFFF:
        return 'registered (RegisterWindowMessage) 0x%X' % i
    return '0x%X' % i


def canon(out, stage, merged_path, meta_path):
    """The committed form of a stage's reduced trace (format poly-chrometrace/1)."""
    m = json.load(open(merged_path))
    try:
        meta = json.load(open(meta_path))
    except Exception:
        meta = {}
    res = {'format': 'poly-chrometrace/1', 'stage': stage, 'recorded': meta, 'runs': m['runs'], 'process_types': {}}
    for t in sorted(m['types']):
        v = m['types'][t]
        calls = []
        for key, f in sorted(v['stable'].items(), key=lambda kv: (kv[1]['rank'], kv[0])):
            dll, fn = key.split('!', 1)
            if f['ok_never']:
                ret = 'never-ok'
            elif f['zero_always']:
                ret = 'ok'
            else:
                ret = 'mixed'
            calls.append([dll, fn, f['n_min'], f['n_max'], ret, [c for c in f['errs'] if c], f['rank']])
        res['process_types'][t] = {
            'processes_per_run': v['processes_per_run'],
            'calls': calls,
            'objects': v['objects'],
            'messages': [[int(m[0]), wm_name(m[0])] + m[1:] for m in v.get('messages', [])],
            'unstable_calls': len(v['unstable']),
            'unstable_objects': v['objects_unstable'],
        }
    text = json.dumps(res, indent=None, separators=(',', ':'), sort_keys=False)
    # machine and user names are not part of the behaviour
    for pat, rep in ((r'(?i)marc\.blattmann', '<user>'), (r'(?i)server\.homeserver2', '<user>'), (r'(?i)homeserver2', '<machine>'),
                     (r'DG-CH-NBD-<n>', '<machine>'), (r'"WINDI"', '"<machine>"'), (r'(?i)harddiskvolume\d+', 'harddiskvolume<n>')):
        text = re.sub(pat, rep, text)
    with open(out, 'w', encoding='utf-8', newline='\n') as f:
        f.write(text)
    print(out, os.path.getsize(out), 'bytes')

if __name__ == '__main__':
    if len(sys.argv) >= 4 and sys.argv[1] == 'run':
        run(sys.argv[2], sys.argv[3])
    elif len(sys.argv) >= 5 and sys.argv[1] == 'merge':
        merge(sys.argv[2], sys.argv[3:])
    elif len(sys.argv) >= 6 and sys.argv[1] == 'canon':
        canon(sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5])
    else:
        print(__doc__)
        sys.exit(2)
