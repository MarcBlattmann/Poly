/* A swap header the kernel does not accept is EINVAL (22). The header is
 * the first 4096-byte page: signature at bytes 4086-4095, version (32-bit)
 * at byte 1024, last_page at 1028, nr_badpages at 1032 (mkswap(8) layout).
 * Rejected: the old "SWAP-SPACE" signature, version 0, version 2, and
 * version 1 with last_page 0 (an empty area). The file is left as it was.
 * poly-diff: needs=root (only root gets past the CAP_SYS_ADMIN check)
 */
#include "poly_ipc.h"

#define SYS_pread64 17

#define SYS_swapon 167
#define SYS_swapoff 168

/* A failed swapon must not have enabled anything; if it did, undo it. */
static __attribute__((unused)) void undo(const char *path, i64 r) {
    if (r == 0) syscall1(SYS_swapoff, path);
}

static __attribute__((unused)) void make_file(const char *path, const char *data, i64 len) {
    syscall3(SYS_unlinkat, AT_FDCWD, path, 0);
    i64 fd = syscall4(SYS_openat, AT_FDCWD, path, O_CREAT | O_WRONLY | O_TRUNC, 0600);
    if (len > 0) syscall3(SYS_write, fd, data, len);
    syscall1(SYS_close, fd);
}

#define F "/tmp/swapon_root_badhdr"
#define PAGES 16

static char img[PAGES * 4096];
static char back[4096];

static void put32(int off, unsigned v) {
    img[off] = (char)v;
    img[off + 1] = (char)(v >> 8);
    img[off + 2] = (char)(v >> 16);
    img[off + 3] = (char)(v >> 24);
}

static void header(const char *sig, unsigned version, unsigned last_page) {
    memset(img, 0, sizeof img);
    for (int i = 0; i < 10; i++) img[4086 + i] = sig[i];
    put32(1024, version);
    put32(1028, last_page);
    put32(1032, 0);
    make_file(F, img, sizeof img);
}

static int unchanged(void) {
    i64 fd = syscall4(SYS_openat, AT_FDCWD, F, O_RDONLY, 0);
    if (fd < 0) return 0;
    i64 n = syscall4(SYS_pread64, fd, back, sizeof back, 0);
    syscall1(SYS_close, fd);
    if (n != 4096) return 0;
    for (int i = 0; i < 4096; i++)
        if (back[i] != img[i]) return 0;
    return 1;
}

int test_main(void) {
    header("SWAP-SPACE", 1, PAGES - 1);
    i64 r = syscall2(SYS_swapon, F, 0);
    check("old_signature_einval", r == -EINVAL, r);
    undo(F, r);
    check("old_signature_unchanged", unchanged(), 0);

    header("SWAPSPACE2", 0, PAGES - 1);
    r = syscall2(SYS_swapon, F, 0);
    check("version0_einval", r == -EINVAL, r);
    undo(F, r);

    header("SWAPSPACE2", 2, PAGES - 1);
    r = syscall2(SYS_swapon, F, 0);
    check("version2_einval", r == -EINVAL, r);
    undo(F, r);
    check("version2_unchanged", unchanged(), 0);

    header("SWAPSPACE2", 1, 0);
    r = syscall2(SYS_swapon, F, 0);
    check("empty_area_einval", r == -EINVAL, r);
    undo(F, r);
    check("empty_area_unchanged", unchanged(), 0);

    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
