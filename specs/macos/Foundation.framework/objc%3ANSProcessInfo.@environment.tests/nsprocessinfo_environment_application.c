/* poly-mac: foundation */
/* environment is a dictionary of string to string with the variables the program
 * was started with (here FOO=bar, EMPTY= and a UTF-8 value), an empty value kept as an
 * empty string, a missing variable nil; the system adds a variable of its
 * own (__CF_USER_TEXT_ENCODING) to an empty environment, which is why a
 * program started with nothing has one entry. */
#include "poly_mfn.h"

int test_main(void) {
    static char foo[] = "FOO=bar", empty[] = "EMPTY=", utf[] = "U=caf\xc3\xa9";
    static char *envp[] = {foo, empty, utf, 0};
    if (!cf_enter_app_ex(CF_APP_PLIST, NULL, envp)) return 1;
    NSDictionary *env = [NSProcessInfo processInfo].environment;
    CK(env != nil);
    CK(ns_is(env[@"FOO"], "bar"));
    CK(env[@"EMPTY"] != nil && [env[@"EMPTY"] length] == 0);
    CK(ns_is(env[@"U"], "caf\xc3\xa9"));
    CK(env[@"NOPE"] == nil);
    CK(env.count == 4);
    CK(env[@"__CF_USER_TEXT_ENCODING"] != nil);
    for (id k in env) CK([k isKindOfClass:[NSString class]] && [env[k] isKindOfClass:[NSString class]]);
    return finish();
}
