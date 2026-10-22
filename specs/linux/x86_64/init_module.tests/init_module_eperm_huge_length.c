/* The privilege check comes before any memory is set aside for the image:
 * huge lengths (2^32, 2^40, 2^63, all ones) give EPERM, not ENOMEM or
 * EFAULT, and return at once. Only the result matters here, the length
 * register is taken whole (no truncation to 32 bits is visible).
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_init_module 175
/* asm-generic/errno-base.h */
#define ENOEXEC 8
/* linux/capability.h */
#define CAP_VERSION_3 0x20080522
#define CAP_SYS_MODULE 16

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

static __attribute__((unused)) i64 init_module(const void *image, u64 len, const char *params) {
    return syscall3(SYS_init_module, image, len, params);
}

/* Empty all three capability sets (any user may do that; root then fails
 * every privilege check too) and confirm with capget that nothing is
 * left: 1 when the drop worked. */
static __attribute__((unused)) int drop_caps(void) {
    struct cap_header h = { CAP_VERSION_3, 0 };
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    if (syscall2(SYS_capset, &h, d) != 0) return 0;
    h.version = CAP_VERSION_3;
    h.pid = 0;
    memset(d, 0xff, sizeof d);
    if (syscall2(SYS_capget, &h, d) != 0) return 0;
    return d[0].effective == 0 && d[1].effective == 0 && d[0].permitted == 0 && d[1].permitted == 0;
}

static __attribute__((unused)) void *map_pages(u64 n, int prot) {
    i64 r = syscall6(SYS_mmap, 0, n * 4096, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    return r < 0 ? (void *)0 : (void *)r;
}

/* A buffer that starts like an x86-64 ELF relocatable object (the start of
 * Elf64_Ehdr from linux/elf.h: magic, ELFCLASS64, ELFDATA2LSB, EV_CURRENT,
 * e_type ET_REL 1, e_machine EM_X86_64 62); the rest is zero. */
static __attribute__((unused)) unsigned char elf_image[8192];
static __attribute__((unused)) void fill_elf(void) {
    memset(elf_image, 0, sizeof elf_image);
    elf_image[0] = 0x7f;
    elf_image[1] = 'E';
    elf_image[2] = 'L';
    elf_image[3] = 'F';
    elf_image[4] = 2;
    elf_image[5] = 1;
    elf_image[6] = 1;
    elf_image[16] = 1;
    elf_image[18] = 62;
}
int test_main(void) {
    int dropped = drop_caps();
    check("caps_dropped", dropped, dropped);
    if (!dropped) return finish();
    fill_elf();

    static const u64 lens[] = { 0x80000000UL, 0xffffffffUL, 0x100000000UL, 0x100000040UL,
                                1UL << 40, 1UL << 63, ~0UL };
    i64 t0 = now_ns();
    for (u64 i = 0; i < sizeof lens / sizeof lens[0]; i++) {
        i64 r = init_module(elf_image, lens[i], "");
        check("huge_len_eperm", r == -EPERM, r);
    }
    i64 r = init_module(0, ~0UL, 0);
    check("null_huge_len_eperm", r == -EPERM, r);
    i64 dt = now_ns() - t0;
    check("returns_at_once", dt < 1000000000L, dt);
    return finish();
}
