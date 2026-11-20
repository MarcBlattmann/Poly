/* CGPathAddEllipseInRect: the elements of the path (four curves and a
 * close) with the exact bits of every point, for a circle and an ellipse,
 * and the bounding box; the starting point and direction. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static int idx;
static void each(void *info, const CGPathElement *e) {
    (void)info;
    static const char *names[] = {"move", "line", "quad", "curve", "close"};
    int pts = e->type == kCGPathElementMoveToPoint || e->type == kCGPathElementAddLineToPoint ? 1 : e->type == kCGPathElementAddQuadCurveToPoint ? 2 : e->type == kCGPathElementAddCurveToPoint ? 3 : 0;
    char label[64] = "element ";
    label[8] = (char)('0' + idx);
    label[9] = ' ';
    const char *n = names[e->type];
    int k = 10;
    while (*n) label[k++] = *n++;
    label[k] = 0;
    value_str(label, "");
    for (int i = 0; i < pts; i++) {
        value_dbl("   x", e->points[i].x);
        value_dbl("   y", e->points[i].y);
    }
    idx++;
}

int test_main(void) {
    CGMutablePathRef p = CGPathCreateMutable();
    CGPathAddEllipseInRect(p, NULL, CGRectMake(0, 0, 10, 10));
    idx = 0;
    value_str("circle (0,0,10,10)", "");
    CGPathApply(p, NULL, each);
    CGMutablePathRef q = CGPathCreateMutable();
    CGPathAddEllipseInRect(q, NULL, CGRectMake(1, 2, 8, 4));
    idx = 0;
    value_str("ellipse (1,2,8,4)", "");
    CGPathApply(q, NULL, each);
    CGRect bb = CGPathGetBoundingBox(q);
    value_dbl("bounding box x", bb.origin.x);
    value_dbl("bounding box y", bb.origin.y);
    value_dbl("bounding box w", bb.size.width);
    value_dbl("bounding box h", bb.size.height);
    CGRect pb = CGPathGetPathBoundingBox(q);
    value_dbl("path bounding box x", pb.origin.x);
    value_dbl("path bounding box w", pb.size.width);
    check("the path contains the centre", CGPathContainsPoint(q, NULL, CGPointMake(5, 4), false), 0);
    check("not the corner of the rect", !CGPathContainsPoint(q, NULL, CGPointMake(1.2, 2.2), false), 0);
    return finish();
}
