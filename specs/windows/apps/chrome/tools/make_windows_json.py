#!/usr/bin/env python3
"""windows.json: the recorded scenario results. Usage: make_windows_json.py <work> <hs2 work> <out>"""
import json, os, re, sys, hashlib

work, hs2, out = sys.argv[1], sys.argv[2], sys.argv[3]


def sha(p):
    return hashlib.sha256(open(p, 'rb').read()).hexdigest()


def result(p):
    raw = open(p, 'rb').read()
    txt = raw.decode('cp1252') if b'\x96' in raw else raw.decode('utf-8-sig')
    d = json.loads(txt)
    for k in list(d):
        if isinstance(d[k], str):
            d[k] = d[k].replace('–', '-')  # keep it ASCII here; the real title has an en dash
    return d


res = {'format': 'poly-chromerecording/1', 'app': {'version': '157.0.8103.38', 'zip_sha256': 'ae0deef27d449e1d7529df7705d609bab102de48464da13567ed3dd2ee1a0586'},
       'pages': {n: sha(os.path.join(work, 'pages', n)) for n in ('s1.html', 's3.html', 's4.html')},
       'note': 'titles: the real window title has an EN DASH (U+2013) before "Google Chrome for Testing"; written as "-" here',
       'stages': {}}
tr = os.path.join(work, 'tr')
for st, dirs in (('s1', ['s1orig-1', 's1orig-2']), ('s2', ['s2orig-1', 's2orig-2'])):
    runs = []
    for d in dirs:
        p = os.path.join(tr, d, 'stdout.txt')
        runs.append({'untraced': True, 'exit_code': 0, 'stdout_bytes': os.path.getsize(p), 'stdout_sha256': sha(p)})
    res['stages'][st] = {'machine': 'owner PC', 'runs': runs}
for st, dirs in (('s4-headless', ['s4-run1', 's4-run2']), ('s5', ['s5-run1', 's5-run2']), ('s1-traced', ['s1-run1', 's1-run2'])):
    runs = []
    for d in dirs:
        p = os.path.join(tr, d, 'stdout.txt')
        runs.append({'traced': True, 'exit_code': 0, 'stdout_bytes': os.path.getsize(p), 'stdout_sha256': sha(p)})
    res['stages'][st] = {'machine': 'owner PC', 'runs': runs}
for st in ('s3', 's4'):
    runs = []
    for kind in ('orig', 'chrome'):
        for r in (1, 2):
            p = os.path.join(hs2, 'tr', '%s-%s-run%d' % (st, kind, r), 'result.json')
            if os.path.exists(p):
                d = result(p)
                d['traced'] = (kind == 'chrome')
                runs.append(d)
    res['stages'][st + '-window'] = {'machine': 'homeserver2', 'runs': runs}
ab = os.path.join(tr, 'ablate.log')
if os.path.exists(ab):
    rows = []
    for line in open(ab, encoding='utf-8', errors='replace'):
        m = re.match(r'(\S+)\s+exit=(-?\d+) timeout=(\w+) dom_ok=(\w+)', line)
        if m:
            rows.append({'dll': m.group(1), 'exit_code': int(m.group(2)), 'timeout': m.group(3) == 'True', 'dom_ok': m.group(4) == 'True'})
    res['ablation_s1_delay_loaded_dll_missing'] = rows
json.dump(res, open(out, 'w', encoding='utf-8', newline='\n'), indent=1)
print(out, os.path.getsize(out))
