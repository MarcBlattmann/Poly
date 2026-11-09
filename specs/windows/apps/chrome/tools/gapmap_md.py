#!/usr/bin/env python3
"""Markdown tables of the gap map: gapmap_md.py <out.md> s1=<merged.json> s2=<merged.json> ...

One table per area; a row per API Chrome called in every run of at least one stage."""
import json, os, sys, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import covmap, gapmap

LETTER = {'browser': 'B', 'gpu-process': 'G', 'crashpad-handler': 'C', 'utility:storage.mojom.StorageService': 'S',
          'utility:network.mojom.NetworkService': 'N', 'utility:chrome.mojom.ProcessorMetrics': 'M'}
ERR = {0: 'OK?', 2: 'FILE_NOT_FOUND', 3: 'PATH_NOT_FOUND', 5: 'ACCESS_DENIED', 6: 'INVALID_HANDLE', 18: 'NO_MORE_FILES',
       24: 'BAD_LENGTH', 32: 'SHARING_VIOLATION', 38: 'HANDLE_EOF', 50: 'NOT_SUPPORTED', 87: 'INVALID_PARAMETER',
       109: 'BROKEN_PIPE', 122: 'INSUFFICIENT_BUFFER', 126: 'MOD_NOT_FOUND', 127: 'PROC_NOT_FOUND', 158: 'NOT_LOCKED',
       183: 'ALREADY_EXISTS', 203: 'ENVVAR_NOT_FOUND', 232: 'NO_DATA', 234: 'MORE_DATA', 258: 'WAIT_TIMEOUT',
       259: 'NO_MORE_ITEMS', 267: 'DIRECTORY', 535: 'PIPE_CONNECTED', 536: 'PIPE_LISTENING', 995: 'OPERATION_ABORTED', 997: 'IO_PENDING',
       998: 'NOACCESS', 1008: 'NO_TOKEN', 1168: 'NOT_FOUND', 1314: 'PRIVILEGE_NOT_HELD', 1400: 'INVALID_WINDOW_HANDLE',
       1812: 'RESOURCE_DATA_NOT_FOUND', 1813: 'RESOURCE_TYPE_NOT_FOUND', 14007: 'SXS_KEY_NOT_FOUND', 2: 'FILE_NOT_FOUND'}


def letter(t):
    if t.startswith('renderer'):
        return 'R'
    return LETTER.get(t, '?')


AREAS_ORDER = ['process / thread / IPC', 'sync / TLS', 'memory', 'loader / exceptions / debug', 'file system',
               'registry', 'console / process environment', 'NLS / text', 'time', 'system information',
               'security: tokens / ACLs / SIDs', 'security: jobs', 'security: mitigations / process policy']


def main(out, stages):
    data = {}
    for name, path in stages:
        m = json.load(open(path))
        for t, v in m['types'].items():
            for key, f in v['stable'].items():
                dll, fn = key.split('!', 1)
                e = data.setdefault((dll, fn), {'stages': set(), 'by': set(), 'n': {}, 'errs': set(), 'z': 0, 'nz': 0, 'rank': 10**9})
                e['stages'].add(name)
                e['by'].add(letter(t))
                e['n'][name] = max(e['n'].get(name, 0), f['n_max'])
                e['errs'] |= set(f['errs'])
                e['z'] = max(e['z'], f.get('z_max', 0))
                e['nz'] += f.get('nz_min', 0)
                if t == 'browser':
                    e['rank'] = min(e['rank'], f['rank'])
    rows = []
    for (dll, fn), e in data.items():
        st, det = covmap.state(dll, fn)
        real = covmap.resolve(dll, fn)
        rows.append({'dll': dll, 'fn': real if real == fn else '%s (%s)' % (real, fn), 'raw': fn, 'area': gapmap.area(dll, real), 'state': st, 'detail': det, **e})
    stage_names = [n for n, _ in stages]
    by_area = collections.defaultdict(list)
    for r in rows:
        by_area[r['area']].append(r)
    order = [a for a in AREAS_ORDER if a in by_area] + sorted(a for a in by_area if a not in AREAS_ORDER)
    sym = {'green': 'green', 'claimed': 'claimed', 'stub': 'stub', 'missing': 'MISSING', 'nolisted': 'NOT LISTED'}
    lines = []
    w = lines.append
    tot = collections.Counter(r['state'] for r in rows)
    w('Totals over every stage: %d APIs; %s.\n' % (len(rows), ', '.join('%s %d' % (sym[k], v) for k, v in sorted(tot.items()))))
    for a in order:
        rs = sorted(by_area[a], key=lambda r: (r['dll'], r['fn'].lower()))
        cnt = collections.Counter(r['state'] for r in rs)
        w('### %s  (%d APIs: %s)\n' % (a, len(rs), ', '.join('%s %d' % (sym[k], v) for k, v in sorted(cnt.items()))))
        w('| DLL | API | Poly | By | %s | Max calls | Failures seen on Windows |' % ' '.join(s.upper() for s in stage_names))
        w('|---|---|---|---|%s|---:|---|' % ('|'.join(['---'] * len(stage_names))))
        for r in rs:
            fails = []
            errs = sorted(c for c in r['errs'] if c != 0)
            if errs:
                fails.append(', '.join(ERR.get(c, str(c)) for c in errs))
            if r['z'] and not errs:
                fails.append('returned 0/NULL %d x' % r['z'])
            st = sym[r['state']] + ((' (' + r['detail'] + ')') if r['detail'] else '')
            w('| %s | `%s` | %s | %s | %s | %s | %s |' % (
                r['dll'], r['fn'], st, ''.join(sorted(r['by'])),
                ' '.join('x' if s in r['stages'] else '.' for s in stage_names),
                max(r['n'].values()), '; '.join(fails)))
        w('')
    open(out, 'w', encoding='utf-8', newline='\n').write('\n'.join(lines))
    json.dump([{k: (sorted(v) if isinstance(v, set) else v) for k, v in r.items()} for r in rows],
              open(out + '.json', 'w'), indent=0)
    print(len(rows), 'APIs;', dict(tot))


if __name__ == '__main__':
    main(sys.argv[1], [tuple(a.split('=', 1)) for a in sys.argv[2:]])
