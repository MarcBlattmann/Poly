/* statfs: plausible fields for the file system of /tmp (any type): block
 * size, counts that do not exceed the totals, name length 255. */
#include "a_sys.h"

int test_main(void) {
    struct a_statfs s;
    memset(&s, 0xff, sizeof s);
    expect("statfs_tmp", syscall2(SYS_statfs, "/tmp", &s), 0);
    check("type_nonzero", s.f_type != 0, s.f_type);
    check("bsize_positive", s.f_bsize > 0, s.f_bsize);
    check("frsize_positive", s.f_frsize > 0, s.f_frsize);
    check("bfree_le_blocks", s.f_bfree <= s.f_blocks, 0);
    check("bavail_le_bfree", s.f_bavail <= s.f_bfree, 0);
    check("ffree_le_files", s.f_ffree <= s.f_files, 0);
    expect("namelen_255", s.f_namelen, 255);
    check("spare_zeroed", s.f_spare[0] == 0 && s.f_spare[3] == 0, s.f_spare[0]);
    struct a_statfs r;
    expect("statfs_root", syscall2(SYS_statfs, "/", &r), 0);
    check("root_bsize_positive", r.f_bsize > 0, r.f_bsize);
    expect("statfs_file_in_tmp", a_make_file("/tmp/a_statfs_fields", "", 0644), 0);
    struct a_statfs f;
    expect("statfs_file", syscall2(SYS_statfs, "/tmp/a_statfs_fields", &f), 0);
    check("file_same_fs_type", f.f_type == s.f_type && f.f_bsize == s.f_bsize, f.f_type);
    check("file_same_fsid", f.f_fsid[0] == s.f_fsid[0] && f.f_fsid[1] == s.f_fsid[1], 0);
    expect("cleanup", a_unlink("/tmp/a_statfs_fields"), 0);
    return finish();
}
