/* sync() ignores whatever is in the argument registers: garbage numbers,
 * -1, wild pointers and kernel addresses all give 0. */
#include "a_sys.h"

#define SYS_sync 162

int test_main(void) {
    expect("zeros", syscall6(SYS_sync, 0, 0, 0, 0, 0, 0), 0);
    expect("minus_one", syscall6(SYS_sync, -1, -1, -1, -1, -1, -1), 0);
    expect("bad_fd_like", syscall1(SYS_sync, 9999), 0);
    expect("wild_ptr", syscall2(SYS_sync, 0x10, 0xdead0000), 0);
    expect("kernel_addr", syscall3(SYS_sync, (i64)0xffff800000000000UL, (i64)0xffffffffff600000UL, 1), 0);
    expect("high_bits", syscall6(SYS_sync, (i64)0x8000000000000000UL, 1, 2, 3, 4, 5), 0);
    return finish();
}
