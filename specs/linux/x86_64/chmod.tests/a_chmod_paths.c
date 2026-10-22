/* chmod path errors: missing name or empty path ENOENT; a file used as a
 * directory (also with a trailing slash) ENOTDIR; a name over 255 bytes or
 * a path of 4096 bytes or more ENAMETOOLONG; an unreadable path EFAULT. */
#include "a_sys.h"

#define D "/tmp/a_chmod_paths"

static char buf[4200];

/* D "/" followed by `n` bytes of 'a'. */
static char *long_name(u64 n) {
    char *p = str_cpy(buf, D "/");
    for (u64 i = 0; i < n; i++) *p++ = 'a';
    *p = 0;
    return buf;
}

/* "/tmp/" then "./" repeated, then "a_chmod_paths/f", `total` bytes in all. */
static char *long_path(u64 total) {
    const char *tail = "a_chmod_paths/f";
    char *p = str_cpy(buf, "/tmp/");
    u64 fill = total - 5 - str_len(tail);
    for (u64 i = 0; i < fill / 2; i++) p = str_cpy(p, "./");
    if (fill % 2) *p++ = '/';
    str_cpy(p, tail);
    return buf;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("missing_enoent", syscall2(SYS_chmod, D "/nope", 0600), -ENOENT);
    expect("missing_dir_enoent", syscall2(SYS_chmod, D "/nope/f", 0600), -ENOENT);
    expect("empty_enoent", syscall2(SYS_chmod, "", 0600), -ENOENT);
    expect("file_as_dir_enotdir", syscall2(SYS_chmod, D "/f/x", 0600), -ENOTDIR);
    expect("file_trailing_slash_enotdir", syscall2(SYS_chmod, D "/f/", 0600), -ENOTDIR);
    expect("file_trailing_dot_enotdir", syscall2(SYS_chmod, D "/f/.", 0600), -ENOTDIR);
    expect("missing_trailing_slash_enoent", syscall2(SYS_chmod, D "/nope/", 0600), -ENOENT);
    struct a_stat st;
    expect("mode_untouched", a_stat_path(D "/f", &st) == 0 && st.st_mode == (S_IFREG | 0644), 1);

    expect("name_255_enoent", syscall2(SYS_chmod, long_name(255), 0600), -ENOENT);
    expect("name_256_enametoolong", syscall2(SYS_chmod, long_name(256), 0600), -ENAMETOOLONG);
    expect("path_4095_ok", syscall2(SYS_chmod, long_path(4095), 0640), 0);
    expect("path_4095_applied", a_stat_path(D "/f", &st) == 0 && st.st_mode == (S_IFREG | 0640), 1);
    expect("path_4096_enametoolong", syscall2(SYS_chmod, long_path(4096), 0600), -ENAMETOOLONG);

    expect("null_efault", syscall2(SYS_chmod, 0, 0600), -EFAULT);
    expect("low_pointer_efault", syscall2(SYS_chmod, 1, 0600), -EFAULT);
    expect("kernel_pointer_efault", syscall2(SYS_chmod, 0xffff800000000000UL, 0600), -EFAULT);
    /* A path running into an inaccessible page. */
    char *page = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("setup_mmap", (i64)page > 0, (i64)page);
    expect("setup_mprotect", syscall3(SYS_mprotect, page + A_PAGE, A_PAGE, PROT_NONE), 0);
    for (int i = 0; i < A_PAGE; i++) page[i] = 'b';
    expect("unterminated_efault", syscall2(SYS_chmod, page + A_PAGE - 8, 0600), -EFAULT);
    str_cpy(page + A_PAGE - 21, D "/f");
    expect("ends_at_page_end_ok", syscall2(SYS_chmod, page + A_PAGE - 21, 0604), 0);
    syscall2(SYS_munmap, page, 2 * A_PAGE);

    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
