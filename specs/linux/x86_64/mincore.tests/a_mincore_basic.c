/* mincore: one byte per page; bit 0 is set for pages that are resident.
 * Pages that were written are resident; only bit 0 is meaningful. */
#include "a_sys.h"

int test_main(void) {
    unsigned char vec[8];
    i64 m = syscall6(SYS_mmap, 0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    char *p = (char *)m;
    for (int i = 0; i < 4; i++) p[i * A_PAGE] = 1;
    memset(vec, 0xee, sizeof vec);
    expect("mincore_4_pages", syscall3(SYS_mincore, m, 4 * A_PAGE, vec), 0);
    check("written_pages_resident", (vec[0] & 1) && (vec[1] & 1) && (vec[2] & 1) && (vec[3] & 1), vec[0]);
    check("only_low_bits_used", (vec[0] & 0xfe) == 0, vec[0]);
    check("bytes_after_untouched", vec[4] == 0xee, vec[4]);
    expect("partial_page_length", syscall3(SYS_mincore, m, A_PAGE + 1, vec), 0);
    check("rounded_to_two_pages", (vec[1] & 1) && vec[2] != 0xee, vec[2]);
    expect("length_0_ok", syscall3(SYS_mincore, m, 0, vec), 0);
    /* Text of this program is mapped and resident while it runs. */
    i64 text = (i64)&test_main & ~(i64)(A_PAGE - 1);
    expect("own_code_page", syscall3(SYS_mincore, text, A_PAGE, vec), 0);
    check("own_code_resident", vec[0] & 1, vec[0]);
    syscall2(SYS_munmap, m, 4 * A_PAGE);
    return finish();
}
