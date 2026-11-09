#!/usr/bin/env python3
"""Write the committed traces (traces/s*.trace.json) and windows.json from the merged runs and the
drivers' result files. Usage: make_artifacts.py <work dir with tr/ and kit/> <out dir>"""
import json, os, subprocess, sys, hashlib

work, out = sys.argv[1], sys.argv[2]
tr = os.path.join(work, 'tr')
kit = json.load(open(os.path.join(work, 'kit', 'polytrace-kit.json'), encoding='utf-8-sig'))

APP = {
    'name': 'Chrome for Testing', 'version': '157.0.8103.38', 'channel': 'Stable', 'platform': 'win64',
    'url': 'https://storage.googleapis.com/chrome-for-testing-public/157.0.8103.38/win64/chrome-win64.zip',
    'zip_sha256': 'ae0deef27d449e1d7529df7705d609bab102de48464da13567ed3dd2ee1a0586',
}
PC = {
    'machine': 'owner PC (oracle, ADR 0075)', 'os': 'Windows 11 Enterprise 25H2, build 26200.9711',
    'cpu': 'AMD Ryzen AI Max+ PRO 395, 32 logical processors, 96 GB', 'ansi_code_page': 1252,
    'session': 'interactive session, headless stages',
}
HS2 = {
    'machine': 'homeserver2 (oracle over SSH, ADR 0097)', 'os': 'Windows 11 Home 25H2, build 26200.9711 (ProductName reads "Windows 10")',
    'cpu': 'Intel Core i5-9500T, 6 logical processors, 7.8 GB, UHD Graphics 630 1920x1080', 'ansi_code_page': 1252,
    'time_zone': 'W. Europe Standard Time', 'session': 'interactive session 1 of user server (scheduled task, ADR 0075)',
}
STAGES = {
    's1': ('headless --no-sandbox --disable-gpu --dump-dom pages/s1.html', PC, 's1'),
    's2': ('headless --disable-gpu --dump-dom pages/s1.html (SANDBOX ON; no renderer could be traced, see README)', PC, 's2'),
    's3': ('visible window, --no-sandbox (traced renderers cannot run sandboxed) --disable-gpu --no-first-run, pages/s3.html, mouse + keys + WM_CLOSE', HS2, 's3'),
    's4': ('headless --no-sandbox --disable-gpu --dump-dom pages/s4.html (http + https frames)', PC, 's4'),
    's4-window': ('visible window, --no-sandbox --disable-gpu --no-first-run, http://example.com/ then https://www.iana.org/ through the address bar', HS2, 's4h'),
    's5': ('headless --no-sandbox (GPU flags at their defaults) --dump-dom pages/s1.html', PC, 's5'),
}
kitinfo = {'format': kit['format'], 'mode': kit['mode'], 'files_sha256': kit['files'],
           'proxies': len(kit['proxies']), 'proxied_functions': sum(len(p['functions']) for p in kit['proxies']),
           'untraced_dlls': kit['untraced_dlls'], 'programs': kit['programs']}
for stage, (cmd, system, key) in STAGES.items():
    merged = os.path.join(tr, key + '.merged.json')
    if not os.path.exists(merged):
        print('skip', stage)
        continue
    meta = {'app': APP, 'system': system, 'command': cmd, 'kit': kitinfo,
            'recorded_by': 'spec-chrome-win', 'date': '2026-11-09', 'runs': 2}
    mp = os.path.join(tr, key + '.meta.json')
    json.dump(meta, open(mp, 'w'))
    subprocess.check_call([sys.executable, '-I', os.path.join(os.path.dirname(os.path.abspath(__file__)), 'chrometrace.py'),
                           'canon', os.path.join(out, 'traces', stage + '.trace.json'), stage, merged, mp])
