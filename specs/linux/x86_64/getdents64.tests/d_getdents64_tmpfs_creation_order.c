/* getdents64 on tmpfs: "." then "..", then the entries newest first
 * (the reverse of the order they were created in, whatever their names),
 * for files, directories, symbolic links and hard links alike. Each
 * record's d_off is printed as its rank among the listing's d_off values:
 * "." has the smallest, ".." the cookie of the newest entry, each entry's
 * the next (older) entry's, and the last record the largest (end). */
#include "a_sys.h"

/* linux/magic.h */
#define TMPFS_MAGIC 0x01021994

static char buf[8192];

/* The records of the last read: name, d_off and d_type of each. */
#define MAXR 64
static char rec_name[MAXR][48];
static i64 rec_off[MAXR];
static u8 rec_type[MAXR];
static int nrec;

static A_UNUSED void take(i64 n) {
    for (i64 o = 0; o < n;) {
        struct a_dirent64 *d = (struct a_dirent64 *)(buf + o);
        if (nrec < MAXR) {
            str_cpy(rec_name[nrec], d->d_name);
            rec_off[nrec] = d->d_off;
            rec_type[nrec] = d->d_type;
            nrec++;
        }
        o += d->d_reclen;
    }
}

/* getdents64 calls of `size` bytes from the current position until one
 * returns 0 (or an error, which is returned). */
static A_UNUSED i64 read_rest(i64 fd, i64 size) {
    i64 n;
    nrec = 0;
    while ((n = syscall3(SYS_getdents64, fd, buf, size)) > 0) take(n);
    return n;
}

/* One getdents64 call of `size` bytes; returns its result. */
static A_UNUSED i64 read_call(i64 fd, i64 size) {
    nrec = 0;
    i64 n = syscall3(SYS_getdents64, fd, buf, size);
    if (n > 0) take(n);
    return n;
}

/* Rewind (lseek to 0) and read the whole listing. */
static A_UNUSED i64 read_all(i64 fd) {
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    return read_rest(fd, sizeof buf);
}

static char text[2048];

/* The names of the records, in the order returned, space-separated. */
static A_UNUSED const char *names_text(void) {
    char *p = text;
    *p = 0;
    for (int i = 0; i < nrec; i++) {
        if (i) p = str_cpy(p, " ");
        p = str_cpy(p, rec_name[i]);
    }
    return text;
}

/* Each record's d_off as its rank among the distinct d_off values of
 * these records (1 = the smallest): the raw values are not compared,
 * only their order. */
static A_UNUSED const char *ranks_text(void) {
    char *p = text, num[24];
    *p = 0;
    for (int i = 0; i < nrec; i++) {
        int rank = 1;
        for (int j = 0; j < nrec; j++) {
            if (rec_off[j] >= rec_off[i]) continue;
            int first = 1;
            for (int k = 0; k < j; k++)
                if (rec_off[k] == rec_off[j]) first = 0;
            rank += first;
        }
        if (i) p = str_cpy(p, " ");
        p = str_cpy(p, a_utoa(num, (u64)rank));
    }
    return text;
}

static char label_buf[128];

/* Print "<label>: <names>" and check the names; then, unless want_ranks
 * is 0, print "<label> d_off: <ranks>" and check those. */
static A_UNUSED void show(const char *label, const char *want_names, const char *want_ranks) {
    const char *got = names_text();
    put(label);
    put(": ");
    put(got);
    put("\n");
    check(str_cat2(label_buf, label, "_names"), str_eq(got, want_names), nrec);
    if (!want_ranks) return;
    got = ranks_text();
    put(label);
    put(" d_off: ");
    put(got);
    put("\n");
    check(str_cat2(label_buf, label, "_d_off"), str_eq(got, want_ranks), nrec);
}

/* "<dir>/<name>" in one of four rotating buffers. */
static A_UNUSED const char *at(const char *dir, const char *name) {
    static char paths[4][256];
    static int next;
    char *p = paths[next++ & 3];
    str_cpy(str_cpy(str_cpy(p, dir), "/"), name);
    return p;
}

static A_UNUSED i64 mk(const char *dir, const char *name) { return a_make_file(at(dir, name), "", 0644); }

static A_UNUSED i64 mv(const char *from_dir, const char *from, const char *to_dir, const char *to) {
    return syscall2(SYS_rename, at(from_dir, from), at(to_dir, to));
}

/* Create each space-separated name of `names` in `dir`, in that order;
 * returns how many failed. */
static A_UNUSED int mk_all(const char *dir, const char *names) {
    char one[32];
    int bad = 0;
    while (*names) {
        int n = 0;
        while (*names && *names != ' ') one[n++] = *names++;
        one[n] = 0;
        while (*names == ' ') names++;
        bad += mk(dir, one) != 0;
    }
    return bad;
}

/* Unlink each space-separated name of `names` in `dir`; returns how many
 * failed. */
static A_UNUSED int rm_all(const char *dir, const char *names) {
    char one[32];
    int bad = 0;
    while (*names) {
        int n = 0;
        while (*names && *names != ' ') one[n++] = *names++;
        one[n] = 0;
        while (*names == ' ') names++;
        bad += a_unlink(at(dir, one)) != 0;
    }
    return bad;
}

/* The test directories are on the tmpfs at /dev/shm (on Linux and on
 * Poly); say so if it is not one. */
static A_UNUSED void check_tmpfs(void) {
    struct a_statfs sf;
    i64 r = syscall2(SYS_statfs, "/dev/shm", &sf);
    check("setup_dev_shm_is_tmpfs", r == 0 && sf.f_type == TMPFS_MAGIC, r == 0 ? sf.f_type : r);
}

#define D "/dev/shm/d_getdents64_tmpfs_creation_order"

int test_main(void) {
    check_tmpfs();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_files", mk_all(D, "mm cc xx aa zz bb yy dd"), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("read_to_end", read_rest(fd, sizeof buf), 0);
    show("created", ". .. dd yy bb zz aa xx cc mm", "1 9 8 7 6 5 4 3 2 10");
    expect("end_0", read_call(fd, sizeof buf), 0);
    expect("end_still_0", read_call(fd, sizeof buf), 0);

    /* Other kinds of entry are new entries too. */
    expect("setup_subdir", a_mkdir(D "/sub", 0755), 0);
    expect("setup_symlink", syscall3(SYS_symlinkat, "mm", AT_FDCWD, D "/sl"), 0);
    expect("setup_hardlink", syscall5(SYS_linkat, AT_FDCWD, D "/mm", AT_FDCWD, D "/hl", 0), 0);
    expect("read_all", read_all(fd), 0);
    show("kinds", ". .. hl sl sub dd yy bb zz aa xx cc mm", "1 12 11 10 9 8 7 6 5 4 3 2 13");
    check("kinds_types", rec_type[0] == DT_DIR && rec_type[1] == DT_DIR && rec_type[2] == DT_REG &&
                             rec_type[3] == DT_LNK && rec_type[4] == DT_DIR && rec_type[5] == DT_REG,
          0);
    a_close(fd);

    /* An empty directory: "." and "..", whose d_off is the end. */
    fd = a_open(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    expect("empty_read", read_rest(fd, sizeof buf), 0);
    show("empty", ". ..", "1 2");
    /* Entries made in the subdirectory: newest first there too. */
    expect("setup_sub_files", mk_all(D "/sub", "p q r"), 0);
    expect("sub_read", read_all(fd), 0);
    show("sub", ". .. r q p", "1 4 3 2 5");
    a_close(fd);

    /* A directory renamed into another one. */
    expect("setup_dir2", a_mkdir(D "/sub/inner", 0755), 0);
    expect("move_dir_up", mv(D "/sub", "inner", D, "inner"), 0);
    fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("moved_read", read_rest(fd, sizeof buf), 0);
    show("moved_dir", ". .. inner hl sl sub dd yy bb zz aa xx cc mm", 0);
    a_close(fd);

    expect("cleanup_inner", a_rmdir(D "/inner"), 0);
    expect("cleanup_sub_files", rm_all(D "/sub", "p q r"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_files", rm_all(D, "mm cc xx aa zz bb yy dd sl hl"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
