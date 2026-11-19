"""chrome-qa: drive a Poly desktop over QMP from a command file (runs inside polyd).

usage: python3 drive.py PORT CMDFILE SHOTDIR SERIAL_LOG

Lines appended to CMDFILE are run in order; results go to CMDFILE.out. A line `end` stops.
Several commands may share a line, separated by ' ; '.

Commands (screen pixels of the guest output, e.g. 2560x1600):
  shot NAME                      PNG screenshot SHOTDIR/NAME.png
  key QCODE | chord a+b+c        press keys (QEMU qcodes)
  type TEXT                      type US-layout text (gap 40 ms)
  typegap MS TEXT                type with a given gap between keys
  move X Y | click X Y | rclick X Y | dclick X Y
  down [btn] | up [btn]          button press / release (left default)
  drag X1 Y1 X2 Y2 STEPS MS      press, move in STEPS steps MS apart, release
  dragshots NAME X1 Y1 X2 Y2 STEPS MS  as drag, PNG during the move (half way) and while held
  wheel up|down N
  sleep S
  lat NAME X0 Y0 X1 Y1 TIMEOUT ACTION...   do ACTION (any one command), poll the region with
                                 PPM screendumps until it changes; then until it stays the same
                                 for 300 ms; report first change and settle in ms
  watch NAME X0 Y0 X1 Y1 SECS    poll the region for SECS; report each change time (flicker count)
  grep TEXT                      count/last lines of TEXT in the serial log
  tail N                         last N serial lines
  mark TEXT                      write a mark line to the .out
"""
import json, os, socket, sys, time

W, H = int(os.environ.get('QA_W', '2560')), int(os.environ.get('QA_H', '1600'))
SHIFTED = {'!': '1', '@': '2', '#': '3', '$': '4', '%': '5', '^': '6', '&': '7', '*': '8', '(': '9', ')': '0',
           '_': 'minus', '+': 'equal', '{': 'bracket_left', '}': 'bracket_right', '|': 'backslash', ':': 'semicolon',
           '"': 'apostrophe', '<': 'comma', '>': 'dot', '?': 'slash', '~': 'grave_accent'}
PLAIN = {' ': 'spc', '-': 'minus', '=': 'equal', '[': 'bracket_left', ']': 'bracket_right', '\\': 'backslash',
         ';': 'semicolon', "'": 'apostrophe', ',': 'comma', '.': 'dot', '/': 'slash', '`': 'grave_accent', '\n': 'ret'}
PPM = '/dev/shm/qa.ppm'


class Qmp:
    def __init__(self, port):
        for _ in range(900):
            try:
                self.s = socket.create_connection(('127.0.0.1', port), timeout=60)
                break
            except OSError:
                time.sleep(1)
        else:
            raise SystemExit('no QMP')
        self.f = self.s.makefile('rwb')
        self.f.readline()
        self.cmd('qmp_capabilities')

    def cmd(self, name, **args):
        msg = {'execute': name}
        if args:
            msg['arguments'] = args
        self.f.write(json.dumps(msg).encode() + b'\n')
        self.f.flush()
        while True:
            r = json.loads(self.f.readline())
            if 'return' in r or 'error' in r:
                return r

    def ev(self, *evs):
        return self.cmd('input-send-event', events=list(evs))


def key(q, down):
    return {'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': q}}}


def btn(b, down):
    return {'type': 'btn', 'data': {'down': down, 'button': b}}


def pos(x, y):
    return [{'type': 'abs', 'data': {'axis': 'x', 'value': round(int(x) * 0x7FFF / (W - 1))}},
            {'type': 'abs', 'data': {'axis': 'y', 'value': round(int(y) * 0x7FFF / (H - 1))}}]


class Driver:
    def __init__(self, q, shots, serial, out):
        self.q, self.shots, self.serial, self.out = q, shots, serial, out

    def say(self, s):
        line = f'{time.strftime("%H:%M:%S")} {s}'
        print(line, flush=True)
        with open(self.out, 'a') as f:
            f.write(line + '\n')

    def chord(self, *ks):
        self.q.ev(*[key(k, True) for k in ks])
        time.sleep(0.03)
        self.q.ev(*[key(k, False) for k in reversed(ks)])

    def typ(self, text, gap=0.04):
        for ch in text:
            if ch.isalpha() and ch.isupper():
                self.chord('shift', ch.lower())
            elif ch.isalnum():
                self.chord(ch)
            elif ch in SHIFTED:
                self.chord('shift', SHIFTED[ch])
            else:
                self.chord(PLAIN[ch])
            time.sleep(gap)

    def click(self, x, y, b='left', n=1):
        self.q.ev(*pos(x, y))
        time.sleep(0.05)
        for _ in range(n):
            self.q.ev(btn(b, True))
            time.sleep(0.05)
            self.q.ev(btn(b, False))
            time.sleep(0.08)

    def shot(self, name):
        p = os.path.join(self.shots, name + '.png')
        self.q.cmd('screendump', filename=p, format='png')
        return p

    def grab(self, box):
        """The region's bytes from a PPM screendump."""
        self.q.cmd('screendump', filename=PPM, format='ppm')
        with open(PPM, 'rb') as f:
            data = f.read()
        # header: P6\nW H\n255\n
        off, nl = 0, 0
        while nl < 3:
            off = data.index(b'\n', off) + 1
            nl += 1
        w = int(data[3:].split()[0])
        x0, y0, x1, y1 = box
        return b''.join(data[off + (y * w + x0) * 3: off + (y * w + x1) * 3] for y in range(y0, y1, 2))

    def lat(self, name, box, timeout, action):
        base = self.grab(box)
        t0 = time.monotonic()
        self.run_one(action)
        tsend = time.monotonic()
        first = None
        last_change = None
        prev = base
        polls = 0
        while time.monotonic() - t0 < timeout:
            cur = self.grab(box)
            polls += 1
            now = time.monotonic()
            if cur != prev:
                if first is None:
                    first = now
                last_change = now
                prev = cur
            elif first is not None and now - last_change > 0.3:
                break
        f = lambda t: '-' if t is None else f'{(t - t0) * 1000:.0f}'
        self.say(f'LAT {name}: first change {f(first)} ms, settled {f(last_change)} ms, '
                 f'action took {(tsend - t0) * 1000:.0f} ms, polls {polls} (~{(time.monotonic() - t0) * 1000 / max(polls, 1):.0f} ms each)')

    def watch(self, name, box, secs):
        prev = self.grab(box)
        t0 = time.monotonic()
        changes = []
        polls = 0
        while time.monotonic() - t0 < secs:
            cur = self.grab(box)
            polls += 1
            if cur != prev:
                changes.append(round((time.monotonic() - t0) * 1000))
                prev = cur
        self.say(f'WATCH {name}: {len(changes)} changes in {polls} polls over {secs}s: {changes[:60]}')

    def run_one(self, line):
        p = line.split()
        c, a = p[0], p[1:]
        rest = line.split(None, 1)[1] if len(p) > 1 else ''
        if c == 'shot':
            self.say('shot ' + self.shot(a[0]))
        elif c == 'key':
            self.chord(a[0])
        elif c == 'chord':
            self.chord(*a[0].split('+'))
        elif c == 'type':
            self.typ(rest)
        elif c == 'typegap':
            ms, text = rest.split(None, 1)
            self.typ(text, int(ms) / 1000)
        elif c == 'move':
            self.q.ev(*pos(a[0], a[1]))
        elif c == 'click':
            self.click(a[0], a[1])
        elif c == 'rclick':
            self.click(a[0], a[1], 'right')
        elif c == 'dclick':
            self.click(a[0], a[1], n=2)
        elif c == 'down':
            self.q.ev(btn(a[0] if a else 'left', True))
        elif c == 'up':
            self.q.ev(btn(a[0] if a else 'left', False))
        elif c in ('drag', 'dragshots'):
            name = None
            if c == 'dragshots':
                name, a = a[0], a[1:]
            x1, y1, x2, y2, steps, ms = map(int, a)
            self.q.ev(*pos(x1, y1))
            time.sleep(0.1)
            self.q.ev(btn('left', True))
            time.sleep(0.15)
            for i in range(1, steps + 1):
                self.q.ev(*pos(x1 + (x2 - x1) * i // steps, y1 + (y2 - y1) * i // steps))
                time.sleep(ms / 1000)
                if name and i == steps // 2:
                    self.shot(name + '-during')
            if name:
                time.sleep(0.5)
                self.shot(name + '-held')
            self.q.ev(btn('left', False))
        elif c == 'wheel':
            b = 'wheel-up' if a[0] == 'up' else 'wheel-down'
            for _ in range(int(a[1]) if len(a) > 1 else 1):
                self.q.ev(btn(b, True))
                self.q.ev(btn(b, False))
                time.sleep(0.03)
        elif c == 'sleep':
            time.sleep(float(a[0]))
        elif c == 'lat':
            name, x0, y0, x1, y1, to = a[0], *map(int, a[1:5]), float(a[5])
            self.lat(name, (x0, y0, x1, y1), to, ' '.join(a[6:]))
        elif c == 'watch':
            self.watch(a[0], tuple(map(int, a[1:5])), float(a[5]))
        elif c == 'grep':
            with open(self.serial, 'rb') as f:
                lines = [l for l in f.read().decode('utf-8', 'replace').splitlines() if rest in l]
            self.say(f'GREP {rest!r}: {len(lines)}')
            for l in lines[-8:]:
                self.say('  | ' + l[:300])
        elif c == 'tail':
            with open(self.serial, 'rb') as f:
                lines = f.read().decode('utf-8', 'replace').splitlines()
            for l in lines[-int(a[0]):]:
                self.say('  | ' + l[:300])
        elif c == 'mark':
            self.say('MARK ' + rest)
        else:
            self.say('unknown ' + c)

    def follow(self, cmdfile):
        pos_ = 0
        while True:
            try:
                with open(cmdfile) as f:
                    f.seek(pos_)
                    chunk = f.read()
            except OSError:
                chunk = ''
            if '\n' not in chunk:
                time.sleep(0.3)
                continue
            done = chunk[:chunk.rindex('\n') + 1]
            pos_ += len(done.encode())
            for line in done.splitlines():
                line = line.strip()
                if not line or line.startswith('#'):
                    continue
                if line == 'end':
                    self.say('END')
                    return
                for part in line.split(' ; '):
                    t = time.monotonic()
                    try:
                        self.run_one(part.strip())
                    except Exception as e:  # keep following
                        self.say(f'ERROR {part!r}: {e!r}')
                    self.say(f'ok {part.strip()[:120]} ({(time.monotonic() - t) * 1000:.0f} ms)')


if __name__ == '__main__':
    port, cmdfile, shots, serial = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
    os.makedirs(shots, exist_ok=True)
    q = Qmp(port)
    d = Driver(q, shots, serial, cmdfile + '.out')
    d.say('driver connected')
    d.follow(cmdfile)
    q.cmd('quit')
