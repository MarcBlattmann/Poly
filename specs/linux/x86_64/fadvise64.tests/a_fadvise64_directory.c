/* fadvise64 on a directory descriptor: every valid advice succeeds; a bad
 * advice or a negative length is EINVAL. The directory still lists its
 * entries afterwards. */
#include "a_sys.h"

#define SYS_fadvise64 221
#define D "/tmp/fadvise64_directory"

static i64 fadv(i64 fd, i64 off, i64 len, i64 advice) { return syscall4(SYS_fadvise64, fd, off, len, advice); }

int test_main(void) {
    a_mkdir(D, 0755);
    expect("make_file", a_make_file(D "/entry", "e", 0644), 0);
    i64 d = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("open_dir", d >= 0, d);
    expect("normal", fadv(d, 0, 0, 0), 0);
    expect("random", fadv(d, 0, 0, 1), 0);
    expect("sequential", fadv(d, 0, 0, 2), 0);
    expect("willneed", fadv(d, 0, 0, 3), 0);
    expect("dontneed", fadv(d, 0, 0, 4), 0);
    expect("noreuse", fadv(d, 0, 0, 5), 0);
    expect("bad_advice", fadv(d, 0, 0, 99), -EINVAL);
    expect("negative_len", fadv(d, 0, -1, 0), -EINVAL);
    expect("negative_offset", fadv(d, -1, 4096, 3), 0);
    static char ents[1024];
    i64 n = syscall3(SYS_getdents64, d, ents, sizeof ents);
    int found = 0;
    for (i64 off = 0; off < n;) {
        struct a_dirent64 *e = (struct a_dirent64 *)(ents + off);
        if (str_eq(e->d_name, "entry")) found = 1;
        off += e->d_reclen;
    }
    check("getdents_sees_entry", found, n);
    a_close(d);
    i64 root = a_open("/", O_RDONLY | O_DIRECTORY, 0);
    check("open_root", root >= 0, root);
    expect("root_willneed", fadv(root, 0, 0, 3), 0);
    a_close(root);
    a_unlink(D "/entry");
    a_rmdir(D);
    return finish();
}
