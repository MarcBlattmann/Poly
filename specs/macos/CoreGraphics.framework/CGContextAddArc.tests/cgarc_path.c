/* The path an arc makes: current point, bounding box, and the elements
 * (as CGPathApply reports them) for several arcs. The control points are
 * printed as the exact bits of the doubles. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static int idx;
static void each(void *info, const CGPathElement *e) {
    (void)info;
    static const char *names[] = {"move", "line", "quad", "curve", "close"};
    int pts = e->type == kCGPathElementMoveToPoint || e->type == kCGPathElementAddLineToPoint ? 1 : e->type == kCGPathElementAddQuadCurveToPoint ? 2 : e->type == kCGPathElementAddCurveToPoint ? 3 : 0;
    char label[64] = "element ";
    label[8] = (char)('0' + idx / 10);
    label[9] = (char)('0' + idx % 10);
    label[10] = ' ';
    const char *n = names[e->type];
    int k = 11;
    while (*n) label[k++] = *n++;
    label[k] = 0;
    value_str(label, "");
    for (int i = 0; i < pts; i++) {
        value_dbl("  x", e->points[i].x);
        value_dbl("  y", e->points[i].y);
    }
    idx++;
}

int test_main(void) {
    const double pi = 3.14159265358979323846;
    CGMutablePathRef p = CGPathCreateMutable();
    CGPathAddArc(p, NULL, 10, 10, 5, 0, pi / 2, false);
    idx = 0;
    value_str("arc 0 .. pi/2 counterclockwise", "");
    CGPathApply(p, NULL, each);
    CGPoint cur = CGPathGetCurrentPoint(p);
    value_dbl("current point x", cur.x);
    value_dbl("current point y", cur.y);
    CGRect bb = CGPathGetBoundingBox(p);
    value_dbl("bounding box x", bb.origin.x);
    value_dbl("bounding box y", bb.origin.y);
    value_dbl("bounding box w", bb.size.width);
    value_dbl("bounding box h", bb.size.height);
    CGMutablePathRef q = CGPathCreateMutable();
    CGPathAddArc(q, NULL, 0, 0, 1, 0, pi, true);
    idx = 0;
    value_str("arc 0 .. pi clockwise (flag 1), radius 1 at the origin", "");
    CGPathApply(q, NULL, each);
    return finish();
}
