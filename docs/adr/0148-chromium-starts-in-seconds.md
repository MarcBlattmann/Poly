# 0148: Chromium starts in seconds, not minutes

**Status:** Accepted.

## Context

On `main`, Chromium (`cargo xtask chromium run`) took 60 to 95 s from the
Enter key in the launcher to its first window on the owner's PC (QEMU with
WHPX), 170 to 210 s when we measured it again for this ADR on the same PC
with other builds running, and about 11 minutes in the container (KVM
nested in WSL2, ten containers busy). Real Linux starts it in 1 to 2 s.
Targets: first launch on a fresh disk 5 s or less, later launches 2 to 3 s,
on WHPX.

Nothing was measured, so the first step was a number for every launch and a
profile of where the time goes (`docs/DEBUGGING.md`, "Where the time
goes"). A guess would have gone to fork and the 1000 Hz tick; the profile
said otherwise.

## Findings (WHPX, fresh disk, `poly.debug.profile`)

The machine was **idle 87% of the time** (4 vCPUs; 14 s of user time in
200 s), so the time was waiting, and most of the waiting was the disk:

- **710000 disk read requests of 512 bytes** (366 MB) and 187000 write
  requests (95 MB) to start. The disk's buffer cache was 64 KiB: 128
  blocks of 512 bytes, which one 64 KiB read-ahead of the page cache
  replaced entirely. A file system's metadata (inodes, extent trees,
  directories) was read from the disk again and again, and every read was
  one request per 512-byte block, merged only when requests happened to
  queue up together. Each request costs about 0.5 ms through QEMU on
  Windows.
- Dirty blocks were written one block at a time when evicted, or in
  slot order when synced, so `fdatasync` (217 calls, 72 s of thread time)
  wrote its megabytes as thousands of requests.
- A page fault took the memory's lock, then the file's inode lock and the
  file system's volume lock (to ask for the file's size), all of them held
  across disk reads: 105000 faults on Chromium's code took 387 s of thread
  time, 3.7 ms each, and 172000 faults waited 117 s for the memory's lock.

After those, with the rest of the profile (see the table):

- Each of the 100000 minor faults on the 300 MB program mapped one page.
- Every `Mutex::release` took the scheduler's one global lock to look for a
  waiter, and a waiter slept at once (a switch and a wake-up IPI, 25 µs
  each in a VM, 150000 of them) where the holder was done in microseconds.
- Every page-cache hit moved the page in the LRU order (two B-tree
  updates under the cache's global lock).

What did **not** matter, measured: the cost of the first fsync flush (583
flushes, 2 s), the tick (4000 timer interrupts a second cost 0.1% of the
critical path), fork (3 clones of 40 ms), and zeroing memory (1.7 s of
thread time, though touching RAM for the first time costs 50 µs a page
under WHPX on this PC, `poly.debug.membench`: 55 MB/s against 41 GB/s).
Reading the 300 MB program ahead of time in the background (tried, 1 MiB
reads from a kernel thread on exec) moved the waiting around but did not
shorten the launch, and was removed. A longer read-ahead for sequential
misses (64 pages) cut misses by 17% and nothing else, and was removed.

## Decision

In the order of measured impact:

1. **A buffer cache that fits the disk, and requests that fit the work**
   (`poly-block`, `kernel/src/block.rs`). A disk gets 1/32 of its size and
   at most 1/16 of the free memory, between 64 KiB and 16 MiB (physically
   contiguous frames from `frames::allocate_contiguous` when above 2 MiB).
   `BufferCache::read_blocks` copies the cached blocks of a read and reads
   each run of uncached ones from the device in one request straight into
   the caller's buffer; a run of up to 64 blocks (metadata) is kept in the
   cache too, a longer one (file data, which the page cache keeps) is not,
   so a big read does not push the metadata out. Memory the device cannot
   reach falls back to the old path. A cache with staging memory (128 KiB)
   writes a dirty block together with its dirty neighbours in one request,
   for an evicted block and in `sync`.
2. **Fault-around** (`kernel/src/user/memory.rs`): a fault on a file
   mapping also maps the pages of the same aligned 64 KiB window that the
   page cache holds, read-only, found with one range lookup of the cache
   (`CachedFs::cached_run`) and one walk of the page tables.
3. **Fewer locks on the fault path.** A page the cache holds is found
   without the file's size (which took the file system's volume lock, held
   across disk reads by whoever was reading): `cached_page`, `peek_cached`
   and `fill` ask for the size only for a page that is missing or reaches
   past the mapping's limit. The page cache moves a page in its LRU order
   only if it was last used more than 1/16 of the cache's uses ago.
4. **A mutex that does not bother the scheduler** (`kernel/src/sync.rs`):
   a thread that finds it held looks 64 times before it sleeps, and a
   release wakes a waiter only if one is sleeping (a counter, ordered with
   `SeqCst` against the release, so no wake-up is lost).
5. **Measurement as a permanent feature.** The compositor prints
   `launch-to-window` for every launch (`userland/poly-compositor/src/launch_timer.rs`,
   host-tested); `poly.debug.profile` gains the lines in the table in
   `docs/DEBUGGING.md` (system calls, disk, faults, process timeline with
   CPU times, a sampler of where each program's first thread waits, IPIs
   and halts); `poly.strace.slow` shows when a call ended;
   `poly.debug.membench`; `POLY_CHROMIUM_QEMU` for QEMU arguments.

## Timeline

Launch to first window, guest seconds, WHPX on the owner's PC with other
agents' builds and containers running (host CPU 75 to 100% busy; the same
build varied by about 10% between runs). "Second" is a second launch in the
same boot after the first Chromium was killed.

| Step | First launch (fresh disk) | Second, same boot |
|---|---|---|
| `main` | 171 s, 177 s, 193 s, 212 s | not measured |
| 1: cache sized to the disk, big reads, staged writes | 15.4 s | 5.5 s |
| + 2: fault-around | 11.6 s | 4.5 s |
| + 3: page-cache lookups (lazy LRU, one range lookup, no size lookup on a hit) | 11.5 s, 10.2 s | 3.0 s, 2.8 s |
| + 4: mutex | 11.3 s | 2.9 s |
| final, with the profile on | 10.3 to 11.1 s | 2.3 to 2.7 s |
| final, no profile, host a little quieter | 9.2 s | 1.7 s |

Counts of the profile, `main` (one launch) against the final kernel (the
first launch and the second one in the same boot):

| | `main` | final |
|---|---|---|
| disk read requests (user level) | 715000 (366 MB) | 8200 (320 MB) |
| disk write requests | 187000 (95 MB) | 4400 (110 MB) |
| requests to the device, all | 603000 | 13000 |
| time in disk requests, summed over threads | 532 s | 4.4 s |
| page faults in file mappings, first launch | 127000 | 39000 |
| reschedule IPIs (steps 1 to 3, then 4, then the end) | 152000 | 59000, 47000 |

A later boot on the same disk (profile and caches there, page cache cold):
8.2 s, and 1.9 s for a second launch in that boot.

KVM in the container (nested in WSL2, ten other containers running), first launch on a fresh disk: 676 s on `main`, 86 s with this ADR. (Nested KVM turns every VM exit into several host exits, so it is no guide to the owner's PC; the WHPX column is.)

## What is left

The first launch is still 10 s, twice the target; the second launch is in
it. The sampler says the browser's first thread is, during start-up, 58%
running (most of it in the kernel: 11.3 s of system time against 1.4 s of
user time in all Chromium processes), 35% blocked in page faults, 7% waiting
for a CPU. Where it goes next, not done here:

- Cold misses of the 300 MB program still take 0.8 ms each (10600 of them,
  9 s of thread time), of which the disk is 0.33 ms. The file system's
  volume lock is held across the disk read, so concurrent misses queue.
  A read path without that lock (extent lookup under the lock, the disk
  read outside it) would let several go to the disk at once.
- Private file mappings (relocations written into the program's data, 31000
  copies) read one page at a time through `read_at`.
- `fdatasync` writes back the whole volume (ADR 0040: no per-file sync) and
  commits the journal: 254 calls at 14 ms.
- The tick is periodic at 1000 Hz on idle CPUs. It costs little on the
  critical path, but a tickless idle would stop 4000 timer interrupts a
  second in a virtual machine at rest.

## Consequences

- Every disk file system reads and writes in requests of 4 KiB to 1 MiB
  instead of 512 bytes. A disk of the size of a test disk keeps its 64 KiB
  cache; the test disks' read-ahead assertion now says one request.
- The root disk's cache takes up to 16 MiB of contiguous memory at boot.
- A page-cache page is found without the file's attributes; the invariant
  it relies on (a page past the end of a file is never cached) is the one
  `truncate` already keeps.
- `Mutex` spins for up to a few microseconds before it sleeps: a little
  CPU for much less scheduling.
- The new profile lines cost nothing without `poly.debug.profile`: one
  atomic load per system call, fault and halt.
