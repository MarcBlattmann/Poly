/* membarrier(MEMBARRIER_CMD_QUERY 0, 0, 0) returns the bit mask of
 * supported commands: 0x3ff here (GLOBAL 1, GLOBAL_EXPEDITED 2,
 * REGISTER_GLOBAL_EXPEDITED 4, PRIVATE_EXPEDITED 8,
 * REGISTER_PRIVATE_EXPEDITED 16, PRIVATE_EXPEDITED_SYNC_CORE 32,
 * REGISTER_PRIVATE_EXPEDITED_SYNC_CORE 64, PRIVATE_EXPEDITED_RSEQ 128,
 * REGISTER_PRIVATE_EXPEDITED_RSEQ 256, GET_REGISTRATIONS 512; a kernel
 * booted with nohz_full leaves out GLOBAL). QUERY with flags non-zero is
 * EINVAL; the third argument (cpu_id) is ignored. cmd is a 32-bit int
 * (upper half ignored). Anything that is not one command (3, 1024, -1,
 * 0x80000000) is EINVAL.
 */
#include "a_sys.h"

#define SYS_membarrier 324

static i64 mb(u64 cmd, u64 flags, u64 cpu) { return syscall3(SYS_membarrier, cmd, flags, cpu); }

int test_main(void) {
    expect("query", mb(0, 0, 0), 0x3ff);
    expect("query_cpu_ignored", mb(0, 0, 12345), 0x3ff);
    expect("query_flags_1_einval", mb(0, 1, 0), -EINVAL);
    expect("query_flags_2_einval", mb(0, 2, 0), -EINVAL);
    expect("query_cmd_upper_bits", mb(1UL << 32, 0, 0), 0x3ff);
    expect("cmd_3_einval", mb(3, 0, 0), -EINVAL);
    expect("cmd_6_einval", mb(6, 0, 0), -EINVAL);
    expect("cmd_1024_einval", mb(1024, 0, 0), -EINVAL);
    expect("cmd_minus1_einval", mb(0xffffffffUL, 0, 0), -EINVAL);
    expect("cmd_bit31_einval", mb(0x80000000UL, 0, 0), -EINVAL);
    return finish();
}
