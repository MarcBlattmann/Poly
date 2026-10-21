/* access: mode is F_OK (0) or an OR of R_OK, W_OK, X_OK; any other bit is
 * EINVAL, checked before the path is looked at (so a missing path or an
 * unreadable pointer with a bad mode is EINVAL). The mode is a 32-bit
 * value: bits above 31 are ignored. */
#include "a_sys.h"

#define D "/tmp/a_access_mode"

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("bit3_einval", syscall2(SYS_access, D "/f", 8), -EINVAL);
    expect("bit4_einval", syscall2(SYS_access, D "/f", 0x10), -EINVAL);
    expect("r_plus_bit3_einval", syscall2(SYS_access, D "/f", R_OK | 8), -EINVAL);
    expect("at_eaccess_bit_einval", syscall2(SYS_access, D "/f", 0x200), -EINVAL);
    expect("bit31_einval", syscall2(SYS_access, D "/f", 0x80000000L), -EINVAL);
    expect("minus1_einval", syscall2(SYS_access, D "/f", -1), -EINVAL);
    expect("high32_ignored", syscall2(SYS_access, D "/f", 0x100000000L | R_OK), 0);
    expect("high32_only_ignored", syscall2(SYS_access, D "/f", 0x100000000L), 0);
    expect("einval_before_enoent", syscall2(SYS_access, D "/nope", 8), -EINVAL);
    expect("einval_before_efault", syscall2(SYS_access, 0, 8), -EINVAL);
    expect("einval_before_empty", syscall2(SYS_access, "", 0x10), -EINVAL);
    int all = 1;
    for (int m = 0; m < 8; m++) all &= syscall2(SYS_access, D, m) == 0;
    check("modes_0_to_7_dir", all, 0);
    int bad = 1;
    for (int m = 8; m < 64; m++) bad &= syscall2(SYS_access, D, m) == -EINVAL;
    check("modes_8_to_63_einval", bad, 0);
    expect("cleanup_unlink", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
