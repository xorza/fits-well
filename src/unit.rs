//! FITS unit strings (§4.3): an optional numeric multiplier, an optional SI prefix and a
//! base unit, resolved against a table of the base units one kind of quantity admits.

use std::f64::consts::PI;

/// Which SI prefixes a base unit takes — as wcslib's unit lexer admits them for the units
/// of FITS Table 4: SI units take all of them, years only the multiples, and the
/// sexagesimal angles, minutes, hours, days and centuries none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Prefixes {
    All,
    Multiples,
    None,
}

/// One base unit of a kind of quantity: its symbol, its size in the kind's reference unit,
/// and the SI prefixes it takes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BaseUnit {
    pub(crate) symbol: &'static str,
    pub(crate) scale: f64,
    pub(crate) prefixes: Prefixes,
}

const fn base(symbol: &'static str, scale: f64, prefixes: Prefixes) -> BaseUnit {
    BaseUnit {
        symbol,
        scale,
        prefixes,
    }
}

const SECONDS_PER_DAY: f64 = 86_400.0;

/// Angles, in degrees.
pub(crate) const ANGLE: &[BaseUnit] = &[
    base("deg", 1.0, Prefixes::None),
    base("arcmin", 1.0 / 60.0, Prefixes::None),
    base("arcsec", 1.0 / 3600.0, Prefixes::None),
    base("mas", 1.0 / 3_600_000.0, Prefixes::None),
    base("rad", 180.0 / PI, Prefixes::All),
    base("turn", 360.0, Prefixes::None),
];

/// Times, in seconds; the Julian year and century of 365.25 days.
pub(crate) const TIME: &[BaseUnit] = &[
    base("s", 1.0, Prefixes::All),
    base("min", 60.0, Prefixes::None),
    base("h", 3600.0, Prefixes::None),
    base("d", SECONDS_PER_DAY, Prefixes::None),
    base("a", 365.25 * SECONDS_PER_DAY, Prefixes::Multiples),
    base("yr", 365.25 * SECONDS_PER_DAY, Prefixes::Multiples),
    base("cy", 36_525.0 * SECONDS_PER_DAY, Prefixes::None),
];

/// Frequencies, in hertz.
pub(crate) const FREQUENCY: &[BaseUnit] = &[base("Hz", 1.0, Prefixes::All)];

/// Lengths, in metres.
pub(crate) const LENGTH: &[BaseUnit] = &[
    base("m", 1.0, Prefixes::All),
    base("Angstrom", 1e-10, Prefixes::None),
    base("angstrom", 1e-10, Prefixes::None),
];

/// Energies, in joules; the electron-volt is exact since the 2019 SI.
pub(crate) const ENERGY: &[BaseUnit] = &[
    base("J", 1.0, Prefixes::All),
    base("eV", 1.602_176_634e-19, Prefixes::All),
    base("erg", 1e-7, Prefixes::None),
];

/// `unit` — `[multiplier] [prefix] base` — as a multiple of the reference unit of the
/// kind `bases` tabulates, or `None` when it names no unit of that kind. A symbol that is
/// itself a base unit (`min`, `mas`, `cy`) is never read as a prefixed one.
pub(crate) fn resolve(unit: &str, bases: &[BaseUnit]) -> Option<f64> {
    let scaled = split_numeric_multiplier(unit)?;
    let symbol = scaled.base;
    if let Some(unit) = bases.iter().find(|unit| unit.symbol == symbol) {
        return Some(scaled.factor * unit.scale);
    }
    SI_PREFIXES.into_iter().find_map(|(prefix, factor)| {
        let rest = symbol.strip_prefix(prefix)?;
        let unit = bases.iter().find(|unit| unit.symbol == rest)?;
        let admitted = match unit.prefixes {
            Prefixes::All => true,
            Prefixes::Multiples => factor > 1.0,
            Prefixes::None => false,
        };
        admitted.then_some(scaled.factor * factor * unit.scale)
    })
}

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
    let exponent = &expression[exponent_start..position];
    if parenthesized {
        if bytes.get(position) != Some(&b')') {
            return None;
        }
        position += 1;
    }
    // `1e<exponent>` parses to the f64 nearest the power of ten; `powi` multiplies its way
    // there and is a few ULP off below 10⁻²².
    let factor: f64 = format!("1e{exponent}").parse().ok()?;
    (factor.is_finite() && factor > 0.0).then_some(ParsedMultiplier {
        factor,
        length: position,
    })
}

#[cfg(test)]
mod tests {
    use crate::unit::{ANGLE, ENERGY, LENGTH, TIME, resolve, split_numeric_multiplier};
    use std::f64::consts::PI;

    /// Each table, exact base units, prefixes where they are admitted, and the refusals.
    #[test]
    fn units_resolve_against_their_kind() {
        for (unit, table, scale) in [
            ("deg", ANGLE, Some(1.0)),
            ("mrad", ANGLE, Some(1e-3 * (180.0 / PI))),
            ("mas", ANGLE, Some(1.0 / 3_600_000.0)),
            ("mdeg", ANGLE, None),
            ("karcsec", ANGLE, None),
            ("10**-3 deg", ANGLE, Some(1e-3)),
            ("min", TIME, Some(60.0)),
            ("ms", TIME, Some(1e-3)),
            ("Ma", TIME, Some(1e6 * 365.25 * 86_400.0)),
            ("ma", TIME, None),
            ("kd", TIME, None),
            ("cy", TIME, Some(36_525.0 * 86_400.0)),
            ("nm", LENGTH, Some(1e-9)),
            ("Angstrom", LENGTH, Some(1e-10)),
            ("keV", ENERGY, Some(1e3 * 1.602_176_634e-19)),
            ("Hz", ANGLE, None),
            ("", ANGLE, None),
        ] {
            assert_eq!(resolve(unit, table), scale, "{unit:?}");
        }
    }

    #[test]
    fn numeric_unit_multipliers_follow_fits_syntax() {
        let cases = [
            ("Hz", 1.0, "Hz"),
            ("10**9 Hz", 1e9, "Hz"),
            ("10^(-6)m", 1e-6, "m"),
            ("10+3m/s", 1e3, "m/s"),
            ("10-2 J", 1e-2, "J"),
            ("(10**12) Hz", 1e12, "Hz"),
            // 10⁻²⁴ by repeated multiplication is 1.0000000000000001e-24.
            ("10**-24 m", 1e-24, "m"),
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
