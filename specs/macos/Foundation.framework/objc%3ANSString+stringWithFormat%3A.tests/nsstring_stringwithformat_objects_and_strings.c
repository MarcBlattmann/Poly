/* poly-mac: foundation */
/* %@ puts in an object's description: a string as itself, a number as its
 * value, a Boolean as 1 or 0 (the number 1), nil as (null), arrays and
 * dictionaries in their multi-line form; %s reads a C string as Mac Roman (not
 * UTF-8), %S a UniChar string; positional arguments reorder. */
#pragma clang diagnostic ignored "-Wformat"
#include "poly_mfn.h"

#define F(...) [NSString stringWithFormat:__VA_ARGS__]
#define IS(s, text) check(text, ns_is((s), text), 0)

int test_main(void) {
    IS(F(@"%@", @"text"), "text");
    IS(F(@"<%@>", @42), "<42>");
    IS(F(@"%@ %@", @YES, @NO), "1 0");
    IS(F(@"%@", @1.5), "1.5");
    IS(F(@"%@", (id)nil), "(null)");
    IS(F(@"%@", [NSNull null]), "<null>");
    IS(F(@"%@", @[@"a", @2]), "(\n    a,\n    2\n)");
    IS(F(@"%@", @{@"k": @"v"}), "{\n    k = v;\n}");
    IS(F(@"%@", [NSObject class]), "NSObject");
    IS(F(@"%s", "plain"), "plain");
    IS(F(@"%s", "h\xc3\xa9"), "h\xe2\x88\x9a\xc2\xa9");
    IS(F(@"[%5s][%-5s][%.1s]", "ab", "ab", "abc"), "[   ab][ab   ][a]");
    IS(F(@"%S", (const unichar[]){0x68, 0x69, 0}), "hi");
    IS(F(@"%2$@ %1$@", @"one", @"two"), "two one");
    IS(F(@"%2$d-%1$d", 1, 2), "2-1");
    IS(F(@"%@", F(@"%@", @"nested")), "nested");
    IS(F(@""), "");
    return finish();
}
