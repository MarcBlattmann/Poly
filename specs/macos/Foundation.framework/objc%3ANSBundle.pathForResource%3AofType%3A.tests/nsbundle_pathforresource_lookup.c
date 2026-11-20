/* poly-mac: foundation */
/* pathForResource:ofType: in an application: name and type find hello.txt in
 * Contents/Resources (an absolute path that exists), a type nil with the full file name
 * works, name 'hello' with type nil does not, a wrong or empty type, a missing
 * file give nil; the in-directory form and a language-only resource work; the
 * URL form gives the same path. */
#include "poly_mfn.h"

int test_main(void) {
    if (!cf_enter_app(CF_APP_PLIST)) return 1;
    NSBundle *b = [NSBundle mainBundle];
    NSString *p = [b pathForResource:@"hello" ofType:@"txt"];
    CK(p != nil && [p hasSuffix:@"/T.app/Contents/Resources/hello.txt"] && [p hasPrefix:@"/"]);
    CK([[NSFileManager defaultManager] fileExistsAtPath:p]);
    CK([[b pathForResource:@"hello.txt" ofType:nil] isEqualToString:p]);
    CK([b pathForResource:@"hello" ofType:nil] == nil);
    CK([b pathForResource:@"hello" ofType:@"png"] == nil);
    CK([b pathForResource:@"hello" ofType:@""] == nil);
    CK([b pathForResource:@"nothere" ofType:@"txt"] == nil);
    CK([[b pathForResource:@"deep" ofType:@"txt" inDirectory:@"sub"] hasSuffix:@"/Contents/Resources/sub/deep.txt"]);
    CK([b pathForResource:@"deep" ofType:@"txt"] == nil);
    CK([[b pathForResource:@"loc" ofType:@"txt"] hasSuffix:@"/Contents/Resources/en.lproj/loc.txt"]);
    CK([[b URLForResource:@"hello" withExtension:@"txt"].path isEqualToString:p]);
    return finish();
}
