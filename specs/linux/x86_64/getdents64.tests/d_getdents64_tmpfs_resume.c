/* getdents64 on tmpfs with entries removed or renamed between two calls
 * of one listing. The position after a call is the cookie of the next
 * entry to return. An entry not yet returned and removed is not
 * returned; if it was the next one, the listing goes on with the entry
 * after it. One already returned and removed changes nothing. One not
 * yet returned and renamed (to a new name, or over another entry) has
 * moved to the front, so this pass does not return it. */
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

#define D1 "/dev/shm/d_getdents64_tmpfs_resume1"
#define D2 "/dev/shm/d_getdents64_tmpfs_resume2"
#define D3 "/dev/shm/d_getdents64_tmpfs_resume3"
#define D4 "/dev/shm/d_getdents64_tmpfs_resume4"
#define D5 "/dev/shm/d_getdents64_tmpfs_resume5"
#define D6 "/dev/shm/d_getdents64_tmpfs_resume6"
#define NAMES "a b c d e f g h"

/* A directory with a..h (listed ". .. h g f e d c b a"), open, and the
 * first `records` records read. */
static i64 start(const char *dir, int records, const char *want) {
    expect("setup_mkdir", a_mkdir(dir, 0755), 0);
    expect("setup_files", mk_all(dir, NAMES), 0);
    i64 fd = a_open(dir, O_RDONLY | O_DIRECTORY, 0);
    expect("setup_first_call", read_call(fd, 24 * records), 24 * records);
    show("first", want, 0);
    return fd;
}

static void finish_dir(i64 fd, const char *dir, const char *names) {
    a_close(fd);
    expect("cleanup_files", rm_all(dir, names), 0);
    expect("cleanup_rmdir", a_rmdir(dir), 0);
}

int test_main(void) {
    check_tmpfs();

    /* Not yet returned, removed: skipped. */
    i64 fd = start(D1, 3, ". .. h");
    expect("unlink_later", rm_all(D1, "e c"), 0);
    expect("rest1", read_rest(fd, sizeof buf), 0);
    show("unlinked_later", "g f d b a", 0);
    finish_dir(fd, D1, "a b d f g h");

    /* The next one removed: the one after it comes next. */
    fd = start(D2, 4, ". .. h g");
    expect("unlink_next", a_unlink(D2 "/f"), 0);
    expect("rest2", read_rest(fd, sizeof buf), 0);
    show("unlinked_next", "e d c b a", 0);
    /* The next two removed. */
    expect("rewind2", read_all(fd), 0);
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    expect("again2", read_call(fd, 72), 72);
    show("first2", ". .. h", 0);
    expect("unlink_next_two", rm_all(D2, "g e"), 0);
    expect("rest2b", read_rest(fd, sizeof buf), 0);
    show("unlinked_next_two", "d c b a", 0);
    finish_dir(fd, D2, "a b c d h");

    /* Already returned, removed: no change. */
    fd = start(D3, 5, ". .. h g f");
    expect("unlink_returned", rm_all(D3, "f h"), 0);
    expect("rest3", read_rest(fd, sizeof buf), 0);
    show("unlinked_returned", "e d c b a", 0);
    finish_dir(fd, D3, "a b c d e g");

    /* Not yet returned, renamed to a new name: not in this pass; first
     * after a rewind. */
    fd = start(D4, 3, ". .. h");
    expect("rename_later", mv(D4, "c", D4, "c2"), 0);
    expect("rename_next", mv(D4, "g", D4, "g2"), 0);
    expect("rest4", read_rest(fd, sizeof buf), 0);
    show("renamed_later", "f e d b a", 0);
    expect("read_all4", read_all(fd), 0);
    show("renamed_rewound", ". .. g2 c2 h f e d b a", 0);
    finish_dir(fd, D4, "a b c2 d e f g2 h");

    /* Returned, renamed over one not yet returned: neither is returned. */
    fd = start(D5, 4, ". .. h g");
    expect("rename_over_later", mv(D5, "g", D5, "c"), 0);
    expect("rest5", read_rest(fd, sizeof buf), 0);
    show("renamed_over_later", "f e d b a", 0);
    expect("read_all5", read_all(fd), 0);
    show("renamed_over_rewound", ". .. c h f e d b a", "1 4 8 7 6 5 3 2 9");
    finish_dir(fd, D5, "a b c d e f h");

    /* Not yet returned, renamed over one already returned: the rest
     * without it. */
    fd = start(D6, 3, ". .. h");
    expect("rename_over_returned", mv(D6, "b", D6, "h"), 0);
    expect("rest6", read_rest(fd, sizeof buf), 0);
    show("renamed_over_returned", "g f e d c a", 0);
    expect("read_all6", read_all(fd), 0);
    show("renamed_over_returned_rewound", ". .. h g f e d c a", "1 8 7 6 5 4 3 2 9");
    finish_dir(fd, D6, "a c d e f g h");
    return finish();
}
