/*
 * Support for macOS test programs about Foundation's own classes and functions
 * (ADR 0182, the Foundation batch). A test that includes this header says so in
 * its header comment with the line `poly-mac: foundation` (`mac build` then
 * compiles the `.c` file as Objective-C with ARC, links it with
 * `-framework Foundation`, and lets it import Foundation's, CoreFoundation's
 * and the Objective-C runtime's own symbols; all other tests import nothing,
 * see poly_mtest.h), and defines `int test_main(void)`. Classes, constants and
 * types come from the public SDK headers (<Foundation/Foundation.h>).
 *
 * What is reported does not go through the code under test: output is the raw
 * `write` system call of poly_mtest.h (`check`, `value`, `finish` as in
 * poly_mcf.h, which this header includes), never NSLog or stdio. The helpers
 * below read strings with CoreFoundation's CFStringGetCString, not with
 * Foundation's own UTF8String, so a test of NSString's methods does not
 * depend on the method it checks.
 *
 * A test about a class ARC forbids (NSAutoreleasePool) has the extra line
 * `poly-mac: no-arc` and is built without ARC; everything else is built with it.
 *
 * `test_main` runs inside an autorelease pool of its own (the program's
 * `main` makes it), so objects a test makes and drops are freed when it ends.
 *
 * Output protocol (as poly_mtest.h and the Linux corpus):
 *
 *     ok <check>
 *     not ok <check> (got <value>)
 *     value <name> <value>
 *     summary <passed> passed, <failed> failed
 *
 * Never print addresses, thread ids, times or anything else that differs
 * between runs: every test runs several times per architecture and must print
 * the same bytes. A value may differ between x86_64 and arm64; each is
 * recorded on its own. A test starts in a fresh, empty scratch directory with
 * an empty environment.
 *
 * Written for Poly from public sources and from observation only. No Apple or
 * third-party source (including swift-corelibs-foundation) was read; no code
 * is copied from anywhere.
 */
#ifndef POLY_MFN_H
#define POLY_MFN_H

#import <Foundation/Foundation.h>
#define POLY_MCF_NO_MAIN 1
#include "poly_mcf.h"

/* Whether `o` is a string with exactly the UTF-8 text `c`. */
static __attribute__((unused)) int ns_is(id o, const char *c) { return cf_is((__bridge CFTypeRef)o, c); }

/* Print `name` with the text of the string `o` (or `<null>` / `<not a string>`). */
static __attribute__((unused)) void value_ns(const char *name, id o) {
    if (!o) return value_str(name, "<null>");
    if (CFGetTypeID((__bridge CFTypeRef)o) != CFStringGetTypeID()) return value_str(name, "<not a string>");
    value_cf(name, (__bridge CFStringRef)o);
}

/* `NSLiteral`: a string made at run time from UTF-8 (never a compile-time
 * constant), for tests where the object's origin matters. */
static __attribute__((unused)) NSString *ns_make(const char *c) {
#if __has_feature(objc_arc)
    return (__bridge_transfer NSString *)CFStringCreateWithCString(kCFAllocatorDefault, c, kCFStringEncodingUTF8);
#else
    return [(NSString *)CFStringCreateWithCString(kCFAllocatorDefault, c, kCFStringEncodingUTF8) autorelease];
#endif
}

int main(int argc, char **argv) {
    poly_argc = argc;
    poly_argv = argv;
    int rc;
    @autoreleasepool {
        rc = test_main();
    }
    poly_exit(rc);
}

#endif
