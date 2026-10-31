/* getcpu with an unwritable pointer (a low address, read-only memory,
 * a non-canonical address) for cpu or node is EFAULT. The two values are
 * stored independently: when only cpu is bad, node is still written, and
 * the other way round. getcpu takes no input, so nothing else fails.
 */
#include "a_sys.h"

#define SYS_getcpu 309

int test_main(void) {
    u32 *ro = (u32 *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    u32 cpu = 0xdead, node = 0xbeef;
    expect("bad_cpu_efault", syscall3(SYS_getcpu, 8, &node, 0), -EFAULT);
    expect("node_written_anyway", node, 0);
    expect("bad_node_efault", syscall3(SYS_getcpu, &cpu, 8, 0), -EFAULT);
    check("cpu_written_anyway", cpu != 0xdead, cpu);
    expect("read_only_cpu_efault", syscall3(SYS_getcpu, ro, 0, 0), -EFAULT);
    expect("read_only_node_efault", syscall3(SYS_getcpu, 0, ro, 0), -EFAULT);
    expect("read_only_untouched", *ro, 0);
    expect("non_canonical_efault", syscall3(SYS_getcpu, 0x800000000000UL, 0, 0), -EFAULT);
    expect("both_bad_efault", syscall3(SYS_getcpu, 8, 16, 0), -EFAULT);
    return finish();
}
