"""chrome-qa: compositor --stats lines of a serial log as a compact table. stats.py LOG [FROM_LINE_MATCH]"""
import re, sys
lines = open(sys.argv[1], encoding='utf-8', errors='replace').read().splitlines()
keys = ['fps', 'compose_avg_ms', 'compose_max_ms', 'take_avg_ms', 'keys', 'key_to_frame_avg_ms', 'key_to_frame_max_ms']
for i, l in enumerate(lines):
    if 'poly-compositor: stats' not in l:
        if any(m in l for m in sys.argv[2:]):
            print(f'{i:6} {l[:160]}')
        continue
    kv = dict(re.findall(r'(\w+)=([\d.]+)', l))
    dmg = re.search(r'\(([\d.]+)%\)', l)
    print(f'{i:6} ' + ' '.join(f'{k[:14]}={kv.get(k, "-")}' for k in keys) + f' dmg={dmg.group(1) if dmg else "-"}%')
