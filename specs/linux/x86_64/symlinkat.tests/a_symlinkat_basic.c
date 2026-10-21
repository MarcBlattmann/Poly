/* symlinkat: the target text is stored as given (relative, absolute,
 * dangling); the name must not exist; an empty target is ENOENT. Length
 * limits, in characters without the terminating NUL: a target or a
 * linkpath of up to 4095 characters works and 4096 is ENAMETOOLONG; a
 * linkpath component of 256 characters is ENAMETOOLONG (255 works), a
 * target component of any length is fine. When several errors apply: the
 * target first, then the linkpath's length, then its components from left
 * to right, the first that fails deciding (a missing directory before a
 * 256-character name is ENOENT, a file before one ENOTDIR). */
#include "a_sys.h"

#define D "/tmp/a_symlinkat_basic"

static char buf[256];
static char lng[4200], big[4200];

static i64 rl(const char *p) { return syscall4(SYS_readlinkat, AT_FDCWD, p, buf, sizeof buf); }

/* lng = n characters of 'a', one component (no slash). */
static const char *text_of(int n) {
    for (int i = 0; i < n; i++) lng[i] = 'a';
    lng[n] = 0;
    return lng;
}

/* lng = D "/", then "./" repeated, then a last component of `last`
 * characters of 'l': n characters in all (n - str_len(D "/") - last even). */
static const char *path_of(int n, int last) {
    int p = 0;
    for (const char *d = D "/"; *d; d++) lng[p++] = *d;
    while (p < n - last) {
        lng[p++] = '.';
        lng[p++] = '/';
    }
    for (int i = 0; i < last; i++) lng[p++] = 'l';
    lng[p] = 0;
    return lng;
}

static void lengths(void) {
    expect("target_4095_ok", syscall3(SYS_symlinkat, text_of(4095), AT_FDCWD, D "/long"), 0);
    expect("target_4095_kept", syscall4(SYS_readlinkat, AT_FDCWD, D "/long", big, sizeof big), 4095);
    check("target_4095_text", mem_eq(big, lng, 4095), 0);
    expect("cleanup_long", a_unlink(D "/long"), 0);
    expect("target_4096_enametoolong", syscall3(SYS_symlinkat, text_of(4096), AT_FDCWD, D "/long"), -ENAMETOOLONG);
    expect("target_component_300_ok", syscall3(SYS_symlinkat, text_of(300), AT_FDCWD, D "/long"), 0);
    expect("cleanup_long_component", a_unlink(D "/long"), 0);
    /* str_len(D "/") is 23, so a last component of 200 leaves an even
     * number of characters for the "./" parts at 4095, and 201 at 4096. */
    expect("linkpath_4095_ok", syscall3(SYS_symlinkat, "x", AT_FDCWD, path_of(4095, 200)), 0);
    expect("cleanup_linkpath_4095", a_unlink(lng), 0);
    expect("linkpath_4096_enametoolong", syscall3(SYS_symlinkat, "x", AT_FDCWD, path_of(4096, 201)), -ENAMETOOLONG);
    expect("name_255_ok", syscall3(SYS_symlinkat, "x", AT_FDCWD, path_of(23 + 255, 255)), 0);
    expect("cleanup_name_255", a_unlink(lng), 0);
    expect("name_256_enametoolong", syscall3(SYS_symlinkat, "x", AT_FDCWD, path_of(23 + 256, 256)), -ENAMETOOLONG);
}

/* lng = D "/" a (256 characters of 'n') b: a and b may be empty. */
static const char *around_256(const char *a, const char *b) {
    int p = 0;
    for (const char *d = D "/"; *d; d++) lng[p++] = *d;
    for (; *a; a++) lng[p++] = *a;
    for (int i = 0; i < 256; i++) lng[p++] = 'n';
    for (; *b; b++) lng[p++] = *b;
    lng[p] = 0;
    return lng;
}

/* Which error wins when a linkpath has more than one problem. */
static void precedence(void) {
    expect("missing_dir_before_long_name_enoent", syscall3(SYS_symlinkat, "x", AT_FDCWD, around_256("no/", "/x")),
           -ENOENT);
    expect("missing_parent_of_long_last_enoent", syscall3(SYS_symlinkat, "x", AT_FDCWD, around_256("no/", "")),
           -ENOENT);
    expect("missing_dirs_before_long_last_enoent",
           syscall3(SYS_symlinkat, "x", AT_FDCWD, around_256("no/no2/", "")), -ENOENT);
    expect("file_before_long_name_enotdir", syscall3(SYS_symlinkat, "x", AT_FDCWD, around_256("f/", "")), -ENOTDIR);
    expect("file_before_long_middle_enotdir", syscall3(SYS_symlinkat, "x", AT_FDCWD, around_256("f/", "/x")),
           -ENOTDIR);
    expect("long_name_before_missing_dir_enametoolong",
           syscall3(SYS_symlinkat, "x", AT_FDCWD, around_256("", "/no/x")), -ENAMETOOLONG);
    /* The whole length is checked before any component is looked up:
     * D "/./././..." with its first "." made "N", a directory that does
     * not exist. */
    path_of(4096, 201);
    lng[23] = 'N';
    expect("linkpath_4096_missing_dir_enametoolong", syscall3(SYS_symlinkat, "x", AT_FDCWD, lng), -ENAMETOOLONG);
    /* The target is checked before the linkpath. */
    for (int i = 0; i < 4096; i++) big[i] = 'a';
    big[4096] = 0;
    expect("target_4096_before_missing_dir", syscall3(SYS_symlinkat, big, AT_FDCWD, D "/no/x"), -ENAMETOOLONG);
    expect("empty_target_before_missing_dir", syscall3(SYS_symlinkat, "", AT_FDCWD, D "/no/x"), -ENOENT);
    expect("empty_target_before_eexist", syscall3(SYS_symlinkat, "", AT_FDCWD, D "/f"), -ENOENT);
    expect("bad_target_before_long_name", syscall3(SYS_symlinkat, (const char *)8, AT_FDCWD, big), -EFAULT);
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("relative", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/rel"), 0);
    expect("relative_text", rl(D "/rel"), 1);
    expect("absolute", syscall3(SYS_symlinkat, D "/f", AT_FDCWD, D "/abs"), 0);
    expect("absolute_text_length", rl(D "/abs"), (i64)str_len(D "/f"));
    check("absolute_text", mem_eq(buf, D "/f", str_len(D "/f")), 0);
    expect("dangling_allowed", syscall3(SYS_symlinkat, "../nowhere/x", AT_FDCWD, D "/dang"), 0);
    expect("dangling_text", rl(D "/dang"), 12);
    check("dangling_text_kept", mem_eq(buf, "../nowhere/x", 12), 0);
    expect("weird_text_kept", syscall3(SYS_symlinkat, "a//b/./c/", AT_FDCWD, D "/weird"), 0);
    expect("weird_text_length", rl(D "/weird"), 9);
    check("weird_text", mem_eq(buf, "a//b/./c/", 9), 0);
    expect("existing_name_eexist", syscall3(SYS_symlinkat, "x", AT_FDCWD, D "/f"), -EEXIST);
    expect("existing_symlink_eexist", syscall3(SYS_symlinkat, "x", AT_FDCWD, D "/rel"), -EEXIST);
    expect("empty_target_enoent", syscall3(SYS_symlinkat, "", AT_FDCWD, D "/empty"), -ENOENT);
    expect("missing_parent_enoent", syscall3(SYS_symlinkat, "x", AT_FDCWD, D "/no/l"), -ENOENT);
    expect("parent_is_file_enotdir", syscall3(SYS_symlinkat, "x", AT_FDCWD, D "/f/l"), -ENOTDIR);
    expect("bad_target_efault", syscall3(SYS_symlinkat, (const char *)8, AT_FDCWD, D "/bad"), -EFAULT);
    expect("bad_name_efault", syscall3(SYS_symlinkat, "x", AT_FDCWD, (const char *)8), -EFAULT);
    lengths();
    precedence();
    expect("lstat_symlink", a_lstat_path(D "/abs", &st), 0);
    expect("symlink_mode", st.st_mode, S_IFLNK | 0777);
    expect("symlink_nlink", st.st_nlink, 1);
    i64 fd = a_open(D "/abs", O_RDONLY, 0);
    expect("open_through_absolute", a_read(fd, buf, sizeof buf), 4);
    a_close(fd);
    expect("old_symlink_syscall", syscall2(SYS_symlink, "f", D "/old"), 0);
    expect("old_symlink_text", rl(D "/old"), 1);
    const char *names[] = {D "/rel", D "/abs", D "/dang", D "/weird", D "/old", D "/f"};
    int ok = 1;
    for (int i = 0; i < 6; i++) ok &= a_unlink(names[i]) == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
