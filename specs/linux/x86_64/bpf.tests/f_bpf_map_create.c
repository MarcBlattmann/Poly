/* bpf(2) BPF_MAP_CREATE: the attr size and the descriptor. The size may
 * be shorter than the union (fields past it count as 0: 16 bytes give
 * type, key, value size and max_entries) or up to a page if the excess is
 * zero; non-zero bytes past the kernel's union E2BIG. map_name: up to 15
 * characters of [A-Za-z0-9_.] and a NUL; another character or no NUL in
 * the 16 bytes EINVAL. Accepted flags on ARRAY: BPF_F_RDONLY,
 * BPF_F_RDONLY_PROG, BPF_F_MMAPABLE. The new descriptor is O_RDWR with
 * close-on-exec, linked as "anon_inode:bpf-map", and its fdinfo names
 * map_type, key_size, value_size and max_entries.
 * poly-diff: needs=root (bpf() needs CAP_BPF while kernel.unprivileged_bpf_disabled is set, the default)
 * poly-diff: syscalls=names (the fdinfo text read back carries inode numbers and map ids)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_lseek 8
#define SYS_madvise 28
#define SYS_readlinkat 267
#define SYS_perf_event_open 298
#define SYS_bpf 321
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define E2BIG 7
#define ENOSPC 28
#define EBADFD 77
#define EOPNOTSUPP 95
/* asm-generic/mman-common.h */
#define MADV_NOHUGEPAGE 15
#define BAD(r) ((u64)(r) > (u64)-4096L)
/* linux/prctl.h */
#define PR_TASK_PERF_EVENTS_DISABLE 31
#define PR_TASK_PERF_EVENTS_ENABLE 32

/* linux/perf_event.h: enum perf_type_id */
#define PERF_TYPE_HARDWARE 0
#define PERF_TYPE_SOFTWARE 1
#define PERF_TYPE_TRACEPOINT 2
/* enum perf_sw_ids */
#define PERF_COUNT_SW_CPU_CLOCK 0
#define PERF_COUNT_SW_TASK_CLOCK 1
#define PERF_COUNT_SW_PAGE_FAULTS 2
#define PERF_COUNT_SW_CONTEXT_SWITCHES 3
#define PERF_COUNT_SW_CPU_MIGRATIONS 4
#define PERF_COUNT_SW_PAGE_FAULTS_MIN 5
#define PERF_COUNT_SW_PAGE_FAULTS_MAJ 6
#define PERF_COUNT_SW_ALIGNMENT_FAULTS 7
#define PERF_COUNT_SW_EMULATION_FAULTS 8
#define PERF_COUNT_SW_DUMMY 9
#define PERF_COUNT_SW_BPF_OUTPUT 10
#define PERF_COUNT_SW_CGROUP_SWITCHES 11
/* enum perf_event_sample_format */
#define PERF_SAMPLE_IP (1UL << 0)
#define PERF_SAMPLE_TID (1UL << 1)
#define PERF_SAMPLE_BRANCH_STACK (1UL << 11)
#define PERF_SAMPLE_REGS_USER (1UL << 12)
#define PERF_SAMPLE_STACK_USER (1UL << 13)
/* enum perf_event_read_format */
#define PERF_FORMAT_TOTAL_TIME_ENABLED (1UL << 0)
#define PERF_FORMAT_TOTAL_TIME_RUNNING (1UL << 1)
#define PERF_FORMAT_ID (1UL << 2)
#define PERF_FORMAT_GROUP (1UL << 3)
#define PERF_FORMAT_LOST (1UL << 4)
/* perf_event_open flags */
#define PERF_FLAG_FD_NO_GROUP (1UL << 0)
#define PERF_FLAG_FD_OUTPUT (1UL << 1)
#define PERF_FLAG_PID_CGROUP (1UL << 2)
#define PERF_FLAG_FD_CLOEXEC (1UL << 3)
/* attr sizes */
#define PERF_ATTR_SIZE_VER0 64
#define PERF_ATTR_SIZE_VER8 136
/* ioctls: _IO('$', n), _IOW('$', n, ...), _IOR('$', 7, __u64 *) */
#define PERF_EVENT_IOC_ENABLE 0x2400
#define PERF_EVENT_IOC_DISABLE 0x2401
#define PERF_EVENT_IOC_REFRESH 0x2402
#define PERF_EVENT_IOC_RESET 0x2403
#define PERF_EVENT_IOC_PERIOD 0x40082404
#define PERF_EVENT_IOC_SET_OUTPUT 0x2405
#define PERF_EVENT_IOC_SET_FILTER 0x40082406
#define PERF_EVENT_IOC_ID 0x80082407
#define PERF_EVENT_IOC_SET_BPF 0x40042408
#define PERF_EVENT_IOC_PAUSE_OUTPUT 0x40042409
#define PERF_EVENT_IOC_QUERY_BPF 0xc008240a
#define PERF_EVENT_IOC_MODIFY_ATTRIBUTES 0x4008240b
#define PERF_IOC_FLAG_GROUP 1

/* struct perf_event_attr, PERF_ATTR_SIZE_VER8 (136 bytes). The flag bits
 * (disabled, inherit, ...) are one u64 at offset 40; bit numbers from the
 * bit-field order in linux/perf_event.h. */
struct k_perf_attr {
    unsigned type;            /* 0 */
    unsigned size;            /* 4 */
    u64 config;               /* 8 */
    u64 sample_period;        /* 16, or sample_freq */
    u64 sample_type;          /* 24 */
    u64 read_format;          /* 32 */
    u64 flags;                /* 40 */
    unsigned wakeup_events;   /* 48 */
    unsigned bp_type;         /* 52 */
    u64 config1;              /* 56 */
    u64 config2;              /* 64 */
    u64 branch_sample_type;   /* 72 */
    u64 sample_regs_user;     /* 80 */
    unsigned sample_stack_user; /* 88 */
    int clockid;              /* 92 */
    u64 sample_regs_intr;     /* 96 */
    unsigned aux_watermark;   /* 104 */
    unsigned short sample_max_stack; /* 108 */
    unsigned short reserved_2;       /* 110 */
    unsigned aux_sample_size; /* 112 */
    unsigned aux_action;      /* 116 */
    u64 sig_data;             /* 120 */
    u64 config3;              /* 128 */
};
#define PA_DISABLED (1UL << 0)
#define PA_INHERIT (1UL << 1)
#define PA_PINNED (1UL << 2)
#define PA_EXCLUSIVE (1UL << 3)
#define PA_EXCLUDE_USER (1UL << 4)
#define PA_EXCLUDE_KERNEL (1UL << 5)
#define PA_EXCLUDE_HV (1UL << 6)
#define PA_EXCLUDE_IDLE (1UL << 7)
#define PA_MMAP (1UL << 8)
#define PA_COMM (1UL << 9)
#define PA_FREQ (1UL << 10)
#define PA_ENABLE_ON_EXEC (1UL << 12)
#define PA_USE_CLOCKID (1UL << 25)
#define PA_REMOVE_ON_EXEC (1UL << 36)
#define PA_SIGTRAP (1UL << 37)
#define PA_RESERVED_1 (1UL << 63)

/* struct perf_event_mmap_page (linux/perf_event.h): offsets used here. */
#define PMP_DATA_HEAD 1024
#define PMP_DATA_TAIL 1032
#define PMP_DATA_OFFSET 1040
#define PMP_DATA_SIZE 1048

static __attribute__((unused)) void sw_attr(struct k_perf_attr *a, u64 config) {
    memset(a, 0, sizeof *a);
    a->type = PERF_TYPE_SOFTWARE;
    a->size = sizeof *a;
    a->config = config;
    a->flags = PA_EXCLUDE_KERNEL | PA_EXCLUDE_HV;
}
static __attribute__((unused)) i64 perf_open(void *attr, i64 pid, i64 cpu, i64 group, u64 flags) {
    return syscall5(SYS_perf_event_open, attr, pid, cpu, group, flags);
}
static __attribute__((unused)) i64 ioctl3(i64 fd, u64 req, i64 arg) { return syscall3(SYS_ioctl, fd, req, arg); }
static __attribute__((unused)) void *map_anon(u64 len) {
    void *p = (void *)syscall6(SYS_mmap, 0, (i64)len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    /* No transparent huge pages: one fault per touched page. */
    syscall3(SYS_madvise, p, len, MADV_NOHUGEPAGE);
    return p;
}
static __attribute__((unused)) void touch_pages(char *p, int n) {
    for (int i = 0; i < n; i++) ((volatile char *)p)[(u64)i * 4096] = 1;
}
static __attribute__((unused)) void busy(long n) {
    for (volatile long i = 0; i < n; i++) {
    }
}
static __attribute__((unused)) i64 wait_child(i64 pid, int *status) {
    return syscall4(SYS_wait4, pid, status, 0, 0);
}
static __attribute__((unused)) int drop_to_nobody(void) {
    if (syscall2(SYS_setgroups, 0, 0) != 0) return 0;
    if (syscall3(SYS_setresgid, 65534, 65534, 65534) != 0) return 0;
    if (syscall3(SYS_setresuid, 65534, 65534, 65534) != 0) return 0;
    return 1;
}
static __attribute__((unused)) int link_is(i64 fd, const char *want) {
    char path[32] = "/proc/self/fd/", buf[64];
    int i = 14;
    char digits[12];
    int n = 0;
    u64 v = (u64)fd;
    do digits[n++] = (char)('0' + v % 10);
    while (v /= 10);
    while (n) path[i++] = digits[--n];
    path[i] = 0;
    i64 r = syscall4(SYS_readlinkat, AT_FDCWD, path, buf, sizeof buf - 1);
    if (r < 0) return 0;
    buf[r] = 0;
    return str_eq(buf, want);
}

/* linux/bpf.h: enum bpf_cmd */
#define BPF_MAP_CREATE 0
#define BPF_MAP_LOOKUP_ELEM 1
#define BPF_MAP_UPDATE_ELEM 2
#define BPF_MAP_DELETE_ELEM 3
#define BPF_MAP_GET_NEXT_KEY 4
#define BPF_PROG_LOAD 5
#define BPF_OBJ_GET_INFO_BY_FD 15
#define BPF_MAP_FREEZE 22
/* enum bpf_map_type */
#define BPF_MAP_TYPE_UNSPEC 0
#define BPF_MAP_TYPE_HASH 1
#define BPF_MAP_TYPE_ARRAY 2
/* enum bpf_prog_type */
#define BPF_PROG_TYPE_SOCKET_FILTER 1
/* map flags */
#define BPF_F_NO_PREALLOC (1U << 0)
#define BPF_F_RDONLY (1U << 3)
#define BPF_F_WRONLY (1U << 4)
#define BPF_F_RDONLY_PROG (1U << 7)
#define BPF_F_MMAPABLE (1U << 10)
/* update flags */
#define BPF_ANY 0
#define BPF_NOEXIST 1
#define BPF_EXIST 2
#define BPF_F_LOCK 4
#define BPF_OBJ_NAME_LEN 16

/* union bpf_attr, the BPF_MAP_CREATE member (leading fields). */
struct k_bpf_map_create {
    unsigned map_type, key_size, value_size, max_entries, map_flags, inner_map_fd, numa_node;
    char map_name[BPF_OBJ_NAME_LEN];
    unsigned map_ifindex, btf_fd, btf_key_type_id, btf_value_type_id, btf_vmlinux_value_type_id;
    u64 map_extra;
};
/* The element commands' member. */
struct k_bpf_elem {
    unsigned map_fd, pad;
    u64 key, value_or_next, flags;
};
/* BPF_OBJ_GET_INFO_BY_FD's member. */
struct k_bpf_info {
    unsigned bpf_fd, info_len;
    u64 info;
};
/* struct bpf_map_info (104 bytes on 6.18; leading fields). */
struct k_bpf_map_info {
    unsigned type, id, key_size, value_size, max_entries, map_flags;
    char name[BPF_OBJ_NAME_LEN];
    unsigned char rest[48];
};
/* BPF_PROG_LOAD's member (leading fields). */
struct k_bpf_prog_load {
    unsigned prog_type, insn_cnt;
    u64 insns, license;
    unsigned log_level, log_size;
    u64 log_buf;
    unsigned kern_version, prog_flags;
};
struct k_bpf_insn {
    unsigned char code, regs;
    short off;
    int imm;
};
#define BPF_ATTR_ROOM 256 /* bytes handed to bpf(): more than any member used here */

static __attribute__((unused)) i64 bpf(i64 cmd, void *attr, u64 size) { return syscall3(SYS_bpf, cmd, attr, size); }
static __attribute__((unused)) i64 map_create(unsigned type, unsigned ks, unsigned vs, unsigned n, unsigned flags) {
    union {
        struct k_bpf_map_create m;
        unsigned char room[BPF_ATTR_ROOM];
    } a;
    memset(&a, 0, sizeof a);
    a.m.map_type = type;
    a.m.key_size = ks;
    a.m.value_size = vs;
    a.m.max_entries = n;
    a.m.map_flags = flags;
    return bpf(BPF_MAP_CREATE, &a, sizeof a);
}
static __attribute__((unused)) i64 map_elem(i64 cmd, i64 fd, const void *key, void *value_or_next, u64 flags) {
    union {
        struct k_bpf_elem e;
        unsigned char room[BPF_ATTR_ROOM];
    } a;
    memset(&a, 0, sizeof a);
    a.e.map_fd = (unsigned)fd;
    a.e.key = (u64)key;
    a.e.value_or_next = (u64)value_or_next;
    a.e.flags = flags;
    return bpf(cmd, &a, sizeof a);
}
#define map_lookup(fd, k, v) map_elem(BPF_MAP_LOOKUP_ELEM, (fd), (k), (v), 0)
#define map_update(fd, k, v, f) map_elem(BPF_MAP_UPDATE_ELEM, (fd), (k), (v), (f))
#define map_delete(fd, k) map_elem(BPF_MAP_DELETE_ELEM, (fd), (k), 0, 0)
#define map_next_key(fd, k, n) map_elem(BPF_MAP_GET_NEXT_KEY, (fd), (k), (n), 0)

static unsigned char big[4096];

static int has_line(const char *text, const char *line) {
    u64 n = str_len(line);
    for (const char *p = text; *p; p++) {
        if ((p == text || p[-1] == '\n') && mem_eq(p, line, n) && p[n] == '\n') return 1;
    }
    return 0;
}

int test_main(void) {
    i64 r;
    struct k_bpf_map_create *m = (struct k_bpf_map_create *)big;
    m->map_type = BPF_MAP_TYPE_ARRAY;
    m->key_size = 4;
    m->value_size = 8;
    m->max_entries = 4;
    r = bpf(BPF_MAP_CREATE, big, 16);
    check("size_16_ok", r >= 0, r);
    close((int)r);
    r = bpf(BPF_MAP_CREATE, big, 4);
    check("size_4_key_missing_einval", r == -EINVAL, r);
    r = bpf(BPF_MAP_CREATE, big, 4096);
    check("size_page_zero_tail_ok", r >= 0, r);
    close((int)r);
    big[4000] = 1;
    r = bpf(BPF_MAP_CREATE, big, 4096);
    check("nonzero_tail_e2big", r == -E2BIG, r);
    big[4000] = 0;

    static const char good[] = "ok_name.1";
    memcpy(m->map_name, good, sizeof good);
    r = bpf(BPF_MAP_CREATE, big, 256);
    check("name_ok", r >= 0, r);
    close((int)r);
    m->map_name[0] = '!';
    r = bpf(BPF_MAP_CREATE, big, 256);
    check("name_bad_char_einval", r == -EINVAL, r);
    memset(m->map_name, 'a', 16);
    r = bpf(BPF_MAP_CREATE, big, 256);
    check("name_no_nul_einval", r == -EINVAL, r);
    memset(m->map_name, 'a', 15);
    m->map_name[15] = 0;
    r = bpf(BPF_MAP_CREATE, big, 256);
    check("name_15_ok", r >= 0, r);
    close((int)r);
    memset(m->map_name, 0, 16);

    static const unsigned flags[] = { BPF_F_RDONLY, BPF_F_RDONLY_PROG, BPF_F_MMAPABLE };
    int ok = 1;
    for (int i = 0; i < 3; i++) {
        r = map_create(BPF_MAP_TYPE_ARRAY, 4, 8, 4, flags[i]);
        if (r < 0) ok = 0;
        else close((int)r);
    }
    check("array_flags_ok", ok, r);

    i64 fd = map_create(BPF_MAP_TYPE_HASH, 4, 8, 2, 0);
    r = syscall2(SYS_fcntl, fd, F_GETFD);
    check("cloexec", r == FD_CLOEXEC, r);
    r = syscall2(SYS_fcntl, fd, F_GETFL);
    check("rdwr", (r & 3) == O_RDWR, r);
    check("fd_link", link_is(fd, "anon_inode:bpf-map"), 0);
    char path[40] = "/proc/self/fdinfo/";
    int i = 18;
    if (fd >= 10) path[i++] = (char)('0' + fd / 10 % 10);
    path[i++] = (char)('0' + fd % 10);
    path[i] = 0;
    static char text[2048];
    i64 fi = syscall4(SYS_openat, AT_FDCWD, path, O_RDONLY, 0);
    r = read((int)fi, text, sizeof text - 1);
    text[r > 0 ? r : 0] = 0;
    check("fdinfo_map_type", has_line(text, "map_type:\t1"), 0);
    check("fdinfo_key_size", has_line(text, "key_size:\t4"), 0);
    check("fdinfo_value_size", has_line(text, "value_size:\t8"), 0);
    check("fdinfo_max_entries", has_line(text, "max_entries:\t2"), 0);
    close((int)fi);
    close((int)fd);
    return finish();
}
