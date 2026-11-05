/* perf_event_open(2): ioctls and the mmap ring of a software counter.
 * Unknown ioctl ENOTTY, perf ioctls on a pipe ENOTTY; REFRESH and PERIOD
 * on a counting (non-sampling) event EINVAL; SET_OUTPUT to itself or to
 * an event without a buffer EINVAL, to a closed fd EBADF, to -1 (all 64
 * bits) clears the redirection and succeeds; SET_FILTER on a software
 * event EINVAL; PAUSE_OUTPUT without a buffer EINVAL; MODIFY_ATTRIBUTES
 * EOPNOTSUPP; ENABLE/RESET ignore an unknown argument. mmap: MAP_SHARED
 * of 1 page (header only, data_size 0) or 1 + 2^n pages (data_offset
 * 4096, data_size 2^n pages, data_head 0); another size while one is
 * mapped EINVAL, 1 + 3 pages EINVAL, MAP_PRIVATE EINVAL, a non-zero
 * offset EINVAL; a read-only 1 + 1 mapping works.
 * poly-diff: needs=root (unprivileged callers depend on kernel.perf_event_paranoid; distributions with 3 or 4 refuse them first)
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

static i64 mm(u64 len, int prot, int flags, i64 fd, i64 off) {
    return syscall6(SYS_mmap, 0, (i64)len, prot, flags, fd, off);
}

int test_main(void) {
    struct k_perf_attr a;
    sw_attr(&a, PERF_COUNT_SW_PAGE_FAULTS);
    i64 f = perf_open(&a, 0, -1, -1, 0);
    sw_attr(&a, PERF_COUNT_SW_DUMMY);
    i64 g = perf_open(&a, 0, -1, -1, 0);
    check("open_two", f >= 0 && g >= 0, f);
    i64 r;
    r = ioctl3(f, 0x1234, 0);
    check("unknown_ioctl_enotty", r == -ENOTTY, r);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    r = ioctl3(p[0], PERF_EVENT_IOC_ENABLE, 0);
    check("pipe_enotty", r == -ENOTTY, r);
    r = ioctl3(f, PERF_EVENT_IOC_REFRESH, 1);
    check("refresh_counting_einval", r == -EINVAL, r);
    u64 period = 1000;
    r = ioctl3(f, PERF_EVENT_IOC_PERIOD, (i64)&period);
    check("period_counting_einval", r == -EINVAL, r);
    r = ioctl3(g, PERF_EVENT_IOC_SET_OUTPUT, g);
    check("set_output_self_einval", r == -EINVAL, r);
    r = ioctl3(g, PERF_EVENT_IOC_SET_OUTPUT, f);
    check("set_output_unbuffered_einval", r == -EINVAL, r);
    r = ioctl3(g, PERF_EVENT_IOC_SET_OUTPUT, -1);
    check("set_output_minus1_clears", r == 0, r);
    r = ioctl3(g, PERF_EVENT_IOC_SET_OUTPUT, 999);
    check("set_output_closed_fd_ebadf", r == -EBADF, r);
    r = ioctl3(g, PERF_EVENT_IOC_SET_FILTER, (i64)"x");
    check("set_filter_sw_einval", r == -EINVAL, r);
    r = ioctl3(f, PERF_EVENT_IOC_PAUSE_OUTPUT, 1);
    check("pause_output_unbuffered_einval", r == -EINVAL, r);
    r = ioctl3(g, PERF_EVENT_IOC_MODIFY_ATTRIBUTES, (i64)&a);
    check("modify_attributes_eopnotsupp", r == -EOPNOTSUPP, r);
    r = ioctl3(g, PERF_EVENT_IOC_ENABLE, 5);
    check("enable_odd_arg_ok", r == 0, r);
    r = ioctl3(g, PERF_EVENT_IOC_RESET, 2);
    check("reset_odd_arg_ok", r == 0, r);

    r = mm(4096, PROT_READ, MAP_SHARED, f, 0);
    check("mmap_header_ro", !BAD(r), r);
    if (!BAD(r)) {
        check("header_data_size_0", *(u64 *)(r + PMP_DATA_SIZE) == 0, 0);
        syscall2(SYS_munmap, r, 4096);
    }
    r = mm(3 * 4096, PROT_READ | PROT_WRITE, MAP_SHARED, f, 0);
    check("mmap_1_plus_2", !BAD(r), r);
    if (!BAD(r)) {
        check("data_offset_4096", *(u64 *)(r + PMP_DATA_OFFSET) == 4096, 0);
        check("data_size_8192", *(u64 *)(r + PMP_DATA_SIZE) == 8192, 0);
        check("data_head_0", *(u64 *)(r + PMP_DATA_HEAD) == 0, 0);
        i64 q = mm(5 * 4096, PROT_READ | PROT_WRITE, MAP_SHARED, f, 0);
        check("other_size_while_mapped_einval", q == -EINVAL, q);
        syscall2(SYS_munmap, r, 3 * 4096);
    }
    r = mm(4 * 4096, PROT_READ | PROT_WRITE, MAP_SHARED, f, 0);
    check("mmap_1_plus_3_einval", r == -EINVAL, r);
    r = mm(2 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE, f, 0);
    check("mmap_private_einval", r == -EINVAL, r);
    r = mm(2 * 4096, PROT_READ | PROT_WRITE, MAP_SHARED, f, 4096);
    check("mmap_offset_einval", r == -EINVAL, r);
    r = mm(2 * 4096, PROT_READ, MAP_SHARED, f, 0);
    check("mmap_1_plus_1_ro", !BAD(r), r);
    close(p[0]);
    close(p[1]);
    close((int)g);
    close((int)f);
    return finish();
}
