/* mlockall: flags must be a non-empty combination of MCL_CURRENT,
 * MCL_FUTURE and MCL_ONFAULT, and MCL_ONFAULT needs one of the other two;
 * anything else is EINVAL. Only the low 32 bits of the argument count. */
#include "a_sys.h"

int test_main(void) {
    expect("zero_einval", syscall1(SYS_mlockall, 0), -EINVAL);
    expect("onfault_alone_einval", syscall1(SYS_mlockall, MCL_ONFAULT), -EINVAL);
    expect("unknown_bit_8_einval", syscall1(SYS_mlockall, 8), -EINVAL);
    expect("unknown_bit_with_current_einval", syscall1(SYS_mlockall, MCL_CURRENT | 8), -EINVAL);
    expect("unknown_bit_0x10_einval", syscall1(SYS_mlockall, MCL_FUTURE | 0x10), -EINVAL);
    expect("all_ones_einval", syscall1(SYS_mlockall, -1), -EINVAL);
    expect("bit31_einval", syscall1(SYS_mlockall, 0x80000001L), -EINVAL);
    expect("current", syscall1(SYS_mlockall, MCL_CURRENT), 0);
    expect("future", syscall1(SYS_mlockall, MCL_FUTURE), 0);
    expect("current_future", syscall1(SYS_mlockall, MCL_CURRENT | MCL_FUTURE), 0);
    expect("current_onfault", syscall1(SYS_mlockall, MCL_CURRENT | MCL_ONFAULT), 0);
    expect("future_onfault", syscall1(SYS_mlockall, MCL_FUTURE | MCL_ONFAULT), 0);
    expect("all_three", syscall1(SYS_mlockall, MCL_CURRENT | MCL_FUTURE | MCL_ONFAULT), 0);
    expect("upper_32_bits_ignored", syscall1(SYS_mlockall, 0x100000001L), 0);
    expect("upper_32_bits_only_einval", syscall1(SYS_mlockall, 0x100000000L), -EINVAL);
    expect("again_ok", syscall1(SYS_mlockall, MCL_CURRENT), 0);
    return finish();
}
