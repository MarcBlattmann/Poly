/* Source of fixture.exe, a tiny freestanding program used by poly-pe's tests.
 * It imports fixture.dll by name and by ordinal, delay-loads delayed.dll
 * (an import library from delayed.def; no such DLL exists), has TLS
 * variables with one callback, a load config directory with a security
 * cookie, resources and a CodeView debug record. Built by build.sh with
 * clang + lld-link; no C runtime, no Windows SDK. The TLS and load config
 * structures below follow the layouts in Microsoft's PE Format documentation. */

typedef unsigned long long u64;
typedef void (*tls_callback)(void *module, unsigned long reason, void *reserved);

__declspec(dllimport) int fixture_add(int a, int b);
__declspec(dllimport) int fixture_hidden(void);
__declspec(dllimport) int fixture_counter;
__declspec(dllimport) int delayed_function(void);

/* ---- TLS ---- */
#pragma section(".tls", read, write)
#pragma section(".tls$ZZZ", read, write)
#pragma section(".CRT$XLA", read)
#pragma section(".CRT$XLB", read)
#pragma section(".CRT$XLZ", read)

__declspec(allocate(".tls")) char _tls_start = 0;
__declspec(allocate(".tls$ZZZ")) char _tls_end = 0;
unsigned long _tls_index = 0;

static int callback_runs;
static void on_tls(void *module, unsigned long reason, void *reserved) {
    (void)module;
    (void)reserved;
    callback_runs += (int)reason;
}

__declspec(allocate(".CRT$XLA")) tls_callback __xl_a = 0;
__declspec(allocate(".CRT$XLB")) tls_callback __xl_b = on_tls;
__declspec(allocate(".CRT$XLZ")) tls_callback __xl_z = 0;

struct tls_directory {
    u64 start_of_raw_data;
    u64 end_of_raw_data;
    u64 address_of_index;
    u64 address_of_callbacks;
    unsigned size_of_zero_fill;
    unsigned characteristics;
};

const struct tls_directory _tls_used = {
    (u64)&_tls_start, (u64)&_tls_end, (u64)&_tls_index, (u64)(&__xl_a + 1), 0, 0,
};

__declspec(thread) int tls_value = 42;
__declspec(thread) int tls_zeroed;

/* ---- load config ---- */
u64 __security_cookie = 0x2b992ddfa232ull;

struct load_config {
    unsigned size;
    unsigned time_date_stamp;
    unsigned short major_version;
    unsigned short minor_version;
    unsigned global_flags_clear;
    unsigned global_flags_set;
    unsigned critical_section_default_timeout;
    u64 de_commit_free_block_threshold;
    u64 de_commit_total_free_threshold;
    u64 lock_prefix_table;
    u64 maximum_allocation_size;
    u64 virtual_memory_threshold;
    u64 process_affinity_mask;
    unsigned process_heap_flags;
    unsigned short csd_version;
    unsigned short dependent_load_flags;
    u64 edit_list;
    u64 security_cookie;
    u64 se_handler_table;
    u64 se_handler_count;
    u64 guard_cf_check_function_pointer;
    u64 guard_cf_dispatch_function_pointer;
    u64 guard_cf_function_table;
    u64 guard_cf_function_count;
    unsigned guard_flags;
};

const struct load_config _load_config_used = {
    .size = sizeof(struct load_config),
    .security_cookie = (u64)&__security_cookie,
};

/* ---- delay-load helper (never runs in the tests) ---- */
void *__delayLoadHelper2(const void *descriptor, void **slot) {
    (void)descriptor;
    return *slot;
}

int mainCRTStartup(void) {
    return fixture_add(tls_value, fixture_hidden()) + fixture_counter + tls_zeroed + callback_runs + delayed_function();
}
