/* link: name length limits and bad pointers. A last component of 255
 * bytes is fine, 256 bytes is ENAMETOOLONG, for either name; a path
 * string of 4095 bytes (4096 with its NUL) is still looked up, one of
 * 4096 bytes or more is ENAMETOOLONG; an unreadable pointer is EFAULT for
 * either name. */
#include "a_sys.h"

#define D "/tmp/a_link_names"

static char nm[300], longp[4200];

static void make_name(const char *prefix, int n, char c) {
    int i = 0;
    for (; prefix[i]; i++) nm[i] = prefix[i];
    for (int k = 0; k < n; k++) nm[i++] = c;
    nm[i] = 0;
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    make_name(D "/", 255, 'a');
    expect("new_255_ok", syscall2(SYS_link, D "/f", nm), 0);
    expect("source_255_ok", syscall2(SYS_link, nm, D "/g"), 0);
    expect("unlink_255", a_unlink(nm), 0);
    make_name(D "/", 256, 'b');
    expect("new_256_enametoolong", syscall2(SYS_link, D "/f", nm), -ENAMETOOLONG);
    expect("source_256_enametoolong", syscall2(SYS_link, nm, D "/x"), -ENAMETOOLONG);
    /* "/" followed by "./" repeated: valid components naming "/" */
    int i = 0;
    longp[i++] = '/';
    while (i < 4095) { longp[i++] = '.'; longp[i++] = '/'; }
    longp[4095] = 0;
    expect("source_4095_looked_up", syscall2(SYS_link, longp, D "/x"), -EPERM);
    expect("new_4095_looked_up", syscall2(SYS_link, D "/f", longp), -EEXIST);
    longp[4095] = '.';
    longp[4096] = 0;
    expect("source_4096_enametoolong", syscall2(SYS_link, longp, D "/x"), -ENAMETOOLONG);
    expect("new_4096_enametoolong", syscall2(SYS_link, D "/f", longp), -ENAMETOOLONG);
    expect("source_null_efault", syscall2(SYS_link, 0, D "/x"), -EFAULT);
    expect("new_null_efault", syscall2(SYS_link, D "/f", 0), -EFAULT);
    expect("source_bad_efault", syscall2(SYS_link, (void *)1, D "/x"), -EFAULT);
    expect("new_bad_efault", syscall2(SYS_link, D "/f", (void *)-4096), -EFAULT);
    expect("nothing_made", a_lstat_path(D "/x", &st), -ENOENT);
    a_stat_path(D "/f", &st);
    expect("nlink_2", st.st_nlink, 2);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
