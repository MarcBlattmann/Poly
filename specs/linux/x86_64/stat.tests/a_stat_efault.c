/* stat: unreadable path pointers and unwritable buffers; which error wins
 * when both the path and the buffer are bad; the buffer is left alone on
 * failure and nothing past its 144 bytes is written. */
#include "a_sys.h"

#define D "/tmp/a_stat_efault"
#define F D "/f"

static i64 st(const char *p, void *s) { return syscall2(SYS_stat, p, s); }

int test_main(void) {
    a_unlink(F);
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "abc", 0644), 0);

    /* Two pages; the second is made inaccessible. */
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("setup_mmap", (i64)m > 0, (i64)m);
    expect("setup_mprotect", syscall3(SYS_mprotect, m + A_PAGE, A_PAGE, PROT_NONE), 0);

    struct a_stat s;
    expect("null_path_efault", st(0, &s), -EFAULT);
    expect("low_path_efault", st((const char *)16, &s), -EFAULT);
    expect("unmapped_path_efault", st(m + A_PAGE, &s), -EFAULT);
    expect("kernel_path_efault", st((const char *)0xffff800000000000UL, &s), -EFAULT);
    /* A path whose NUL would be in the inaccessible page. */
    char *tail = m + A_PAGE - 4;
    tail[0] = '/';
    tail[1] = 't';
    tail[2] = 'm';
    tail[3] = 'p';
    expect("unterminated_path_efault", st(tail, &s), -EFAULT);
    /* The same text ending right at the boundary is fine. */
    tail = m + A_PAGE - 5;
    str_cpy(tail, "/tmp");
    expect("path_ending_at_boundary_ok", st(tail, &s), 0);

    expect("null_buf_efault", st(F, 0), -EFAULT);
    expect("low_buf_efault", st(F, (void *)16), -EFAULT);
    expect("protnone_buf_efault", st(F, m + A_PAGE), -EFAULT);
    expect("kernel_buf_efault", st(F, (void *)0xffff800000000000UL), -EFAULT);
    expect("readonly_buf_setup", syscall3(SYS_mprotect, m + A_PAGE, A_PAGE, PROT_READ), 0);
    expect("readonly_buf_efault", st(F, m + A_PAGE), -EFAULT);
    expect("restore_prot_none", syscall3(SYS_mprotect, m + A_PAGE, A_PAGE, PROT_NONE), 0);
    /* A buffer that starts in the good page and runs into the bad one. */
    expect("straddling_buf_efault", st(F, m + A_PAGE - 72), -EFAULT);
    /* A buffer that ends exactly at the boundary is fine. */
    struct a_stat *edge = (struct a_stat *)(m + A_PAGE - sizeof(struct a_stat));
    expect("buf_ending_at_boundary_ok", st(F, edge), 0);
    expect("buf_ending_at_boundary_size", edge->st_size, 3);

    /* The path is looked up before the buffer is written. */
    expect("missing_path_null_buf_enoent", st(D "/nope", 0), -ENOENT);
    expect("enotdir_path_null_buf_enotdir", st(F "/x", 0), -ENOTDIR);
    expect("bad_path_bad_buf_efault", st(0, 0), -EFAULT);

    /* Exactly 144 bytes are written; a failed call writes nothing. */
    unsigned char big[200];
    for (int i = 0; i < 200; i++) big[i] = 0xa5;
    expect("canary_stat", st(F, big), 0);
    int beyond = 1;
    for (int i = 144; i < 200; i++) beyond &= big[i] == 0xa5;
    check("nothing_past_144_bytes", beyond, 0);
    expect("struct_size_144", sizeof(struct a_stat), 144);
    for (int i = 0; i < 200; i++) big[i] = 0xa5;
    expect("failed_stat", st(D "/nope", big), -ENOENT);
    int untouched = 1;
    for (int i = 0; i < 200; i++) untouched &= big[i] == 0xa5;
    check("failed_stat_writes_nothing", untouched, 0);

    syscall2(SYS_munmap, m, 2 * A_PAGE);
    expect("cleanup_unlink", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
