//! [`PhaseAxis`]: the §9.6 `'PHASE'` axis folding parameters.

use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::Header;
use crate::keyword::AltSuffix;
use crate::keyword::KeyBuf;
use crate::keyword::key;
use crate::time_coordinates::time_axis_kind::TimeAxisKind;

/// The §9.6 `'PHASE'` axis folding parameters: the zero-phase reference time
/// `CZPHSia` and the period `CPERIia`, in `TIMEUNIT` relative to the time
/// reference (the `TSTART` convention).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaseAxis {
    pub zero_phase: f64,
    /// A constant non-zero `CPERI` value; `None` means the period is undefined or
    /// varies with time.
    pub period: Option<f64>,
}

impl PhaseAxis {
    /// The §9.6 `'PHASE'` parameters for an image WCS `axis` (1-based), or `None`
    /// when that axis is not a phase axis.
    pub fn from_header(
        header: &Header,
        axis: usize,
        alt: Option<char>,
    ) -> Result<Option<PhaseAxis>> {
        if axis == 0 {
            return Err(FitsError::OneBasedIndexRequired { kind: "WCS axis" });
        }
        PhaseAxis::from_keywords(header, PhaseAxisKeywords::image(axis, alt))
    }

    /// The §9.6 `'PHASE'` parameters for a binary-table pixel-list `column` (1-based).
    pub fn from_pixel_list(
        header: &Header,
        column: usize,
        alt: Option<char>,
    ) -> Result<Option<PhaseAxis>> {
        if column == 0 {
            return Err(FitsError::OneBasedIndexRequired {
                kind: "table column",
            });
        }
        PhaseAxis::from_keywords(header, PhaseAxisKeywords::pixel_list(column, alt))
    }

    /// The §9.6 `'PHASE'` parameters for `axis` of an array-valued table `column`
    /// (both 1-based).
    pub fn from_array_column(
        header: &Header,
        axis: usize,
        column: usize,
        alt: Option<char>,
    ) -> Result<Option<PhaseAxis>> {
        if axis == 0 {
            return Err(FitsError::OneBasedIndexRequired { kind: "WCS axis" });
        }
        if column == 0 {
            return Err(FitsError::OneBasedIndexRequired {
                kind: "table column",
            });
        }
        PhaseAxis::from_keywords(header, PhaseAxisKeywords::array_column(axis, column, alt))
    }

    fn from_keywords(header: &Header, keywords: PhaseAxisKeywords) -> Result<Option<PhaseAxis>> {
        let Some(ctype) = header.get_text(keywords.ctype.as_str())? else {
            return Ok(None);
        };
        if TimeAxisKind::from_ctype(ctype) != Some(TimeAxisKind::Phase) {
            return Ok(None);
        }
        let zero_phase = header
            .get_real(keywords.zero_phase.as_str())?
            .ok_or_else(|| FitsError::InvalidTime {
                detail: format!("PHASE axis requires {}", keywords.zero_phase.as_str()),
            })?;
        if !zero_phase.is_finite() {
            return Err(FitsError::InvalidTime {
                detail: format!("{} must be finite", keywords.zero_phase.as_str()),
            });
        }
        let period = header.get_real(keywords.period.as_str())?;
        if period.is_some_and(|value| !value.is_finite()) {
            return Err(FitsError::InvalidTime {
                detail: format!("{} must be finite", keywords.period.as_str()),
            });
        }
        Ok(Some(PhaseAxis {
            zero_phase,
            period: period.filter(|value| *value != 0.0),
        }))
    }
}

#[derive(Debug)]
struct PhaseAxisKeywords {
    ctype: KeyBuf,
    zero_phase: KeyBuf,
    period: KeyBuf,
}

impl PhaseAxisKeywords {
    fn image(axis: usize, alt: Option<char>) -> PhaseAxisKeywords {
        let suffix = AltSuffix::new(alt);
        PhaseAxisKeywords {
            ctype: key!("CTYPE{axis}{suffix}"),
            zero_phase: key!("CZPHS{axis}{suffix}"),
            period: key!("CPERI{axis}{suffix}"),
        }
    }

    fn pixel_list(column: usize, alt: Option<char>) -> PhaseAxisKeywords {
        match alt {
            Some(alt) => PhaseAxisKeywords {
                ctype: key!("TCTY{column}{alt}"),
                zero_phase: key!("TCZP{column}{alt}"),
                period: key!("TCPR{column}{alt}"),
            },
            None => PhaseAxisKeywords {
                ctype: key!("TCTYP{column}"),
                zero_phase: key!("TCZPH{column}"),
                period: key!("TCPER{column}"),
            },
        }
    }

    fn array_column(axis: usize, column: usize, alt: Option<char>) -> PhaseAxisKeywords {
        match alt {
            Some(alt) => PhaseAxisKeywords {
                ctype: key!("{axis}CTY{column}{alt}"),
                zero_phase: key!("{axis}CZP{column}{alt}"),
                period: key!("{axis}CPR{column}{alt}"),
            },
            None => PhaseAxisKeywords {
                ctype: key!("{axis}CTYP{column}"),
                zero_phase: key!("{axis}CZPH{column}"),
                period: key!("{axis}CPER{column}"),
            },
        }
    }
}
