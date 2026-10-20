/* arch_prctl: set the FS base, read it back, load through %fs. */
#include "poly_test.h"

static u64 block[4] = {0x1122334455667788UL, 0, 0, 0};

int test_main(void) {
    block[1] = (u64)block; /* a TLS block conventionally points at itself */
    i64 r = syscall2(SYS_arch_prctl, ARCH_SET_FS, block);
    check("arch_prctl_set_fs", r == 0, r);

    u64 got = 0;
    r = syscall2(SYS_arch_prctl, ARCH_GET_FS, &got);
    check("arch_prctl_get_fs", r == 0, r);
    check("arch_prctl_get_fs_matches", got == (u64)block, 0);

    u64 via_fs;
    __asm__ volatile("mov %%fs:0, %0" : "=r"(via_fs));
    check("fs_relative_load_uses_base", via_fs == 0x1122334455667788UL, 0);

    r = syscall2(SYS_arch_prctl, 0x9999, 0);
    check("arch_prctl_bad_code_einval", r == -EINVAL, r);
    r = syscall2(SYS_arch_prctl, ARCH_GET_FS, 8);
    check("arch_prctl_get_fs_bad_pointer_efault", r == -EFAULT, r);
    return finish();
}
