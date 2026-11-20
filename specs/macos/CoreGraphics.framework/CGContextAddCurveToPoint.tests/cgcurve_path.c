/* A quadratic curve in a path is kept as a quadratic element; CGPathApply
 * reports the elements and the points as given, and a current point
 * that follows. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static int idx;
static void each(void *info, const CGPathElement *e) {
    (void)info;
    static const char *names[] = {"move", "line", "quad", "curve", "close"};
    int pts = e->type == kCGPathElementMoveToPoint || e->type == kCGPathElementAddLineToPoint ? 1 : e->type == kCGPathElementAddQuadCurveToPoint ? 2 : e->type == kCGPathElementAddCurveToPoint ? 3 : 0;
    char label[40] = "element ";
    label[8] = (char)('0' + idx);
    label[9] = ' ';
    const char *n = names[e->type];
    int k = 10;
    while (*n) label[k++] = *n++;
    label[k] = 0;
    char coords[200];
    size_t m = 0;
    for (int i = 0; i < pts; i++) {
        coords[m++] = (char)('0' + (int)e->points[i].x / 10);
        coords[m++] = (char)('0' + (int)e->points[i].x % 10);
        coords[m++] = ',';
        coords[m++] = (char)('0' + (int)e->points[i].y / 10);
        coords[m++] = (char)('0' + (int)e->points[i].y % 10);
        coords[m++] = ' ';
    }
    coords[m] = 0;
    value_str(label, coords);
    idx++;
}

int test_main(void) {
    CGMutablePathRef p = CGPathCreateMutable();
    check("a new path is empty", CGPathIsEmpty(p), 0);
    CGPathMoveToPoint(p, NULL, 1, 2);
    CGPathAddLineToPoint(p, NULL, 3, 4);
    CGPathAddQuadCurveToPoint(p, NULL, 5, 6, 7, 8);
    CGPathAddCurveToPoint(p, NULL, 9, 10, 11, 12, 13, 14);
    CGPathCloseSubpath(p);
    check("a path with elements is not empty", !CGPathIsEmpty(p), 0);
    idx = 0;
    CGPathApply(p, NULL, each);
    check("five elements", idx == 5, idx);
    CGPoint cur = CGPathGetCurrentPoint(p);
    value_dbl("current point x after closing: back at the subpath start", cur.x);
    value_dbl("current point y", cur.y);
    return finish();
}
