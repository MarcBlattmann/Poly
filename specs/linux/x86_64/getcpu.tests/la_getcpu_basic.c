/* getcpu(cpu, node, tcache) stores the number of the CPU the caller
 * runs on at *cpu and its NUMA node at *node (u32 each; 0 on a single
 * node machine) and returns 0. Either pointer may be NULL (that value is
 * not stored); the third argument is ignored, even an invalid address.
 * The CPU is one the thread may run on (a bit of its sched_getaffinity
 * mask); after pinning the thread to one CPU, getcpu reports that CPU.
 * poly-diff: syscalls=names (sched_getaffinity returns the kernel's mask size, which depends on the machine)
 */
#include "a_sys.h"

#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204
#define SYS_getcpu 309

static u64 mask[16];

int test_main(void) {
    u32 cpu = 0xdead, node = 0xbeef;
    expect("returns_0", syscall3(SYS_getcpu, &cpu, &node, 0), 0);
    check("cpu_written", cpu != 0xdead && cpu < 1024, cpu);
    expect("node_0", node, 0);

    i64 n = syscall3(SYS_sched_getaffinity, 0, sizeof mask, mask);
    check("affinity_ok", n > 0, n);
    check("cpu_in_affinity", (mask[cpu / 64] >> (cpu % 64)) & 1, cpu);

    u32 only = 0xdead;
    expect("node_null", syscall3(SYS_getcpu, &only, 0, 0), 0);
    check("cpu_alone_written", only < 1024, only);
    node = 0xbeef;
    expect("cpu_null", syscall3(SYS_getcpu, 0, &node, 0), 0);
    expect("node_alone", node, 0);
    expect("both_null", syscall3(SYS_getcpu, 0, 0, 0), 0);
    expect("tcache_ignored", syscall3(SYS_getcpu, &cpu, &node, 8), 0);
    expect("tcache_garbage_ignored", syscall3(SYS_getcpu, &cpu, &node, 0x800000000000UL), 0);

    /* Pin to the lowest CPU allowed, then to the current one again. */
    u32 low = 0;
    while (!((mask[low / 64] >> (low % 64)) & 1)) low++;
    u64 one[16] = { 0 };
    one[low / 64] = 1UL << (low % 64);
    expect("pin_lowest", syscall3(SYS_sched_setaffinity, 0, sizeof one, one), 0);
    cpu = 0xdead;
    syscall3(SYS_getcpu, &cpu, 0, 0);
    expect("reports_pinned_lowest", cpu, low);

    u32 high = 1023;
    while (!((mask[high / 64] >> (high % 64)) & 1)) high--;
    u64 other[16] = { 0 };
    other[high / 64] = 1UL << (high % 64);
    expect("pin_highest", syscall3(SYS_sched_setaffinity, 0, sizeof other, other), 0);
    cpu = 0xdead;
    syscall3(SYS_getcpu, &cpu, 0, 0);
    expect("reports_pinned_highest", cpu, high);
    expect("restore_mask", syscall3(SYS_sched_setaffinity, 0, sizeof mask, mask), 0);
    return finish();
}
