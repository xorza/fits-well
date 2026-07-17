#[derive(Debug, Clone, Copy)]
pub(crate) struct ScaledUnit<'a> {
    pub(crate) factor: f64,
    pub(crate) base: &'a str,
}

pub(crate) const SI_PREFIXES: [(&str, f64); 20] = [
    ("da", 1e1),
    ("d", 1e-1),
    ("c", 1e-2),
    ("m", 1e-3),
    ("u", 1e-6),
    ("n", 1e-9),
    ("p", 1e-12),
    ("f", 1e-15),
    ("a", 1e-18),
    ("z", 1e-21),
    ("y", 1e-24),
    ("h", 1e2),
    ("k", 1e3),
    ("M", 1e6),
    ("G", 1e9),
    ("T", 1e12),
    ("P", 1e15),
    ("E", 1e18),
    ("Z", 1e21),
    ("Y", 1e24),
];

pub(crate) fn si_prefix(prefix: &str) -> Option<f64> {
    SI_PREFIXES
        .into_iter()
        .find_map(|(candidate, factor)| (prefix == candidate).then_some(factor))
}

pub(crate) fn split_numeric_multiplier(unit: &str) -> Option<ScaledUnit<'_>> {
    let unit = unit.trim();
    if let Some(wrapped) = unit.strip_prefix('(') {
        let close = wrapped.find(')')?;
        let expression = &wrapped[..close];
        let factor = parse_multiplier_expression(expression)?;
        let base = wrapped[close + 1..].trim();
        return (!base.is_empty()).then_some(ScaledUnit { factor, base });
    }
    if !unit.starts_with("10") {
        return Some(ScaledUnit {
            factor: 1.0,
            base: unit,
        });
    }
    let parsed = parse_multiplier_prefix(unit)?;
    let base = unit[parsed.length..].trim();
    (!base.is_empty()).then_some(ScaledUnit {
        factor: parsed.factor,
        base,
    })
}

#[derive(Debug, Clone, Copy)]
struct ParsedMultiplier {
    factor: f64,
    length: usize,
}

fn parse_multiplier_expression(expression: &str) -> Option<f64> {
    let parsed = parse_multiplier_prefix(expression.trim())?;
    (parsed.length == expression.trim().len()).then_some(parsed.factor)
}

fn parse_multiplier_prefix(expression: &str) -> Option<ParsedMultiplier> {
    let bytes = expression.as_bytes();
    if !bytes.starts_with(b"10") {
        return None;
    }
    let mut position = 2;
    if bytes.get(position..position + 2) == Some(b"**") {
        position += 2;
    } else if bytes.get(position) == Some(&b'^') {
        position += 1;
    } else if !matches!(bytes.get(position), Some(b'+' | b'-')) {
        return None;
    }
    while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
        position += 1;
    }
    let parenthesized = bytes.get(position) == Some(&b'(');
    if parenthesized {
        position += 1;
    }
    let exponent_start = position;
    if matches!(bytes.get(position), Some(b'+' | b'-')) {
        position += 1;
    }
    let digit_start = position;
    while bytes.get(position).is_some_and(u8::is_ascii_digit) {
        position += 1;
    }
    if position == digit_start {
        return None;
    }
    let exponent = expression[exponent_start..position].parse::<i32>().ok()?;
    if parenthesized {
        if bytes.get(position) != Some(&b')') {
            return None;
        }
        position += 1;
    }
    let factor = 10.0_f64.powi(exponent);
    (factor.is_finite() && factor > 0.0).then_some(ParsedMultiplier {
        factor,
        length: position,
    })
}

#[cfg(test)]
mod tests {
    use crate::unit::split_numeric_multiplier;

    #[test]
    fn numeric_unit_multipliers_follow_fits_syntax() {
        let cases = [
            ("Hz", 1.0, "Hz"),
            ("10**9 Hz", 1e9, "Hz"),
            ("10^(-6)m", 1e-6, "m"),
            ("10+3m/s", 1e3, "m/s"),
            ("10-2 J", 1e-2, "J"),
            ("(10**12) Hz", 1e12, "Hz"),
        ];
        for (source, factor, base) in cases {
            let parsed = split_numeric_multiplier(source).unwrap();
            assert_eq!(parsed.factor, factor, "{source}");
            assert_eq!(parsed.base, base, "{source}");
        }
        for invalid in ["10Hz", "10** Hz", "10**999 Hz", "(10**9 Hz", "10**9"] {
            assert_eq!(
                split_numeric_multiplier(invalid).map(|unit| unit.base),
                None
            );
        }
    }
}
