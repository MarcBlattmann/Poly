/* What a mounted fuse file system looks like from outside: statfs reports
 * f_type 0x65735546 (FUSE_SUPER_MAGIC) and, once the server answered
 * FUSE_STATFS, its numbers (blocks, free, available, files, ffree, bsize,
 * namelen, frsize); f_flags carries ST_NOSUID (2) and ST_NODEV (4) of the
 * mount. /proc/self/mountinfo shows the mount with file system type
 * "fuse", or "fuse.NAME" when the type was given as "fuse.NAME" or the
 * option subtype=NAME was passed; source "pk", and the super block options
 * "rw,user_id=0,group_id=0" followed by default_permissions, allow_other,
 * max_read=N in this order when given. /proc/filesystems lists "fuse" as a
 * nodev file system. The root directory is the server's node 1.
 * (linux/magic.h, proc(5))
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

static char info[65536];

/* Does the line of /proc/self/mountinfo for our mount point contain `what`? */
static int mountinfo_has(const char *dir, const char *what) {
    i64 n = fz_slurp("/proc/self/mountinfo", info, sizeof info - 1);
    if (n <= 0) return 0;
    info[n] = 0;
    size_t dl = str_len(dir), wl = str_len(what);
    for (char *l = info; *l;) {
        char *e = l;
        while (*e && *e != '\n') e++;
        char save = *e;
        *e = 0;
        int has_dir = 0, has_what = 0;
        for (char *p = l; *p; p++) {
            if (p[0] == ' ' && mem_eq(p + 1, dir, dl) && p[1 + dl] == ' ') has_dir = 1;
            if (mem_eq(p, what, wl)) has_what = 1;
        }
        *e = save;
        if (has_dir && has_what) return 1;
        l = save ? e + 1 : e;
    }
    return 0;
}

static int proc_has(const char *file, const char *what) {
    i64 n = fz_slurp(file, info, sizeof info - 1);
    if (n <= 0) return 0;
    info[n] = 0;
    size_t wl = str_len(what);
    for (char *p = info; *p; p++)
        if (mem_eq(p, what, wl)) return 1;
    return 0;
}

int test_main(void) {
    expect("setup", fz_setup("/tmp/pk_fuse_info"), 0);
    expect("open", fz_open() >= 0, 1);
    expect("mount", fz_mount("default_permissions,allow_other,max_read=65536"), 0);
    expect("start", fz_start() > 0, 1);

    struct a_statfs sf;
    expect("statfs", syscall2(SYS_statfs, fz_dir, &sf), 0);
    expect("f_type", sf.f_type, FUSE_SUPER_MAGIC);
    expect("f_bsize", sf.f_bsize, 4096);
    expect("f_blocks", (i64)sf.f_blocks, 1000);
    expect("f_bfree", (i64)sf.f_bfree, 500);
    expect("f_bavail", (i64)sf.f_bavail, 400);
    expect("f_files", (i64)sf.f_files, 100);
    expect("f_ffree", (i64)sf.f_ffree, 50);
    expect("f_namelen", sf.f_namelen, 255);
    expect("f_frsize", sf.f_frsize, 4096);
    expect("f_flags_nosuid_nodev", sf.f_flags & 6, 6);
    check("init_before_statfs", fz->init_count == 1 && fz_count(FZ_STATFS) >= 1, fz_count(FZ_STATFS));

    struct a_stat st;
    expect("stat_root", a_stat_path(fz_dir, &st), 0);
    expect("root_is_dir", st.st_mode & S_IFMT, S_IFDIR);
    expect("root_mode", st.st_mode & 07777, 0755);

    check("mountinfo_type_and_options",
          mountinfo_has(fz_dir, " - fuse pk rw,user_id=0,group_id=0,default_permissions,allow_other,max_read=65536"), 0);
    check("mountinfo_nosuid_nodev", mountinfo_has(fz_dir, " rw,nosuid,nodev"), 0);
    check("proc_filesystems_fuse", proc_has("/proc/filesystems", "nodev\tfuse\n"), 0);

    /* The sub type from the type name and from the option. */
    fz_stop();
    syscall2(SYS_umount2, fz_dir, MNT_DETACH);
    i64 fd = a_open("/dev/fuse", O_RDWR, 0);
    expect("mount_fuse_dot_name", fz_mount_fd(fd, "fuse.pkname", MS_NOSUID | MS_NODEV, 0), 0);
    check("mountinfo_fuse_dot_name", mountinfo_has(fz_dir, " - fuse.pkname pk rw,user_id=0,group_id=0"), 0);
    syscall2(SYS_umount2, fz_dir, MNT_DETACH);
    a_close(fd);
    fd = a_open("/dev/fuse", O_RDWR, 0);
    expect("mount_subtype_option", fz_mount_fd(fd, 0, MS_NOSUID | MS_NODEV, "subtype=opt"), 0);
    check("mountinfo_subtype_option", mountinfo_has(fz_dir, " - fuse.opt pk rw,user_id=0,group_id=0"), 0);
    a_close(fd);
    fz_cleanup();
    return finish();
}
