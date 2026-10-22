/* The privilege check comes before the parameter string is read: a NULL,
 * low, unmapped, PROT_NONE or non-canonical param_values pointer, a string
 * that runs into an unmapped page without a NUL and a very long string
 * all give EPERM without CAP_SYS_MODULE, never EFAULT or E2BIG.
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

    i64 r = init_module(elf_image, sizeof elf_image, 0);
    check("null_params_eperm", r == -EPERM, r);
    r = init_module(elf_image, sizeof elf_image, (const char *)1);
    check("low_params_eperm", r == -EPERM, r);
    r = init_module(elf_image, sizeof elf_image, (const char *)0x8000000000000000UL);
    check("non_canonical_params_eperm", r == -EPERM, r);

    char *p = map_pages(3, PROT_READ | PROT_WRITE);
    check("mmap", p != 0, 0);
    if (!p) return finish();
    memset(p, 'a', 3 * 4096);
    p[3 * 4096 - 1] = 0;
    r = init_module(elf_image, sizeof elf_image, p);
    check("long_params_eperm", r == -EPERM, r);
    syscall2(SYS_munmap, p + 2 * 4096, 4096);
    r = init_module(elf_image, sizeof elf_image, p + 4096);
    check("unterminated_params_eperm", r == -EPERM, r);
    r = init_module(elf_image, sizeof elf_image, p + 2 * 4096);
    check("unmapped_params_eperm", r == -EPERM, r);
    syscall3(SYS_mprotect, p, 4096, PROT_NONE);
    r = init_module(elf_image, sizeof elf_image, p);
    check("prot_none_params_eperm", r == -EPERM, r);
    r = init_module(0, 0, p + 2 * 4096);
    check("all_bad_eperm", r == -EPERM, r);
    return finish();
}
