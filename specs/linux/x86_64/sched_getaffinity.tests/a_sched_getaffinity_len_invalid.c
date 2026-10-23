/* len must be a multiple of 8 (the size of a long) and not 0: len 0, 1,
 * 4, 7, 9, 12, 15, 17, 100 are EINVAL (22), and nothing is written. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_getaffinity 204

int test_main(void) {
    static const i64 lens[] = {0, 1, 4, 7, 9, 12, 15, 17, 100};
    static const char *const names[] = {"len0", "len1", "len4", "len7", "len9", "len12", "len15", "len17", "len100"};
    u64 m[16];
    for (int i = 0; i < 9; i++) {
        for (int j = 0; j < 16; j++) m[j] = 0xaaaaaaaaaaaaaaaaUL;
        expect(names[i], syscall3(SYS_sched_getaffinity, 0, lens[i], m), -EINVAL);
        int same = 1;
        for (int j = 0; j < 16; j++) same &= m[j] == 0xaaaaaaaaaaaaaaaaUL;
        check(names[i], same, 0);
    }
    /* The error is not sticky. */
    expect("len8_after", syscall3(SYS_sched_getaffinity, 0, 8, m), 8);
    check("len8_after_mask", m[0] != 0 && m[0] != 0xaaaaaaaaaaaaaaaaUL, 0);
    return finish();
}
