//! JavaScript number semantics for figures reported to clients, so the Rust and Node servers return
//! identical JSON.

use serde_json::Value;

/// `Number(x.toFixed(digits))`: rounds half away from zero on the exact binary value, as
/// `toFixed` does (Rust's formatter rounds exact ties to even).
pub fn to_fixed(x: f64, digits: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let exact = format!("{:.*}", digits + 30, x.abs());
    let (int, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let (keep, rest) = frac.split_at(digits.min(frac.len()));
    let mut kept: Vec<u8> = int.bytes().chain(keep.bytes()).map(|b| b - b'0').collect();
    if rest.as_bytes().first().is_some_and(|d| *d >= b'5') {
        // Round up at the last kept digit, carrying.
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, 1);
                break;
            }
            i -= 1;
            if kept[i] == 9 {
                kept[i] = 0;
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    let int_len = kept.len() - keep.len();
    let digits_str: String = kept.iter().map(|d| char::from(b'0' + d)).collect();
    let text = format!("{}.{}", &digits_str[..int_len], &digits_str[int_len..]);
    let v: f64 = text.trim_end_matches('.').parse().unwrap_or(f64::NAN);
    if x < 0.0 { -v } else { v }
}

/// `round()` from the Node metrics: whole numbers from 1000, 2 decimals from 10, else 3.
pub fn round_metric(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let a = x.abs();
    to_fixed(
        x,
        if a >= 1000.0 {
            0
        } else if a >= 10.0 {
            2
        } else {
            3
        },
    )
}

/// A number as `JSON.stringify` writes it: whole numbers without a fraction, NaN/∞ as null.
pub fn json_num(x: f64) -> Value {
    if !x.is_finite() {
        Value::Null
    } else if x.fract() == 0.0 && x.abs() < 9.007e15 {
        Value::from(x as i64)
    } else {
        Value::from(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_fixed_matches_javascript() {
        assert_eq!(to_fixed(1.005, 2), 1.0); // 1.005 is 1.00499999... in binary
        assert_eq!(to_fixed(2.5, 0), 3.0);
        assert_eq!(to_fixed(0.125, 2), 0.13);
        assert_eq!(to_fixed(-0.125, 2), -0.13);
        assert_eq!(to_fixed(9.999, 2), 10.0);
        assert_eq!(to_fixed(1234.5, 0), 1235.0);
    }
}
