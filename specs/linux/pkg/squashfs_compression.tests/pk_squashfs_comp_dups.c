/* Images with duplicate file contents. mksquashfs stores three files with the same content
 * (one, two, three) once when it removes duplicates (default) and three times with
 * -no-duplicates; either way the file system has five inodes (root, four files), the three
 * files are separate inodes (different st_ino, link count 1 each), each reads its content
 * ("same content here\n", 18 bytes; "other\n" for four), and the fragment block is shared.
 * Image: mksquashfs dups X -b 4096 -comp gzip [-no-duplicates] -noappend -mkfs-time 0
 * -all-time 0 -all-root -processors 1.
 * poly-diff: needs=root (loop device, mount)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

static const unsigned char img_dedup[] = {
    104,115,113,115,5,0,0,0,0,0,0,0,0,16,0,0,1,0,0,0,1,0,12,0,
    192,0,1,0,4,0,0,0,128,0,0,0,0,0,0,0,35,1,0,0,0,0,0,0,
    27,1,0,0,0,0,0,0,255,255,255,255,255,255,255,255,120,0,0,0,0,0,0,0,
    169,0,0,0,0,0,0,0,237,0,0,0,0,0,0,0,13,1,0,0,0,0,0,0,
    111,116,104,101,114,10,115,97,109,101,32,99,111,110,116,101,110,116,32,104,101,114,101,10,
    47,0,120,218,99,98,88,194,200,0,5,112,6,20,176,1,49,19,146,60,19,154,156,16,
    154,60,51,1,121,22,44,242,140,12,111,225,242,172,72,246,216,65,213,0,0,88,18,4,
    47,48,0,120,218,99,102,128,0,70,40,205,196,192,204,144,150,95,90,164,0,20,97,2,
    194,252,188,84,7,48,131,133,161,36,163,40,53,53,1,40,15,226,150,148,231,3,0,147,
    181,7,78,16,128,96,0,0,0,0,0,0,0,24,0,0,1,0,0,0,0,219,0,0,
    0,0,0,0,0,22,0,120,218,99,96,128,0,5,40,237,0,165,19,160,116,3,148,6,
    0,20,40,1,65,245,0,0,0,0,0,0,0,4,128,0,0,0,0,21,1,0,0,0,
    0,0,0,
};
static const unsigned char img_nodedup[] = {
    104,115,113,115,5,0,0,0,0,0,0,0,0,16,0,0,1,0,0,0,1,0,12,0,
    128,0,1,0,4,0,0,0,128,0,0,0,0,0,0,0,46,1,0,0,0,0,0,0,
    38,1,0,0,0,0,0,0,255,255,255,255,255,255,255,255,128,0,0,0,0,0,0,0,
    182,0,0,0,0,0,0,0,248,0,0,0,0,0,0,0,24,1,0,0,0,0,0,0,
    120,218,203,47,201,72,45,226,42,78,204,77,85,72,206,207,43,73,205,43,81,0,10,164,
    18,37,2,0,163,241,21,218,52,0,120,218,99,98,88,194,200,0,5,112,6,20,176,1,
    49,19,146,60,19,154,156,16,154,60,51,146,188,4,22,121,22,36,121,45,168,60,35,195,
    91,184,60,43,146,61,118,80,59,0,98,194,4,101,48,0,120,218,99,102,128,0,70,40,
    205,196,192,204,144,150,95,90,164,0,20,97,2,194,252,188,84,7,48,131,133,161,36,163,
    40,53,53,1,40,15,226,150,148,231,3,0,147,181,7,78,14,0,120,218,75,96,128,0,
    5,40,13,0,7,16,0,129,232,0,0,0,0,0,0,0,22,0,120,218,99,96,128,0,
    5,40,237,0,165,19,160,116,3,148,6,0,20,40,1,65,0,1,0,0,0,0,0,0,
    4,128,0,0,0,0,32,1,0,0,0,0,0,0,
};

static struct p_sq sq;

static void one(const char *tag, const unsigned char *img, size_t len) {
    char name[64], path[96], buf[64];
    i64 r = p_sq_setup(&sq, "/tmp/pk_sc_dups", img, len, 0, 0);
    check(str_cat2(name, tag, ":setup"), r == 0, r);
    if (r == 0) {
        r = p_sq_mount(&sq);
        check(str_cat2(name, tag, ":mount"), r == 0, r);
        if (r == 0) {
            struct a_statfs fs;
            memset(&fs, 0, sizeof fs);
            syscall2(SYS_statfs, sq.mnt, &fs);
            check(str_cat2(name, tag, ":five_inodes"), fs.f_files == 5, fs.f_files);
            static const char *names[] = {"/one", "/two", "/three", "/four"};
            u64 inos[4];
            for (int i = 0; i < 4; i++) {
                struct a_stat st;
                memset(&st, 0, sizeof st);
                str_cat2(path, sq.mnt, names[i]);
                i64 sr = syscall2(SYS_stat, path, &st);
                check(str_cat2(name, tag, ":stat"), sr == 0 && st.st_nlink == 1 && st.st_mode == (S_IFREG | 0644) &&
                                  st.st_size == (i == 3 ? 6 : 18), sr);
                inos[i] = st.st_ino;
                memset(buf, 0, sizeof buf);
                i64 fd = a_open(path, O_RDONLY, 0);
                i64 n = fd < 0 ? fd : a_read(fd, buf, sizeof buf);
                int ok = i == 3 ? (n == 6 && mem_eq(buf, "other\n", 6)) : (n == 18 && mem_eq(buf, "same content here\n", 18));
                check(str_cat2(name, tag, ":content"), ok, n);
                if (fd >= 0) a_close(fd);
            }
            check(str_cat2(name, tag, ":distinct_inodes"), inos[0] != inos[1] && inos[1] != inos[2] && inos[0] != inos[2] &&
                                                               inos[0] != inos[3], 0);
        }
    }
    p_sq_teardown(&sq);
}

int test_main(void) {
    one("dedup", img_dedup, sizeof img_dedup);
    one("nodedup", img_nodedup, sizeof img_nodedup);
    return finish();
}
