/* Exactly 4 bytes (struct sched_param's one int) are written, at any
 * alignment: the bytes around them keep their values, and the last 4
 * bytes of a page followed by an unmapped page work. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_getparam 143

int test_main(void) {
    u8 buf[32];
    for (int i = 0; i < 32; i++) buf[i] = 0xa5;
    expect("aligned", syscall2(SYS_sched_getparam, 0, buf + 8), 0);
    int ok = 1;
    for (int i = 0; i < 32; i++) ok &= buf[i] == ((i >= 8 && i < 12) ? 0 : 0xa5);
    check("aligned_only_4_bytes", ok, 0);
    for (int i = 0; i < 32; i++) buf[i] = 0x5a;
    expect("unaligned", syscall2(SYS_sched_getparam, 0, buf + 13), 0);
    ok = 1;
    for (int i = 0; i < 32; i++) ok &= buf[i] == ((i >= 13 && i < 17) ? 0 : 0x5a);
    check("unaligned_only_4_bytes", ok, 0);
    u8 *m = (u8 *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("unmap_second", syscall2(SYS_munmap, m + A_PAGE, A_PAGE), 0);
    for (int i = 0; i < 16; i++) m[A_PAGE - 16 + i] = 0x77;
    expect("page_end", syscall2(SYS_sched_getparam, 0, m + A_PAGE - 4), 0);
    expect("page_end_val", *(i32 *)(m + A_PAGE - 4), 0);
    expect("before_kept", *(i32 *)(m + A_PAGE - 8), 0x77777777);
    return finish();
}
