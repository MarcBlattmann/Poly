//! The profile, the init scripts and the Python and C programs the boot test runs.

/// `/etc/profile` of the image (bash reads it as a login shell).
pub const PROFILE: &str = "\
# Read by login shells (tools/xtask/src/rootdisk.rs).
export PATH=/usr/local/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
export HOME=/root
export TERM=${TERM:-linux}
export PS1='\\u@\\h:\\w\\$ '
cd \"$HOME\"
";

/// `/usr/local/sbin/poweroff` of the image: asks init (outside the chroot).
pub(crate) const POWEROFF: &str = "#!/bin/bash\n# BusyBox's init (pid 1) powers off on SIGUSR2.\nsync\nkill -USR2 1\n";
/// `/usr/local/sbin/reboot` of the image.
pub(crate) const REBOOT: &str = "#!/bin/bash\n# BusyBox's init (pid 1) reboots on SIGTERM.\nsync\nkill -TERM 1\n";

/// `/etc/inputrc`: readline without bracketed paste (the boot test types
/// into the shell; the sequences would only clutter its output).
pub(super) const INPUTRC: &str = "set enable-bracketed-paste off\n";

/// The Python script of the boot test, `/root/poly-test.py`: arithmetic,
/// file I/O on the disk, a few standard modules and a coreutils program
/// started with `subprocess`. Prints one line per check; the boot test
/// compares them with [`PYTHON_EXPECTED`].
pub const PYTHON_TEST: &str = r#"import collections, json, os, re, subprocess, sys
primes = [n for n in range(2, 10000) if all(n % d for d in range(2, int(n ** 0.5) + 1))]
print("primes", len(primes), sum(primes))
fib = [0, 1]
while len(fib) < 30:
    fib.append(fib[-1] + fib[-2])
print("fib", fib[-1], collections.Counter("mississippi").most_common(2))
data = {"name": "poly", "items": [1, 2, 3], "nested": {"ok": True}}
path = "/root/poly-test.json"
with open(path, "w") as f:
    json.dump(data, f, sort_keys=True)
with open(path) as f:
    back = json.load(f)
print("json", back == data, json.dumps(back, sort_keys=True))
os.unlink(path)
print("exists", os.path.exists(path))
print("re", re.sub(r"(\w+)@(\w+)", r"\2 at \1", "root@poly"))
print("listdir", sorted(n for n in os.listdir("/") if n in ("bin", "etc", "usr")))
out = subprocess.run(["sort", "-r"], input="a\nc\nb\n", capture_output=True, text=True)
print("subprocess", out.returncode, out.stdout.split())
print("wc", subprocess.check_output("printf 'x\\ny\\n' | wc -l", shell=True).decode().strip())
print("done", sys.version_info[0])
"#;

/// What [`PYTHON_TEST`] prints when every check passes.
pub const PYTHON_EXPECTED: &str = "primes 1229 5736396\n\
fib 514229 [('i', 4), ('s', 4)]\n\
json True {\"items\": [1, 2, 3], \"name\": \"poly\", \"nested\": {\"ok\": true}}\n\
exists False\n\
re poly at root\n\
listdir ['bin', 'etc', 'usr']\n\
subprocess 0 ['c', 'b', 'a']\n\
wc 2\n\
done 3\n";

/// The second Python script of the boot test, `/root/poly-test-sys.py`:
/// what Python asks of the system (threads, memory maps and a growing
/// buffer, fork, record and `flock` locks across processes, SQLite,
/// temporary files, signals, the terminal's size, resource usage, big file
/// I/O, `posix_spawn`). Compared with [`PYTHON_SYS_EXPECTED`].
pub const PYTHON_SYS_TEST: &str = r#"import fcntl, hashlib, mmap, os, resource, shutil, signal, sqlite3, subprocess, sys, tempfile, threading, time, zlib
results = [0] * 4
def work(i):
    results[i] = sum(range(i * 100000, (i + 1) * 100000))
threads = [threading.Thread(target=work, args=(i,)) for i in range(4)]
for t in threads:
    t.start()
for t in threads:
    t.join()
print("threads", sum(results) == sum(range(400000)))
b = bytearray()
for i in range(48):
    b += bytes([i]) * (1 << 20)
print("grow", len(b), b[0], b[-1])
del b
path = "/root/poly-mmap.bin"
with open(path, "wb") as f:
    f.write(b"\0" * 8192)
with open(path, "r+b") as f:
    m = mmap.mmap(f.fileno(), 0)
    m[0:4] = b"poly"
    m[8190:8192] = b"!!"
    m.resize(16384)
    m[16380:16384] = b"tail"
    m.flush()
    m.close()
with open(path, "rb") as f:
    d = f.read()
print("mmap", len(d), d[:4], d[8190:8192], d[-4:])
os.unlink(path)
a = mmap.mmap(-1, 4096)
pid = os.fork()
if pid == 0:
    a[0:5] = b"child"
    os._exit(7)
_, status = os.waitpid(pid, 0)
print("fork", os.waitstatus_to_exitcode(status), a[0:5])
TRY = """import fcntl, os
fd = os.open('/root/poly-lock', os.O_RDWR)
try:
    fcntl.lockf(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    print('locked')
except OSError as e:
    print('busy', e.errno in (11, 13))
try:
    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    print('flocked')
except OSError as e:
    print('flock-busy', e.errno)
"""
fd = os.open("/root/poly-lock", os.O_RDWR | os.O_CREAT, 0o644)
fcntl.lockf(fd, fcntl.LOCK_EX)
print("lock", subprocess.run([sys.executable, "-c", TRY], capture_output=True, text=True).stdout.split())
fcntl.lockf(fd, fcntl.LOCK_UN)
print("unlocked", subprocess.run([sys.executable, "-c", TRY], capture_output=True, text=True).stdout.split())
fcntl.lockf(fd, fcntl.LOCK_EX)
WAIT = "import fcntl, os\nfd = os.open('/root/poly-lock', os.O_RDWR)\nfcntl.lockf(fd, fcntl.LOCK_EX)\nprint('waited')"
p = subprocess.Popen([sys.executable, "-c", WAIT], stdout=subprocess.PIPE, text=True)
time.sleep(1)
fcntl.lockf(fd, fcntl.LOCK_UN)
print("lockw", p.communicate()[0].strip(), p.returncode)
os.close(fd)
os.unlink("/root/poly-lock")
db = sqlite3.connect("/root/poly-test.db")
db.execute("create table t (k integer primary key, v text)")
db.executemany("insert into t (v) values (?)", [(str(i),) for i in range(100)])
db.commit()
print("sqlite", db.execute("select count(*), sum(k) from t").fetchone())
db.close()
os.unlink("/root/poly-test.db")
with tempfile.TemporaryFile() as t:
    t.write(b"temporary")
    t.seek(0)
    print("tempfile", t.read())
with tempfile.NamedTemporaryFile("w", dir="/tmp", delete=False) as t:
    t.write("named")
    name = t.name
print("named", open(name).read(), os.path.dirname(name))
os.unlink(name)
d = tempfile.mkdtemp()
shutil.copyfile("/etc/hostname", d + "/h")
print("copyfile", open(d + "/h").read().strip())
shutil.rmtree(d)
h = hashlib.sha256(b"poly").hexdigest()
s = subprocess.run(["sha256sum"], input=b"poly", capture_output=True).stdout.split()[0].decode()
print("sha256", h == s, len(h))
print("zlib", zlib.decompress(zlib.compress(b"poly" * 1000)) == b"poly" * 1000)
got = []
signal.signal(signal.SIGALRM, lambda n, f: got.append(n))
signal.setitimer(signal.ITIMER_REAL, 0.2)
time.sleep(1)
print("alarm", got)
print("terminal", os.get_terminal_size())
print("nofile", resource.getrlimit(resource.RLIMIT_NOFILE)[0] > 0)
r = resource.getrusage(resource.RUSAGE_CHILDREN)
print("rusage", r.ru_utime + r.ru_stime > 0)
print("cpus", os.cpu_count())
st = os.statvfs("/")
print("statvfs", st.f_bsize, st.f_blocks > 0)
print("exe", os.readlink("/proc/self/exe") == os.path.realpath(sys.executable))
data = os.urandom(1 << 20) * 24
with open("/root/poly-big.bin", "wb") as f:
    f.write(data)
with open("/root/poly-big.bin", "rb") as f:
    back = f.read()
print("bigfile", len(back), back == data)
os.unlink("/root/poly-big.bin")
pid = os.posix_spawn("/usr/bin/true", ["true"], os.environ)
print("posix_spawn", os.waitpid(pid, 0)[1])
print("done")
"#;

/// What [`PYTHON_SYS_TEST`] prints when every check passes, for a boot on
/// `cpus` CPUs.
pub fn python_sys_expected(cpus: u32) -> String {
    format!(
        "threads True\n\
grow 50331648 0 47\n\
mmap 16384 b'poly' b'!!' b'tail'\n\
fork 7 b'child'\n\
lock ['busy', 'True', 'flocked']\n\
unlocked ['locked', 'flocked']\n\
lockw waited 0\n\
sqlite (100, 5050)\n\
tempfile b'temporary'\n\
named named /tmp\n\
copyfile poly\n\
sha256 True 64\n\
zlib True\n\
alarm [14]\n\
terminal os.terminal_size(columns=250, lines=50)\n\
nofile True\n\
rusage True\n\
cpus {cpus}\n\
statvfs 4096 True\n\
exe True\n\
bigfile 25165824 True\n\
posix_spawn 0\n\
done\n"
    )
}

/// A C program of two files and a header, `/root/cc/`: separate
/// compilation, linking with libm, and the program forking a child that
/// runs a coreutils program through a pipe.
pub const C_MAIN: &str = r#"#include <math.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>
#include "util.h"

int main(void) {
    int fds[2];
    if (pipe(fds) != 0) return 1;
    pid_t pid = fork();
    if (pid == 0) {
        dup2(fds[1], 1);
        close(fds[0]);
        close(fds[1]);
        execl("/usr/bin/echo", "echo", "from", "the", "child", (char *)0);
        _exit(127);
    }
    close(fds[1]);
    char buf[64] = {0};
    ssize_t n = read(fds[0], buf, sizeof buf - 1);
    int status = 0;
    waitpid(pid, &status, 0);
    if (n > 0 && buf[n - 1] == '\n') buf[n - 1] = 0;
    printf("%s; sum %ld; sqrt %.3f; child %d\n", buf, sum_to(1000), sqrt(2.0), WEXITSTATUS(status));
    return 0;
}
"#;

/// `/root/cc/util.h`.
pub const C_UTIL_H: &str = "long sum_to(long n);\n";

/// `/root/cc/util.c`.
pub const C_UTIL: &str = "#include \"util.h\"\nlong sum_to(long n) {\n    long s = 0;\n    for (long i = 1; i <= n; i++) s += i;\n    return s;\n}\n";

/// The C program of the boot test, `/root/hello.c`.
pub const HELLO_C: &str = "#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n\
int main(int argc, char **argv) {\n\
    char *s = malloc(64);\n\
    strcpy(s, \"hello from gcc on Poly\");\n\
    printf(\"%s, argc=%d\\n\", s, argc);\n\
    free(s);\n\
    return 42;\n\
}\n";
