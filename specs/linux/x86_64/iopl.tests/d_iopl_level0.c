/* iopl(0) at level 0 (the level every process starts with, whoever runs
 * it) returns 0, any number of times; it needs no privilege, also with all
 * capabilities dropped. Lowering to the current level is a no-op.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_iopl 172
/* linux/capability.h */
#define LINUX_CAPABILITY_VERSION_3 0x20080522

int test_main(void) {
    i64 r = syscall1(SYS_iopl, 0);
    check("iopl0_returns_0", r == 0, r);
    r = syscall1(SYS_iopl, 0);
    check("iopl0_again_returns_0", r == 0, r);
    struct { unsigned version; int pid; } hdr = { LINUX_CAPABILITY_VERSION_3, 0 };
    unsigned data[6] = { 0 };
    r = syscall2(SYS_capset, &hdr, data);
    check("capset_drop_all", r == 0, r);
    r = syscall1(SYS_iopl, 0);
    check("iopl0_without_caps_returns_0", r == 0, r);
    for (int i = 0; i < 3; i++) syscall1(SYS_iopl, 0);
    r = syscall1(SYS_iopl, 0);
    check("iopl0_repeated_returns_0", r == 0, r);
    return finish();
}
