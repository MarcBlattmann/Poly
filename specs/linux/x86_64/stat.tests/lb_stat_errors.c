/* stat(2) errors: ENOENT, ENOTDIR, ENAMETOOLONG, EFAULT for the path and
 * for the buffer (only once the path resolved), EACCES for a directory
 * without search permission (root passes); the buffer is left alone on
 * error. */
#include "a_sys.h"

#define D "/tmp/lb_stat_errors"

static i64 stat_(const char *p, void *st) { return syscall2(SYS_stat, p, st); }

static char buf[8192];
static unsigned char raw[160];

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_g", a_make_file(D "/sub/g", "x", 0644), 0);

    expect("missing", stat_(D "/nope", &st), -ENOENT);
    expect("missing_dir", stat_(D "/nope/x", &st), -ENOENT);
    expect("empty", stat_("", &st), -ENOENT);
    expect("file_as_dir", stat_(D "/f/x", &st), -ENOTDIR);
    expect("file_slash", stat_(D "/f/", &st), -ENOTDIR);
    expect("file_dot", stat_(D "/f/.", &st), -ENOTDIR);
    expect("null_path", stat_((const char *)0, &st), -EFAULT);
    expect("kernel_path", stat_((const char *)0xffffffff81000000ul, &st), -EFAULT);
    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 's';
    p[256] = 0;
    expect("name_256", stat_(buf, &st), -ENAMETOOLONG);
    p[255] = 0;
    expect("name_255", stat_(buf, &st), -ENOENT);
    for (int i = 0; i < 4095; i++) buf[i] = (i % 2) ? '.' : '/';
    buf[4095] = 0;
    expect("path_4095", stat_(buf, &st), 0);
    buf[4095] = '/';
    buf[4096] = 0;
    expect("path_4096", stat_(buf, &st), -ENAMETOOLONG);

    /* the buffer */
    expect("null_buf", stat_(D "/f", (void *)0), -EFAULT);
    char *ro = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("readonly_buf", stat_(D "/f", ro), -EFAULT);
    char *two = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("unmap_second", syscall2(SYS_munmap, two + A_PAGE, A_PAGE), 0);
    expect("buf_runs_off_end", stat_(D "/f", two + A_PAGE - 100), -EFAULT);
    expect("buf_ends_exactly", stat_(D "/f", two + A_PAGE - 144), 0);
    /* order: the path first */
    expect("missing_and_null_buf", stat_(D "/nope", (void *)0), -ENOENT);
    expect("notdir_and_null_buf", stat_(D "/f/x", (void *)0), -ENOTDIR);
    expect("null_path_and_null_buf", stat_((const char *)0, (void *)0), -EFAULT);
    /* the buffer is untouched on error */
    for (unsigned i = 0; i < sizeof raw; i++) raw[i] = 0x5A;
    expect("error_leaves_buf", stat_(D "/nope", raw), -ENOENT);
    expect("buf_untouched", raw[0] == 0x5A && raw[143] == 0x5A, 1);

    /* search permission */
    expect("chmod_sub_0600", syscall2(SYS_chmod, D "/sub", 0600), 0);
    expect("through_closed", stat_(D "/sub/g", &st), root ? 0 : -EACCES);
    expect("closed_missing", stat_(D "/sub/nope", &st), root ? -ENOENT : -EACCES);
    expect("closed_itself", stat_(D "/sub", &st), 0);
    expect("chmod_sub_0100", syscall2(SYS_chmod, D "/sub", 0100), 0);
    expect("search_only", stat_(D "/sub/g", &st), 0);
    expect("chmod_sub_back", syscall2(SYS_chmod, D "/sub", 0755), 0);
    /* the file's own mode does not matter */
    expect("chmod_f_0", syscall2(SYS_chmod, D "/f", 0), 0);
    expect("mode_0_file", stat_(D "/f", &st) == 0 && st.st_mode == S_IFREG, 1);

    expect("cleanup_g", a_unlink(D "/sub/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
