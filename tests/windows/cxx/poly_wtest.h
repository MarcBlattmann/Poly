// poly_wtest.h: the support header of the C++ Windows test programs
// (ADR 0075, ADR 0098), the C++ counterpart of the Rust crate `poly-wtest`
// (tests/windows/support) and of tests/linux/include/poly_test.h.
//
// A C++ test is one `.cpp` file in tests/windows/ or a spec entry's
// `<symbol>.tests/` directory. `cargo xtask pipeline win build` compiles it
// with clang++ for x86_64-pc-windows-msvc, freestanding and without any C
// or C++ runtime of its own (-ffreestanding -nostdlib, C++ exceptions on),
// and links it with lld-link against import libraries it makes from the
// `.def` files beside this header: the program imports by name from
// kernel32.dll and msvcrt.dll only, so the same .exe runs on real Windows
// (whose msvcrt.dll exports the C++ exception support) and on Poly.
//
//   #include "poly_wtest.h"
//   static void test(poly::Test& t) {
//       int got = 0;
//       try { throw 7; } catch (int v) { got = v; }
//       t.expect("caught", got, 7);
//   }
//   POLY_TEST_MAIN(test)
//
// Output protocol (the same as the Rust tests'): observations go into the
// observation file `poly-test.out` in the executable's directory, one line
// each:
//
//   ok <check>
//   not ok <check> (got <value>)
//   value <name> <v>
//   summary <passed> passed, <failed> failed
//
// and the process ends with ExitProcess: 0 when every check passed, 1
// otherwise, 2 when the observation file cannot be created. Never print
// addresses, handles or times: the comparison is exact.
//
// Only documented calls (Microsoft Learn's kernel32 and C runtime
// reference pages) and the language's own features are used; the
// constants cite the SDK header they come from. Written for Poly; no code
// is copied from anywhere.

#pragma once

typedef unsigned long long poly_u64;
typedef long long poly_i64;
typedef unsigned int poly_u32;

// ----- kernel32.dll (Learn's prototypes) ---------------------------------------------

extern "C" {
__declspec(dllimport) poly_u32 __stdcall GetModuleFileNameW(void* module, wchar_t* name, poly_u32 size);
__declspec(dllimport) void* __stdcall GetModuleHandleW(const wchar_t* name);
__declspec(dllimport) void* __stdcall CreateFileW(const wchar_t* name, poly_u32 access, poly_u32 share, void* security,
                                                  poly_u32 disposition, poly_u32 flags, void* template_file);
__declspec(dllimport) int __stdcall WriteFile(void* file, const void* buffer, poly_u32 length, poly_u32* written,
                                              void* overlapped);
__declspec(dllimport) int __stdcall CloseHandle(void* handle);
__declspec(dllimport) __declspec(noreturn) void __stdcall ExitProcess(poly_u32 code);
__declspec(dllimport) void __stdcall RaiseException(poly_u32 code, poly_u32 flags, poly_u32 count,
                                                    const poly_u64* arguments);
// The top-level filter: LONG (WINAPI *)(EXCEPTION_POINTERS*).
__declspec(dllimport) void* __stdcall SetUnhandledExceptionFilter(void* filter);
}

// winnt.h / fileapi.h
#define POLY_GENERIC_WRITE 0x40000000u
#define POLY_FILE_SHARE_READ 0x1u
#define POLY_CREATE_ALWAYS 2u
#define POLY_FILE_ATTRIBUTE_NORMAL 0x80u
// excpt.h: what an exception filter returns.
#define POLY_EXCEPTION_EXECUTE_HANDLER 1
#define POLY_EXCEPTION_CONTINUE_SEARCH 0
#define POLY_EXCEPTION_CONTINUE_EXECUTION (-1)
// winnt.h: EXCEPTION_NONCONTINUABLE, EXCEPTION_MAXIMUM_PARAMETERS.
#define POLY_EXCEPTION_NONCONTINUABLE 0x1u
#define POLY_EXCEPTION_MAXIMUM_PARAMETERS 15

// winnt.h: EXCEPTION_RECORD (x64 layout) and EXCEPTION_POINTERS.
struct PolyExceptionRecord {
    poly_u32 ExceptionCode;
    poly_u32 ExceptionFlags;
    PolyExceptionRecord* ExceptionRecord;
    void* ExceptionAddress;
    poly_u32 NumberParameters;
    poly_u64 ExceptionInformation[POLY_EXCEPTION_MAXIMUM_PARAMETERS];
};
struct PolyExceptionPointers {
    PolyExceptionRecord* ExceptionRecord;
    void* ContextRecord;
};

// ----- msvcrt.dll: the C++ runtime functions a test names itself ----------------------

// Learn: set_terminate (<eh.h>); the C++ name msvcrt.dll exports,
// ?set_terminate@@YAP6AXXZP6AXXZ@Z, is this declaration's.
typedef void (*poly_terminate_handler)();
__declspec(dllimport) poly_terminate_handler set_terminate(poly_terminate_handler handler);
// Learn: terminate (<eh.h>, CRT); ?terminate@@YAXXZ.
__declspec(dllimport) __declspec(noreturn) void terminate();

// What the compiler's code refers to without a declaration in the source:
//
// - `??_7type_info@@6B@`, the virtual table of `type_info`, which every
//   type descriptor the compiler makes for a thrown type starts with.
//   msvcrt.dll does not export it (a program's own copy normally comes from
//   the static part of the C runtime), so the test has its own. Nothing
//   calls through it: exception matching compares type descriptors by
//   their names.
// - `__std_terminate`: what clang calls when an exception leaves a
//   `noexcept` function (a terminate scope). msvcrt.dll does not export it
//   (vcruntime140.dll does); the test's own one counts the call in
//   `poly_std_terminate_calls` (so a test can tell the compiler's terminate
//   scope from the runtime's own decision to terminate) and calls msvcrt's
//   terminate().
extern "C" const void* const poly_type_info_vftable[2] __asm__("??_7type_info@@6B@") = {0, 0};
static int poly_std_terminate_calls = 0;
extern "C" __declspec(noreturn) void __std_terminate() {
    poly_std_terminate_calls++;
    terminate();
}

// memset and memcpy, which the compiler may call for copies and
// initialisation, come from msvcrt.dll (the import library has them).
//
// A class with a virtual destructor gets a "deleting destructor" that
// refers to operator delete. Tests never allocate (no `new`), so it is
// never called; these keep the program free of msvcrt's heap.
void operator delete(void*) noexcept {}
void operator delete(void*, decltype(sizeof 0)) noexcept {}

// ----- the observation file ------------------------------------------------------------

namespace poly {

static void* g_output = 0;

static void write_raw(const char* bytes, poly_u32 n) {
    if (g_output == 0 || n == 0) return;
    poly_u32 written = 0;
    WriteFile(g_output, bytes, n, &written, 0);
}

// One line at a time: one WriteFile per line.
struct Line {
    char buf[512];
    poly_u32 len;
    constexpr Line() : buf{}, len(0) {}
    Line& str(const char* s) {
        while (*s) {
            if (len == sizeof buf) flush();
            buf[len++] = *s++;
        }
        return *this;
    }
    Line& dec(poly_i64 v) {
        char d[24];
        int i = 24;
        poly_u64 u = v < 0 ? 0 - (poly_u64)v : (poly_u64)v;
        do {
            d[--i] = (char)('0' + u % 10);
            u /= 10;
        } while (u);
        if (v < 0) str("-");
        while (i < 24) {
            char c[2] = {d[i++], 0};
            str(c);
        }
        return *this;
    }
    Line& hex(poly_u64 v) {
        static const char digits[] = "0123456789abcdef";
        int n = 1;
        while (n < 16 && (v >> (4 * n)) != 0) n++;
        str("0x");
        for (int i = n - 1; i >= 0; i--) {
            char c[2] = {digits[(v >> (4 * i)) & 0xf], 0};
            str(c);
        }
        return *this;
    }
    void flush() {
        write_raw(buf, len);
        len = 0;
    }
    void end() {
        str("\n");
        flush();
    }
};

static bool streq(const char* a, const char* b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

// The checks of one run.
struct Test {
    poly_u32 passed;
    poly_u32 failed;
    constexpr Test() : passed(0), failed(0) {}

    void pass(const char* name) {
        passed++;
        Line().str("ok ").str(name).end();
    }
    // `ok <name>` if `cond`, else `not ok <name> (got <got>)`.
    bool check(const char* name, bool cond, poly_i64 got) {
        if (cond) {
            pass(name);
        } else {
            failed++;
            Line().str("not ok ").str(name).str(" (got ").dec(got).str(")").end();
        }
        return cond;
    }
    bool expect(const char* name, poly_i64 got, poly_i64 want) { return check(name, got == want, got); }
    // Hexadecimal on failure (codes, flags).
    bool expect_hex(const char* name, poly_u64 got, poly_u64 want) {
        if (got == want) {
            pass(name);
            return true;
        }
        failed++;
        Line().str("not ok ").str(name).str(" (got ").hex(got).str(", want ").hex(want).str(")").end();
        return false;
    }
    // Two strings (an event log, a type name); the one got on failure.
    bool expect_str(const char* name, const char* got, const char* want) {
        if (streq(got, want)) {
            pass(name);
            return true;
        }
        failed++;
        Line().str("not ok ").str(name).str(" (got \"").str(got).str("\")").end();
        return false;
    }
    // Observations that are not checks (the same on every run and machine).
    void value(const char* name, poly_i64 v) { Line().str("value ").str(name).str(" ").dec(v).end(); }
    void value_hex(const char* name, poly_u64 v) { Line().str("value ").str(name).str(" ").hex(v).end(); }
    void line(const char* s) { Line().str(s).end(); }
    // `summary <p> passed, <f> failed`; the exit status.
    poly_u32 finish() {
        Line().str("summary ").dec(passed).str(" passed, ").dec(failed).str(" failed").end();
        return failed != 0;
    }
};

// End the process with `code`, the observation file closed first.
__declspec(noreturn) static void exit(poly_u32 code) {
    if (g_output) {
        CloseHandle(g_output);
        g_output = 0;
    }
    ExitProcess(code);
}

// The observation file beside the executable (GetModuleFileNameW of the
// executable, its directory, poly-test.out).
static bool open_output() {
    wchar_t path[600];
    poly_u32 n = GetModuleFileNameW(0, path, 560);
    if (n == 0 || n >= 560) return false;
    poly_u32 dir = 0;
    for (poly_u32 i = 0; i < n; i++)
        if (path[i] == L'\\' || path[i] == L'/') dir = i + 1;
    const wchar_t* name = L"poly-test.out";
    poly_u32 j = dir;
    while (*name) path[j++] = *name++;
    path[j] = 0;
    void* h = CreateFileW(path, POLY_GENERIC_WRITE, POLY_FILE_SHARE_READ, 0, POLY_CREATE_ALWAYS,
                          POLY_FILE_ATTRIBUTE_NORMAL, 0);
    if (h == (void*)-1 || h == 0) return false;
    g_output = h;
    return true;
}

// An event log: tests append short words ("A+", "~A", "catch") as things
// happen and compare the whole sequence with `Test::expect_str`.
struct Log {
    char buf[1024];
    poly_u32 len;
    constexpr Log() : buf{}, len(0) {}
    void add(const char* s) {
        if (len && len < sizeof buf - 1) buf[len++] = ' ';
        while (*s && len < sizeof buf - 1) buf[len++] = *s++;
        buf[len] = 0;
    }
    void add_num(const char* s, poly_i64 v) {
        char tmp[48];
        poly_u32 k = 0;
        while (*s && k < 20) tmp[k++] = *s++;
        char d[24];
        int i = 24;
        poly_u64 u = v < 0 ? 0 - (poly_u64)v : (poly_u64)v;
        do {
            d[--i] = (char)('0' + u % 10);
            u /= 10;
        } while (u);
        if (v < 0) tmp[k++] = '-';
        while (i < 24) tmp[k++] = d[i++];
        tmp[k] = 0;
        add(tmp);
    }
    const char* str() const { return buf; }
    void clear() {
        len = 0;
        buf[0] = 0;
    }
};

// The run's checks, for code that has no `Test&` at hand (a terminate
// handler, an exception filter). No C runtime initialises globals here: a
// global must be constant-initialised (constexpr constructors only).
static Test g_test;

// End the run from anywhere: the summary, then exit with its status.
__declspec(noreturn) static void finish_and_exit() { exit(g_test.finish()); }

}  // namespace poly

// The program's entry point (the linker's /ENTRY): open the observation
// file, run `fn(poly::Test&)`, write the summary, exit with its status.
#define POLY_TEST_MAIN(fn)                       \
    extern "C" void mainCRTStartup() {           \
        if (!poly::open_output()) poly::exit(2); \
        fn(poly::g_test);                        \
        poly::finish_and_exit();                 \
    }
