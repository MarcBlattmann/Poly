//! Names of CPU features for the `flags` line of `/proc/cpuinfo`.
//!
//! The names are the feature mnemonics of the Intel 64 and IA-32
//! Architectures Software Developer's Manual (vol. 2A, CPUID, "Feature
//! Information" tables for leaves 01H and 07H) and, for leaf 80000001H,
//! the AMD64 Architecture Programmer's Manual (vol. 3, CPUID
//! Fn8000_0001), lower-cased, with `.` and `-` written as `_` (`SSE4.1` is
//! `sse4_1`). Bits a manual marks reserved or that have no short mnemonic
//! are left out. Programs look for names such as `sse2`, `sse4_2`, `avx2`,
//! `aes` or `lm`, which come out the same; a few names may differ from
//! what Linux prints (to be confirmed by the spec agent).

use crate::source::CpuFeatures;
use alloc::vec::Vec;

/// CPUID.01H:EDX.
const LEAF1_EDX: &[(u32, &str)] = &[
    (0, "fpu"),
    (1, "vme"),
    (2, "de"),
    (3, "pse"),
    (4, "tsc"),
    (5, "msr"),
    (6, "pae"),
    (7, "mce"),
    (8, "cx8"),
    (9, "apic"),
    (11, "sep"),
    (12, "mtrr"),
    (13, "pge"),
    (14, "mca"),
    (15, "cmov"),
    (16, "pat"),
    (17, "pse36"),
    (18, "psn"),
    (19, "clflush"),
    (21, "ds"),
    (22, "acpi"),
    (23, "mmx"),
    (24, "fxsr"),
    (25, "sse"),
    (26, "sse2"),
    (27, "ss"),
    (28, "ht"),
    (29, "tm"),
    (31, "pbe"),
];

/// CPUID.01H:ECX.
const LEAF1_ECX: &[(u32, &str)] = &[
    (0, "sse3"),
    (1, "pclmulqdq"),
    (2, "dtes64"),
    (3, "monitor"),
    (4, "ds_cpl"),
    (5, "vmx"),
    (6, "smx"),
    (7, "est"),
    (8, "tm2"),
    (9, "ssse3"),
    (10, "cnxt_id"),
    (11, "sdbg"),
    (12, "fma"),
    (13, "cx16"),
    (14, "xtpr"),
    (15, "pdcm"),
    (17, "pcid"),
    (18, "dca"),
    (19, "sse4_1"),
    (20, "sse4_2"),
    (21, "x2apic"),
    (22, "movbe"),
    (23, "popcnt"),
    (24, "tsc_deadline_timer"),
    (25, "aes"),
    (26, "xsave"),
    (27, "osxsave"),
    (28, "avx"),
    (29, "f16c"),
    (30, "rdrand"),
];

/// CPUID.(EAX=07H, ECX=0):EBX.
const LEAF7_EBX: &[(u32, &str)] = &[
    (0, "fsgsbase"),
    (1, "tsc_adjust"),
    (2, "sgx"),
    (3, "bmi1"),
    (4, "hle"),
    (5, "avx2"),
    (7, "smep"),
    (8, "bmi2"),
    (9, "erms"),
    (10, "invpcid"),
    (11, "rtm"),
    (14, "mpx"),
    (16, "avx512f"),
    (17, "avx512dq"),
    (18, "rdseed"),
    (19, "adx"),
    (20, "smap"),
    (21, "avx512ifma"),
    (23, "clflushopt"),
    (24, "clwb"),
    (25, "intel_pt"),
    (26, "avx512pf"),
    (27, "avx512er"),
    (28, "avx512cd"),
    (29, "sha_ni"),
    (30, "avx512bw"),
    (31, "avx512vl"),
];

/// CPUID.(EAX=07H, ECX=0):ECX.
const LEAF7_ECX: &[(u32, &str)] = &[
    (0, "prefetchwt1"),
    (1, "avx512vbmi"),
    (2, "umip"),
    (3, "pku"),
    (4, "ospke"),
    (5, "waitpkg"),
    (6, "avx512_vbmi2"),
    (8, "gfni"),
    (9, "vaes"),
    (10, "vpclmulqdq"),
    (11, "avx512_vnni"),
    (12, "avx512_bitalg"),
    (14, "avx512_vpopcntdq"),
    (16, "la57"),
    (22, "rdpid"),
    (25, "cldemote"),
    (27, "movdiri"),
    (28, "movdir64b"),
];

/// CPUID.(EAX=07H, ECX=0):EDX.
const LEAF7_EDX: &[(u32, &str)] = &[
    (4, "fsrm"),
    (8, "avx512_vp2intersect"),
    (10, "md_clear"),
    (14, "serialize"),
    (16, "tsxldtrk"),
    (18, "pconfig"),
    (20, "ibt"),
    (22, "amx_bf16"),
    (23, "avx512_fp16"),
    (24, "amx_tile"),
    (25, "amx_int8"),
    (28, "flush_l1d"),
    (29, "arch_capabilities"),
];

/// CPUID.80000001H:EDX (AMD names).
const EXT1_EDX: &[(u32, &str)] = &[(11, "syscall"), (20, "nx"), (26, "pdpe1gb"), (27, "rdtscp"), (29, "lm")];

/// CPUID.80000001H:ECX (AMD names).
const EXT1_ECX: &[(u32, &str)] = &[(0, "lahf_lm"), (5, "abm"), (8, "3dnowprefetch")];

/// The names of the features `f` has, in register order.
pub fn flag_names(f: &CpuFeatures) -> Vec<&'static str> {
    let regs: [(u32, &[(u32, &str)]); 7] = [
        (f.leaf1_edx, LEAF1_EDX),
        (f.ext1_edx, EXT1_EDX),
        (f.leaf1_ecx, LEAF1_ECX),
        (f.ext1_ecx, EXT1_ECX),
        (f.leaf7_ebx, LEAF7_EBX),
        (f.leaf7_ecx, LEAF7_ECX),
        (f.leaf7_edx, LEAF7_EDX),
    ];
    let mut names = Vec::new();
    for (value, table) in regs {
        for &(bit, name) in table {
            if value & (1 << bit) != 0 {
                names.push(name);
            }
        }
    }
    names
}
