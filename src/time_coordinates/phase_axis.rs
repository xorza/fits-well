//! [`PhaseAxis`]: the §9.6 `'PHASE'` axis folding parameters.

use crate::alternate::Alternate;
use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::Header;
use crate::keyword::Keyword;
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
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) -> Result<Option<Self>> {
        let alt = Alternate::of(alt)?;
        if axis == 0 {
            return Err(FitsError::OneBasedIndexRequired { kind: "WCS axis" });
        }
        Self::from_keywords(header, PhaseAxisKeywords::image(axis, alt))
    }

    /// The §9.6 `'PHASE'` parameters for a binary-table pixel-list `column` (1-based).
    pub fn from_pixel_list(
        header: &Header,
        column: usize,
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) -> Result<Option<Self>> {
        let alt = Alternate::of(alt)?;
        if column == 0 {
            return Err(FitsError::OneBasedIndexRequired {
                kind: "table column",
            });
        }
        Self::from_keywords(header, PhaseAxisKeywords::pixel_list(column, alt))
    }

    /// The §9.6 `'PHASE'` parameters for `axis` of an array-valued table `column`
    /// (both 1-based).
    pub fn from_array_column(
        header: &Header,
        axis: usize,
        column: usize,
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) -> Result<Option<Self>> {
        let alt = Alternate::of(alt)?;
        if axis == 0 {
            return Err(FitsError::OneBasedIndexRequired { kind: "WCS axis" });
        }
        if column == 0 {
            return Err(FitsError::OneBasedIndexRequired {
                kind: "table column",
            });
        }
        Self::from_keywords(header, PhaseAxisKeywords::array_column(axis, column, alt))
    }

    /// The axis `keywords` name; `None` when they are no FITS keywords, as a wide axis or
    /// column number makes them, and so no card can hold them.
    fn from_keywords(header: &Header, keywords: Option<PhaseAxisKeywords>) -> Result<Option<Self>> {
        let Some(keywords) = keywords else {
            return Ok(None);
        };
        let Some(ctype) = header.get_text(keywords.ctype)? else {
            return Ok(None);
        };
        if TimeAxisKind::from_ctype(ctype) != Some(TimeAxisKind::Phase) {
            return Ok(None);
        }
        let zero_phase =
            header
                .get_real(keywords.zero_phase)?
                .ok_or_else(|| FitsError::InvalidTime {
                    detail: format!("PHASE axis requires {}", keywords.zero_phase.as_str()),
                })?;
        if !zero_phase.is_finite() {
            return Err(FitsError::InvalidTime {
                detail: format!("{} must be finite", keywords.zero_phase.as_str()),
            });
        }
        let period = header.get_real(keywords.period)?;
        if period.is_some_and(|value| !value.is_finite()) {
            return Err(FitsError::InvalidTime {
                detail: format!("{} must be finite", keywords.period.as_str()),
            });
        }
        Ok(Some(Self {
            zero_phase,
            period: period.filter(|value| *value != 0.0),
        }))
    }
}

#[derive(Debug)]
struct PhaseAxisKeywords {
    ctype: Keyword,
    zero_phase: Keyword,
    period: Keyword,
}

impl PhaseAxisKeywords {
    fn image(axis: usize, alt: Alternate) -> Option<Self> {
        Some(Self {
            ctype: Keyword::try_from(format_args!("CTYPE{axis}{alt}")).ok()?,
            zero_phase: Keyword::try_from(format_args!("CZPHS{axis}{alt}")).ok()?,
            period: Keyword::try_from(format_args!("CPERI{axis}{alt}")).ok()?,
        })
    }

    fn pixel_list(column: usize, alt: Alternate) -> Option<Self> {
        if alt.is_alternate() {
            Some(Self {
                ctype: Keyword::try_from(format_args!("TCTY{column}{alt}")).ok()?,
                zero_phase: Keyword::try_from(format_args!("TCZP{column}{alt}")).ok()?,
                period: Keyword::try_from(format_args!("TCPR{column}{alt}")).ok()?,
            })
        } else {
            Some(Self {
                ctype: Keyword::try_from(format_args!("TCTYP{column}")).ok()?,
                zero_phase: Keyword::try_from(format_args!("TCZPH{column}")).ok()?,
                period: Keyword::try_from(format_args!("TCPER{column}")).ok()?,
            })
        }
    }

    fn array_column(axis: usize, column: usize, alt: Alternate) -> Option<Self> {
        if alt.is_alternate() {
            Some(Self {
                ctype: Keyword::try_from(format_args!("{axis}CTY{column}{alt}")).ok()?,
                zero_phase: Keyword::try_from(format_args!("{axis}CZP{column}{alt}")).ok()?,
                period: Keyword::try_from(format_args!("{axis}CPR{column}{alt}")).ok()?,
            })
        } else {
            Some(Self {
                ctype: Keyword::try_from(format_args!("{axis}CTYP{column}")).ok()?,
                zero_phase: Keyword::try_from(format_args!("{axis}CZPH{column}")).ok()?,
                period: Keyword::try_from(format_args!("{axis}CPER{column}")).ok()?,
            })
        }
    }
}
