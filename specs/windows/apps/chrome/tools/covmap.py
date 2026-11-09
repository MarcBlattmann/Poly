#!/usr/bin/env python3
"""Coverage lookup for (dll, function) pairs against Poly's coverage map (coverage/windows) and its tree.

state(dll, fn) -> (state, detail) with state one of
  green      verified by spec tests (coverage status green)
  claimed    claimed in the map (detail: claimant)  [+ 'in tree' when the name is defined in the DLL's sources]
  stub       a stub
  missing    an entry without status / no entry
  nolisted   no entry at all in the map (add one)
"""
import json, glob, os, re, sys

REPO = os.environ.get('POLY_REPO') or os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), *(['..'] * 5)))
API = os.path.join(REPO, 'coverage', 'windows', 'api')

_cache = {}


def entries(dllfile):
    if dllfile not in _cache:
        p = os.path.join(API, dllfile + '.json')
        _cache[dllfile] = {e['symbol']: e for e in json.load(open(p, encoding='utf-8'))} if os.path.exists(p) else None
    return _cache[dllfile]


# an API-set contract is hosted by these DLLs (Poly's kernelbase hosts every api-ms-win-core-*)
API_SET_HOSTS = ['kernelbase.dll', 'kernel32.dll', 'ntdll.dll', 'combase.dll', 'shcore.dll', 'powrprof.dll', 'ole32.dll']

# Poly's sources per DLL (a name that appears there is "in tree")
SRC = {
    'kernel32.dll': ['personalities/nt/win32/kernelbase/src', 'personalities/nt/win32/kernel32/src'],
    'kernelbase.dll': ['personalities/nt/win32/kernelbase/src'],
    'ntdll.dll': ['personalities/nt/ntdll/src', 'personalities/nt/src'],
    'advapi32.dll': ['personalities/nt/win32/advapi32/src'],
    'user32.dll': ['personalities/nt/win32/user32/src', 'personalities/nt/win32/win32u/src', 'personalities/nt/win32/gui/src'],
    'gdi32.dll': ['personalities/nt/win32/gdi32/src', 'personalities/nt/win32/win32u/src'],
    'ws2_32.dll': ['personalities/nt/win32/ws2_32/src'],
    'bcrypt.dll': ['personalities/nt/win32/bcrypt/src'],
    'crypt32.dll': ['personalities/nt/win32/crypt32/src'],
    'oleaut32.dll': ['personalities/nt/win32/oleaut32/src'],
    'winhttp.dll': ['personalities/nt/win32/winhttp/src'],
    'wininet.dll': ['personalities/nt/win32/wininet/src'],
}
_src_text = {}


def src_text(dll):
    if dll not in _src_text:
        buf = []
        for d in SRC.get(dll, []):
            root = os.path.join(REPO, d)
            for dp, _, fs in os.walk(root):
                for f in fs:
                    if f.endswith('.rs'):
                        try:
                            buf.append(open(os.path.join(dp, f), encoding='utf-8', errors='replace').read())
                        except OSError:
                            pass
        _src_text[dll] = '\n'.join(buf)
    return _src_text[dll]


def in_tree(dll, fn):
    t = src_text(dll)
    if not t:
        return False
    return re.search(r'\b' + re.escape(fn) + r'\b', t) is not None


_sys = {}


_svc = set()


def service_table():
    if not _svc:
        p = os.path.join(REPO, 'personalities', 'nt', 'src', 'services.rs')
        _svc.update(re.findall(r'"((?:Nt|Zw)\w+)"', open(p, encoding='utf-8', errors='replace').read()))
    return _svc


def syscall_state(fn):
    if not _sys:
        for f in ('ntoskrnl', 'win32k'):
            p = os.path.join(REPO, 'coverage', 'windows', 'syscall', f + '.json')
            if os.path.exists(p):
                for e in json.load(open(p, encoding='utf-8')):
                    _sys.setdefault(e['symbol'], e)
    e = _sys.get(fn)
    if e is None and fn.startswith('Zw'):
        e = _sys.get('Nt' + fn[2:])
    return e


WS2_ORD = {1: 'accept', 2: 'bind', 3: 'closesocket', 4: 'connect', 5: 'getpeername', 6: 'getsockname', 7: 'getsockopt',
           8: 'htonl', 9: 'htons', 10: 'ioctlsocket', 11: 'inet_addr', 12: 'inet_ntoa', 13: 'listen', 14: 'ntohl',
           15: 'ntohs', 16: 'recv', 17: 'recvfrom', 18: 'select', 19: 'send', 20: 'sendto', 21: 'setsockopt',
           22: 'shutdown', 23: 'socket', 111: 'WSAGetLastError', 112: 'WSASetLastError', 115: 'WSAStartup', 116: 'WSACleanup'}
OLE_ORD = {2: 'SysAllocString', 4: 'SysAllocStringLen', 6: 'SysFreeString', 7: 'SysReAllocString', 9: 'VariantClear',
           8: 'SysReAllocStringLen', 10: 'VariantCopy', 11: 'SysStringLen', 8: 'SysReAllocStringLen'}


def resolve(dll, fn):
    """The export name for an ordinal import where the ordinal is a documented/known one."""
    if fn.startswith('#'):
        n = int(fn[1:])
        if dll.lower() == 'ws2_32.dll' and n in WS2_ORD:
            return WS2_ORD[n]
        if dll.lower() == 'oleaut32.dll' and n in OLE_ORD:
            return OLE_ORD[n]
    return fn


def state(dll, fn):
    dll = dll.lower()
    fn = resolve(dll, fn)
    if dll == 'ntdll.dll' and (fn.startswith('Nt') or fn.startswith('Zw')):
        e = syscall_state(fn)
        tab = fn in service_table() or ('Nt' + fn[2:]) in service_table()
        st = (e or {}).get('status')
        if st == 'green':
            return 'green', 'syscall'
        if st == 'claimed' or tab:
            return 'claimed', ((e or {}).get('claimed_by', '') + (' ' if e else '') + 'syscall' + (' in service table' if tab else '')).strip()
        if st == 'stub':
            return 'stub', 'syscall'
        if e is not None:
            return 'missing', 'syscall'
    cands = [dll]
    if dll.startswith('api-ms-win') or dll.startswith('ext-ms-win'):
        cands = API_SET_HOSTS
    first_listed = None
    best = None
    rank = {'green': 4, 'claimed': 3, 'stub': 2, 'missing': 1}
    for c in cands:
        ents = entries(c)
        if ents is None:
            continue
        e = ents.get(fn)
        if e is None:
            continue
        if first_listed is None:
            first_listed = c
        st = e.get('status')
        if st == 'green':
            s, det = 'green', ''
        elif st == 'claimed':
            s, det = 'claimed', e.get('claimed_by', '')
        elif st == 'stub':
            s, det = 'stub', ''
        elif st == 'app-provided':
            s, det = 'missing', 'app-provided'
        else:
            s, det = 'missing', ''
        tree = in_tree(c, fn) or (c == 'kernelbase.dll' and in_tree('kernel32.dll', fn))
        if tree and s in ('missing',):
            s, det = 'missing', 'name appears in tree'
        elif tree:
            det = (det + ' in tree').strip()
        if best is None or rank[s] > rank[best[0]]:
            best = (s, det, c)
    if best is None:
        # the DLL may not have a map file at all
        if all(entries(c) is None for c in cands):
            return ('nolisted', 'no map file for ' + dll)
        return ('nolisted', 'no entry')
    return best[0], best[1]


if __name__ == '__main__':
    for a in sys.argv[1:]:
        d, f = a.split('!')
        print(a, state(d, f))
