/* With CAP_SYS_MODULE, an image shorter than an ELF header (64 bytes on
 * x86-64) is ENOEXEC, and the length is checked before the image or the
 * parameters are read: lengths 0, 1 and 63 give ENOEXEC even with a NULL,
 * unmapped or non-canonical image pointer and a bad param_values pointer.
 * Nothing is loaded: these images can never be a module.
 * poly-diff: needs=root (only a caller with CAP_SYS_MODULE gets past the privilege check)
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
    fill_elf();
    i64 r = init_module(0, 0, 0);
    check("null_len0_enoexec", r == -ENOEXEC, r);
    if (r == -EPERM) return finish();
    r = init_module(elf_image, 0, "");
    check("len0_enoexec", r == -ENOEXEC, r);
    r = init_module(elf_image, 1, "");
    check("len1_enoexec", r == -ENOEXEC, r);
    r = init_module(elf_image, 63, "");
    check("len63_enoexec", r == -ENOEXEC, r);
    r = init_module(elf_image, 63, "a=1 b");
    check("len63_params_enoexec", r == -ENOEXEC, r);
    r = init_module(0, 63, 0);
    check("null_len63_enoexec", r == -ENOEXEC, r);
    r = init_module((void *)0x8000000000000000UL, 32, (const char *)1);
    check("non_canonical_short_enoexec", r == -ENOEXEC, r);

    unsigned char *p = map_pages(1, PROT_READ | PROT_WRITE);
    check("mmap", p != 0, 0);
    if (!p) return finish();
    syscall2(SYS_munmap, p, 4096);
    r = init_module(p, 63, (const char *)p);
    check("unmapped_short_enoexec", r == -ENOEXEC, r);
    return finish();
}
