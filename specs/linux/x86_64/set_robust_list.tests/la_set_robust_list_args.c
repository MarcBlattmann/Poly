/* set_robust_list(head, len) records the calling thread's robust futex
 * list head and returns 0. len must be exactly 24 (the size of struct
 * robust_list_head on x86-64): 0, 23, 25, 48 and 24 with bits above 31
 * set are EINVAL (len is a full 64-bit size). head is not checked or
 * read at the call: NULL, a small number, an unmapped address all
 * work. get_robust_list(0, ...) reads back the last head set and len
 * 24; a new program starts with head NULL (and len 24).
 */
#include "a_sys.h"

#define SYS_set_robust_list 273
#define SYS_get_robust_list 274

static i64 set(u64 head, u64 len) { return syscall2(SYS_set_robust_list, head, len); }
static u64 got_head, got_len;
static i64 get(void) {
    got_head = 0x5555;
    got_len = 0x5555;
    return syscall3(SYS_get_robust_list, 0, &got_head, &got_len);
}

int test_main(void) {
    expect("initial_get", get(), 0);
    expect("initial_head_null", (i64)got_head, 0);
    expect("initial_len_24", (i64)got_len, 24);

    expect("set_ok", set(0x1234, 24), 0);
    get();
    expect("head_read_back", (i64)got_head, 0x1234);
    expect("len_read_back", (i64)got_len, 24);

    expect("len_0_einval", set(0x9999, 0), -EINVAL);
    expect("len_23_einval", set(0x9999, 23), -EINVAL);
    expect("len_25_einval", set(0x9999, 25), -EINVAL);
    expect("len_48_einval", set(0x9999, 48), -EINVAL);
    expect("len_upper_bits_einval", set(0x9999, (1UL << 32) | 24), -EINVAL);
    get();
    expect("failed_set_keeps_head", (i64)got_head, 0x1234);

    expect("null_head_ok", set(0, 24), 0);
    get();
    expect("null_head_read_back", (i64)got_head, 0);
    char *hole = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, hole, A_PAGE);
    expect("unmapped_head_ok", set((u64)hole, 24), 0);
    get();
    check("unmapped_head_read_back", got_head == (u64)hole, 0);
    expect("non_canonical_head_ok", set(0x800000000000UL, 24), 0);
    get();
    check("non_canonical_read_back", got_head == 0x800000000000UL, 0);
    expect("reset_null", set(0, 24), 0);
    return finish();
}
