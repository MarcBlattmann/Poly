/* mountinfo as seen by a process with another root directory. After
 * chroot into the root directory of a mount, that mount is listed with mount
 * point "/" (root field "/"), mounts below it are listed relative to the new
 * root, and mounts outside it are not listed at all. When the new root is a
 * plain subdirectory of a mount (not a mount point itself), the mount that
 * contains it is not listed (its mount point is not below the root) but the
 * mounts below the new root are, relative to it. A bind mount of a
 * subdirectory has that directory as its root field.
 * poly-diff: needs=root (mount and chroot need CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_mir"
#define R B "/r"

static int none_outside(void) {
    const char *p = pm_mi_text;
    struct pm_mi cur;
    int bad = 0;
    while (*p) {
        p = pm_mi_parse(p, &cur);
        if (cur.mp[0] != '/') bad++;
        if (pm_find(cur.mp, "pk_mir") >= 0) bad++;
    }
    return bad;
}

static int in_chroot_mount_root(void) {
    expect("chroot", syscall1(SYS_chroot, R), 0);
    expect("chdir", syscall1(SYS_chdir, "/"), 0);
    expect("load", pm_mi_load("/proc/self/mountinfo") > 0, 1);
    struct pm_mi mi;
    expect("root_mount_listed", pm_mi_get("/", 0, &mi), 1);
    expect("root_mount_fstype", str_eq(mi.fstype, "tmpfs"), 1);
    expect("root_mount_source", str_eq(mi.source, "pkroot"), 1);
    expect("root_mount_root", str_eq(mi.root, "/"), 1);
    expect("inner_listed", pm_mi_get("/inner", 0, &mi), 1);
    expect("inner_source", str_eq(mi.source, "pkinner"), 1);
    expect("proc_listed", pm_mi_get("/proc", 0, &mi), 1);
    expect("bound_listed", pm_mi_get("/bound", 0, &mi), 1);
    expect("bound_root_field", str_eq(mi.root, "/subdir"), 1);
    expect("nothing_outside", none_outside(), 0);
    int lines, bad;
    pm_mi_count(&lines, &bad);
    expect("lines_well_formed", bad, 0);
    expect("exactly_four_mounts", lines, 4);
    expect("outside_mount_not_listed", pm_mi_get("/tmp", 0, &mi), 0);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

static int in_chroot_subdir(void) {
    expect("chroot", syscall1(SYS_chroot, B "/plain/sub"), 0);
    expect("chdir", syscall1(SYS_chdir, "/"), 0);
    expect("load", pm_mi_load("/proc/self/mountinfo") > 0, 1);
    struct pm_mi mi;
    expect("slash_not_listed", pm_mi_get("/", 0, &mi), 0);
    expect("below_root_listed", pm_mi_get("/below", 0, &mi), 1);
    expect("below_source", str_eq(mi.source, "pkbelow"), 1);
    expect("proc_listed", pm_mi_get("/proc", 0, &mi), 1);
    expect("nothing_outside", none_outside(), 0);
    int lines, bad;
    pm_mi_count(&lines, &bad);
    expect("lines_well_formed", bad, 0);
    expect("exactly_two_mounts", lines, 2);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

static void run(const char *name, int (*fn)(void)) {
    i32 p = passed, f = failed;
    passed = failed = 0;
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, fn());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    passed = p + a_shared()[0];
    failed = f + a_shared()[1];
    expect(name, st, 0);
}

static int body(void) {
    pm_mkdirs(R);
    expect("mount_r", pm_mount("pkroot", R, "tmpfs", 0, 0), 0);
    pm_mkdirs(R "/inner");
    pm_mkdirs(R "/proc");
    pm_mkdirs(R "/bound");
    pm_mkdirs(R "/subdir");
    expect("mount_inner", pm_mount("pkinner", R "/inner", "tmpfs", 0, 0), 0);
    expect("mount_proc", pm_mount("proc", R "/proc", "proc", 0, 0), 0);
    expect("bind_subdir", pm_mount(R "/subdir", R "/bound", 0, MS_BIND, 0), 0);
    run("chroot_mount_root_child", in_chroot_mount_root);

    /* a plain subdirectory as root */
    pm_mkdirs(B "/plain/sub/below");
    pm_mkdirs(B "/plain/sub/proc");
    expect("mount_below", pm_mount("pkbelow", B "/plain/sub/below", "tmpfs", 0, 0), 0);
    expect("mount_proc2", pm_mount("proc", B "/plain/sub/proc", "proc", 0, 0), 0);
    run("chroot_subdir_child", in_chroot_subdir);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    pm_rm_rf(B);
    pm_mkdirs(B);
    pm_in_ns(body);
    pm_rm_rf(B);
    expect("cleanup", pm_exists(B), 0);
    return finish();
}
