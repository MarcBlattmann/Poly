#include "rt.h"
#include <arm_neon.h>
static u8 img[256 * 256], out[256 * 256];
static float fa[1024], fb[1024];
static const i32 cvt_in[4] = {-7, 0, 9, 1 << 20};
int run(void) {
    for (int i = 0; i < 256 * 256; i++) img[i] = (u8)((i * 37) ^ (i >> 8));
    /* 3x1 box blur with widening adds and narrowing shifts */
    for (int y = 0; y < 256; y++) for (int x = 16; x < 240; x += 16) {
        const u8 *p = img + y * 256 + x;
        uint16x8_t lo = vaddl_u8(vget_low_u8(vld1q_u8(p - 1)), vget_low_u8(vld1q_u8(p + 1)));
        uint16x8_t hi = vaddl_u8(vget_high_u8(vld1q_u8(p - 1)), vget_high_u8(vld1q_u8(p + 1)));
        lo = vaddw_u8(lo, vget_low_u8(vld1q_u8(p))); hi = vaddw_u8(hi, vget_high_u8(vld1q_u8(p)));
        vst1q_u8(out + y * 256 + x, vcombine_u8(vshrn_n_u16(lo, 2), vshrn_n_u16(hi, 2)));
    }
    uint32x4_t acc = vdupq_n_u32(0);
    for (int i = 0; i < 256 * 256; i += 16) acc = vpadalq_u16(acc, vpaddlq_u8(vld1q_u8(out + i)));
    puts_("blur sum="); putu(vaddvq_u32(acc)); nl();
    for (int i = 0; i < 1024; i++) { fa[i] = (float)i * 0.5f - 100.0f; fb[i] = 1.0f / (float)(i + 1); }
    float32x4_t s = vdupq_n_f32(0), mx = vdupq_n_f32(-1e30f);
    for (int i = 0; i < 1024; i += 4) { float32x4_t a = vld1q_f32(fa + i), b = vld1q_f32(fb + i); s = vfmaq_f32(s, a, b); mx = vmaxq_f32(mx, vabsq_f32(vsubq_f32(a, b))); }
    puts_("dot="); putf(vaddvq_f32(s), 4); puts_(" max="); putf(vmaxvq_f32(mx), 2); nl();
    int16x8_t q = vdupq_n_s16(30000); q = vqaddq_s16(q, vdupq_n_s16(10000)); int8x8_t nq = vqmovn_s16(vdupq_n_s16(-300));
    puts_("sat="); puti(vgetq_lane_s16(q, 3)); putc_(' '); puti(vget_lane_s8(nq, 1)); nl();
    int32x4_t m = vmull_s16(vdup_n_s16(-1234), vdup_n_s16(321)); uint64x2_t um = vmull_u32(vdup_n_u32(0xFFFFFFFF), vdup_n_u32(3));
    puts_("mull="); puti(vgetq_lane_s32(m, 2)); putc_(' '); putu(vgetq_lane_u64(um, 1)); nl();
    float32x4_t cv = vcvtq_f32_s32(vld1q_s32(cvt_in)); int32x4_t ci = vcvtq_s32_f32(vmulq_n_f32(cv, 2.5f));
    puts_("cvt="); puti(vgetq_lane_s32(ci, 0)); putc_(' '); puti(vgetq_lane_s32(ci, 3)); nl();
    uint8x16_t tb = vqtbl1q_u8(vld1q_u8(img), vandq_u8(vld1q_u8(img + 100), vdupq_n_u8(15)));
    puts_("tbl="); putu(vaddlvq_u8(tb)); nl();
    return 0;
}
