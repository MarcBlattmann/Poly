/* poly-mac: foundation */
/* A nil key or value raises NSInvalidArgumentException; an immutable dictionary
 * does not answer setObject:forKey:; a mutable one adds, replaces and removes
 * (setObject:nil raises); a key that is a mutable string is copied, so changing
 * it later does not lose the entry. */
#include "poly_mfn.h"

static NSString *caught(void (^block)(void)) {
    @try {
        block();
    } @catch (NSException *e) {
        return e.name;
    }
    return nil;
}

int test_main(void) {
    static id keys[1], vals[1];
    keys[0] = @"k";
    vals[0] = @"v";
    NSString *r = caught(^{
        static id nk[1];
        (void)[NSDictionary dictionaryWithObjects:vals forKeys:nk count:1];
    });
    CK(r != nil && ns_is(r, "NSInvalidArgumentException"));
    r = caught(^{
        static id nv[1];
        (void)[NSDictionary dictionaryWithObjects:nv forKeys:keys count:1];
    });
    CK(r != nil && ns_is(r, "NSInvalidArgumentException"));
    NSDictionary *d = [NSDictionary dictionaryWithObjects:vals forKeys:keys count:1];
    r = caught(^{ [(NSMutableDictionary *)d setObject:@"x" forKey:@"y"]; });
    CK(r != nil && ns_is(r, "NSInvalidArgumentException"));
    NSMutableDictionary *m = [NSMutableDictionary dictionaryWithDictionary:d];
    m[@"k3"] = @"v3";
    m[@"k"] = @"replaced";
    CK(m.count == 2 && ns_is(m[@"k"], "replaced"));
    [m removeObjectForKey:@"k3"];
    [m removeObjectForKey:@"never"];
    CK(m.count == 1);
    id nothing = nil;
    r = caught(^{ [m setObject:nothing forKey:@"q"]; });
    CK(r != nil && ns_is(r, "NSInvalidArgumentException"));
    NSMutableString *mk = [NSMutableString stringWithString:@"mk"];
    NSMutableDictionary *m2 = [NSMutableDictionary dictionary];
    m2[mk] = @"value";
    [mk appendString:@"!"];
    CK(ns_is(m2[@"mk"], "value") && m2[@"mk!"] == nil);
    CK([m2.allKeys[0] isKindOfClass:[NSString class]] && m2.allKeys[0] != mk);
    CK(d.count == 1);
    return finish();
}
