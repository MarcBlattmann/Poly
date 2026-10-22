/* symlink: length limits, in characters without the terminating NUL. A
 * target of up to 4095 characters is stored whole and 4096 or more is
 * ENAMETOOLONG; a component of the target may be any length. A linkpath
 * of up to 4095 characters works (counted as given, "./" included) and
 * 4096 is ENAMETOOLONG even when it names a missing directory; a
 * linkpath component of 255 characters works and 256 is ENAMETOOLONG. */
#include "a_sys.h"

#define D "/tmp/c_symlink_lengths"
#define DLEN 23 /* str_len(D "/") */

static char lng[4200], big[4200];

static const char *text_of(int n, char c) {
    for (int i = 0; i < n; i++) lng[i] = c;
    lng[n] = 0;
    return lng;
}

/* D "/" then "./" repeated, then a last component of `last` characters:
 * n characters in all (n - DLEN - last must be even). */
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

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    check("dlen", str_len(D "/") == DLEN, str_len(D "/"));

    expect("target_4095_ok", syscall2(SYS_symlink, text_of(4095, 'a'), D "/t"), 0);
    expect("target_4095_readlink", syscall3(SYS_readlink, D "/t", big, sizeof big), 4095);
    check("target_4095_text", mem_eq(big, lng, 4095), 0);
    expect("target_4095_lstat", a_lstat_path(D "/t", &st), 0);
    expect("target_4095_size", st.st_size, 4095);
    expect("cleanup_t", a_unlink(D "/t"), 0);
    expect("target_4096_enametoolong", syscall2(SYS_symlink, text_of(4096, 'a'), D "/t"), -ENAMETOOLONG);
    expect("target_4096_not_made", a_lstat_path(D "/t", &st), -ENOENT);
    expect("target_4100_enametoolong", syscall2(SYS_symlink, text_of(4100, 'b'), D "/t"), -ENAMETOOLONG);
    expect("target_component_300_ok", syscall2(SYS_symlink, text_of(300, 'c'), D "/t"), 0);
    expect("target_component_300_len", syscall3(SYS_readlink, D "/t", big, sizeof big), 300);
    expect("cleanup_t2", a_unlink(D "/t"), 0);
    expect("target_1_ok", syscall2(SYS_symlink, "z", D "/t"), 0);
    expect("cleanup_t3", a_unlink(D "/t"), 0);

    /* 4095 - DLEN - 200 and 4096 - DLEN - 201 are even. */
    expect("linkpath_4095_ok", syscall2(SYS_symlink, "x", path_of(4095, 200)), 0);
    expect("linkpath_4095_lstat", a_lstat_path(lng, &st), 0);
    expect("cleanup_linkpath_4095", a_unlink(lng), 0);
    expect("linkpath_4096_enametoolong", syscall2(SYS_symlink, "x", path_of(4096, 201)), -ENAMETOOLONG);
    path_of(4096, 201);
    lng[DLEN] = 'N'; /* first "./" becomes "N/": a missing directory */
    expect("linkpath_4096_missing_dir_enametoolong", syscall2(SYS_symlink, "x", lng), -ENAMETOOLONG);

    expect("name_255_ok", syscall2(SYS_symlink, "x", path_of(DLEN + 255, 255)), 0);
    expect("name_255_lstat", a_lstat_path(lng, &st), 0);
    expect("cleanup_name_255", a_unlink(lng), 0);
    expect("name_256_enametoolong", syscall2(SYS_symlink, "x", path_of(DLEN + 256, 256)), -ENAMETOOLONG);
    expect("name_300_enametoolong", syscall2(SYS_symlink, "x", path_of(DLEN + 300, 300)), -ENAMETOOLONG);

    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
