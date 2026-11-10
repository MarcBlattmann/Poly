/* MS_BIND | MS_REC against plain MS_BIND on a tree of mounts two levels
 * deep. Under a source directory S there is a mount on S/a and, inside
 * that mount, another on S/a/deep. A recursive bind of S copies all three:
 * the files of both submounts are reachable below the new path, writes
 * through the copy reach the original submount, and /proc/self/mountinfo
 * lists a mount at the new path, at <new>/a and at <new>/a/deep. A plain
 * bind copies only the top: <new>/a is the empty directory the submount
 * covers. A copy of a submount keeps the per-mount flags its original had
 * (nodev), and a remount of the copied top leaves the copies below it as
 * they were. A recursive bind may also start at a submount (S/a brings
 * S/a/deep along); a plain bind of S/a shows its files and an empty deep.
 * Each copied mount is a mount of its own: umount2 of the top with the
 * copies below it is EBUSY, also of the middle one while deep is mounted;
 * they unmount from the bottom up, or all at once with MNT_DETACH. The
 * source tree is untouched by all of it. An unbindable mount in the tree
 * is left out of a recursive bind (its directory is empty in the copy).
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_mbt"
#define S B "/s"

static u64 flags_of(const char *p) {
    struct a_statfs fs;
    if (syscall2(SYS_statfs, p, &fs) < 0) return ~0UL;
    return (u64)fs.f_flags & (ST_RDONLY | ST_NOSUID | ST_NODEV | ST_NOEXEC);
}

static int listed(const char *mp) {
    struct pm_mi mi;
    if (pm_mi_load("/proc/self/mountinfo") <= 0) return -1;
    return pm_mi_get(mp, -1, &mi);
}

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(S "/a");
    pm_mkdirs(S "/plain");
    pm_mkdirs(B "/r");
    pm_mkdirs(B "/p");
    pm_mkdirs(B "/ra");
    pm_mkdirs(B "/pa");
    pm_mkdirs(B "/u");
    pm_put(S, "/top", "T", 0644);
    expect("mount_a", pm_mount("pka", S "/a", "tmpfs", MS_NODEV, 0), 0);
    pm_mkdirs(S "/a/deep");
    pm_put(S, "/a/fa", "in-a", 0644);
    expect("mount_deep", pm_mount("pkdeep", S "/a/deep", "tmpfs", 0, 0), 0);
    pm_put(S, "/a/deep/fd", "in-deep", 0644);

    /* recursive bind of the whole tree */
    expect("rbind", pm_mount(S, B "/r", 0, MS_BIND | MS_REC, 0), 0);
    pm_expect_file("rbind_top_file", B "/r/top", "T");
    pm_expect_file("rbind_first_level", B "/r/a/fa", "in-a");
    pm_expect_file("rbind_second_level", B "/r/a/deep/fd", "in-deep");
    expect("rbind_listed_top", listed(B "/r"), 1);
    expect("rbind_listed_a", listed(B "/r/a"), 1);
    expect("rbind_listed_deep", listed(B "/r/a/deep"), 1);
    expect("write_through_copy", a_make_file(B "/r/a/deep/w", "W", 0644), 0);
    pm_expect_file("write_reaches_original", S "/a/deep/w", "W");
    expect("copy_keeps_nodev", flags_of(B "/r/a"), ST_NODEV);
    expect("copy_of_deep_plain", flags_of(B "/r/a/deep"), 0);
    expect("remount_copied_top_nosuid", pm_mount(0, B "/r", 0, MS_REMOUNT | MS_BIND | MS_NOSUID, 0), 0);
    expect("top_has_nosuid", flags_of(B "/r"), ST_NOSUID);
    expect("copy_below_unchanged", flags_of(B "/r/a"), ST_NODEV);
    expect("source_top_unchanged", flags_of(S), 0);

    /* plain bind of the same directory */
    expect("bind_plain", pm_mount(S, B "/p", 0, MS_BIND, 0), 0);
    pm_expect_file("plain_top_file", B "/p/top", "T");
    pm_expect_list("plain_a_is_the_covered_directory", B "/p/a", "");
    expect("plain_no_deep", pm_exists(B "/p/a/deep"), 0);
    expect("plain_listed_top", listed(B "/p"), 1);
    expect("plain_a_not_listed", listed(B "/p/a"), 0);
    expect("umount_plain", pm_umount(B "/p", 0), 0);

    /* starting at a submount */
    expect("rbind_of_submount", pm_mount(S "/a", B "/ra", 0, MS_BIND | MS_REC, 0), 0);
    pm_expect_file("ra_file", B "/ra/fa", "in-a");
    pm_expect_file("ra_deep_file", B "/ra/deep/fd", "in-deep");
    expect("bind_of_submount", pm_mount(S "/a", B "/pa", 0, MS_BIND, 0), 0);
    pm_expect_file("pa_file", B "/pa/fa", "in-a");
    pm_expect_list("pa_deep_empty", B "/pa/deep", "");
    expect("umount_pa", pm_umount(B "/pa", 0), 0);
    expect("umount_ra_busy", pm_umount(B "/ra", 0), -EBUSY);
    expect("umount_ra_detach", pm_umount(B "/ra", MNT_DETACH), 0);
    pm_expect_list("ra_empty_again", B "/ra", "");

    /* unmounting the copy, bottom up */
    expect("umount_top_busy", pm_umount(B "/r", 0), -EBUSY);
    expect("umount_middle_busy", pm_umount(B "/r/a", 0), -EBUSY);
    expect("umount_deep", pm_umount(B "/r/a/deep", 0), 0);
    pm_expect_list("copy_deep_now_empty", B "/r/a/deep", "");
    pm_expect_file("original_deep_still_mounted", S "/a/deep/fd", "in-deep");
    expect("umount_middle", pm_umount(B "/r/a", 0), 0);
    expect("umount_top", pm_umount(B "/r", 0), 0);
    expect("umount_top_again_einval", pm_umount(B "/r", 0), -EINVAL);
    pm_expect_file("source_intact", S "/a/fa", "in-a");

    /* an unbindable mount inside the tree */
    expect("make_deep_unbindable", pm_mount(0, S "/a/deep", 0, MS_UNBINDABLE, 0), 0);
    expect("rbind_with_unbindable_inside", pm_mount(S, B "/u", 0, MS_BIND | MS_REC, 0), 0);
    pm_expect_file("u_first_level_copied", B "/u/a/fa", "in-a");
    pm_expect_list("u_unbindable_left_out", B "/u/a/deep", "");
    expect("u_deep_not_listed", listed(B "/u/a/deep"), 0);
    expect("umount_u_detach", pm_umount(B "/u", MNT_DETACH), 0);

    expect("umount_src_deep", pm_umount(S "/a/deep", 0), 0);
    expect("umount_src_a", pm_umount(S "/a", 0), 0);
    pm_expect_list("source_plain_again", S, "a,plain,top");
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B);
    pm_in_ns(body);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
