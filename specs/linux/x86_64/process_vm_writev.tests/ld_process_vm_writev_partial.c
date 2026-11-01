/* process_vm_writev partial transfers (on the caller itself): the copy
 * stops at the first remote byte that cannot be written (unmapped,
 * read-only, PROT_NONE) or local byte that cannot be read, returning the
 * bytes copied so far, or EFAULT when nothing could be copied. Writing to
 * read-only memory of the target is EFAULT (no forced write).
 * poly-diff: syscalls=names (the caller's pid is an argument)
 */
#include "a_sys.h"

#define SYS_process_vm_writev 311

static char src[64] = "0123456789abcdefghij";

static i64 wr(i64 pid, struct a_iovec *l, i64 nl, struct a_iovec *r, i64 nr) {
    return syscall6(SYS_process_vm_writev, pid, (i64)l, nl, (i64)r, nr, 0);
}

int test_main(void) {
    i64 me = syscall0(SYS_getpid);
    /* Mapped first, so that no later mapping fills the hole made below. */
    char *ro = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    char *none = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    char *m =(char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("unmap_second_page", syscall2(SYS_munmap, m + A_PAGE, A_PAGE), 0);

    struct a_iovec l = {src, 20};
    struct a_iovec r = {m + A_PAGE - 3, 20};
    expect("remote_runs_into_hole_3", wr(me, &l, 1, &r, 1), 3);
    check("partial_data", mem_eq(m + A_PAGE - 3, "012", 3), m[A_PAGE - 3]);
    struct a_iovec r2[2] = {{m, 5}, {m + A_PAGE, 5}};
    expect("second_remote_unmapped_5", wr(me, &l, 1, r2, 2), 5);
    struct a_iovec rbad = {m + A_PAGE, 5};
    expect("remote_unmapped_efault", wr(me, &l, 1, &rbad, 1), -EFAULT);

    struct a_iovec rro = {ro, 5};
    expect("remote_readonly_efault", wr(me, &l, 1, &rro, 1), -EFAULT);
    check("readonly_untouched", ro[0] == 0, ro[0]);
    struct a_iovec rnone = {none, 5};
    expect("remote_prot_none_efault", wr(me, &l, 1, &rnone, 1), -EFAULT);

    struct a_iovec lh = {m + A_PAGE - 2, 10};
    static char dst[16];
    struct a_iovec rd = {dst, 10};
    m[A_PAGE - 2] = 'Y';
    m[A_PAGE - 1] = 'Z';
    expect("local_runs_into_hole_2", wr(me, &lh, 1, &rd, 1), 2);
    check("local_partial_data", dst[0] == 'Y' && dst[1] == 'Z' && dst[2] == 0, dst[0]);
    struct a_iovec lbad = {m + A_PAGE, 10};
    expect("local_unmapped_efault", wr(me, &lbad, 1, &rd, 1), -EFAULT);
    /* Local read-only memory is a fine source. */
    struct a_iovec lro = {ro, 4};
    memset(dst, 7, 4);
    expect("local_readonly_source_4", wr(me, &lro, 1, &rd, 1), 4);
    check("copied_zeros", dst[0] == 0 && dst[3] == 0, dst[0]);
    return finish();
}
