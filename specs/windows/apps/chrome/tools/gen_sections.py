#!/usr/bin/env python3
"""Markdown fragments for the gap map: census, dynamic resolutions, loaded DLLs, window messages."""
import json, sys, os, collections, glob
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import covmap, chrometrace

work = sys.argv[1]
tr = os.path.join(work, 'tr')
imports = json.load(open(sys.argv[2]))   # pe_imports.py output
STAGES = [('s1', 's1'), ('s2', 's2'), ('s3', 's3'), ('s4 (headless)', 's4'), ('s4 (window)', 's4h'), ('s5', 's5'), ('s5 (window)', 's5w')]
out = {}

# --- census
types = ['browser', 'renderer', 'renderer:extension', 'renderer:webui', 'gpu-process', 'utility:network.mojom.NetworkService',
         'utility:storage.mojom.StorageService', 'utility:chrome.mojom.ProcessorMetrics', 'crashpad-handler']
short = {'renderer:extension': 'renderer (extension)', 'renderer:webui': 'renderer (top-chrome WebUI)',
         'utility:network.mojom.NetworkService': 'network service', 'utility:storage.mojom.StorageService': 'storage service',
         'utility:chrome.mojom.ProcessorMetrics': 'processor-metrics utility', 'gpu-process': 'GPU process', 'crashpad-handler': 'crashpad handler'}
lines = ['| Process type | ' + ' | '.join(n for n, _ in STAGES) + ' |', '|---|' + '---|' * len(STAGES)]
for t in types:
    cells = []
    for n, k in STAGES:
        m = json.load(open(os.path.join(tr, k + '.merged.json')))
        v = m['types'].get(t)
        cells.append('%s / %d' % ('+'.join(str(x) for x in v['processes_per_run']), len(v['stable'])) if v else '-')
    lines.append('| %s | %s |' % (short.get(t, t), ' | '.join(cells)))
out['census'] = '\n'.join(lines)

# --- dynamic resolutions: names given to GetProcAddress that are not imports
imp = set()
for f in ('chrome.exe', 'chrome.dll', 'chrome_elf.dll'):
    for kind in ('static', 'delay'):
        for dll, fs in imports[f][kind].items():
            imp |= set(fs)
dyn = collections.defaultdict(set)
libs = collections.defaultdict(set)
for n, k in STAGES:
    m = json.load(open(os.path.join(tr, k + '.merged.json')))
    for t, v in m['types'].items():
        L = 'R' if t.startswith('renderer') else {'browser': 'B', 'gpu-process': 'G', 'crashpad-handler': 'C'}.get(t, 'N' if 'Network' in t else 'S' if 'Storage' in t else 'M')
        for fn, name in v['objects']:
            if fn == 'GetProcAddress' and name not in imp and not name.startswith('#'):
                dyn[name].add(L)
            if fn.startswith('LoadLibrary') or fn.startswith('GetModuleHandle'):
                libs[name].add(L)
idx = collections.defaultdict(list)
for f in glob.glob(os.path.join(covmap.API, '*.json')):
    d = os.path.basename(f)[:-5]
    for e in json.load(open(f, encoding='utf-8')):
        idx[e['symbol']].append(d)
PREF = ['kernel32.dll', 'kernelbase.dll', 'ntdll.dll', 'user32.dll', 'advapi32.dll', 'gdi32.dll', 'shell32.dll', 'ole32.dll', 'win32u.dll']
rows = []
for name, ls in sorted(dyn.items()):
    d = idx.get(name, [])
    pref = [x for x in PREF if x in d] or d[:1]
    if pref:
        st, det = covmap.state(pref[0], name)
        rows.append((pref[0], name, st, det, ''.join(sorted(ls))))
    else:
        rows.append(('?', name, 'nolisted', 'no entry in any DLL file', ''.join(sorted(ls))))
sym = {'green': 'green', 'claimed': 'claimed', 'stub': 'stub', 'missing': 'MISSING', 'nolisted': 'NOT LISTED'}
lines = ['| DLL (map) | Name resolved with `GetProcAddress` | Poly | By |', '|---|---|---|---|']
for d, name, st, det, by in sorted(rows, key=lambda r: (r[2] != 'missing' and r[2] != 'nolisted', r[0], r[1])):
    lines.append('| %s | `%s` | %s | %s |' % (d, name, sym[st] + ((' (' + det + ')') if det else ''), by))
out['dynamic'] = '\n'.join(lines)
out['dynamic_count'] = collections.Counter(r[2] for r in rows)

# --- intentionally loaded modules (library names Chrome asked for), minus the tracer's own and machine noise
NOISE = ('pt_', '<app>\\chrome.dll', 'chrome.dll', 'polytrace')
keep = {}
for name, ls in libs.items():
    low = name.lower()
    if low.startswith('pt_') or 'polytrace' in low:
        continue
    keep[name] = ''.join(sorted(ls))
out['libs'] = keep

# --- window messages (S3, browser)
c = json.load(open(os.path.join(tr, 's3.merged.json')))
lines = ['| Message | Sent (to the window procedure) | Posted (from the queue) |', '|---|---:|---:|']
for mid, s0, s1, p0, p1 in c['types']['browser']['messages']:
    mid = int(mid)
    nm = chrometrace.wm_name(mid)
    lines.append('| %s (0x%X) | %s | %s |' % (nm, mid, ('%d-%d' % (s0, s1)) if s1 else '', ('%d-%d' % (p0, p1)) if p1 else ''))
out['messages'] = '\n'.join(lines)
out['messages_other'] = {t: [(chrometrace.wm_name(m[0]), m[1], m[3]) for m in v['messages']] for t, v in c['types'].items() if v['messages'] and t != 'browser'}
json.dump({k: (v if not isinstance(v, collections.Counter) else dict(v)) for k, v in out.items()}, open(os.path.join(tr, 'sections.json'), 'w'), indent=1)
for k in ('census',):
    print(out[k])
print(dict(out['dynamic_count']))
print(sorted(keep.items()))
