/* poly-mac: foundation */
/* compare: as a sort selector orders strings in unit order (capitals before lower
 * case, digits before letters), a stable total order; with caseInsensitiveCompare:
 * letters sort together and the numeric option sorts file names naturally. */
#include "poly_mfn.h"

int test_main(void) {
    NSArray *words = @[@"banana", @"Apple", @"cherry", @"apple", @"10", @"9", @"Banana"];
    CK(ns_is([[words sortedArrayUsingSelector:@selector(compare:)] componentsJoinedByString:@","], "10,9,Apple,Banana,apple,banana,cherry"));
    CK(ns_is([[words sortedArrayUsingSelector:@selector(caseInsensitiveCompare:)] componentsJoinedByString:@","], "10,9,Apple,apple,banana,Banana,cherry"));
    NSArray *files = @[@"file10.txt", @"file2.txt", @"file1.txt"];
    NSArray *sorted = [files sortedArrayUsingComparator:^NSComparisonResult(NSString *a, NSString *b) {
        return [a compare:b options:NSNumericSearch];
    }];
    CK(ns_is([sorted componentsJoinedByString:@","], "file1.txt,file2.txt,file10.txt"));
    return finish();
}
