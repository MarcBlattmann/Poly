/* sched_setaffinity(pid, len, mask) sets the CPU affinity of thread pid
 * (0: the caller) to the CPUs in the first len bytes of mask and returns
 * 0; the thread then runs only there (getcpu reports the pinned CPU) and
 * sched_getaffinity reads the new mask back. A short mask (len smaller
 * than the kernel's mask size, any byte count, not only multiples of 8)
 * means the missing bytes are 0. Bits of CPUs that do not exist are
 * ignored as long as one existing allowed CPU is in the mask. The
 * caller's own pid works like 0. The original mask can be put back.
 * poly-diff: syscalls=names (sched_getaffinity returns the kernel's mask size, which depends on the machine)
 */
#include "a_sys.h"

#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204
#define SYS_getcpu 309

static u64 orig[16], got[16], m[16];

static int only_bit(const u64 *mask, i64 n, u32 cpu) {
    for (int i = 0; i < n / 8; i++)
        if (mask[i] != (i == (int)(cpu / 64) ? 1UL << (cpu % 64) : 0)) return 0;
    return n > 0;
}

static u32 where(void) {
    u32 c = 0xdead;
    syscall3(SYS_getcpu, &c, 0, 0);
    return c;
}

int test_main(void) {
    i64 n = syscall3(SYS_sched_getaffinity, 0, sizeof orig, orig);
    check("get_original", n > 0, n);
    u32 cpu = where();

    memset(m, 0, sizeof m);
    m[cpu / 64] = 1UL << (cpu % 64);
    expect("pin_current", syscall3(SYS_sched_setaffinity, 0, sizeof m, m), 0);
    memset(got, 0, sizeof got);
    syscall3(SYS_sched_getaffinity, 0, sizeof got, got);
    check("read_back_one_bit", only_bit(got, n, cpu), cpu);
    expect("runs_there", where(), cpu);

    /* The shortest mask that holds the bit: cpu / 8 + 1 bytes. */
    expect("restore_before_short", syscall3(SYS_sched_setaffinity, 0, sizeof orig, orig), 0);
    expect("short_mask", syscall3(SYS_sched_setaffinity, 0, cpu / 8 + 1, m), 0);
    memset(got, 0, sizeof got);
    syscall3(SYS_sched_getaffinity, 0, sizeof got, got);
    check("short_mask_rest_zero", only_bit(got, n, cpu), cpu);

    /* A nonexistent CPU's bit (1023) next to a real one: ignored. */
    m[15] |= 1UL << 63;
    expect("extra_nonexistent_bit", syscall3(SYS_sched_setaffinity, 0, sizeof m, m), 0);
    memset(got, 0, sizeof got);
    syscall3(SYS_sched_getaffinity, 0, sizeof got, got);
    check("nonexistent_bit_dropped", n == 128 || only_bit(got, n, cpu), cpu);

    expect("own_pid", syscall3(SYS_sched_setaffinity, syscall0(SYS_getpid), sizeof orig, orig), 0);
    memset(got, 0, sizeof got);
    syscall3(SYS_sched_getaffinity, 0, sizeof got, got);
    check("original_restored", mem_eq(got, orig, (size_t)n), 0);
    return finish();
}
