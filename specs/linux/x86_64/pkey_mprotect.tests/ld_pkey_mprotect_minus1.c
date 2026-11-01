/* pkey_mprotect(addr, len, prot, -1) is mprotect: it changes the
 * protection of the pages and returns 0. Memory made PROT_READ and then
 * PROT_READ|PROT_WRITE again keeps its contents and can be written. pkey
 * is a 32-bit int, so 0xffffffff is -1 too. len is rounded up to whole
 * pages, len 0 does nothing (0). PROT_SEM (8) is accepted on x86-64.
 * Works the same with or without protection keys.
 */
#include "a_sys.h"

#define SYS_pkey_mprotect 329
/* asm-generic/mman-common.h */
#define PROT_SEM 0x8

static unsigned char vec[4];

int test_main(void) {
    char *m = (char *)syscall6(SYS_mmap, 0, 3 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    m[0] = 'a';
    m[A_PAGE] = 'b';
    expect("read_only", syscall4(SYS_pkey_mprotect, m, A_PAGE, PROT_READ, -1), 0);
    check("readable", m[0] == 'a', m[0]);
    expect("read_write", syscall4(SYS_pkey_mprotect, m, A_PAGE, PROT_READ | PROT_WRITE, -1), 0);
    m[0] = 'c';
    check("writable", m[0] == 'c', m[0]);
    expect("key_0xffffffff_is_minus1", syscall4(SYS_pkey_mprotect, m, A_PAGE, PROT_READ, 0xffffffffUL), 0);
    expect("back_rw", syscall4(SYS_pkey_mprotect, m, 3 * A_PAGE, PROT_READ | PROT_WRITE, -1), 0);
    m[0] = 'd';
    check("writable_2", m[0] == 'd', m[0]);
    /* A short len covers its whole page; the next page is untouched. */
    expect("short_len_none", syscall4(SYS_pkey_mprotect, m + 2 * A_PAGE, 1, PROT_NONE, -1), 0);
    m[A_PAGE] = 'e';
    check("next_page_untouched", m[A_PAGE] == 'e', m[A_PAGE]);
    expect("mincore_still_mapped", syscall3(SYS_mincore, m, 3 * A_PAGE, vec), 0);
    expect("back_rw_2", syscall4(SYS_pkey_mprotect, m + 2 * A_PAGE, A_PAGE, PROT_READ | PROT_WRITE, -1), 0);
    m[2 * A_PAGE] = 'f';
    check("third_page_writable", m[2 * A_PAGE] == 'f', m[2 * A_PAGE]);
    expect("len_0", syscall4(SYS_pkey_mprotect, m, 0, PROT_NONE, -1), 0);
    m[0] = 'g';
    check("len_0_changed_nothing", m[0] == 'g', m[0]);
    expect("prot_sem", syscall4(SYS_pkey_mprotect, m, A_PAGE, PROT_READ | PROT_WRITE | PROT_SEM, -1), 0);
    m[0] = 'h';
    check("after_prot_sem", m[0] == 'h', m[0]);
    expect("exec", syscall4(SYS_pkey_mprotect, m + A_PAGE, A_PAGE, PROT_READ | PROT_EXEC, -1), 0);
    check("exec_page_readable", m[A_PAGE] == 'e', m[A_PAGE]);
    expect("munmap", syscall2(SYS_munmap, m, 3 * A_PAGE), 0);
    return finish();
}
