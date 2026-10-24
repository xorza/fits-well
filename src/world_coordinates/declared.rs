//! [`Declared`]: a WCS description as its header declares it, to write back.

use crate::alternate::Alternate;
use crate::error::FitsError;
use crate::error::Result;
use crate::error::WcsConflict;
use crate::header_model::Header;
use crate::header_model::record::RecordView;
use crate::header_model::value::Value;
use crate::keyword::Keyword;
use crate::world_coordinates::axis::Axis;
use crate::world_coordinates::distortion::sip::Sip;
use crate::world_coordinates::linear_transform::DeclaredMatrix;

/// The per-description keywords a WCS carries besides its axes and matrix, written back
/// as they were read: the pole, the frames, and the spectral references (§8.2, §8.4).
const CARRIED: [&str; 14] = [
    "WCSNAME", "LONPOLE", "LATPOLE", "RADESYS", "EQUINOX", "EPOCH", "RESTFRQ", "RESTWAV",
    "SPECSYS", "SSYSOBS", "SSYSSRC", "VELOSYS", "ZSOURCE", "VELANGL",
];

/// The primary-only legacy spellings the reader takes for a carried keyword, as
/// (legacy, standard) pairs: carried under the standard name, which takes an alternate
/// letter, as wcslib writes them (`wcshdo`).
const LEGACY: [(&str, &str); 2] = [("RADECSYS", "RADESYS"), ("RESTFREQ", "RESTFRQ")];

/// The per-axis families, `{root}i`, a WCS carries besides the axes' own keywords.
const CARRIED_AXIS: [&str; 3] = ["CNAME", "CRDER", "CSYER"];

/// The per-axis families a description is made of, removed before a write.
const AXIS_FAMILIES: [&str; 9] = [
    "CTYPE", "CUNIT", "CRVAL", "CRPIX", "CDELT", "CROTA", "CNAME", "CRDER", "CSYER",
];

/// The `{root}i_m` families a description is made of, removed before a write.
const PAIR_FAMILIES: [&str; 4] = ["PC", "CD", "PV", "PS"];

/// A description as the header declared it: the matrix in wcslib's form, and every other
/// record of it, its alternate letter dropped, in the order the write gives them.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Declared {
    pub(super) matrix: DeclaredMatrix,
    carried: Vec<(String, Value)>,
}

impl Declared {
    /// The records of the description `alt` of `header`, of `naxis` axes, beside the
    /// declared `matrix`.
    pub(super) fn read(
        header: &Header,
        alt: Alternate,
        naxis: usize,
        matrix: DeclaredMatrix,
    ) -> Result<Self> {
        let mut carried = Vec::new();
        for name in CARRIED {
            if let Some(keyword) = Keyword::fitting(format_args!("{name}{alt}"))
                && let Some(value) = header.get(keyword)?
            {
                carried.push((name.to_string(), value.clone()));
            }
        }
        if alt == Alternate::PRIMARY {
            for (legacy, standard) in LEGACY {
                if !carried.iter().any(|(name, _)| name == standard)
                    && let Some(value) = header.get(legacy)?
                {
                    carried.push((standard.to_string(), value.clone()));
                }
            }
        }
        for root in CARRIED_AXIS {
            for axis in 1..=naxis {
                if let Some(keyword) = Keyword::fitting(format_args!("{root}{axis}{alt}"))
                    && let Some(value) = header.get(keyword)?
                {
                    carried.push((format!("{root}{axis}"), value.clone()));
                }
            }
        }
        for root in ["PV", "PS"] {
            for record in header {
                let RecordView::Valued { keyword, value, .. } = record else {
                    continue;
                };
                if let Some(stem) = stem(keyword.as_str(), alt)
                    && let Some([axis, _]) = pair_index(stem, root)
                    && (1..=naxis).contains(&axis)
                {
                    carried.push((stem.to_string(), value.clone()));
                }
            }
        }
        Ok(Self { matrix, carried })
    }

    /// Replace the description `alt` in `header` with this one of `axes` and `sip`, in
    /// wcslib's order (`wcshdo`): `WCSAXES`, `CRPIX`, `PC` (an identity term omitted),
    /// `CDELT`, `CUNIT`, `CTYPE`, `CRVAL`, the carried records, then SIP.
    pub(super) fn write(
        &self,
        header: &mut Header,
        alt: Alternate,
        axes: &[Axis],
        sip: Option<&Sip>,
    ) -> Result<()> {
        if sip.is_some() && alt != Alternate::PRIMARY {
            return Err(FitsError::ConflictingWcsKeywords {
                conflict: WcsConflict::SipAlternate,
            });
        }
        let primary = alt == Alternate::PRIMARY;
        header.remove_where(|keyword| {
            let name = keyword.as_str();
            is_description_keyword(name, alt) || primary && is_sip_keyword(name)
        });
        let naxis = axes.len();
        let keyword =
            |name: std::fmt::Arguments<'_>| -> Result<Keyword> { Keyword::try_from(name) };
        header.set(
            keyword(format_args!("WCSAXES{alt}"))?,
            i64::try_from(naxis).expect("an axis count fits i64"),
        )?;
        for (i, axis) in axes.iter().enumerate() {
            header.set(keyword(format_args!("CRPIX{}{alt}", i + 1))?, axis.crpix)?;
        }
        for row in 0..naxis {
            for column in 0..naxis {
                let value = self.matrix.pc[row * naxis + column];
                if value != if row == column { 1.0 } else { 0.0 } {
                    header.set(
                        keyword(format_args!("PC{}_{}{alt}", row + 1, column + 1))?,
                        value,
                    )?;
                }
            }
        }
        for (i, &cdelt) in self.matrix.cdelt.iter().enumerate() {
            header.set(keyword(format_args!("CDELT{}{alt}", i + 1))?, cdelt)?;
        }
        for (i, axis) in axes.iter().enumerate() {
            if !axis.cunit.is_empty() {
                header.set(
                    keyword(format_args!("CUNIT{}{alt}", i + 1))?,
                    axis.cunit.as_str(),
                )?;
            }
        }
        for (i, axis) in axes.iter().enumerate() {
            header.set(
                keyword(format_args!("CTYPE{}{alt}", i + 1))?,
                axis.ctype.as_str(),
            )?;
        }
        for (i, axis) in axes.iter().enumerate() {
            header.set(keyword(format_args!("CRVAL{}{alt}", i + 1))?, axis.crval)?;
        }
        for (name, value) in &self.carried {
            header.set(keyword(format_args!("{name}{alt}"))?, value.clone())?;
        }
        if let Some(sip) = sip {
            sip.write(header)?;
        }
        Ok(())
    }
}

/// `name` without the alternate letter of `alt`: as it is for the primary description,
/// `None` when it does not end in that letter.
fn stem(name: &str, alt: Alternate) -> Option<&str> {
    if alt == Alternate::PRIMARY {
        Some(name)
    } else {
        name.strip_suffix(alt.as_str())
    }
}

/// A keyword index as a name spells it: decimal digits with no leading zero.
fn index(digits: &str) -> Option<usize> {
    let canonical = !digits.is_empty()
        && !digits.starts_with('0')
        && digits.bytes().all(|byte| byte.is_ascii_digit());
    digits.parse().ok().filter(|_| canonical)
}

/// The indices of a `{root}i_m` name (`PC1_2`, `PV2_0`); `m` may be 0 for `PV`/`PS`.
fn pair_index(stem: &str, root: &str) -> Option<[usize; 2]> {
    let (i, m) = stem.strip_prefix(root)?.split_once('_')?;
    let m = if m == "0" { Some(0) } else { index(m) };
    Some([index(i)?, m?])
}

/// Whether `name` is a record of the description `alt`.
fn is_description_keyword(name: &str, alt: Alternate) -> bool {
    let Some(stem) = stem(name, alt) else {
        return false;
    };
    CARRIED.contains(&stem)
        || alt == Alternate::PRIMARY && LEGACY.iter().any(|&(legacy, _)| legacy == stem)
        || stem == "WCSAXES"
        || AXIS_FAMILIES
            .iter()
            .any(|root| stem.strip_prefix(root).and_then(index).is_some())
        || PAIR_FAMILIES
            .iter()
            .any(|root| pair_index(stem, root).is_some())
}

/// Whether `name` is a SIP keyword, which only the primary description has.
fn is_sip_keyword(name: &str) -> bool {
    ["A", "B", "AP", "BP"].iter().any(|root| {
        name.strip_prefix(root).and_then(|rest| rest.strip_prefix('_')).is_some_and(|rest| {
            rest == "ORDER" || rest == "DMAX" && root.len() == 1 || {
                let mut digits = rest.split('_');
                matches!((digits.next(), digits.next(), digits.next()), (Some(p), Some(q), None)
                    if p.len() == 1 && q.len() == 1 && p.bytes().chain(q.bytes()).all(|byte| byte.is_ascii_digit()))
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use crate::alternate::Alternate;
    use crate::world_coordinates::declared::{is_description_keyword, is_sip_keyword};

    #[test]
    fn description_and_sip_keywords_are_told_apart() {
        let primary = Alternate::PRIMARY;
        let a = Alternate::new('A');
        for name in [
            "CTYPE1", "PC1_2", "CD2_1", "PV2_0", "PS1_3", "WCSAXES", "RADESYS", "LONPOLE", "CNAME3",
        ] {
            assert!(is_description_keyword(name, primary), "{name}");
            assert!(!is_description_keyword(name, a), "{name}");
            assert!(is_description_keyword(&format!("{name}A"), a), "{name}A");
        }
        for name in [
            "CTYPE1A", "CTYPE01", "NAXIS1", "OBJECT", "PC1", "PV1_01", "DATE-OBS",
        ] {
            assert!(!is_description_keyword(name, primary), "{name}");
        }
        assert!(is_description_keyword("RADECSYS", primary));
        assert!(!is_description_keyword("RADECSYS", Alternate::new('S')));
        for name in [
            "A_ORDER", "B_ORDER", "AP_ORDER", "A_2_0", "BP_0_1", "A_DMAX",
        ] {
            assert!(is_sip_keyword(name), "{name}");
        }
        for name in ["AP_DMAX", "A_10_0", "A_X", "AB_ORDER", "A_1_2_3"] {
            assert!(!is_sip_keyword(name), "{name}");
        }
    }
}
