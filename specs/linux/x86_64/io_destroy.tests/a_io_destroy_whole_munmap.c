/* munmap of the whole ring does not end the context, it only hides it:
 * io_destroy of the handle is EINVAL while nothing readable is there
 * (unmapped, or a PROT_NONE placeholder the test maps in its place), and
 * the context keeps its index (a new context gets the next one). Once
 * the memory at the old address is readable and holds the old index in
 * its first word, io_destroy of the handle returns 0, unmaps that memory
 * and frees the index. Other contexts stay alive.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

#define MAP_FIXED_NOREPLACE 0x100000

int test_main(void) {
    u64 a = 0, b = 0, c = 0, d = 0;
    unsigned char v[4];
    i64 r = io_setup(1, &a);
    check("setup_a_ok", r == 0, r);
    r = io_setup(1, &b);
    check("setup_b_ok", r == 0, r);
    if (!a || !b) return finish();
    u32 ia = ((volatile u32 *)a)[0];
    check("a_index_0", ia == 0, ia);
    r = syscall2(SYS_munmap, a, 4096);
    check("munmap_ok", r == 0, r);
    r = io_destroy(a);
    check("destroy_unmapped_einval", r == -EINVAL, r);
    /* a placeholder keeps the next ring from landing at the old address */
    i64 m = syscall6(SYS_mmap, a, 4096, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    check("placeholder_at_old_address", m == (i64)a, m);
    if (m != (i64)a) return finish();
    r = io_destroy(a);
    check("destroy_placeholder_einval", r == -EINVAL, r);
    r = io_setup(1, &c);
    check("setup_c_ok", r == 0, r);
    if (!c) return finish();
    check("index_0_still_taken", ((volatile u32 *)c)[0] == 2, ((volatile u32 *)c)[0]);

    r = syscall3(SYS_mprotect, a, 4096, PROT_READ | PROT_WRITE);
    check("placeholder_rw", r == 0, r);
    ((volatile u32 *)a)[0] = 1;
    r = io_destroy(a);
    check("wrong_index_einval", r == -EINVAL, r);
    ((volatile u32 *)a)[0] = ia;
    r = io_destroy(a);
    check("old_index_destroys", r == 0, r);
    r = syscall3(SYS_mincore, a, 4096, v);
    check("fresh_memory_unmapped", r == -ENOMEM, r);
    r = io_destroy(a);
    check("again_einval", r == -EINVAL, r);
    r = io_setup(1, &d);
    check("setup_d_ok", r == 0, r);
    check("index_0_free_again", d && ((volatile u32 *)d)[0] == 0, d ? ((volatile u32 *)d)[0] : 99);
    r = io_destroy(b);
    check("b_alive", r == 0, r);
    r = io_destroy(c);
    check("c_alive", r == 0, r);
    if (d) io_destroy(d);
    return finish();
}
