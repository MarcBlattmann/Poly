#!/usr/bin/env python3
"""Generate reference.txt: exact values of the poly-libm functions.

Each line is `function x [y] hi lo` (hexadecimal bit patterns of f64): the
exact value of the function at the given doubles, computed with Python's
`decimal` module to 60+ significant digits, split into the nearest double
`hi` and the nearest double `lo` to the rest. The tests measure each result's
error in ulps against `hi + lo`.

The values are computed independently of the Rust code: `decimal`'s own
`exp`, `ln`, `log10`, `sqrt` and `**` (correctly rounded at the working
precision), and for the circular functions Taylor series after reducing the
argument modulo 2π with π to 2500 bits (Machin's formula in integers).

Run from the repository root: python3 crates/poly-libm/tests/gen_reference.py
"""
from decimal import Decimal as D, getcontext, localcontext
import random
import struct

getcontext().prec = 70


def bits(f):
    return struct.unpack('<Q', struct.pack('<d', f))[0]


def from_bits(b):
    return struct.unpack('<d', struct.pack('<Q', b))[0]


def pi_int(nbits):
    guard = 64
    one = 1 << (nbits + guard)

    def arctan_inv(x):
        total = term = one // x
        x2 = x * x
        n = 1
        sign = -1
        while term:
            term //= x2
            total += sign * (term // (2 * n + 1))
            sign = -sign
            n += 1
        return total
    return (4 * (4 * arctan_inv(5) - arctan_inv(239))) >> guard


PI_BITS = 2500
PI_BIG = D(pi_int(PI_BITS))
with localcontext() as c:
    c.prec = 800
    PI = PI_BIG / (D(2) ** PI_BITS)
    TWO_PI = 2 * PI


def reduce_2pi(x):
    """x mod 2π in [-π, π] with enough digits for any double."""
    with localcontext() as c:
        c.prec = 800
        x = D(x)
        k = (x / TWO_PI).to_integral_value()
        r = x - k * TWO_PI
        return +r


def sin_d(x):
    r = reduce_2pi(x)
    with localcontext() as c:
        c.prec = 90
        r = +r
        term = r
        total = r
        r2 = r * r
        n = 1
        while abs(term) > D(10) ** -100:
            term = -term * r2 / ((2 * n) * (2 * n + 1))
            total += term
            n += 1
        return +total


def cos_d(x):
    r = reduce_2pi(x)
    with localcontext() as c:
        c.prec = 90
        r = +r
        term = D(1)
        total = D(1)
        r2 = r * r
        n = 1
        while abs(term) > D(10) ** -100:
            term = -term * r2 / ((2 * n - 1) * (2 * n))
            total += term
            n += 1
        return +total


def atan_d(x):
    with localcontext() as c:
        c.prec = 90
        x = D(x)
        neg = x < 0
        x = abs(x)
        inv = x > 1
        if inv:
            x = 1 / x
        # halve the angle until small: atan(x) = 2 atan(x / (1 + sqrt(1 + x²)))
        halvings = 0
        while x > D('0.01'):
            x = x / (1 + (1 + x * x).sqrt())
            halvings += 1
        term = x
        total = x
        x2 = x * x
        n = 1
        while abs(term) > D(10) ** -100:
            term = -term * x2
            total += term / (2 * n + 1)
            n += 1
        total *= 2 ** halvings
        if inv:
            total = PI / 2 - total
        return -total if neg else total


def atan2_d(y, x):
    with localcontext() as c:
        c.prec = 90
        y, x = D(y), D(x)
        if x > 0:
            return atan_d(y / x) if y != 0 else y
        a = atan_d(abs(y) / abs(x))
        r = PI - a
        return r if y > 0 else -r


def asin_d(x):
    with localcontext() as c:
        c.prec = 90
        x = D(x)
        if abs(x) == 1:
            return PI / 2 * x
        return atan_d(x / (1 - x * x).sqrt())


def acos_d(x):
    with localcontext() as c:
        c.prec = 90
        return PI / 2 - asin_d(x)


def exp_d(x):
    with localcontext() as c:
        c.prec = 90
        return D(x).exp()


def expm1_d(x):
    with localcontext() as c:
        c.prec = 400
        return D(x).exp() - 1


def ln_d(x):
    with localcontext() as c:
        c.prec = 90
        return D(x).ln()


def log1p_d(x):
    with localcontext() as c:
        c.prec = 400
        return (1 + D(x)).ln()


def sinh_d(x):
    with localcontext() as c:
        c.prec = 400
        e = D(x).exp()
        return (e - 1 / e) / 2


def cosh_d(x):
    with localcontext() as c:
        c.prec = 90
        e = D(x).exp()
        return (e + 1 / e) / 2


def tanh_d(x):
    with localcontext() as c:
        c.prec = 400
        e = (2 * D(x)).exp()
        return (e - 1) / (e + 1)


def pow_d(x, y):
    with localcontext() as c:
        c.prec = 90
        x, y = D(x), D(y)
        if x < 0:
            r = (-x) ** y
            return -r if int(y) % 2 else r
        return x ** y


def cbrt_d(x):
    with localcontext() as c:
        c.prec = 90
        x = D(x)
        r = abs(x) ** (D(1) / 3)
        # polish: the third root to 90 digits
        for _ in range(3):
            r = r - (r ** 3 - abs(x)) / (3 * r * r)
        return -r if x < 0 else r


def split(v):
    with localcontext() as c:
        c.prec = 90
        hi = float(v)
        lo = float(v - D(hi)) if hi not in (float('inf'), float('-inf')) else 0.0
        return hi, lo


def log_uniform(rng, lo_exp, hi_exp, neg=False):
    m = rng.random() + 1.0
    e = rng.uniform(lo_exp, hi_exp)
    v = m * 2.0 ** int(e)
    if neg and rng.random() < 0.5:
        v = -v
    return v


def main():
    rng = random.Random(84)
    out = []

    def emit(name, args, value):
        hi, lo = split(value)
        if hi in (float('inf'), float('-inf')) or hi == 0.0 and value != 0:
            return
        if abs(hi) < 2.2250738585072014e-308 and hi != 0.0:
            return  # subnormal results are tested exactly elsewhere
        out.append(' '.join([name] + [f'{bits(a):016x}' for a in args] + [f'{bits(hi):016x}', f'{bits(lo):016x}']))

    def uni(a, b):
        return rng.uniform(a, b)

    for _ in range(150):
        x = uni(-30, 30) if rng.random() < 0.5 else log_uniform(rng, -30, 1023, True)
        emit('sin', [x], sin_d(x))
        emit('cos', [x], cos_d(x))
        s, c = sin_d(x), cos_d(x)
        with localcontext() as ctx:
            ctx.prec = 90
            emit('tan', [x], s / c)
    for x in [0.5, 1.0, 2.0, 3.0, 10.0, 100.0, 1e10, 1e22, 1e100, 1e300, 1.7976931348623157e308,
              6381956970095103.0 * 2.0 ** 797, 8.98846567431158e307, 0.7853981633974483,
              1.5707963267948966, 3.141592653589793, 4.71238898038469]:
        emit('sin', [x], sin_d(x))
        emit('cos', [x], cos_d(x))
        with localcontext() as ctx:
            ctx.prec = 90
            emit('tan', [x], sin_d(x) / cos_d(x))
    for _ in range(150):
        x = log_uniform(rng, -40, 60, True) if rng.random() < 0.6 else uni(-3, 3)
        emit('atan', [x], atan_d(x))
        y = log_uniform(rng, -40, 40, True)
        x2 = log_uniform(rng, -40, 40, True)
        emit('atan2', [y, x2], atan2_d(y, x2))
        a = uni(-1, 1) if rng.random() < 0.6 else (1 - 2.0 ** -rng.randint(1, 52)) * rng.choice([-1, 1])
        emit('asin', [a], asin_d(a))
        emit('acos', [a], acos_d(a))
    for _ in range(200):
        x = uni(-745, 709) if rng.random() < 0.5 else uni(-2, 2)
        emit('exp', [x], exp_d(x))
        x = uni(-1074, 1023) if rng.random() < 0.5 else uni(-2, 2)
        with localcontext() as ctx:
            ctx.prec = 90
            emit('exp2', [x], D(2) ** D(x))
        x = log_uniform(rng, -60, 9, True)
        emit('expm1', [x], expm1_d(x))
        x = log_uniform(rng, -1074, 1023) if rng.random() < 0.5 else uni(0.5, 2)
        emit('log', [x], ln_d(x))
        with localcontext() as ctx:
            ctx.prec = 90
            emit('log2', [x], ln_d(x) / D(2).ln())
            emit('log10', [x], D(x).log10())
        x = log_uniform(rng, -60, 1023) * (1 if rng.random() < 0.7 else -1)
        if x > -1:
            emit('log1p', [x], log1p_d(x))
        x = uni(-710, 710) if rng.random() < 0.5 else log_uniform(rng, -40, 3, True)
        emit('sinh', [x], sinh_d(x))
        emit('cosh', [x], cosh_d(x))
        x = log_uniform(rng, -40, 5, True)
        emit('tanh', [x], tanh_d(x))
        x = log_uniform(rng, -1000, 1000, True)
        emit('cbrt', [x], cbrt_d(x))
        a, b = log_uniform(rng, -500, 500, True), log_uniform(rng, -500, 500, True)
        with localcontext() as ctx:
            ctx.prec = 90
            emit('hypot', [a, b], (D(a) ** 2 + D(b) ** 2).sqrt())
    for _ in range(400):
        x = log_uniform(rng, -20, 20)
        y = uni(-40, 40)
        if rng.random() < 0.3:
            y = float(rng.randint(-60, 60))
            x = -x if rng.random() < 0.5 else x
        emit('pow', [x, y], pow_d(x, y))
        x = uni(0.999, 1.001)
        y = uni(-1e5, 1e5)
        emit('pow', [x, y], pow_d(x, y))
    with open('crates/poly-libm/tests/reference.txt', 'w', newline='\n') as f:
        f.write('\n'.join(out) + '\n')
    print(len(out), 'lines')


if __name__ == '__main__':
    main()
