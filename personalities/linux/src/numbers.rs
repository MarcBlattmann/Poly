// GENERATED from coverage/linux/syscall/x86_64.json by the test
// `table::tests::numbers_match_coverage_map`. Do not edit by hand: run
// `POLY_BLESS=1 cargo test -p poly-personality-linux numbers_match_coverage_map`.
// System call names and numbers are ABI facts (see CLEAN_ROOM.md).

/// Every x86-64 Linux system call as `(number, name)`, sorted by number.
pub const SYSCALLS: [(u64, &str); 370] = [
    (0, "read"),
    (1, "write"),
    (2, "open"),
    (3, "close"),
    (4, "stat"),
    (5, "fstat"),
    (6, "lstat"),
    (7, "poll"),
    (8, "lseek"),
    (9, "mmap"),
    (10, "mprotect"),
    (11, "munmap"),
    (12, "brk"),
    (13, "rt_sigaction"),
    (14, "rt_sigprocmask"),
    (15, "rt_sigreturn"),
    (16, "ioctl"),
    (17, "pread64"),
    (18, "pwrite64"),
    (19, "readv"),
    (20, "writev"),
    (21, "access"),
    (22, "pipe"),
    (23, "select"),
    (24, "sched_yield"),
    (25, "mremap"),
    (26, "msync"),
    (27, "mincore"),
    (28, "madvise"),
    (29, "shmget"),
    (30, "shmat"),
    (31, "shmctl"),
    (32, "dup"),
    (33, "dup2"),
    (34, "pause"),
    (35, "nanosleep"),
    (36, "getitimer"),
    (37, "alarm"),
    (38, "setitimer"),
    (39, "getpid"),
    (40, "sendfile"),
    (41, "socket"),
    (42, "connect"),
    (43, "accept"),
    (44, "sendto"),
    (45, "recvfrom"),
    (46, "sendmsg"),
    (47, "recvmsg"),
    (48, "shutdown"),
    (49, "bind"),
    (50, "listen"),
    (51, "getsockname"),
    (52, "getpeername"),
    (53, "socketpair"),
    (54, "setsockopt"),
    (55, "getsockopt"),
    (56, "clone"),
    (57, "fork"),
    (58, "vfork"),
    (59, "execve"),
    (60, "exit"),
    (61, "wait4"),
    (62, "kill"),
    (63, "uname"),
    (64, "semget"),
    (65, "semop"),
    (66, "semctl"),
    (67, "shmdt"),
    (68, "msgget"),
    (69, "msgsnd"),
    (70, "msgrcv"),
    (71, "msgctl"),
    (72, "fcntl"),
    (73, "flock"),
    (74, "fsync"),
    (75, "fdatasync"),
    (76, "truncate"),
    (77, "ftruncate"),
    (78, "getdents"),
    (79, "getcwd"),
    (80, "chdir"),
    (81, "fchdir"),
    (82, "rename"),
    (83, "mkdir"),
    (84, "rmdir"),
    (85, "creat"),
    (86, "link"),
    (87, "unlink"),
    (88, "symlink"),
    (89, "readlink"),
    (90, "chmod"),
    (91, "fchmod"),
    (92, "chown"),
    (93, "fchown"),
    (94, "lchown"),
    (95, "umask"),
    (96, "gettimeofday"),
    (97, "getrlimit"),
    (98, "getrusage"),
    (99, "sysinfo"),
    (100, "times"),
    (101, "ptrace"),
    (102, "getuid"),
    (103, "syslog"),
    (104, "getgid"),
    (105, "setuid"),
    (106, "setgid"),
    (107, "geteuid"),
    (108, "getegid"),
    (109, "setpgid"),
    (110, "getppid"),
    (111, "getpgrp"),
    (112, "setsid"),
    (113, "setreuid"),
    (114, "setregid"),
    (115, "getgroups"),
    (116, "setgroups"),
    (117, "setresuid"),
    (118, "getresuid"),
    (119, "setresgid"),
    (120, "getresgid"),
    (121, "getpgid"),
    (122, "setfsuid"),
    (123, "setfsgid"),
    (124, "getsid"),
    (125, "capget"),
    (126, "capset"),
    (127, "rt_sigpending"),
    (128, "rt_sigtimedwait"),
    (129, "rt_sigqueueinfo"),
    (130, "rt_sigsuspend"),
    (131, "sigaltstack"),
    (132, "utime"),
    (133, "mknod"),
    (135, "personality"),
    (136, "ustat"),
    (137, "statfs"),
    (138, "fstatfs"),
    (139, "sysfs"),
    (140, "getpriority"),
    (141, "setpriority"),
    (142, "sched_setparam"),
    (143, "sched_getparam"),
    (144, "sched_setscheduler"),
    (145, "sched_getscheduler"),
    (146, "sched_get_priority_max"),
    (147, "sched_get_priority_min"),
    (148, "sched_rr_get_interval"),
    (149, "mlock"),
    (150, "munlock"),
    (151, "mlockall"),
    (152, "munlockall"),
    (153, "vhangup"),
    (154, "modify_ldt"),
    (155, "pivot_root"),
    (156, "_sysctl"),
    (157, "prctl"),
    (158, "arch_prctl"),
    (159, "adjtimex"),
    (160, "setrlimit"),
    (161, "chroot"),
    (162, "sync"),
    (163, "acct"),
    (164, "settimeofday"),
    (165, "mount"),
    (166, "umount2"),
    (167, "swapon"),
    (168, "swapoff"),
    (169, "reboot"),
    (170, "sethostname"),
    (171, "setdomainname"),
    (172, "iopl"),
    (173, "ioperm"),
    (175, "init_module"),
    (176, "delete_module"),
    (179, "quotactl"),
    (186, "gettid"),
    (187, "readahead"),
    (188, "setxattr"),
    (189, "lsetxattr"),
    (190, "fsetxattr"),
    (191, "getxattr"),
    (192, "lgetxattr"),
    (193, "fgetxattr"),
    (194, "listxattr"),
    (195, "llistxattr"),
    (196, "flistxattr"),
    (197, "removexattr"),
    (198, "lremovexattr"),
    (199, "fremovexattr"),
    (200, "tkill"),
    (201, "time"),
    (202, "futex"),
    (203, "sched_setaffinity"),
    (204, "sched_getaffinity"),
    (206, "io_setup"),
    (207, "io_destroy"),
    (208, "io_getevents"),
    (209, "io_submit"),
    (210, "io_cancel"),
    (213, "epoll_create"),
    (216, "remap_file_pages"),
    (217, "getdents64"),
    (218, "set_tid_address"),
    (219, "restart_syscall"),
    (220, "semtimedop"),
    (221, "fadvise64"),
    (222, "timer_create"),
    (223, "timer_settime"),
    (224, "timer_gettime"),
    (225, "timer_getoverrun"),
    (226, "timer_delete"),
    (227, "clock_settime"),
    (228, "clock_gettime"),
    (229, "clock_getres"),
    (230, "clock_nanosleep"),
    (231, "exit_group"),
    (232, "epoll_wait"),
    (233, "epoll_ctl"),
    (234, "tgkill"),
    (235, "utimes"),
    (237, "mbind"),
    (238, "set_mempolicy"),
    (239, "get_mempolicy"),
    (240, "mq_open"),
    (241, "mq_unlink"),
    (242, "mq_timedsend"),
    (243, "mq_timedreceive"),
    (244, "mq_notify"),
    (245, "mq_getsetattr"),
    (246, "kexec_load"),
    (247, "waitid"),
    (248, "add_key"),
    (249, "request_key"),
    (250, "keyctl"),
    (251, "ioprio_set"),
    (252, "ioprio_get"),
    (253, "inotify_init"),
    (254, "inotify_add_watch"),
    (255, "inotify_rm_watch"),
    (256, "migrate_pages"),
    (257, "openat"),
    (258, "mkdirat"),
    (259, "mknodat"),
    (260, "fchownat"),
    (261, "futimesat"),
    (262, "newfstatat"),
    (263, "unlinkat"),
    (264, "renameat"),
    (265, "linkat"),
    (266, "symlinkat"),
    (267, "readlinkat"),
    (268, "fchmodat"),
    (269, "faccessat"),
    (270, "pselect6"),
    (271, "ppoll"),
    (272, "unshare"),
    (273, "set_robust_list"),
    (274, "get_robust_list"),
    (275, "splice"),
    (276, "tee"),
    (277, "sync_file_range"),
    (278, "vmsplice"),
    (279, "move_pages"),
    (280, "utimensat"),
    (281, "epoll_pwait"),
    (282, "signalfd"),
    (283, "timerfd_create"),
    (284, "eventfd"),
    (285, "fallocate"),
    (286, "timerfd_settime"),
    (287, "timerfd_gettime"),
    (288, "accept4"),
    (289, "signalfd4"),
    (290, "eventfd2"),
    (291, "epoll_create1"),
    (292, "dup3"),
    (293, "pipe2"),
    (294, "inotify_init1"),
    (295, "preadv"),
    (296, "pwritev"),
    (297, "rt_tgsigqueueinfo"),
    (298, "perf_event_open"),
    (299, "recvmmsg"),
    (300, "fanotify_init"),
    (301, "fanotify_mark"),
    (302, "prlimit64"),
    (303, "name_to_handle_at"),
    (304, "open_by_handle_at"),
    (305, "clock_adjtime"),
    (306, "syncfs"),
    (307, "sendmmsg"),
    (308, "setns"),
    (309, "getcpu"),
    (310, "process_vm_readv"),
    (311, "process_vm_writev"),
    (312, "kcmp"),
    (313, "finit_module"),
    (314, "sched_setattr"),
    (315, "sched_getattr"),
    (316, "renameat2"),
    (317, "seccomp"),
    (318, "getrandom"),
    (319, "memfd_create"),
    (320, "kexec_file_load"),
    (321, "bpf"),
    (322, "execveat"),
    (323, "userfaultfd"),
    (324, "membarrier"),
    (325, "mlock2"),
    (326, "copy_file_range"),
    (327, "preadv2"),
    (328, "pwritev2"),
    (329, "pkey_mprotect"),
    (330, "pkey_alloc"),
    (331, "pkey_free"),
    (332, "statx"),
    (333, "io_pgetevents"),
    (334, "rseq"),
    (335, "uretprobe"),
    (336, "uprobe"),
    (424, "pidfd_send_signal"),
    (425, "io_uring_setup"),
    (426, "io_uring_enter"),
    (427, "io_uring_register"),
    (428, "open_tree"),
    (429, "move_mount"),
    (430, "fsopen"),
    (431, "fsconfig"),
    (432, "fsmount"),
    (433, "fspick"),
    (434, "pidfd_open"),
    (435, "clone3"),
    (436, "close_range"),
    (437, "openat2"),
    (438, "pidfd_getfd"),
    (439, "faccessat2"),
    (440, "process_madvise"),
    (441, "epoll_pwait2"),
    (442, "mount_setattr"),
    (443, "quotactl_fd"),
    (444, "landlock_create_ruleset"),
    (445, "landlock_add_rule"),
    (446, "landlock_restrict_self"),
    (447, "memfd_secret"),
    (448, "process_mrelease"),
    (449, "futex_waitv"),
    (450, "set_mempolicy_home_node"),
    (451, "cachestat"),
    (452, "fchmodat2"),
    (453, "map_shadow_stack"),
    (454, "futex_wake"),
    (455, "futex_wait"),
    (456, "futex_requeue"),
    (457, "statmount"),
    (458, "listmount"),
    (459, "lsm_get_self_attr"),
    (460, "lsm_set_self_attr"),
    (461, "lsm_list_modules"),
    (462, "mseal"),
    (463, "setxattrat"),
    (464, "getxattrat"),
    (465, "listxattrat"),
    (466, "removexattrat"),
    (467, "open_tree_attr"),
    (468, "file_getattr"),
    (469, "file_setattr"),
    (470, "listns"),
    (471, "rseq_slice_yield"),
    (472, "fchroot"),
];

/// System call numbers, named after the call in upper case.
pub mod nr {
    pub const READ: u64 = 0;
    pub const WRITE: u64 = 1;
    pub const OPEN: u64 = 2;
    pub const CLOSE: u64 = 3;
    pub const STAT: u64 = 4;
    pub const FSTAT: u64 = 5;
    pub const LSTAT: u64 = 6;
    pub const POLL: u64 = 7;
    pub const LSEEK: u64 = 8;
    pub const MMAP: u64 = 9;
    pub const MPROTECT: u64 = 10;
    pub const MUNMAP: u64 = 11;
    pub const BRK: u64 = 12;
    pub const RT_SIGACTION: u64 = 13;
    pub const RT_SIGPROCMASK: u64 = 14;
    pub const RT_SIGRETURN: u64 = 15;
    pub const IOCTL: u64 = 16;
    pub const PREAD64: u64 = 17;
    pub const PWRITE64: u64 = 18;
    pub const READV: u64 = 19;
    pub const WRITEV: u64 = 20;
    pub const ACCESS: u64 = 21;
    pub const PIPE: u64 = 22;
    pub const SELECT: u64 = 23;
    pub const SCHED_YIELD: u64 = 24;
    pub const MREMAP: u64 = 25;
    pub const MSYNC: u64 = 26;
    pub const MINCORE: u64 = 27;
    pub const MADVISE: u64 = 28;
    pub const SHMGET: u64 = 29;
    pub const SHMAT: u64 = 30;
    pub const SHMCTL: u64 = 31;
    pub const DUP: u64 = 32;
    pub const DUP2: u64 = 33;
    pub const PAUSE: u64 = 34;
    pub const NANOSLEEP: u64 = 35;
    pub const GETITIMER: u64 = 36;
    pub const ALARM: u64 = 37;
    pub const SETITIMER: u64 = 38;
    pub const GETPID: u64 = 39;
    pub const SENDFILE: u64 = 40;
    pub const SOCKET: u64 = 41;
    pub const CONNECT: u64 = 42;
    pub const ACCEPT: u64 = 43;
    pub const SENDTO: u64 = 44;
    pub const RECVFROM: u64 = 45;
    pub const SENDMSG: u64 = 46;
    pub const RECVMSG: u64 = 47;
    pub const SHUTDOWN: u64 = 48;
    pub const BIND: u64 = 49;
    pub const LISTEN: u64 = 50;
    pub const GETSOCKNAME: u64 = 51;
    pub const GETPEERNAME: u64 = 52;
    pub const SOCKETPAIR: u64 = 53;
    pub const SETSOCKOPT: u64 = 54;
    pub const GETSOCKOPT: u64 = 55;
    pub const CLONE: u64 = 56;
    pub const FORK: u64 = 57;
    pub const VFORK: u64 = 58;
    pub const EXECVE: u64 = 59;
    pub const EXIT: u64 = 60;
    pub const WAIT4: u64 = 61;
    pub const KILL: u64 = 62;
    pub const UNAME: u64 = 63;
    pub const SEMGET: u64 = 64;
    pub const SEMOP: u64 = 65;
    pub const SEMCTL: u64 = 66;
    pub const SHMDT: u64 = 67;
    pub const MSGGET: u64 = 68;
    pub const MSGSND: u64 = 69;
    pub const MSGRCV: u64 = 70;
    pub const MSGCTL: u64 = 71;
    pub const FCNTL: u64 = 72;
    pub const FLOCK: u64 = 73;
    pub const FSYNC: u64 = 74;
    pub const FDATASYNC: u64 = 75;
    pub const TRUNCATE: u64 = 76;
    pub const FTRUNCATE: u64 = 77;
    pub const GETDENTS: u64 = 78;
    pub const GETCWD: u64 = 79;
    pub const CHDIR: u64 = 80;
    pub const FCHDIR: u64 = 81;
    pub const RENAME: u64 = 82;
    pub const MKDIR: u64 = 83;
    pub const RMDIR: u64 = 84;
    pub const CREAT: u64 = 85;
    pub const LINK: u64 = 86;
    pub const UNLINK: u64 = 87;
    pub const SYMLINK: u64 = 88;
    pub const READLINK: u64 = 89;
    pub const CHMOD: u64 = 90;
    pub const FCHMOD: u64 = 91;
    pub const CHOWN: u64 = 92;
    pub const FCHOWN: u64 = 93;
    pub const LCHOWN: u64 = 94;
    pub const UMASK: u64 = 95;
    pub const GETTIMEOFDAY: u64 = 96;
    pub const GETRLIMIT: u64 = 97;
    pub const GETRUSAGE: u64 = 98;
    pub const SYSINFO: u64 = 99;
    pub const TIMES: u64 = 100;
    pub const PTRACE: u64 = 101;
    pub const GETUID: u64 = 102;
    pub const SYSLOG: u64 = 103;
    pub const GETGID: u64 = 104;
    pub const SETUID: u64 = 105;
    pub const SETGID: u64 = 106;
    pub const GETEUID: u64 = 107;
    pub const GETEGID: u64 = 108;
    pub const SETPGID: u64 = 109;
    pub const GETPPID: u64 = 110;
    pub const GETPGRP: u64 = 111;
    pub const SETSID: u64 = 112;
    pub const SETREUID: u64 = 113;
    pub const SETREGID: u64 = 114;
    pub const GETGROUPS: u64 = 115;
    pub const SETGROUPS: u64 = 116;
    pub const SETRESUID: u64 = 117;
    pub const GETRESUID: u64 = 118;
    pub const SETRESGID: u64 = 119;
    pub const GETRESGID: u64 = 120;
    pub const GETPGID: u64 = 121;
    pub const SETFSUID: u64 = 122;
    pub const SETFSGID: u64 = 123;
    pub const GETSID: u64 = 124;
    pub const CAPGET: u64 = 125;
    pub const CAPSET: u64 = 126;
    pub const RT_SIGPENDING: u64 = 127;
    pub const RT_SIGTIMEDWAIT: u64 = 128;
    pub const RT_SIGQUEUEINFO: u64 = 129;
    pub const RT_SIGSUSPEND: u64 = 130;
    pub const SIGALTSTACK: u64 = 131;
    pub const UTIME: u64 = 132;
    pub const MKNOD: u64 = 133;
    pub const PERSONALITY: u64 = 135;
    pub const USTAT: u64 = 136;
    pub const STATFS: u64 = 137;
    pub const FSTATFS: u64 = 138;
    pub const SYSFS: u64 = 139;
    pub const GETPRIORITY: u64 = 140;
    pub const SETPRIORITY: u64 = 141;
    pub const SCHED_SETPARAM: u64 = 142;
    pub const SCHED_GETPARAM: u64 = 143;
    pub const SCHED_SETSCHEDULER: u64 = 144;
    pub const SCHED_GETSCHEDULER: u64 = 145;
    pub const SCHED_GET_PRIORITY_MAX: u64 = 146;
    pub const SCHED_GET_PRIORITY_MIN: u64 = 147;
    pub const SCHED_RR_GET_INTERVAL: u64 = 148;
    pub const MLOCK: u64 = 149;
    pub const MUNLOCK: u64 = 150;
    pub const MLOCKALL: u64 = 151;
    pub const MUNLOCKALL: u64 = 152;
    pub const VHANGUP: u64 = 153;
    pub const MODIFY_LDT: u64 = 154;
    pub const PIVOT_ROOT: u64 = 155;
    pub const _SYSCTL: u64 = 156;
    pub const PRCTL: u64 = 157;
    pub const ARCH_PRCTL: u64 = 158;
    pub const ADJTIMEX: u64 = 159;
    pub const SETRLIMIT: u64 = 160;
    pub const CHROOT: u64 = 161;
    pub const SYNC: u64 = 162;
    pub const ACCT: u64 = 163;
    pub const SETTIMEOFDAY: u64 = 164;
    pub const MOUNT: u64 = 165;
    pub const UMOUNT2: u64 = 166;
    pub const SWAPON: u64 = 167;
    pub const SWAPOFF: u64 = 168;
    pub const REBOOT: u64 = 169;
    pub const SETHOSTNAME: u64 = 170;
    pub const SETDOMAINNAME: u64 = 171;
    pub const IOPL: u64 = 172;
    pub const IOPERM: u64 = 173;
    pub const INIT_MODULE: u64 = 175;
    pub const DELETE_MODULE: u64 = 176;
    pub const QUOTACTL: u64 = 179;
    pub const GETTID: u64 = 186;
    pub const READAHEAD: u64 = 187;
    pub const SETXATTR: u64 = 188;
    pub const LSETXATTR: u64 = 189;
    pub const FSETXATTR: u64 = 190;
    pub const GETXATTR: u64 = 191;
    pub const LGETXATTR: u64 = 192;
    pub const FGETXATTR: u64 = 193;
    pub const LISTXATTR: u64 = 194;
    pub const LLISTXATTR: u64 = 195;
    pub const FLISTXATTR: u64 = 196;
    pub const REMOVEXATTR: u64 = 197;
    pub const LREMOVEXATTR: u64 = 198;
    pub const FREMOVEXATTR: u64 = 199;
    pub const TKILL: u64 = 200;
    pub const TIME: u64 = 201;
    pub const FUTEX: u64 = 202;
    pub const SCHED_SETAFFINITY: u64 = 203;
    pub const SCHED_GETAFFINITY: u64 = 204;
    pub const IO_SETUP: u64 = 206;
    pub const IO_DESTROY: u64 = 207;
    pub const IO_GETEVENTS: u64 = 208;
    pub const IO_SUBMIT: u64 = 209;
    pub const IO_CANCEL: u64 = 210;
    pub const EPOLL_CREATE: u64 = 213;
    pub const REMAP_FILE_PAGES: u64 = 216;
    pub const GETDENTS64: u64 = 217;
    pub const SET_TID_ADDRESS: u64 = 218;
    pub const RESTART_SYSCALL: u64 = 219;
    pub const SEMTIMEDOP: u64 = 220;
    pub const FADVISE64: u64 = 221;
    pub const TIMER_CREATE: u64 = 222;
    pub const TIMER_SETTIME: u64 = 223;
    pub const TIMER_GETTIME: u64 = 224;
    pub const TIMER_GETOVERRUN: u64 = 225;
    pub const TIMER_DELETE: u64 = 226;
    pub const CLOCK_SETTIME: u64 = 227;
    pub const CLOCK_GETTIME: u64 = 228;
    pub const CLOCK_GETRES: u64 = 229;
    pub const CLOCK_NANOSLEEP: u64 = 230;
    pub const EXIT_GROUP: u64 = 231;
    pub const EPOLL_WAIT: u64 = 232;
    pub const EPOLL_CTL: u64 = 233;
    pub const TGKILL: u64 = 234;
    pub const UTIMES: u64 = 235;
    pub const MBIND: u64 = 237;
    pub const SET_MEMPOLICY: u64 = 238;
    pub const GET_MEMPOLICY: u64 = 239;
    pub const MQ_OPEN: u64 = 240;
    pub const MQ_UNLINK: u64 = 241;
    pub const MQ_TIMEDSEND: u64 = 242;
    pub const MQ_TIMEDRECEIVE: u64 = 243;
    pub const MQ_NOTIFY: u64 = 244;
    pub const MQ_GETSETATTR: u64 = 245;
    pub const KEXEC_LOAD: u64 = 246;
    pub const WAITID: u64 = 247;
    pub const ADD_KEY: u64 = 248;
    pub const REQUEST_KEY: u64 = 249;
    pub const KEYCTL: u64 = 250;
    pub const IOPRIO_SET: u64 = 251;
    pub const IOPRIO_GET: u64 = 252;
    pub const INOTIFY_INIT: u64 = 253;
    pub const INOTIFY_ADD_WATCH: u64 = 254;
    pub const INOTIFY_RM_WATCH: u64 = 255;
    pub const MIGRATE_PAGES: u64 = 256;
    pub const OPENAT: u64 = 257;
    pub const MKDIRAT: u64 = 258;
    pub const MKNODAT: u64 = 259;
    pub const FCHOWNAT: u64 = 260;
    pub const FUTIMESAT: u64 = 261;
    pub const NEWFSTATAT: u64 = 262;
    pub const UNLINKAT: u64 = 263;
    pub const RENAMEAT: u64 = 264;
    pub const LINKAT: u64 = 265;
    pub const SYMLINKAT: u64 = 266;
    pub const READLINKAT: u64 = 267;
    pub const FCHMODAT: u64 = 268;
    pub const FACCESSAT: u64 = 269;
    pub const PSELECT6: u64 = 270;
    pub const PPOLL: u64 = 271;
    pub const UNSHARE: u64 = 272;
    pub const SET_ROBUST_LIST: u64 = 273;
    pub const GET_ROBUST_LIST: u64 = 274;
    pub const SPLICE: u64 = 275;
    pub const TEE: u64 = 276;
    pub const SYNC_FILE_RANGE: u64 = 277;
    pub const VMSPLICE: u64 = 278;
    pub const MOVE_PAGES: u64 = 279;
    pub const UTIMENSAT: u64 = 280;
    pub const EPOLL_PWAIT: u64 = 281;
    pub const SIGNALFD: u64 = 282;
    pub const TIMERFD_CREATE: u64 = 283;
    pub const EVENTFD: u64 = 284;
    pub const FALLOCATE: u64 = 285;
    pub const TIMERFD_SETTIME: u64 = 286;
    pub const TIMERFD_GETTIME: u64 = 287;
    pub const ACCEPT4: u64 = 288;
    pub const SIGNALFD4: u64 = 289;
    pub const EVENTFD2: u64 = 290;
    pub const EPOLL_CREATE1: u64 = 291;
    pub const DUP3: u64 = 292;
    pub const PIPE2: u64 = 293;
    pub const INOTIFY_INIT1: u64 = 294;
    pub const PREADV: u64 = 295;
    pub const PWRITEV: u64 = 296;
    pub const RT_TGSIGQUEUEINFO: u64 = 297;
    pub const PERF_EVENT_OPEN: u64 = 298;
    pub const RECVMMSG: u64 = 299;
    pub const FANOTIFY_INIT: u64 = 300;
    pub const FANOTIFY_MARK: u64 = 301;
    pub const PRLIMIT64: u64 = 302;
    pub const NAME_TO_HANDLE_AT: u64 = 303;
    pub const OPEN_BY_HANDLE_AT: u64 = 304;
    pub const CLOCK_ADJTIME: u64 = 305;
    pub const SYNCFS: u64 = 306;
    pub const SENDMMSG: u64 = 307;
    pub const SETNS: u64 = 308;
    pub const GETCPU: u64 = 309;
    pub const PROCESS_VM_READV: u64 = 310;
    pub const PROCESS_VM_WRITEV: u64 = 311;
    pub const KCMP: u64 = 312;
    pub const FINIT_MODULE: u64 = 313;
    pub const SCHED_SETATTR: u64 = 314;
    pub const SCHED_GETATTR: u64 = 315;
    pub const RENAMEAT2: u64 = 316;
    pub const SECCOMP: u64 = 317;
    pub const GETRANDOM: u64 = 318;
    pub const MEMFD_CREATE: u64 = 319;
    pub const KEXEC_FILE_LOAD: u64 = 320;
    pub const BPF: u64 = 321;
    pub const EXECVEAT: u64 = 322;
    pub const USERFAULTFD: u64 = 323;
    pub const MEMBARRIER: u64 = 324;
    pub const MLOCK2: u64 = 325;
    pub const COPY_FILE_RANGE: u64 = 326;
    pub const PREADV2: u64 = 327;
    pub const PWRITEV2: u64 = 328;
    pub const PKEY_MPROTECT: u64 = 329;
    pub const PKEY_ALLOC: u64 = 330;
    pub const PKEY_FREE: u64 = 331;
    pub const STATX: u64 = 332;
    pub const IO_PGETEVENTS: u64 = 333;
    pub const RSEQ: u64 = 334;
    pub const URETPROBE: u64 = 335;
    pub const UPROBE: u64 = 336;
    pub const PIDFD_SEND_SIGNAL: u64 = 424;
    pub const IO_URING_SETUP: u64 = 425;
    pub const IO_URING_ENTER: u64 = 426;
    pub const IO_URING_REGISTER: u64 = 427;
    pub const OPEN_TREE: u64 = 428;
    pub const MOVE_MOUNT: u64 = 429;
    pub const FSOPEN: u64 = 430;
    pub const FSCONFIG: u64 = 431;
    pub const FSMOUNT: u64 = 432;
    pub const FSPICK: u64 = 433;
    pub const PIDFD_OPEN: u64 = 434;
    pub const CLONE3: u64 = 435;
    pub const CLOSE_RANGE: u64 = 436;
    pub const OPENAT2: u64 = 437;
    pub const PIDFD_GETFD: u64 = 438;
    pub const FACCESSAT2: u64 = 439;
    pub const PROCESS_MADVISE: u64 = 440;
    pub const EPOLL_PWAIT2: u64 = 441;
    pub const MOUNT_SETATTR: u64 = 442;
    pub const QUOTACTL_FD: u64 = 443;
    pub const LANDLOCK_CREATE_RULESET: u64 = 444;
    pub const LANDLOCK_ADD_RULE: u64 = 445;
    pub const LANDLOCK_RESTRICT_SELF: u64 = 446;
    pub const MEMFD_SECRET: u64 = 447;
    pub const PROCESS_MRELEASE: u64 = 448;
    pub const FUTEX_WAITV: u64 = 449;
    pub const SET_MEMPOLICY_HOME_NODE: u64 = 450;
    pub const CACHESTAT: u64 = 451;
    pub const FCHMODAT2: u64 = 452;
    pub const MAP_SHADOW_STACK: u64 = 453;
    pub const FUTEX_WAKE: u64 = 454;
    pub const FUTEX_WAIT: u64 = 455;
    pub const FUTEX_REQUEUE: u64 = 456;
    pub const STATMOUNT: u64 = 457;
    pub const LISTMOUNT: u64 = 458;
    pub const LSM_GET_SELF_ATTR: u64 = 459;
    pub const LSM_SET_SELF_ATTR: u64 = 460;
    pub const LSM_LIST_MODULES: u64 = 461;
    pub const MSEAL: u64 = 462;
    pub const SETXATTRAT: u64 = 463;
    pub const GETXATTRAT: u64 = 464;
    pub const LISTXATTRAT: u64 = 465;
    pub const REMOVEXATTRAT: u64 = 466;
    pub const OPEN_TREE_ATTR: u64 = 467;
    pub const FILE_GETATTR: u64 = 468;
    pub const FILE_SETATTR: u64 = 469;
    pub const LISTNS: u64 = 470;
    pub const RSEQ_SLICE_YIELD: u64 = 471;
    pub const FCHROOT: u64 = 472;
}
