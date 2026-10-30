//! Every function against exact values (`reference.txt`, made by
//! `gen_reference.py` with Python's `decimal` module): the error of each
//! result in ulps of the exact value, which must stay within the design
//! bound of 0.5 + 2^-20 ulp (the documented target is one ulp). Also the
//! `float` forms against the same values rounded to `float`.

use poly_libm as m;

fn ulp(x: f64) -> f64 {
    let e = m::dd::exponent_of(x).max(-1022);
    m::dd::ldexp_exact(1.0, e - 52)
}

fn call(name: &str, a: &[f64]) -> f64 {
    match (name, a) {
        ("sin", [x]) => m::sin(*x),
        ("cos", [x]) => m::cos(*x),
        ("tan", [x]) => m::tan(*x),
        ("asin", [x]) => m::asin(*x),
        ("acos", [x]) => m::acos(*x),
        ("atan", [x]) => m::atan(*x),
        ("atan2", [y, x]) => m::atan2(*y, *x),
        ("exp", [x]) => m::exp(*x),
        ("exp2", [x]) => m::exp2(*x),
        ("expm1", [x]) => m::expm1(*x),
        ("log", [x]) => m::log(*x),
        ("log2", [x]) => m::log2(*x),
        ("log10", [x]) => m::log10(*x),
        ("log1p", [x]) => m::log1p(*x),
        ("sinh", [x]) => m::sinh(*x),
        ("cosh", [x]) => m::cosh(*x),
        ("tanh", [x]) => m::tanh(*x),
        ("cbrt", [x]) => m::cbrt(*x),
        ("hypot", [x, y]) => m::hypot(*x, *y),
        ("pow", [x, y]) => m::pow(*x, *y),
        _ => panic!("unknown reference function {name}"),
    }
}

fn call_single(name: &str, a: &[f32]) -> Option<f32> {
    use m::single as s;
    Some(match (name, a) {
        ("sin", [x]) => s::sinf(*x),
        ("cos", [x]) => s::cosf(*x),
        ("tan", [x]) => s::tanf(*x),
        ("asin", [x]) => s::asinf(*x),
        ("acos", [x]) => s::acosf(*x),
        ("atan", [x]) => s::atanf(*x),
        ("exp", [x]) => s::expf(*x),
        ("log", [x]) => s::logf(*x),
        ("log10", [x]) => s::log10f(*x),
        ("sinh", [x]) => s::sinhf(*x),
        ("cosh", [x]) => s::coshf(*x),
        ("tanh", [x]) => s::tanhf(*x),
        _ => return None,
    })
}

#[test]
fn every_function_is_within_its_bound_of_the_exact_value() {
    let text = include_str!("reference.txt");
    let mut worst: std::collections::BTreeMap<&str, (f64, usize, usize)> = Default::default();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let parts: Vec<&str> = line.split(' ').collect();
        let name = parts[0];
        let nums: Vec<f64> =
            parts[1..].iter().map(|h| f64::from_bits(u64::from_str_radix(h, 16).unwrap())).collect();
        let (args, hi, lo) = (&nums[..nums.len() - 2], nums[nums.len() - 2], nums[nums.len() - 1]);
        let got = call(name, args);
        // (got − hi) is exact when they are close; the error is that minus lo.
        let err = ((got - hi) - lo).abs() / ulp(hi);
        let e = worst.entry(name).or_insert((0.0, 0, 0));
        e.0 = e.0.max(err);
        e.1 += 1;
        if got == hi {
            e.2 += 1;
        }
        assert!(err <= 0.5 + 1e-6, "{name}{args:?} = {got:e}, exact {hi:e} + {lo:e}: {err} ulp");
        // The float form, where its argument is exactly the double one.
        let single: Vec<f32> = args.iter().map(|&a| a as f32).collect();
        if args.iter().zip(&single).all(|(&a, &s)| f64::from(s) == a)
            && let Some(g) = call_single(name, &single)
        {
            let exact = hi + lo;
            let ulp32 = f64::from(f32::EPSILON) * f64::from(2f32.powi(m::dd::exponent_of(exact).clamp(-126, 127)));
            if exact.abs() < f64::from(f32::MAX) && exact.abs() > f64::from(f32::MIN_POSITIVE) {
                let err32 = (f64::from(g) - exact).abs() / ulp32;
                assert!(err32 <= 0.5 + 1e-6, "{name}f{single:?} = {g:e}, exact {exact:e}: {err32} ulp");
            }
        }
    }
    for (name, (w, n, exact)) in &worst {
        std::println!("{name:6} {n:4} values, {exact:4} correctly rounded, worst {w:.6} ulp");
    }
}
