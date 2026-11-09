/* fsconfig sets parameters of a filesystem context before it is created.
 * For tmpfs: size (bytes, or with k/m/g suffix; rounded to 4096-byte
 * blocks) gives statfs f_blocks; nr_blocks gives f_blocks directly;
 * nr_inodes gives f_files; mode, uid and gid describe the root directory;
 * the flag "ro" makes the mount read-only (EROFS, ST_RDONLY) and "rw"
 * cancels it; the string form "ro" with a value is accepted too; "source"
 * is any string. Parameters may be given in any order and later ones win.
 * A parameter the filesystem does not know is EINVAL and leaves the context
 * usable.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_fsconfig_params"

struct kv {
    const char *k, *v;
};

/* Build a tmpfs from SET_STRING parameters, attach it at base/<name>. */
static i64 build(const char *base, const char *name, const struct kv *kv, int n, int flag_ro, int flag_rw, char *out) {
    i64 fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    if (fc < 0) return fc;
    for (int i = 0; i < n; i++) {
        i64 r = pm_fsconfig(fc, FSCONFIG_SET_STRING, kv[i].k, kv[i].v, 0);
        if (r < 0) return r;
    }
    if (flag_ro) pm_fsconfig(fc, FSCONFIG_SET_FLAG, "ro", 0, 0);
    if (flag_rw) pm_fsconfig(fc, FSCONFIG_SET_FLAG, "rw", 0, 0);
    i64 r = pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0);
    if (r < 0) return r;
    i64 m = pm_fsmount(fc, FSMOUNT_CLOEXEC, 0);
    a_close(fc);
    if (m < 0) return m;
    pm_path(out, base, name);
    r = pm_mkdir(out);
    if (r == 0) r = pm_attach(m, out);
    a_close(m);
    return r;
}

static void body(const char *base) {
    char p[128];
    struct kv size1m[] = {{"size", "1m"}};
    expect("size_1m", build(base, "a", size1m, 1, 0, 0, p), 0);
    expect("size_1m_blocks", pm_statfs_blocks(p), 256);
    struct kv size512k[] = {{"size", "512k"}};
    expect("size_512k", build(base, "b", size512k, 1, 0, 0, p), 0);
    expect("size_512k_blocks", pm_statfs_blocks(p), 128);
    struct kv sizebytes[] = {{"size", "1048576"}};
    expect("size_bytes", build(base, "c", sizebytes, 1, 0, 0, p), 0);
    expect("size_bytes_blocks", pm_statfs_blocks(p), 256);
    struct kv size1g[] = {{"size", "1g"}};
    expect("size_1g", build(base, "d", size1g, 1, 0, 0, p), 0);
    expect("size_1g_blocks", pm_statfs_blocks(p), 262144);
    struct kv sizeround[] = {{"size", "5000"}};
    expect("size_5000", build(base, "e", sizeround, 1, 0, 0, p), 0);
    expect("size_5000_rounds_up", pm_statfs_blocks(p), 2);
    struct kv nrblocks[] = {{"nr_blocks", "300"}};
    expect("nr_blocks", build(base, "f", nrblocks, 1, 0, 0, p), 0);
    expect("nr_blocks_value", pm_statfs_blocks(p), 300);
    struct kv inodes[] = {{"size", "1m"}, {"nr_inodes", "10"}};
    expect("nr_inodes", build(base, "g", inodes, 2, 0, 0, p), 0);
    struct a_statfs sf;
    expect("statfs_inodes", syscall2(SYS_statfs, p, &sf), 0);
    expect("nr_inodes_value", (i64)sf.f_files, 10);
    struct kv later[] = {{"size", "1m"}, {"size", "2m"}};
    expect("later_wins", build(base, "h", later, 2, 0, 0, p), 0);
    expect("later_wins_blocks", pm_statfs_blocks(p), 512);

    struct kv mode[] = {{"mode", "0711"}, {"uid", "1000"}, {"gid", "2000"}};
    expect("mode_uid_gid", build(base, "i", mode, 3, 0, 0, p), 0);
    struct a_stat st;
    expect("stat_root", a_stat_path(p, &st), 0);
    expect("root_mode", st.st_mode, 040711);
    expect("root_uid", st.st_uid, 1000);
    expect("root_gid", st.st_gid, 2000);
    struct kv sticky[] = {{"mode", "1777"}};
    expect("mode_sticky", build(base, "j", sticky, 1, 0, 0, p), 0);
    expect("stat_sticky", a_stat_path(p, &st), 0);
    expect("sticky_mode", st.st_mode, 041777);

    expect("flag_ro", build(base, "k", size1m, 1, 1, 0, p), 0);
    expect("flag_ro_erofs", pm_can_create(p), -EROFS);
    expect("flag_ro_statfs", pm_mntflags(p) & ST_RDONLY, ST_RDONLY);
    expect("flag_ro_then_rw", build(base, "l", size1m, 1, 1, 1, p), 0);
    expect("flag_rw_writable", pm_can_create(p), 0);
    expect("flag_rw_statfs", pm_mntflags(p) & ST_RDONLY, 0);

    /* "ro" given as a string, and "source" */
    i64 fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    expect("string_ro", pm_fsconfig(fc, FSCONFIG_SET_STRING, "ro", "1", 0), 0);
    expect("source", pm_fsconfig(fc, FSCONFIG_SET_STRING, "source", "mytmp", 0), 0);
    expect("create", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), 0);
    i64 m = pm_fsmount(fc, FSMOUNT_CLOEXEC, 0);
    check("fsmount", m >= 0, m);
    pm_path(p, base, "m");
    expect("mkdir_m", pm_mkdir(p), 0);
    expect("attach", pm_attach(m, p), 0);
    expect("string_ro_means_read_only", pm_can_create(p), -EROFS);
    a_close(fc);
    a_close(m);

    /* unknown parameters do not spoil the context */
    fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    expect("unknown_string", pm_fsconfig(fc, FSCONFIG_SET_STRING, "bogus", "1", 0), -EINVAL);
    expect("unknown_flag", pm_fsconfig(fc, FSCONFIG_SET_FLAG, "bogus", 0, 0), -EINVAL);
    expect("bad_value", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "abc", 0), -EINVAL);
    expect("bad_mode", pm_fsconfig(fc, FSCONFIG_SET_STRING, "mode", "999", 0), -EINVAL);
    expect("still_usable", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "1m", 0), 0);
    expect("create_after_errors", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), 0);
}

int test_main(void) { return pm_main(BASE, body); }
