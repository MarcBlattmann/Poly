//! The persistent root disk (roadmap Phase 4, "bash, coreutils, Python and
//! a C compiler run from the persistent disk"; ADR 0059).
//!
//! An ext4 image holding a small root tree of the host's own programs and
//! what they need, made at test time and never committed:
//!
//! - bash, GNU coreutils ([`COREUTILS`]), grep and sed from `/usr/bin`;
//! - Python 3 (the interpreter `/usr/bin/python3` leads to) with its
//!   standard library directory, less the parts no program here uses
//!   ([`PYTHON_SKIP`]);
//! - the C compiler: the `gcc` driver, `cc1`, `collect2`, `as`, `ld`, the
//!   start files, libgcc, the C library's link-time files (`libc.so`,
//!   `libc_nonshared.a`, `crt1.o`, ...) and the headers a C program
//!   includes, found by following `#include` lines from [`HEADERS`];
//! - every shared object those programs (and Python's extension modules,
//!   and `ld`'s plugin) need: the `DT_NEEDED` closure and the `PT_INTERP`
//!   interpreter, found the way ld.so(8) searches (`dynamic.rs`), reading
//!   only ELF headers. Nothing is run on the host to find them.
//!
//! Each file goes to the path it has on the host (its directory with
//! symbolic links resolved, so `/lib/x86_64-linux-gnu/libc.so.6` lands in
//! `/usr/lib/x86_64-linux-gnu` when `/lib` is a link to `usr/lib`); symbolic
//! links are copied as links, and the host's top-level links (`/bin` ->
//! `usr/bin`, ...) are made too. A program the host lacks is left out with a
//! note, and the boot test skips what needs it.
//!
//! The image is written with `mke2fs -t ext4 -d` when e2fsprogs are
//! installed (from a staging directory of hard links, or copies, under
//! `target/`), otherwise with `poly-ext2`'s `format` and the driver itself.
//!
//! Boot ([`boot_args`]): BusyBox's init from an initramfs
//! (`initramfs::disk_root_image`) mounts the disk (`/dev/vda`) on
//! `/mnt/root`, binds `/dev`, `/dev/pts`, `/dev/shm`, `/proc`, `/sys` and
//! `/tmp` into it, and runs `chroot /mnt/root /bin/bash -l` on the console.
//! `cargo xtask run --disk-root` boots it interactively (the image
//! `target/poly-root-disk.img` is kept between runs; `--fresh-disk` makes
//! it again); [`boot_test`] types into that bash.

use crate::libsearch::{SearchPath, own_paths};
use poly_elf::{Dynamic, Elf};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The image `cargo xtask run --disk-root` boots (kept between runs).
pub const FILE_NAME: &str = "poly-root-disk.img";
/// The image the boot test makes afresh every time.
pub const TEST_FILE_NAME: &str = "poly-root-disk-test.img";

/// The GNU coreutils programs copied from `/usr/bin` (or `/bin`).
pub const COREUTILS: &[&str] = &[
    "basename",
    "cat",
    "chmod",
    "cp",
    "cut",
    "date",
    "dd",
    "df",
    "dirname",
    "du",
    "echo",
    "env",
    "expr",
    "false",
    "head",
    "id",
    "ln",
    "ls",
    "md5sum",
    "mkdir",
    "mktemp",
    "mv",
    "nproc",
    "od",
    "printf",
    "pwd",
    "readlink",
    "realpath",
    "rm",
    "rmdir",
    "seq",
    "sha256sum",
    "sleep",
    "sort",
    "stat",
    "stty",
    "sync",
    "tail",
    "tee",
    "test",
    "touch",
    "tr",
    "true",
    "tty",
    "uname",
    "uniq",
    "wc",
    "whoami",
    "yes",
];

/// Parts of Python's standard library directory left out: tests, GUI and
/// installer pieces, documentation data and the static library build.
pub const PYTHON_SKIP: &[&str] = &[
    "test",
    "idlelib",
    "tkinter",
    "turtledemo",
    "ensurepip",
    "lib2to3",
    "pydoc_data",
    "venv",
    "site-packages",
    "dist-packages",
];

/// The headers whose `#include` closure goes into the image: what an
/// ordinary C program includes.
pub const HEADERS: &[&str] = &[
    "assert.h",
    "ctype.h",
    "errno.h",
    "fcntl.h",
    "inttypes.h",
    "limits.h",
    "math.h",
    "signal.h",
    "stdarg.h",
    "stdbool.h",
    "stddef.h",
    "stdint.h",
    "stdio.h",
    "stdlib.h",
    "string.h",
    "strings.h",
    "time.h",
    "unistd.h",
    "sys/mman.h",
    "sys/stat.h",
    "sys/time.h",
    "sys/types.h",
    "sys/wait.h",
];

/// Modification time of the files the image gets that are not the host's
/// (configuration, the test files): 2023-11-14 22:13:20 UTC.
const MTIME: i64 = 1_700_000_000;

/// The source of a regular file's contents.
#[derive(Clone, Debug)]
pub enum Source {
    Host(PathBuf),
    Bytes(Vec<u8>),
}

/// One entry of the image, by path (no leading `/`).
#[derive(Clone, Debug)]
pub enum Entry {
    Dir(u16),
    File { mode: u16, source: Source, mtime: i64 },
    Symlink(String),
}

/// The tree the image is made of.
#[derive(Debug, Default)]
pub struct RootTree {
    pub entries: BTreeMap<String, Entry>,
}

impl RootTree {
    fn insert(&mut self, path: &str, entry: Entry) {
        let path = path.trim_start_matches('/').to_string();
        if path.is_empty() {
            return;
        }
        // Parent directories, unless something else is there already.
        let mut parent = Path::new(&path).parent();
        while let Some(dir) = parent.filter(|d| !d.as_os_str().is_empty()) {
            self.entries.entry(dir.to_string_lossy().into_owned()).or_insert(Entry::Dir(0o755));
            parent = dir.parent();
        }
        match (self.entries.get(&path), &entry) {
            // A directory asked for twice keeps the first mode.
            (Some(Entry::Dir(_)), Entry::Dir(_)) => {}
            _ => {
                self.entries.insert(path, entry);
            }
        }
    }

    /// A file the image makes itself.
    pub fn file(&mut self, path: &str, mode: u16, data: &[u8]) {
        self.insert(path, Entry::File { mode, source: Source::Bytes(data.to_vec()), mtime: MTIME });
    }

    pub fn dir(&mut self, path: &str, mode: u16) {
        self.insert(path, Entry::Dir(mode));
    }

    pub fn symlink(&mut self, path: &str, target: &str) {
        self.insert(path, Entry::Symlink(target.to_string()));
    }

    /// Bytes of regular files and how many entries there are.
    fn totals(&self) -> (u64, u64) {
        let mut bytes = 0;
        for entry in self.entries.values() {
            if let Entry::File { source, .. } = entry {
                bytes += match source {
                    Source::Host(p) => std::fs::metadata(p).map_or(0, |m| m.len()),
                    Source::Bytes(b) => b.len() as u64,
                };
            }
        }
        (bytes, self.entries.len() as u64)
    }
}

/// What the image holds and what was left out.
#[derive(Debug, Default)]
pub struct Contents {
    pub tree: RootTree,
    /// Programs found: "bash", "coreutils", "grep", "sed", "python3", "gcc".
    pub have: BTreeSet<&'static str>,
    /// What was left out and why.
    pub notes: Vec<String>,
}

/// Collects host files into a [`RootTree`], following symbolic links and
/// the shared objects ELF files need.
struct Collector {
    tree: RootTree,
    search: SearchPath,
    /// Host paths already added (as given and resolved).
    seen: BTreeSet<PathBuf>,
    /// ELF files whose `DT_NEEDED` entries are still to be added.
    elves: VecDeque<PathBuf>,
}

/// `path` with its directory's symbolic links resolved (the last component
/// kept as it is, so a link stays a link).
fn host_location(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let name = path.file_name()?;
    Some(std::fs::canonicalize(parent).ok()?.join(name))
}

fn mtime_of(meta: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.mtime()
}

impl Collector {
    fn new() -> Collector {
        Collector {
            tree: RootTree::default(),
            search: SearchPath::host(),
            seen: BTreeSet::new(),
            elves: VecDeque::new(),
        }
    }

    /// Add the host's file, link or directory at `path` (a directory with
    /// nothing in it). `Err` if it does not exist.
    fn add(&mut self, path: &Path) -> Result<(), String> {
        self.add_depth(path, 0)
    }

    fn add_depth(&mut self, path: &Path, depth: u32) -> Result<(), String> {
        if depth > 16 {
            return Err(format!("{}: too many levels of symbolic links", path.display()));
        }
        let at = host_location(path).ok_or_else(|| format!("{}: not found", path.display()))?;
        if !self.seen.insert(at.clone()) {
            return Ok(());
        }
        let meta = std::fs::symlink_metadata(&at).map_err(|e| format!("{}: {e}", at.display()))?;
        let image_path = at.to_string_lossy().into_owned();
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&at).map_err(|e| format!("{}: {e}", at.display()))?;
            self.tree.symlink(&image_path, &target.to_string_lossy());
            let resolved = if target.is_absolute() { target } else { at.parent().unwrap().join(target) };
            return self.add_depth(&resolved, depth + 1);
        }
        if meta.is_dir() {
            use std::os::unix::fs::PermissionsExt;
            self.tree.dir(&image_path, (meta.permissions().mode() & 0o7777) as u16);
            return Ok(());
        }
        if !meta.is_file() {
            return Err(format!("{}: not a regular file, directory or link", at.display()));
        }
        use std::os::unix::fs::PermissionsExt;
        let mode = (meta.permissions().mode() & 0o7777) as u16;
        self.tree.insert(&image_path, Entry::File { mode, source: Source::Host(at.clone()), mtime: mtime_of(&meta) });
        if is_elf(&at) {
            self.elves.push_back(at);
        }
        Ok(())
    }

    /// Add a directory and everything under it, leaving out the names in
    /// `skip` (at any depth) and `__pycache__` directories only if asked.
    fn add_tree(&mut self, dir: &Path, skip: &[&str]) -> Result<(), String> {
        self.add(dir)?;
        let at = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut entries: Vec<_> = std::fs::read_dir(&at)
            .map_err(|e| format!("{}: {e}", at.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if skip.contains(&name.as_str()) {
                continue;
            }
            let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if meta.is_dir() {
                self.add_tree(&path, skip)?;
            } else {
                self.add(&path)?;
            }
        }
        Ok(())
    }

    /// Add the shared objects (and interpreters) the ELF files added so far
    /// need, until nothing is missing.
    fn add_needed(&mut self) -> Result<(), String> {
        while let Some(path) = self.elves.pop_front() {
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let Ok(elf) = Elf::parse(&bytes) else { continue };
            if let Some(interp) = elf.interp() {
                let interp = PathBuf::from(String::from_utf8_lossy(interp).into_owned());
                self.add(&interp).map_err(|e| format!("the interpreter of {}: {e}", path.display()))?;
            }
            let Ok(Some(dynamic)) = Dynamic::parse(&elf) else { continue };
            let own = own_paths(&dynamic);
            for needed in dynamic.needed() {
                let name = String::from_utf8_lossy(needed).into_owned();
                let (found, _) = self.search.find(&name, &own).ok_or_else(|| {
                    format!("{name} (needed by {}) is not in the host's library directories", path.display())
                })?;
                self.add(&found)?;
            }
        }
        Ok(())
    }
}

/// Whether the file at `path` starts with the ELF magic.
fn is_elf(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && magic == *b"\x7fELF"
}

/// The first of `names` that exists in `/usr/bin` or `/bin`.
fn find_program(names: &[&str]) -> Option<PathBuf> {
    ["/usr/bin", "/bin"].iter().flat_map(|d| names.iter().map(move |n| Path::new(d).join(n))).find(|p| p.exists())
}

/// Where the host's GCC keeps its programs (`cc1`, `collect2`) and its
/// libraries (start files, libgcc, its headers): `/usr/{libexec,lib}/gcc/
/// <triple>/<version>`, the version the driver's name ends in if several.
fn gcc_dirs(driver: &Path) -> Option<(PathBuf, PathBuf)> {
    let name = driver.file_name()?.to_string_lossy().into_owned();
    let mut found: Vec<(PathBuf, PathBuf)> = Vec::new();
    for base in ["/usr/libexec/gcc", "/usr/lib/gcc"] {
        let Ok(triples) = std::fs::read_dir(base) else { continue };
        for triple in triples.filter_map(|e| e.ok()) {
            let Ok(versions) = std::fs::read_dir(triple.path()) else { continue };
            for version in versions.filter_map(|e| e.ok()) {
                let exec = version.path();
                if !exec.join("cc1").is_file() {
                    continue;
                }
                let lib = Path::new("/usr/lib/gcc").join(triple.file_name()).join(version.file_name());
                if lib.join("crtbegin.o").is_file() {
                    found.push((exec, lib));
                }
            }
        }
    }
    found.sort();
    let suffix = |dirs: &(PathBuf, PathBuf)| {
        let v = dirs.1.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        name.ends_with(&format!("-{v}"))
    };
    found.iter().find(|d| suffix(d)).cloned().or_else(|| found.last().cloned())
}

/// The headers `roots` include, directly or not, in the compiler's search
/// directories `dirs`: every file a `#include` or `#include_next` line can
/// name (conditionals are not evaluated, so this is a superset).
fn header_closure(roots: &[&str], dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    let resolve = |name: &str, from: Option<&Path>, out: &mut Vec<PathBuf>| {
        if let Some(dir) = from {
            let p = dir.join(name);
            if p.is_file() {
                out.push(p);
            }
        }
        for dir in dirs {
            let p = dir.join(name);
            if p.is_file() {
                out.push(p);
            }
        }
    };
    let mut first = Vec::new();
    for root in roots {
        resolve(root, None, &mut first);
    }
    for p in first {
        if seen.insert(p.clone()) {
            queue.push_back(p);
        }
    }
    while let Some(file) = queue.pop_front() {
        let Ok(text) = std::fs::read(&file) else { continue };
        let text = String::from_utf8_lossy(&text);
        for line in text.lines() {
            let Some(rest) = line.trim_start().strip_prefix('#') else { continue };
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix("include_next").or_else(|| rest.strip_prefix("include")) else {
                continue;
            };
            let rest = rest.trim();
            let (name, quoted) = if let Some(r) = rest.strip_prefix('<') {
                (r.split('>').next().unwrap_or_default(), false)
            } else if let Some(r) = rest.strip_prefix('"') {
                (r.split('"').next().unwrap_or_default(), true)
            } else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let mut out = Vec::new();
            resolve(name, quoted.then(|| file.parent()).flatten(), &mut out);
            for p in out {
                if seen.insert(p.clone()) {
                    queue.push_back(p);
                }
            }
        }
    }
    seen.into_iter().collect()
}

/// The absolute paths a linker script (`libc.so`) names.
fn script_paths(path: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .filter(|w| w.starts_with('/') && !w.starts_with("/*"))
        .map(PathBuf::from)
        .collect()
}

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
const POWEROFF: &str = "#!/bin/bash\n# BusyBox's init (pid 1) powers off on SIGUSR2.\nsync\nkill -USR2 1\n";
/// `/usr/local/sbin/reboot` of the image.
const REBOOT: &str = "#!/bin/bash\n# BusyBox's init (pid 1) reboots on SIGTERM.\nsync\nkill -TERM 1\n";

/// `/etc/inputrc`: readline without bracketed paste (the boot test types
/// into the shell; the sequences would only clutter its output).
const INPUTRC: &str = "set enable-bracketed-paste off\n";

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

/// Collect what the image holds (see the module documentation).
pub fn collect() -> Result<Contents, String> {
    let mut c = Collector::new();
    let mut have = BTreeSet::new();
    let mut notes = Vec::new();

    // The host's top-level links (usrmerge: /bin -> usr/bin, ...).
    for top in ["bin", "sbin", "lib", "lib64", "lib32", "libx32"] {
        let host = Path::new("/").join(top);
        if let Ok(target) = std::fs::read_link(&host) {
            c.tree.symlink(top, &target.to_string_lossy());
        }
    }

    match find_program(&["bash"]) {
        Some(p) => {
            c.add(&p)?;
            have.insert("bash");
        }
        None => notes.push("bash: not found on this host".into()),
    }
    let mut missing = Vec::new();
    for name in COREUTILS {
        match find_program(&[name]) {
            Some(p) => c.add(&p)?,
            None => missing.push(*name),
        }
    }
    if missing.iter().any(|m| ["ls", "sort", "head", "seq", "wc", "cat"].contains(m)) {
        notes.push(format!("coreutils: missing {}", missing.join(", ")));
    } else {
        have.insert("coreutils");
        if !missing.is_empty() {
            notes.push(format!("coreutils: left out (not on this host): {}", missing.join(", ")));
        }
    }
    for name in ["grep", "sed"] {
        match find_program(&[name]) {
            Some(p) => {
                c.add(&p)?;
                have.insert(name);
            }
            None => notes.push(format!("{name}: not found on this host")),
        }
    }

    // Python: the interpreter python3 leads to, and its library directory.
    match find_program(&["python3"]).and_then(|p| std::fs::canonicalize(p).ok()) {
        Some(real) => {
            let version = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let lib = Path::new("/usr/lib").join(&version);
            if version.starts_with("python3") && lib.join("os.py").is_file() {
                c.add(Path::new("/usr/bin/python3"))?;
                let mut skip: Vec<&str> = PYTHON_SKIP.to_vec();
                let config = std::fs::read_dir(&lib)
                    .map(|d| {
                        d.filter_map(|e| e.ok())
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .filter(|n| n.starts_with("config-"))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                skip.extend(config.iter().map(String::as_str));
                c.add_tree(&lib, &skip)?;
                // The image's python3 is a link to the interpreter itself,
                // not to the host's alternatives.
                c.tree.symlink("usr/bin/python3", &version);
                have.insert("python3");
            } else {
                notes.push(format!("python3: {} has no standard library at {}", real.display(), lib.display()));
            }
        }
        None => notes.push("python3: not found on this host".into()),
    }

    // The C compiler.
    match gcc_parts() {
        Ok(parts) => {
            for p in &parts.files {
                c.add(p)?;
            }
            for (link, target) in &parts.links {
                c.tree.symlink(link, target);
            }
            have.insert("gcc");
        }
        Err(why) => notes.push(format!("gcc: {why}")),
    }

    c.add_needed()?;

    let mut tree = c.tree;
    for (dir, mode) in [
        ("dev", 0o755),
        ("proc", 0o555),
        ("sys", 0o555),
        ("tmp", 0o1777),
        ("root", 0o700),
        ("mnt", 0o755),
        ("run", 0o755),
        ("home", 0o755),
        ("var/tmp", 0o1777),
        ("usr/local/bin", 0o755),
    ] {
        tree.dir(dir, mode);
    }
    tree.file("etc/passwd", 0o644, crate::initramfs::PASSWD.replace("/bin/sh", "/bin/bash").as_bytes());
    tree.file("etc/group", 0o644, crate::initramfs::GROUP.as_bytes());
    tree.file("etc/hostname", 0o644, b"poly\n");
    tree.file("etc/hosts", 0o644, crate::initramfs::HOSTS.as_bytes());
    tree.file("etc/resolv.conf", 0o644, crate::initramfs::RESOLV_CONF.as_bytes());
    tree.file("etc/profile", 0o644, PROFILE.as_bytes());
    tree.file("etc/inputrc", 0o644, INPUTRC.as_bytes());
    tree.file("etc/shells", 0o644, b"/bin/sh\n/bin/bash\n");
    if have.contains("bash") && !tree.entries.contains_key("usr/bin/sh") {
        tree.symlink("usr/bin/sh", "bash");
    }
    // BusyBox's init (pid 1, outside the chroot) powers off on SIGUSR2 and
    // reboots on SIGTERM (ADR 0051).
    tree.file("usr/local/sbin/poweroff", 0o755, POWEROFF.as_bytes());
    tree.file("usr/local/sbin/reboot", 0o755, REBOOT.as_bytes());
    tree.file("root/poly-test.py", 0o644, PYTHON_TEST.as_bytes());
    tree.file("root/hello.c", 0o644, HELLO_C.as_bytes());
    tree.file("root/poly-test-sys.py", 0o644, PYTHON_SYS_TEST.as_bytes());
    tree.file("root/cc/main.c", 0o644, C_MAIN.as_bytes());
    tree.file("root/cc/util.c", 0o644, C_UTIL.as_bytes());
    tree.file("root/cc/util.h", 0o644, C_UTIL_H.as_bytes());
    Ok(Contents { tree, have, notes })
}

/// The compiler's files and the links to make.
struct GccParts {
    files: Vec<PathBuf>,
    links: Vec<(String, String)>,
}

fn gcc_parts() -> Result<GccParts, String> {
    let driver_link = find_program(&["gcc", "cc"]).ok_or("no gcc or cc on this host")?;
    let driver = std::fs::canonicalize(&driver_link).map_err(|e| format!("{}: {e}", driver_link.display()))?;
    let (exec, lib) = gcc_dirs(&driver).ok_or("no cc1 in /usr/libexec/gcc or /usr/lib/gcc")?;
    let mut files = vec![driver.clone()];
    for name in ["as", "ld"] {
        files.push(find_program(&[name]).ok_or_else(|| format!("no {name} (binutils) on this host"))?);
    }
    if let Some(p) = find_program(&["ld.bfd"]) {
        files.push(p);
    }
    for name in ["cc1", "collect2", "lto-wrapper", "liblto_plugin.so"] {
        let p = exec.join(name);
        if p.exists() {
            files.push(p);
        }
    }
    // Start files, libgcc and its linker script, the plugin's other name.
    let entries: Vec<PathBuf> = std::fs::read_dir(&lib)
        .map_err(|e| format!("{}: {e}", lib.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    for p in &entries {
        let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let wanted = (n.starts_with("crt") && n.ends_with(".o"))
            || ["libgcc.a", "libgcc_eh.a", "libgcc_s.so", "liblto_plugin.so", "specs"].contains(&n.as_str());
        if wanted {
            files.push(p.clone());
        }
    }
    // The C library's link-time files, next to its linker script libc.so.
    let triple = lib.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let libdir = [format!("/usr/lib/{triple}"), "/usr/lib64".into(), "/usr/lib".into()]
        .into_iter()
        .map(PathBuf::from)
        .find(|d| d.join("libc.so").is_file())
        .ok_or("no libc.so (the C library's development files) on this host")?;
    for name in ["libc.so", "libc_nonshared.a", "crt1.o", "Scrt1.o", "rcrt1.o", "crti.o", "crtn.o", "libm.so"] {
        let p = libdir.join(name);
        if p.exists() {
            files.extend(script_paths(&p).into_iter().filter(|q| q.exists()));
            files.push(p);
        }
    }
    let search = SearchPath::host();
    if let Some((p, _)) = search.find("libgcc_s.so.1", &[]) {
        files.push(p);
    }
    // Headers.
    let mut dirs = vec![lib.join("include"), PathBuf::from("/usr/local/include"), lib.join("include-fixed")];
    dirs.push(Path::new("/usr/include").join(&triple));
    dirs.push(PathBuf::from("/usr/include"));
    dirs.retain(|d| d.is_dir());
    let headers = header_closure(HEADERS, &dirs);
    if !headers.iter().any(|h| h.ends_with("stdio.h")) {
        return Err("no stdio.h in the compiler's include directories".into());
    }
    files.extend(headers);
    let driver_name = driver.file_name().unwrap().to_string_lossy().into_owned();
    let links = ["gcc", "cc"]
        .iter()
        .filter(|&&n| n != driver_name)
        .map(|n| (format!("usr/bin/{n}"), driver_name.clone()))
        .collect();
    Ok(GccParts { files, links })
}

/// Whether this process runs as root (`/proc/self` belongs to its user).
fn running_as_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

/// Whether `mke2fs` can be run.
fn have_mke2fs() -> bool {
    Command::new("mke2fs").arg("-V").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

/// The size of the image for `tree`: its contents with room to spare
/// (the boot test writes programs and Python's byte code), whole MiB.
fn image_size(tree: &RootTree) -> (u64, u64) {
    let (bytes, entries) = tree.totals();
    let used = bytes + entries * 4096;
    let size = (used + used / 4 + (160 << 20)).div_ceil(4 << 20) * (4 << 20);
    let inodes = (entries * 2 + 16384).div_ceil(1024) * 1024;
    (size, inodes)
}

/// The mke2fs.conf the image is made with: the ext4 features `poly-ext2`
/// supports, whatever the host's own configuration says.
const MKE2FS_CONF: &str = "\
[defaults]
\tbase_features = sparse_super,large_file,filetype,resize_inode,dir_index,ext_attr
\tblocksize = 4096
\tinode_size = 256
\tinode_ratio = 16384
[fs_types]
\text4 = {
\t\tfeatures = has_journal,extent,huge_file,flex_bg,metadata_csum,64bit,dir_nlink,extra_isize
\t}
";

/// Write `tree` as an ext4 image to `image`; how it was made.
pub fn write_image(tree: &RootTree, image: &Path) -> Result<&'static str, String> {
    let (size, inodes) = image_size(tree);
    let _ = std::fs::remove_file(image);
    // POLY_ROOTDISK_FORMAT=poly-ext2 uses the repository's formatter even with e2fsprogs.
    let ours = std::env::var("POLY_ROOTDISK_FORMAT").is_ok_and(|v| v == "poly-ext2");
    // `mke2fs -d` gives every file the owner of its staged copy, and its
    // `root_owner` option covers only the root directory: run as anyone but
    // root (CI), the image's files would belong to that user. The
    // repository's formatter writes them as root.
    if !ours && have_mke2fs() && running_as_root() {
        with_mke2fs(tree, image, size, inodes)?;
        Ok("mke2fs -t ext4 -d")
    } else {
        with_poly_ext2(tree, image, size)?;
        Ok("poly-ext2's format")
    }
}

fn with_mke2fs(tree: &RootTree, image: &Path, size: u64, inodes: u64) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let target = super::root().join("target");
    let stage = target.join(format!("poly-rootdisk-stage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
    let result = (|| -> Result<(), String> {
        let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
        let mut dirs = Vec::new();
        for (path, entry) in &tree.entries {
            let at = stage.join(path);
            match entry {
                Entry::Dir(mode) => {
                    std::fs::create_dir_all(&at).map_err(|e| err(&at, e))?;
                    dirs.push((at, *mode));
                }
                Entry::Symlink(t) => std::os::unix::fs::symlink(t, &at).map_err(|e| err(&at, e))?,
                Entry::File { mode, source, mtime } => {
                    match source {
                        // A hard link costs no space; the host's file is
                        // never written through it.
                        Source::Host(host) => {
                            if std::fs::hard_link(host, &at).is_err() {
                                std::fs::copy(host, &at).map_err(|e| err(&at, e))?;
                                set_mtime(&at, *mtime)?;
                            }
                        }
                        Source::Bytes(b) => {
                            std::fs::write(&at, b).map_err(|e| err(&at, e))?;
                            std::fs::set_permissions(&at, std::fs::Permissions::from_mode(u32::from(*mode)))
                                .map_err(|e| err(&at, e))?;
                            set_mtime(&at, *mtime)?;
                        }
                    }
                }
            }
        }
        for (at, mode) in dirs.iter().rev() {
            std::fs::set_permissions(at, std::fs::Permissions::from_mode(u32::from(*mode))).map_err(|e| err(at, e))?;
        }
        let conf = stage.with_extension("mke2fs.conf");
        std::fs::write(&conf, MKE2FS_CONF).map_err(|e| err(&conf, e))?;
        let out = Command::new("mke2fs")
            .env("MKE2FS_CONFIG", &conf)
            .args(["-q", "-F", "-t", "ext4", "-b", "4096", "-L", "poly-root"])
            .args(["-U", "504f4c59-726f-6f74-2d64-69736b000001", "-E", "root_owner=0:0", "-N"])
            .arg(inodes.to_string())
            .arg("-d")
            .arg(&stage)
            .arg(image)
            .arg(format!("{}k", size / 1024))
            .output()
            .map_err(|e| format!("mke2fs: {e}"))?;
        let _ = std::fs::remove_file(&conf);
        if !out.status.success() {
            return Err(format!("mke2fs -d failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&stage);
    result
}

fn set_mtime(path: &Path, mtime: i64) -> Result<(), String> {
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime.max(0) as u64);
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(when))
        .map_err(|e| format!("{}: {e}", path.display()))
}

struct FixedClock;

impl poly_vfs::Clock for FixedClock {
    fn now(&self) -> poly_vfs::Timespec {
        poly_vfs::Timespec::new(MTIME, 0)
    }
}

fn with_poly_ext2(tree: &RootTree, image: &Path, size: u64) -> Result<(), String> {
    use poly_block::RamDisk;
    use poly_ext2::{DeviceIo, Ext2, FormatOptions, MountOptions};
    use poly_vfs::{DeviceId, FileSystem, FileType, NewNode, SetAttr, SetTime, Timespec};
    let mut mem = vec![0u8; usize::try_from(size).map_err(|_| "image too big")?];
    let err = |what: &str, e: poly_vfs::Errno| format!("root disk: {what}: {}", e.name());
    {
        let disk = RamDisk::new(&mut mem, 512).ok_or("root disk: bad RAM disk")?;
        let io = DeviceIo::new(disk);
        let opts = FormatOptions {
            uuid: *b"POLYroot-disk\0\0\x01",
            label: *b"poly-root\0\0\0\0\0\0\0",
            time: Timespec::new(MTIME, 0),
            ..FormatOptions::ext4(4096, 4096)
        };
        poly_ext2::format(&io, &opts).map_err(|e| err("format", e))?;
        let fs = Ext2::mount(&io, std::sync::Arc::new(FixedClock), MountOptions::default())
            .map_err(|e| format!("root disk: mount: {e}"))?;
        let mut inos: BTreeMap<String, poly_vfs::Ino> = BTreeMap::new();
        inos.insert(String::new(), fs.root());
        for (path, entry) in &tree.entries {
            let (parent, name) = match path.rsplit_once('/') {
                Some((p, n)) => (p.to_string(), n),
                None => (String::new(), path.as_str()),
            };
            let dir = *inos.get(&parent).ok_or_else(|| format!("root disk: no directory for {path}"))?;
            match entry {
                Entry::Dir(mode) => {
                    let ino = fs.mkdir(dir, name.as_bytes(), *mode, 0, 0).map_err(|e| err(path, e))?;
                    inos.insert(path.clone(), ino);
                }
                Entry::Symlink(t) => {
                    fs.symlink(dir, name.as_bytes(), t.as_bytes(), 0, 0).map_err(|e| err(path, e))?;
                }
                Entry::File { mode, source, mtime } => {
                    let node =
                        NewNode { kind: FileType::Regular, perm: *mode, uid: 0, gid: 0, rdev: DeviceId::new(0, 0) };
                    let ino = fs.create(dir, name.as_bytes(), &node).map_err(|e| err(path, e))?;
                    let data = match source {
                        Source::Host(p) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?,
                        Source::Bytes(b) => b.clone(),
                    };
                    for (i, chunk) in data.chunks(1 << 20).enumerate() {
                        fs.write(ino, (i << 20) as u64, chunk).map_err(|e| err(path, e))?;
                    }
                    let at = SetTime::At(Timespec::new(*mtime, 0));
                    let set = SetAttr { mtime: Some(at), atime: Some(at), ..SetAttr::default() };
                    fs.setattr(ino, &set).map_err(|e| err(path, e))?;
                }
            }
        }
        fs.unmount().map_err(|e| err("unmount", e))?;
    }
    std::fs::write(image, &mem).map_err(|e| format!("{}: {e}", image.display()))
}

/// Collect the tree and write it to `target/<name>`; prints what was left
/// out.
pub fn make(name: &str) -> Result<(PathBuf, Contents), String> {
    let contents = collect()?;
    for note in &contents.notes {
        eprintln!("==> note: root disk: {note}");
    }
    let path = super::root().join("target").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("{}: {e}", path.display()))?;
    let how = write_image(&contents.tree, &path)?;
    let (bytes, entries) = contents.tree.totals();
    eprintln!(
        "==> root disk {}: {} entries, {} MiB of files ({how}); {}",
        path.display(),
        entries,
        bytes >> 20,
        contents.have.iter().copied().collect::<Vec<_>>().join(", ")
    );
    Ok((path, contents))
}

/// The kernel command line of a boot from the root disk.
pub fn boot_args() -> String {
    format!("{} console=ttyS0", crate::busybox::INIT_ARGS)
}

/// Guest memory for boots from the root disk: the compiler and Python
/// need more than the default.
pub const MEMORY: &str = "1G";

/// QEMU arguments attaching `image` as the only virtio-blk device
/// (`virtio0`, `/dev/vda`).
pub fn drive_args(image: &Path) -> Vec<String> {
    vec![
        "-drive".into(),
        format!("if=none,id=root,format=raw,file={}", image.display()),
        "-device".into(),
        "virtio-blk-pci,drive=root,disable-legacy=on".into(),
    ]
}

/// The initramfs of a boot from the root disk (BusyBox's init, see
/// `initramfs::disk_root_image`), written to `target/`; `Err(why)` inside
/// when there is no BusyBox.
pub fn write_initramfs() -> super::Result<std::result::Result<(PathBuf, String), String>> {
    let binary = match crate::busybox::obtain()? {
        Ok(b) => b,
        Err(why) => return Ok(Err(why)),
    };
    let image = crate::initramfs::disk_root_image(&binary.elf);
    let path = crate::initramfs::write(&super::root(), "poly-disk-root-initramfs.cpio", &image)?;
    Ok(Ok((path, binary.origin)))
}

/// `cargo xtask root-disk`: write `target/poly-root-disk.img` afresh.
pub fn command() -> super::Result {
    let (path, _) = make(FILE_NAME)?;
    println!("{}", path.display());
    Ok(())
}

// ---- the boot test -------------------------------------------------------

/// How long one step of the session may take: a C compile or Python's
/// start-up is slow under emulation, and the host may be loaded.
const STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(400);
/// How long the whole session may take.
const SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2400);

/// The boot test of the milestone (see the module documentation): bash,
/// coreutils, Python and gcc from the root disk, typed into the disk's
/// bash on the serial console; then `e2fsck -fn` on the image and the
/// compiled program read back from it on the host.
pub fn boot_test(kernel: &Path, syms: &std::sync::Arc<crate::symbols::Symbols>) -> super::Result {
    eprintln!("==> boot test: bash, coreutils, Python and gcc run from the persistent ext4 root disk");
    let (initrd, origin) = match write_initramfs()? {
        Ok(found) => found,
        Err(why) => {
            eprintln!("==> skipped: the root disk boots through BusyBox's init: {why}");
            return Ok(());
        }
    };
    let (image, contents) = make(TEST_FILE_NAME)?;
    if !contents.have.contains("bash") {
        eprintln!("==> skipped: the root disk boot test: the host has no bash");
        return Ok(());
    }
    eprintln!("==> BusyBox from {origin}");
    let mut s = spawn(kernel, syms, &image, &initrd)?;
    let mut ran = BTreeSet::new();
    let result = session(&mut s, &contents.have, &mut ran);
    if result.is_err() {
        let log = super::root().join("target/poly-rootdisk-boot.log");
        let _ = std::fs::write(&log, s.text());
        eprintln!("==> the session's output is in {}", log.display());
        return result;
    }
    drop(s);
    match crate::ext2disk::e2fsck(&image)? {
        Some(_) => eprintln!("==> e2fsck -fn passes on the root disk after the session"),
        None => eprintln!("==> e2fsck not installed; skipped checking the image"),
    }
    if ran.contains(GCC_STEP) {
        let elf = read_back(&image, "root/hello")?;
        // The format string is in the file whatever the optimiser did with
        // the copied text.
        if !elf.starts_with(b"\x7fELF") || !elf.windows(10).any(|w| w == b"%s, argc=%") {
            return Err("the program gcc wrote is not on the root disk after the boot".into());
        }
        eprintln!("==> /root/hello, compiled on Poly, is on the image ({} bytes)", elf.len());
    }
    // The second boot from the same image: what the first one wrote is
    // there, and the program it compiled runs.
    if !ran.contains(PERSIST_STEP) {
        return Ok(());
    }
    eprintln!("==> boot test: a second boot from the root disk finds what the first one wrote");
    let mut s = spawn(kernel, syms, &image, &initrd)?;
    let result = second_boot(&mut s, ran.contains(GCC_STEP));
    if result.is_err() {
        let log = super::root().join("target/poly-rootdisk-boot2.log");
        let _ = std::fs::write(&log, s.text());
        eprintln!("==> the session's output is in {}", log.display());
    }
    result
}

/// QEMU booting the root disk `image` with the initramfs `initrd`, as a
/// session typed into.
fn spawn(
    kernel: &Path,
    syms: &std::sync::Arc<crate::symbols::Symbols>,
    image: &Path,
    initrd: &Path,
) -> super::Result<crate::busybox::Session> {
    // POLY_ROOTDISK_APPEND: more kernel arguments (`poly.strace=python3.13`, ...).
    let append = format!("{} {}", boot_args(), std::env::var("POLY_ROOTDISK_APPEND").unwrap_or_default());
    let mut cmd = super::qemu(kernel, MEMORY, append.trim_end(), super::Devices::None, "stdio");
    cmd.args(["-display", "none"]).args(drive_args(image)).arg("-initrd").arg(initrd);
    let mut s = crate::busybox::Session::spawn(cmd, syms)?;
    s.set_timeouts(STEP_TIMEOUT, SESSION_TIMEOUT);
    Ok(s)
}

/// The step that writes the file the second boot reads.
const PERSIST_STEP: &str = "a file for the next boot, sync";

/// The file the first boot writes and the second reads.
const PERSIST_TEXT: &str = "written in the first boot";

fn second_boot(s: &mut crate::busybox::Session, compiled: bool) -> super::Result {
    s.expect("rootdisk: /dev/vda mounted on /mnt/root")?;
    s.prompt()?;
    let out = s.run("cat /root/persist.txt")?;
    if out != format!("{PERSIST_TEXT}\n") {
        return Err(format!("root disk on Poly: the second boot reads /root/persist.txt as {out:?}"));
    }
    if compiled {
        let out = s.run("/root/hello again; echo \"status=$?\"")?;
        if out != "hello from gcc on Poly, argc=2\nstatus=42\n" {
            return Err(format!("root disk on Poly: the program compiled in the first boot printed {out:?}"));
        }
    }
    s.send("poweroff\r")?;
    s.expect("reboot: Power down.")?;
    match s.finish()? {
        Some(0) => Ok(()),
        status => Err(format!("root disk on Poly: the second boot's poweroff ended QEMU with {status:?}")),
    }
}

/// The file at `path` on the ext4 image, read with `poly-ext2` (from a
/// copy in memory; the image is not changed).
fn read_back(image: &Path, path: &str) -> super::Result<Vec<u8>> {
    use poly_ext2::{DeviceIo, Ext2, MountOptions};
    use poly_vfs::FileSystem;
    let mut mem = std::fs::read(image).map_err(|e| format!("{}: {e}", image.display()))?;
    let io = DeviceIo::new(poly_block::RamDisk::new(&mut mem, 512).ok_or("bad image size")?);
    let fs = Ext2::mount(&io, std::sync::Arc::new(FixedClock), MountOptions { read_only: true, ordered: true })
        .map_err(|e| format!("{}: {e}", image.display()))?;
    let mut ino = fs.root();
    for name in path.split('/') {
        ino = fs.lookup(ino, name.as_bytes()).map_err(|e| format!("{path} on the image: {}", e.name()))?;
    }
    let size = fs.getattr(ino).map_err(|e| e.name().to_string())?.size;
    let mut buf = vec![0u8; size as usize];
    let n = fs.read(ino, 0, &mut buf).map_err(|e| e.name().to_string())?;
    buf.truncate(n);
    Ok(buf)
}

/// One command line of the session: what it checks, the programs it needs
/// (skipped without them), the line typed into bash, and its output
/// check.
struct Step {
    what: &'static str,
    needs: &'static [&'static str],
    line: String,
    ok: Box<dyn Fn(&str) -> bool>,
}

fn step(what: &'static str, needs: &'static [&'static str], line: &str, want: &str) -> Step {
    let want = want.to_string();
    Step { what, needs, line: line.to_string(), ok: Box::new(move |out| out == want) }
}

impl Step {
    /// The same step with another output check.
    fn then(self, ok: impl Fn(&str) -> bool + 'static) -> Step {
        Step { ok: Box::new(ok), ..self }
    }
}

/// The step that compiles `/root/hello` onto the disk, which the test then
/// reads back from the image.
const GCC_STEP: &str = "gcc compiles hello.c onto the disk and runs it";

/// The session's command lines, in order.
fn steps() -> Vec<Step> {
    let mut steps = vec![
        // Long command lines on one line: readline scrolls them at the
        // terminal's width, which the serial line has not set.
        step("stty cols 250 rows 50", &["coreutils"], "stty cols 250 rows 50 && stty size", "50 250\n"),
        Step {
            what: "bash as the login shell",
            needs: &[],
            line: "echo \"bash $BASH_VERSINFO $0\"; shopt -q login_shell && echo login shell".into(),
            ok: Box::new(|out| out.starts_with("bash ") && out.ends_with(" /bin/bash\nlogin shell\n")),
        },
        step(
            "bash -c 'ls / | sort | head -3'",
            &["coreutils"],
            "bash -c 'ls / | sort | head -3'; echo \"status=$?\"",
            "bin\ndev\netc\nstatus=0\n",
        ),
        step(
            "bash -c 'seq 1 1000 | grep 7 | wc -l'",
            &["coreutils", "grep"],
            "bash -c 'seq 1 1000 | grep 7 | wc -l'; echo \"status=$?\"",
            "271\nstatus=0\n",
        ),
        step(
            "seq 10 | sed -n 3,5p | tr",
            &["coreutils", "sed"],
            "bash -c 'seq 10 | sed -n \"3,5p\" | tr \"\\n\" \" \"; echo'",
            "3 4 5 \n",
        ),
        Step {
            what: "mkdir, cp, mv, cat, wc, rm and ls on the disk",
            needs: &["coreutils"],
            line: "bash -c 'cd /root && mkdir -p d && echo hi > d/a && cp d/a d/b && mv d/b d/c && cat d/a d/c | wc -c && rm -r d && ls d'; echo \"status=$?\"".into(),
            ok: Box::new(|out| out.starts_with("6\nls: ") && out.ends_with("\nstatus=2\n")),
        },
        step(
            "env, date, tail, head",
            &["coreutils"],
            "env -i FOO=bar env; date +%Y | wc -c; tail -n 1 /etc/passwd | head -c 6; echo",
            "FOO=bar\n5\nnobody\n",
        ),
        step("bash -c 'exit 7'", &[], "bash -c 'exit 7'; echo \"status=$?\"", "status=7\n"),
        // The terminal's size (TIOCGWINSZ) as bash's checkwinsize sees it,
        // command substitution, here-strings, arithmetic.
        step(
            "bash: window size, substitution, arithmetic",
            &[],
            "echo \"$COLUMNS $LINES $(echo sub) $((6 * 7))\"; read -r x <<< 'here'; echo $x",
            "250 50 sub 42\nhere\n",
        ),
        step(
            "stat, ls -l, readlink /proc/self/exe, ulimit",
            &["coreutils"],
            "stat -c '%s %a %U %F' /etc/hostname; ls -l /etc/hostname | cut -d' ' -f1,3,5; readlink /proc/self/exe; ulimit -n",
            "5 644 root regular file\n-rw-r--r-- root 5\n/usr/bin/readlink\n1024\n",
        ),
        step(
            "dd 32 MiB through the page cache, wc, sha256sum, rm",
            &["coreutils"],
            "dd if=/dev/zero of=/root/big bs=1M count=32 status=none && wc -c < /root/big && cat /root/big | sha256sum | cut -c1-8 && head -c 33554432 /dev/zero | sha256sum | cut -c1-8 && rm /root/big",
            "33554432\n",
        )
        .then(|out| {
            // The two digests of 32 MiB of zeros agree.
            let lines: Vec<&str> = out.lines().collect();
            lines.len() == 3 && lines[0] == "33554432" && lines[1] == lines[2] && lines[1].len() == 8
        }),
        step("a background job", &["coreutils"], "sleep 30 &", "").then(|out| out.starts_with("[1] ")),
        step("jobs", &["coreutils"], "jobs", "").then(|out| out.starts_with("[1]+  Running") && out.contains("sleep 30 &")),
        step("kill %1 and wait", &["coreutils"], "kill %1; wait %1; echo \"status=$?\"", "")
            .then(|out| out.ends_with("status=143\n")),
        step(
            "python3 /root/poly-test.py",
            &["python3", "coreutils"],
            "python3 /root/poly-test.py; echo \"status=$?\"",
            &format!("{PYTHON_EXPECTED}status=0\n"),
        ),
        step(
            "python3 /root/poly-test-sys.py",
            &["python3", "coreutils"],
            "python3 /root/poly-test-sys.py; echo \"status=$?\"",
            &format!("{}status=0\n", python_sys_expected(super::QEMU_CPUS)),
        ),
        step(
            GCC_STEP,
            &["gcc"],
            "gcc -O1 -Wall -o /root/hello /root/hello.c && /root/hello a b; echo \"status=$?\"",
            "hello from gcc on Poly, argc=3\nstatus=42\n",
        ),
        step(
            "gcc: two files compiled apart, linked with libm, run",
            &["gcc", "coreutils"],
            "cd /root/cc && gcc -O2 -Wall -c util.c && gcc -O2 -Wall -o prog main.c util.o -lm && ./prog; echo \"status=$?\"; cd",
            "from the child; sum 500500; sqrt 1.414; child 0\nstatus=0\n",
        ),
    ];
    steps.push(step(
        PERSIST_STEP,
        &[],
        &format!("echo '{PERSIST_TEXT}' > /root/persist.txt && sync && echo synced"),
        "synced\n",
    ));
    steps
}

/// Lines the kernel logs on the console that are not the programs'
/// output: taken out of each command's output (and checked separately).
fn is_kernel_line(line: &str) -> bool {
    ["syscall: pid ", "linux: pid ", "fs: ", "strace: "].iter().any(|p| line.starts_with(p))
}

/// System calls a program may make that Poly answers with `ENOSYS` on
/// purpose (ADR 0043, 0049, 0059).
const EXPECTED_ENOSYS: &[&str] = &[" rseq not implemented"];

/// Run the steps; `ran` gets the names of those that ran.
fn session(
    s: &mut crate::busybox::Session,
    have: &BTreeSet<&'static str>,
    ran: &mut BTreeSet<&'static str>,
) -> super::Result {
    s.expect("rootdisk: /dev/vda mounted on /mnt/root")?;
    s.prompt()?;
    let mut failed = Vec::new();
    let mut unimplemented = BTreeSet::new();
    // POLY_ROOTDISK_STEPS=<text>: only the steps whose name contains it
    // (and the terminal's size), for debugging one program.
    let only = std::env::var("POLY_ROOTDISK_STEPS").ok().filter(|s| !s.is_empty());
    for step in steps() {
        if let Some(only) = &only
            && !step.what.contains(only.as_str())
            && !step.what.starts_with("stty")
        {
            continue;
        }
        if let Some(missing) = step.needs.iter().find(|n| !have.contains(**n)) {
            eprintln!("==> skipped: {} (no {missing} on this host)", step.what);
            continue;
        }
        ran.insert(step.what);
        let raw = s.run(&step.line)?;
        let mut out = String::new();
        for line in raw.split_inclusive('\n') {
            if is_kernel_line(line) {
                if line.contains(" not implemented") && !EXPECTED_ENOSYS.iter().any(|e| line.contains(e)) {
                    unimplemented.insert(line.trim_end().to_string());
                }
            } else {
                out.push_str(line);
            }
        }
        if !(step.ok)(&out) {
            eprintln!("==> FAILED: {}: {out:?}", step.what);
            failed.push(format!("{} (printed {out:?})", step.what));
        }
    }
    // Ctrl-C ends bash's foreground job (exit status 128 + SIGINT).
    if have.contains("coreutils") && only.is_none() {
        s.send("echo started; sleep 20\r")?;
        s.expect("started\n")?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        s.send("\x03")?;
        s.prompt()?;
        let out = s.run("echo \"status=$?\"")?;
        if out != "status=130\n" {
            failed.push(format!("Ctrl-C in bash (printed {out:?})"));
        }
    }
    // poweroff (BusyBox's init powers off on SIGUSR2), which unmounts the
    // disk and ends QEMU with status 0.
    s.send("poweroff\r")?;
    s.expect("reboot: Power down.")?;
    let status = s.finish()?;
    if status != Some(0) {
        failed.push(format!("poweroff (QEMU status {status:?})"));
    }
    if !unimplemented.is_empty() {
        failed.push(format!("system calls Poly does not implement: {:?}", unimplemented));
    }
    if failed.is_empty() { Ok(()) } else { Err(format!("root disk on Poly: failed: {}", failed.join("; "))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trees_make_parents_and_keep_the_first_directory_mode() {
        let mut t = RootTree::default();
        t.dir("tmp", 0o1777);
        t.file("usr/lib/x/libc.so.6", 0o755, b"elf");
        t.dir("tmp", 0o755);
        t.symlink("lib", "usr/lib");
        assert!(matches!(t.entries.get("tmp"), Some(Entry::Dir(0o1777))));
        assert!(matches!(t.entries.get("usr"), Some(Entry::Dir(0o755))));
        assert!(matches!(t.entries.get("usr/lib/x"), Some(Entry::Dir(0o755))));
        assert!(matches!(t.entries.get("lib"), Some(Entry::Symlink(s)) if s == "usr/lib"));
        // BTreeMap order puts a directory before what is in it.
        let keys: Vec<&String> = t.entries.keys().collect();
        let pos = |k: &str| keys.iter().position(|x| *x == k).unwrap();
        assert!(pos("usr") < pos("usr/lib") && pos("usr/lib/x") < pos("usr/lib/x/libc.so.6"));
    }

    #[test]
    fn header_closure_follows_includes_and_include_next() {
        let dir = std::env::temp_dir().join(format!("poly-hdr-{}", std::process::id()));
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(a.join("sys")).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("top.h"), "#include <sys/one.h>\n# include \"local.h\"\n#include_next <limits.h>\n")
            .unwrap();
        std::fs::write(a.join("sys/one.h"), "#ifdef X\n#  include <missing.h>\n#endif\n").unwrap();
        std::fs::write(a.join("local.h"), "/* nothing */\n").unwrap();
        std::fs::write(a.join("limits.h"), "#include_next <limits.h>\n").unwrap();
        std::fs::write(b.join("limits.h"), "\n").unwrap();
        std::fs::write(b.join("unused.h"), "\n").unwrap();
        let got = header_closure(&["top.h"], &[a.clone(), b.clone()]);
        let _ = std::fs::remove_dir_all(&dir);
        let want: BTreeSet<PathBuf> =
            [a.join("top.h"), a.join("sys/one.h"), a.join("local.h"), a.join("limits.h"), b.join("limits.h")].into();
        assert_eq!(got.into_iter().collect::<BTreeSet<_>>(), want);
    }

    #[test]
    fn linker_script_paths() {
        let dir = std::env::temp_dir().join(format!("poly-ldscript-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("libc.so");
        std::fs::write(&p, "/* GNU ld script */\nGROUP ( /lib/x/libc.so.6 /usr/lib/x/libc_nonshared.a  AS_NEEDED ( /lib64/ld.so.2 ) )\n")
            .unwrap();
        let got = script_paths(&p);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(got, ["/lib/x/libc.so.6", "/usr/lib/x/libc_nonshared.a", "/lib64/ld.so.2"].map(PathBuf::from));
    }

    #[test]
    fn image_sizes_leave_room() {
        let mut t = RootTree::default();
        t.file("a", 0o644, &vec![0; 10 << 20]);
        let (size, inodes) = image_size(&t);
        assert!(size >= (170 << 20) && size % (4 << 20) == 0);
        assert!(inodes >= 16384 && inodes % 1024 == 0);
    }
}
