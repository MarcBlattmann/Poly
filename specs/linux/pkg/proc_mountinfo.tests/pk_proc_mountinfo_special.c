/* Which characters are escaped where in /proc/self/mountinfo and
 * /proc/self/mounts. The source field escapes one character more than the
 * path fields: '#' is written as \043 in the source (every one of them, at
 * the start, in the middle, at the end) and stays '#' in a mount point and
 * in the root field. Space (\040), tab (\011), newline (\012) and backslash
 * (\134) are escaped in all three. A name that already looks like an escape
 * (the four characters \040) is not kept as it is: its backslash is escaped
 * (\134040). A bind mount shows the source string of the mount it was made
 * from, escaped the same way. Both files agree.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_mis"
#define T B "/t"

/* Mount a tmpfs with `source` on T/<dir>; check the escaped mount point and
 * source in both files; leave it mounted when `keep`. */
static void one(const char *name, const char *dir, const char *esc_dir, const char *source, const char *esc_source,
                int keep) {
    char path[160], want[200], chk[96];
    str_cpy(str_cpy(path, T "/"), dir);
    str_cpy(str_cpy(want, T "/"), esc_dir);
    expect(str_cat2(chk, name, "_mkdir"), pm_mkdirs(path), 0);
    expect(str_cat2(chk, name, "_mount"), pm_mount(source, path, "tmpfs", 0, 0), 0);
    struct pm_mi mi;
    pm_mi_load("/proc/self/mountinfo");
    expect(str_cat2(chk, name, "_mountinfo_mount_point"), pm_mi_get(want, -1, &mi), 1);
    expect(str_cat2(chk, name, "_mountinfo_source"), str_eq(mi.source, esc_source), 1);
    expect(str_cat2(chk, name, "_mountinfo_root"), str_eq(mi.root, "/"), 1);
    expect(str_cat2(chk, name, "_mountinfo_fields"), mi.nfields, 10);
    int lines, bad;
    pm_mi_count(&lines, &bad);
    expect(str_cat2(chk, name, "_every_line_well_formed"), bad, 0);
    struct pm_mounts_line ml;
    pm_mi_load("/proc/self/mounts");
    expect(str_cat2(chk, name, "_mounts_mount_point"), pm_mounts_get(want, -1, &ml), 1);
    expect(str_cat2(chk, name, "_mounts_source"), str_eq(ml.source, esc_source), 1);
    expect(str_cat2(chk, name, "_mounts_fields"), ml.nfields, 6);
    if (!keep) expect(str_cat2(chk, name, "_umount"), pm_umount(path, 0), 0);
}

static int body(void) {
    pm_mkdirs(T);
    expect("mount_t", pm_mount("pkspecial", T, "tmpfs", 0, 0), 0);
    one("hash_middle", "m1", "m1", "dev#1", "dev\\0431", 0);
    one("hash_first", "m2", "m2", "#tag", "\\043tag", 0);
    one("hash_last", "m3", "m3", "tag#", "tag\\043", 0);
    one("hash_only", "m4", "m4", "#", "\\043", 0);
    one("hash_twice", "m5", "m5", "a##b", "a\\043\\043b", 0);
    one("hash_and_blank", "m6", "m6", "# a\tb", "\\043\\040a\\011b", 0);
    one("hash_in_mount_point", "dir#1", "dir#1", "plainsrc", "plainsrc", 0);
    one("hash_in_both", "#x#", "#x#", "#x#", "\\043x\\043", 0);
    one("looks_like_escape", "lit\\040x", "lit\\134040x", "s\\043t", "s\\134043t", 0);
    one("percent_and_colon", "p%20:q", "p%20:q", "host:/ex port", "host:/ex\\040port", 0);

    /* a bind mount: the root field keeps '#', the source is the original's */
    one("orig", "orig", "orig", "src #9", "src\\040\\0439", 1);
    pm_mkdirs(T "/orig/sub#dir/in ner");
    pm_mkdirs(T "/bound");
    expect("bind", pm_mount(T "/orig/sub#dir/in ner", T "/bound", 0, MS_BIND, 0), 0);
    struct pm_mi mi;
    pm_mi_load("/proc/self/mountinfo");
    expect("bind_listed", pm_mi_get(T "/bound", -1, &mi), 1);
    expect("bind_root_hash_plain_blank_escaped", str_eq(mi.root, "/sub#dir/in\\040ner"), 1);
    expect("bind_source_of_the_original", str_eq(mi.source, "src\\040\\0439"), 1);
    struct pm_mounts_line ml;
    pm_mi_load("/proc/self/mounts");
    expect("bind_in_mounts", pm_mounts_get(T "/bound", -1, &ml), 1);
    expect("bind_mounts_source", str_eq(ml.source, "src\\040\\0439"), 1);
    expect("umount_bound", pm_umount(T "/bound", 0), 0);
    expect("umount_orig", pm_umount(T "/orig", 0), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(T);
    a_rmdir(B);
    pm_in_ns(body);
    a_rmdir(T);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
