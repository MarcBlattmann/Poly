#!/usr/bin/env python3
"""Where the time of one key press goes: Poly's timeline (ADR 0247), per thread.

Boot with `poly.debug.timeline` (the kernel prints `timeline: ring at phys ... bytes ...`), do the
action, press Pause (the ring freezes), copy the ring out of guest memory with QEMU's `pmemsave`
(`grab` below, or the QA driver's `tlgrab`), press Pause again to go on. Then:

    timeline.py RING.bin SERIAL.log [--key N] [--ms 400] [--lanes 6] [--frames 3]

prints, for the N-th key press in the ring (default: the last one):

- the frames the display sent after it;
- for the first frames, the chain of wake-ups that led to them, back to the key: which thread
  ran, how long it waited for a CPU, how long it was blocked and in which system call, and
  which thread (in which call) woke it;
- every thread busy in the window: running, runnable (waiting for a CPU), blocked, its calls;
- the lanes of the threads on the chain, event by event.

Thread names come from the serial log (`timeline: name`, `timeline: proc`, `timeline: sys`).
"""

import argparse
import collections
import re
import struct
import sys

MAGIC = int.from_bytes(b'POLYTL01', 'little')
HEADER_BYTES = 64
RECORD_BYTES = 40
KERNEL_PID = 0xFFFFFFFF
KINDS = {1: 'switch', 2: 'wake', 3: 'enter', 4: 'exit', 5: 'key', 6: 'frame', 7: 'freeze', 8: 'thaw', 9: 'block', 10: 'sample'}
LEFT = {0: 'runnable', 1: 'blocked', 2: 'exited', 3: 'other'}
TABLES = {0: 'linux', 1: 'nt', 2: 'xnu', 3: 'native'}
IN_INTERRUPT = 1


def grab(qmp_cmd, serial_text, out_path):
    """Copy the ring out of guest memory: `qmp_cmd(name, **args)` runs a QMP command."""
    m = None
    for m in re.finditer(r'timeline: ring at phys (0x[0-9a-f]+) bytes (\d+)', serial_text):
        pass
    if m is None:
        raise SystemExit('no "timeline: ring" line: boot with poly.debug.timeline')
    return qmp_cmd('pmemsave', val=int(m.group(1), 16), size=int(m.group(2)), filename=out_path)


class Rec:
    __slots__ = ('index', 't', 'ident', 'a', 'b', 'kind', 'cpu', 'flags')

    def __init__(self, index, t, ident, a, b, kind, cpu, flags):
        self.index, self.t, self.ident, self.a, self.b = index, t, ident, a, b
        self.kind, self.cpu, self.flags = kind, cpu, flags


def read_ring(data):
    magic, version, capacity, nxt, frozen = struct.unpack_from('<5Q', data, 0)
    if magic != MAGIC or version != 1:
        raise SystemExit('not a timeline ring (wrong magic or version)')
    out = []
    for index in range(max(0, nxt - capacity), nxt):
        off = HEADER_BYTES + (index % capacity) * RECORD_BYTES
        t, ident, a, b, meta = struct.unpack_from('<5Q', data, off)
        if (meta >> 32) != (index + 1) & 0xFFFFFFFF:
            continue
        kind = KINDS.get(meta & 0xFF)
        if kind is None:
            continue
        out.append(Rec(index, t, ident, a, b, kind, (meta >> 8) & 0xFF, (meta >> 16) & 0xFFFF))
    out.sort(key=lambda r: (r.t, r.index))
    return out, frozen


class Names:
    def __init__(self, serial):
        self.thread, self.proc, self.sys, self.queue = {}, {}, {}, {}
        self.modules = collections.defaultdict(list)
        self.user32 = {}
        for line in serial.splitlines():
            i = line.find('timeline: ')
            if i < 0:
                continue
            rest = line[i + 10:]
            if rest.startswith('name '):
                parts = rest[5:].split(' ', 1)
                try:
                    self.thread[int(parts[0], 16)] = parts[1].strip() if len(parts) > 1 else ''
                except ValueError:
                    pass
            elif rest.startswith('proc '):
                parts = rest[5:].split(' ', 1)
                if parts[0].isdigit():
                    self.proc[int(parts[0])] = parts[1] if len(parts) > 1 else ''
            elif rest.startswith('module '):
                parts = rest[7:].split(' ', 3)
                if len(parts) == 4:
                    self.modules[int(parts[0])].append((int(parts[1], 16), int(parts[2], 16), parts[3].strip()))
            elif rest.startswith('user32 '):
                parts = rest[7:].split()
                if len(parts) == 2 and parts[0].isdigit():
                    self.user32[int(parts[0])] = parts[1]
            elif rest.startswith('queue '):
                parts = rest[6:].split()
                if len(parts) == 2:
                    self.queue[int(parts[0], 16)] = parts[1]
            elif rest.startswith('sys '):
                parts = rest[4:].split()
                if len(parts) == 3:
                    self.sys[(int(parts[0]), int(parts[1], 16) & (0x3FFF if parts[0] == '1' else ~0))] = parts[2]

    def role(self, pid):
        line = self.proc.get(pid)
        if line is None:
            return f'pid{pid}'
        m = re.search(r'--type=(\S+)', line)
        if m:
            role = m.group(1)
            sub = re.search(r'--utility-sub-type=(\S+)', line)
            if sub:
                role += ':' + sub.group(1).split('.')[-1]
            return role
        exe = line[1:].split('"', 1)[0] if line.startswith('"') else (line.split() or [line])[0]
        base = re.split(r'[\\/]', exe)[-1]
        return 'browser' if re.match(r'(chrome|chromium)', base, re.I) else base

    def module(self, pid, rip):
        for base, size, name in self.modules.get(pid, ()):
            if base <= rip < base + size:
                return name
        return 'kernel?' if rip >= 1 << 47 else '(no module)'

    def label(self, r):
        """A system call record's name; `PolyUserCall` with its request (`PolyUserCall:PeekMessage`)."""
        name = self.call(r.a)
        if name == 'PolyUserCall' and r.b in self.user32:
            name += ':' + self.user32[r.b]
        return name

    def who(self, ident):
        pid, tid = ident >> 32, ident & 0xFFFFFFFF
        name = self.thread.get(ident)
        if pid == KERNEL_PID:
            return f'kernel:{name or tid}'
        if ident == 0:
            return '?'
        return f'{self.role(pid)}/{name or tid}'

    def call(self, call):
        table, number = call >> 56, call & 0x00FFFFFFFFFFFFFF
        key = (table, number & 0x3FFF) if table == 1 else (table, number)
        return self.sys.get(key) or f'{TABLES.get(table, table)}#{number:#x}'


def idle(names, ident):
    return ident >> 32 == KERNEL_PID and names.thread.get(ident, '').startswith('idle')


class Thread:
    def __init__(self):
        self.state = None  # 'run', 'ready', 'blocked'
        self.since = None
        self.call = None  # (call, t) of the system call in progress
        self.why = None  # for 'ready': the wake record that made it runnable
        self.spans = []  # (state, start, end, detail)
        self.calls = collections.Counter()
        self.wakes = []  # (t, waker ident, waker call, in interrupt)
        self.detail = None
        self.queue = None  # the wait queue of its last block

    def to(self, state, t, detail=None):
        if self.state is not None and self.since is not None and t > self.since:
            self.spans.append((self.state, self.since, t, self.detail))
        self.state, self.since, self.detail = state, t, detail


def replay(recs, names):
    threads = collections.defaultdict(Thread)
    frames, keys = [], []
    for r in recs:
        if r.kind == 'switch':
            frm, to = threads[r.a], threads[r.ident]
            left = LEFT.get(r.b, 'other')
            if left == 'blocked':
                what = frm.call[0] if frm.call else 'kernel wait'
                if frm.queue is not None:
                    q = names.queue.get(frm.queue)
                    what += f' [{q}]' if q else ''
                    frm.queue = None
                frm.to('blocked', r.t, what)
            elif left == 'runnable':
                frm.to('ready', r.t, 'preempted')
            else:
                frm.to(None, r.t)
            to.to('run', r.t, f'cpu{r.cpu}')
        elif r.kind == 'wake':
            th = threads[r.ident]
            waker = threads.get(r.a)
            wcall = waker.call[0] if waker is not None and waker.call else None
            intr = bool(r.flags & IN_INTERRUPT)
            th.wakes.append((r.t, r.a, wcall, intr))
            if th.state != 'run':
                th.to('ready', r.t, ('woken', r.a, wcall, intr))
        elif r.kind == 'enter':
            th = threads[r.ident]
            th.call = (names.label(r), r.t)
            th.calls[th.call[0]] += 1
        elif r.kind == 'exit':
            threads[r.ident].call = None
        elif r.kind == 'block':
            threads[r.ident].queue = r.a
        elif r.kind == 'frame':
            frames.append(r)
        elif r.kind == 'key':
            keys.append(r)
    end = recs[-1].t if recs else 0
    for th in threads.values():
        th.to(None, end)
    return threads, frames, keys


def ms(ns):
    return f'{ns / 1e6:.2f}'


def clip(spans, a, b):
    for st, s, e, d in spans:
        if e <= a or s >= b:
            continue
        yield st, max(s, a), min(e, b), d


def chain(threads, names, ident, t, t0, limit=100000):
    """The wake-ups that led to `ident` running at `t`, back to `t0` (newest first): per step the
    thread, from when it was woken to `t`, its time running, waiting for a CPU and blocked, and
    what woke it."""
    steps = []
    while t > t0 and len(steps) < limit:
        th = threads.get(ident)
        if th is None:
            break
        start, cause = None, None
        for st, s, e, d in reversed(th.spans):
            if s >= t:
                continue
            if st == 'ready' and isinstance(d, tuple) and d[0] == 'woken':
                start, cause = s, d
                break
            if st == 'blocked':
                break
        if start is None:
            steps.append((ident, None, t, 'no wake in the ring (running since before)', 0, 0, 0))
            break
        run = sum(e - s for st, s, e, _ in clip(th.spans, start, t) if st == 'run')
        ready = sum(e - s for st, s, e, _ in clip(th.spans, start, t) if st == 'ready')
        blocked = sum(e - s for st, s, e, _ in clip(th.spans, start, t) if st == 'blocked')
        _, waker, wcall, intr = cause
        how = 'an interrupt' if intr else f'{names.who(waker)}' + (f' in {wcall}' if wcall else '')
        steps.append((ident, start, t, how, run, ready, blocked))
        if intr or waker == 0 or idle(names, waker):
            break
        ident, t = waker, start
    return steps


def show_chain(steps, names, t0, big_ns):
    """Print a chain: steps of `big_ns` or more one per line, runs of smaller ones folded."""
    per = collections.defaultdict(lambda: [0, 0, 0, 0])
    small = []

    def flush():
        if not small:
            return
        a, b = small[-1][1], small[0][2]
        who = collections.Counter(names.who(x[0]) for x in small)
        print(f'  +{ms(a - t0):>8} .. +{ms(b - t0):>8}  {len(small)} short hops: ' +
              ', '.join(f'{w} x{n}' for w, n in who.most_common(4)))
        small.clear()

    for st in steps:
        ident, start, end, how, run, ready, blocked = st
        if start is None:
            flush()
            print(f'  {names.who(ident)}: {how}')
            continue
        p = per[names.who(ident)]
        p[0] += run
        p[1] += ready
        p[2] += blocked
        p[3] += 1
        if end - start >= big_ns:
            flush()
            print(f'  +{ms(start - t0):>8} .. +{ms(end - t0):>8}  {names.who(ident):<42} run {ms(run):>6}  '
                  f'cpu-wait {ms(ready):>6}  blocked {ms(blocked):>6}  <- woken by {how}')
        else:
            small.append(st)
    flush()
    if steps and steps[-1][1] is not None:
        last = steps[-1]
        print(f'  (chain starts at +{ms(last[1] - t0)} ms: {names.who(last[0])} woken by {last[3]})')
    reached = steps and steps[-1][1] is not None and steps[-1][1] <= t0 + big_ns
    total = collections.Counter()
    for v in per.values():
        total['run'] += v[0]
        total['ready'] += v[1]
        total['blocked'] += v[2]
    print(f'  on this chain ({"back to the key" if reached else "not back to the key"}): run {ms(total["run"])}, '
          f'cpu-wait {ms(total["ready"])}, blocked {ms(total["blocked"])} ms; by thread:')
    for who, v in sorted(per.items(), key=lambda kv: -(kv[1][0] + kv[1][1] + kv[1][2]))[:8]:
        print(f'    {who:<46} run {ms(v[0]):>7}  cpu-wait {ms(v[1]):>7}  blocked {ms(v[2]):>7}  ({v[3]} steps)')
    return per


def summary(recs, threads, frames, downs, names, args):
    """One line per key press: the first frame after it, how big, and the lock waits until then."""
    print(f'  {"key":>4} {"at ms":>9} {"frame ms":>9} {"bytes":>9}  {"run":>7} {"cpu-wait":>8}  lock waits until the frame')
    firsts = []
    groups = collections.defaultdict(list)
    for k in downs:
        mode = ''
        nxt = [d.t for d in downs if d.t > k.t]
        end = min([k.t + int(args.ms * 1e6)] + nxt)
        f = next((f for f in frames if k.t <= f.t <= end and f.a >= args.min_bytes), None)
        if f is None:
            print(f'  {k.a:>4} {ms(k.t - recs[0].t):>9} {"-":>9}')
            continue
        firsts.append((f.t - k.t) / 1e6)
        groups[(k.a, mode)].append((f.t - k.t) / 1e6)
        if k.a not in (15, 29, 42, 54, 56, 97):
            groups[('other keys', mode)].append((f.t - k.t) / 1e6)
        run = ready = 0
        locks = collections.Counter()
        for ident, th in threads.items():
            if idle(names, ident):
                continue
            for st, s, e, d in clip(th.spans, k.t, f.t):
                if st == 'run':
                    run += e - s
                elif st == 'ready':
                    ready += e - s
                elif st == 'blocked' and '[' in d:
                    locks[d[d.index('[') + 1:-1]] += e - s
        lk = ', '.join(f'{q} {ms(v)}' for q, v in locks.most_common())
        print(f'  {k.a:>4} {ms(k.t - recs[0].t):>9} {ms(f.t - k.t):>9} {f.a:>9}  {ms(run):>7} {ms(ready):>8}  {lk}')
    for (code, mode), v in sorted(groups.items(), key=lambda kv: str(kv[0])):
        if len(v) > 1:
            v.sort()
            print(f'  key {code}: median {v[len(v) // 2]:.1f} ms, {v[0]:.1f}-{v[-1]:.1f} ms over {len(v)}')
    if firsts:
        firsts.sort()
        print(f'  first frame after a key: median {firsts[len(firsts) // 2]:.1f} ms, '
              f'{firsts[0]:.1f}-{firsts[-1]:.1f} ms over {len(firsts)} keys')


def calls(recs, names, match, t0, t1):
    """For the threads whose name has `match` in it: per system call, how often it ran in the window
    and the time inside it, running and not; and the time running outside calls (the program)."""
    idents = {r.ident for r in recs if match in names.who(r.ident) and not idle(names, r.ident)}
    total = {} if match == '' else None
    for ident in sorted(idents, key=names.who):
        inside = {}  # call -> [count, wall, run]
        user_run = 0
        cur = None  # (call, entered)
        running_since = None
        for r in recs:
            if r.t > t1:
                break
            if r.kind == 'switch' and r.ident == ident:
                running_since = r.t
            elif r.kind == 'switch' and r.a == ident and running_since is not None:
                a, b = max(running_since, t0), r.t
                if b > a:
                    if cur is None:
                        user_run += b - a
                    else:
                        inside.setdefault(cur[0], [0, 0, 0])[2] += b - a
                running_since = None
            elif r.kind == 'enter' and r.ident == ident:
                if running_since is not None and r.t > t0:
                    user_run += r.t - max(running_since, t0)
                    running_since = r.t
                cur = (names.label(r), r.t)
            elif r.kind == 'exit' and r.ident == ident and cur is not None:
                if r.t >= t0:
                    e = inside.setdefault(cur[0], [0, 0, 0])
                    e[0] += 1
                    e[1] += r.t - max(cur[1], t0)
                    if running_since is not None:
                        e[2] += r.t - max(running_since, t0)
                        running_since = r.t
                cur = None
        if total is not None:
            for name, v in inside.items():
                t = total.setdefault(name, [0, 0, 0])
                for i in range(3):
                    t[i] += v[i]
            total.setdefault('(the programs, outside calls)', [0, 0, 0])[2] += user_run
            continue
        tot_run = user_run + sum(v[2] for v in inside.values())
        print(f'\ncalls of {names.who(ident)} in the window: running {ms(tot_run)} ms, of which in its own code '
              f'{ms(user_run)} ms; in calls (count, ms inside, ms of it running):')
        for name, (n, wall, run) in sorted(inside.items(), key=lambda kv: -kv[1][2])[:12]:
            print(f'  {name:<40} {n:>6} {ms(wall):>9} {ms(run):>9}')
    if total is not None:
        print('\nall threads in the window, per call (count, ms inside, ms of it running):')
        for name, (n, wall, run) in sorted(total.items(), key=lambda kv: -kv[1][2])[:16]:
            print(f'  {name:<40} {n:>6} {ms(wall):>9} {ms(run):>9}')


def samples(recs, names, t0, t1, top):
    """The timer's samples of threads in user mode, by module: where the programs' own time went."""
    by_module = collections.Counter()
    by_thread = collections.defaultdict(collections.Counter)
    n = 0
    for r in recs:
        if r.kind != 'sample' or not t0 <= r.t <= t1:
            continue
        n += 1
        m = names.module(r.ident >> 32, r.a)
        by_module[m] += 1
        by_thread[names.who(r.ident)][m] += 1
    print(f'\nuser-mode samples in the window (1 per ms per CPU): {n}')
    for m, c in by_module.most_common(top):
        print(f'  {m:<36} {c:>5}  {100 * c / max(n, 1):5.1f}%')
    print('busiest threads, by module:')
    for who, ctr in sorted(by_thread.items(), key=lambda kv: -sum(kv[1].values()))[:8]:
        print(f'  {who:<44} {sum(ctr.values()):>5}: ' + ', '.join(f'{m} {c}' for m, c in ctr.most_common(4)))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('ring')
    ap.add_argument('serial')
    ap.add_argument('--key', type=int, default=-1, help='which key press (0 = first, -1 = last)')
    ap.add_argument('--ms', type=float, default=400, help='the window after the key')
    ap.add_argument('--frames', type=int, default=3, help='frames whose chains are shown')
    ap.add_argument('--lanes', type=int, default=8, help='threads whose lanes are shown')
    ap.add_argument('--top', type=int, default=25, help='threads in the summary')
    ap.add_argument('--summary', action='store_true', help='one line per key press')
    ap.add_argument('--min-bytes', type=int, default=0, help='--summary: the first frame of at least this size')
    ap.add_argument('--samples', action='store_true', help='user-mode samples by module')
    ap.add_argument('--calls', help='per system call, the time of the threads whose name has this in it')
    ap.add_argument('--lane', help='show the lanes of the threads whose name has this in it')
    ap.add_argument('--big', type=float, default=1.0, help='chain steps of this many ms or more get a line')
    args = ap.parse_args()

    data = open(args.ring, 'rb').read()
    serial = open(args.serial, 'rb').read().decode('utf-8', 'replace')
    names = Names(serial)
    recs, frozen = read_ring(data)
    if not recs:
        raise SystemExit('the ring is empty')
    threads, frames, keys = replay(recs, names)
    downs = [k for k in keys if k.b == 1]
    span_ms = (recs[-1].t - recs[0].t) / 1e6
    print(f'ring: {len(recs)} records over {span_ms:.0f} ms, {"frozen" if frozen else "running"}, '
          f'{len(downs)} key presses, {len(frames)} frames')
    if not downs:
        raise SystemExit('no key press in the ring')
    if args.summary:
        summary(recs, threads, frames, downs, names, args)
        return
    key = downs[args.key]
    t0 = key.t
    t1 = t0 + int(args.ms * 1e6)
    print(f'key {key.a} pressed at {ms(t0 - recs[0].t)} ms into the ring; window {args.ms:.0f} ms\n')

    after = [f for f in frames if t0 <= f.t <= t1]
    print('frames after the key (ms, bytes, by):')
    for f in after[:12]:
        print(f'  +{ms(f.t - t0):>8}  {f.a:>9}  {names.who(f.ident)}')
    print()

    on_chain = collections.OrderedDict()
    for f in after[:args.frames]:
        print(f'chain of the frame at +{ms(f.t - t0)} ms (newest first; run / waiting for a CPU / blocked):')
        steps = chain(threads, names, f.ident, f.t, t0)
        for st in steps:
            on_chain[st[0]] = True
        show_chain(steps, names, t0, int(args.big * 1e6))
        print()

    print(f'threads in the window (ms; the {args.top} busiest):')
    print(f'  {"thread":<46} {"run":>7} {"cpu-wait":>8} {"blocked":>8}  calls')
    rows = []
    for ident, th in threads.items():
        if idle(names, ident):
            continue
        tot = collections.Counter()
        blocked_in = collections.Counter()
        for st, s, e, d in clip(th.spans, t0, t1):
            tot[st] += e - s
            if st == 'blocked':
                blocked_in[d] += e - s
        if tot['run'] == 0 and tot['ready'] == 0:
            continue
        rows.append((tot['run'], ident, tot, blocked_in))
    rows.sort(key=lambda r: -r[0])
    for run, ident, tot, blocked_in in rows[:args.top]:
        top = ', '.join(f'{k} {ms(v)}' for k, v in blocked_in.most_common(2))
        print(f'  {names.who(ident):<46} {ms(run):>7} {ms(tot["ready"]):>8} {ms(tot["blocked"]):>8}  {top}')
    blocked_by = collections.Counter()
    blocks = collections.Counter()
    for ident, th in threads.items():
        for st, s, e, d in clip(th.spans, t0, t1):
            if st == 'blocked':
                blocked_by[d] += e - s
                blocks[d] += 1
    print('\nblocked, all threads, by call (ms, times):')
    for d, v in blocked_by.most_common(14):
        print(f'  {d:<52} {ms(v):>9}  {blocks[d]:>6}')
    busy = collections.Counter()
    for ident, th in threads.items():
        if idle(names, ident):
            continue
        for st, s, e, d in clip(th.spans, t0, t1):
            if st == 'run':
                busy[d] += e - s
    print('\nCPU busy in the window: ' + ', '.join(f'{c} {100 * v / (t1 - t0):.0f}%' for c, v in sorted(busy.items())))

    if args.samples:
        samples(recs, names, t0, t1, 16)
    if args.calls is not None:
        calls(recs, names, args.calls, t0, t1)
    lanes = list(on_chain)[:args.lanes]
    if args.lane:
        lanes = [i for i in threads if args.lane in names.who(i)]
    for ident in lanes:
        th = threads[ident]
        print(f'\nlane {names.who(ident)}:')
        for st, s, e, d in clip(th.spans, t0, t1):
            if st == 'ready' and isinstance(d, tuple):
                _, waker, wcall, intr = d
                d = 'woken by ' + ('an interrupt' if intr else names.who(waker) + (f' in {wcall}' if wcall else ''))
            print(f'  +{ms(s - t0):>8} {ms(e - s):>7} {st:<8} {d}')


if __name__ == '__main__':
    main()
