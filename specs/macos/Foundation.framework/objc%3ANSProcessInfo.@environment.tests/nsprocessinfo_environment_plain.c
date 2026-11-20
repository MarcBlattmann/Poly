/* poly-mac: foundation */
/* With the environment empty, the dictionary has one entry, the variable the system
 * adds (__CF_USER_TEXT_ENCODING, of the form 0xUID:0x0:0x0, not printed); it is a
 * dictionary of strings, HOME and PATH are absent, and its description is a
 * dictionary description. */
#include "poly_mfn.h"

int test_main(void) {
    NSDictionary *env = [NSProcessInfo processInfo].environment;
    CK(env != nil && env.count == 1);
    CK(env[@"HOME"] == nil && env[@"PATH"] == nil && env[@"TMPDIR"] == nil);
    NSString *v = env[@"__CF_USER_TEXT_ENCODING"];
    CK(v != nil && [v hasPrefix:@"0x"] && [v hasSuffix:@":0x0:0x0"]);
    CK(CFGetTypeID((__bridge CFTypeRef)env) == CFDictionaryGetTypeID());
    CK([env isEqualToDictionary:[NSProcessInfo processInfo].environment]);
    return finish();
}
