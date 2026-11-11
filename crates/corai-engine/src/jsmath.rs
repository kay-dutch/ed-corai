//! JavaScript numeric semantics the upstream engine relies on.

/// `Math.round`: rounds to the nearest integer, halves towards +∞ (unlike `f64::round`).
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// JavaScript truthiness of a nullable number: `null`, `0` and `NaN` are falsy.
pub fn truthy(x: Option<f64>) -> bool {
    matches!(x, Some(v) if v != 0.0 && !v.is_nan())
}

/// `x || fallback` for a nullable number.
pub fn or(x: Option<f64>, fallback: f64) -> f64 {
    if truthy(x) {
        x.unwrap_or(fallback)
    } else {
        fallback
    }
}

/// Arithmetic value of a nullable number (`null` coerces to 0 in JS arithmetic).
pub fn num(x: Option<f64>) -> f64 {
    x.unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_halves_up_like_javascript() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(-2.6), -3.0);
        assert_eq!(js_round(1234.4999), 1234.0);
    }

    #[test]
    fn truthiness() {
        assert!(!truthy(None));
        assert!(!truthy(Some(0.0)));
        assert!(!truthy(Some(f64::NAN)));
        assert!(truthy(Some(-1.0)));
        assert_eq!(or(Some(0.0), 1.0), 1.0);
    }
}
