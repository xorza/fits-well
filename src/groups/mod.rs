//! Random-groups primary array (§6) — read only.
//!
//! A legacy structure (radio interferometry `uv` data): `GROUPS = T`, `NAXIS1 =
//! 0`, and the data is `GCOUNT` groups, each `PCOUNT` parameters followed by an
//! array of `NAXIS2 × … × NAXISm` elements. Per "once FITS, always FITS" this is
//! decoded but never written.

use crate::bitpix::Bitpix;
use crate::fits_error::FitsError;
use crate::fits_error::Indexed;
use crate::fits_error::Result;
use crate::header_model::{Header, keywords};
use crate::image_model::data::Data;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::scaling::Scaling;
use crate::image_model::view::View;

/// The last parameter a `PTYPEn`/`PSCALn`/`PZEROn` keyword can describe: each root
/// is five characters, which leaves three digits of an eight-character keyword. A
/// later parameter is legal (§6.1.1 does not bound `PCOUNT`) but has no keyword, so
/// it takes the defaults: no name, `PSCALn = 1`, `PZEROn = 0`.
const LAST_DESCRIBED_PARAMETER: usize = 999;

/// A decoded random-groups primary array.
#[derive(Debug, Clone)]
pub struct RandomGroups {
    /// `PTYPEn` names of the first `min(pcount, 999)` parameters, in order; `None` where
    /// the keyword is absent or empty.
    parameter_names: Vec<Option<String>>,
    /// The per-group array shape (`NAXIS2..NAXISm`; the `NAXIS1` zero sentinel is
    /// dropped).
    group_shape: Vec<usize>,
    gcount: usize,
    pcount: usize,
    array_len: usize,
    array_scaling: Scaling,
    /// `PSCALn`/`PZEROn` of the first `min(pcount, 999)` parameters.
    param_scaling: Vec<ParamScale>,
    /// Flat host-endian samples: `gcount` groups of `pcount + array_len` elements.
    samples: Data,
}

/// Immutable geometry and representation metadata for a random-groups array.
#[derive(Debug, Clone, Copy)]
pub struct RandomGroupsMetadata<'a> {
    /// `PTYPEn` names in parameter order, `None` where the keyword is absent or empty.
    /// Only the first 999 parameters have a `PTYPEn` keyword, so past that the slice is
    /// shorter than `pcount`.
    pub parameter_names: &'a [Option<String>],
    /// Per-group array shape, with the `NAXIS1 = 0` sentinel omitted.
    pub group_shape: &'a [usize],
    /// Number of stored groups.
    pub gcount: usize,
    /// Number of parameters preceding each group's array.
    pub pcount: usize,
    /// Stored sample representation shared by parameters and arrays.
    pub bitpix: Bitpix,
}

/// The group parameters one `PTYPEn` name names, from [`RandomGroups::parameter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupParameter {
    indices: Vec<usize>,
}

/// `PSCALn`/`PZEROn` linear scaling for one group parameter
/// (`physical = pzero + pscal · raw`).
#[derive(Debug, Clone, Copy)]
struct ParamScale {
    pscal: f64,
    pzero: f64,
}

/// Exact stored values for one random group. Parameters and array samples share
/// the HDU's `BITPIX` representation but occupy distinct borrowed slices (§6.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RandomGroupView<'a> {
    pub parameters: View<'a>,
    pub array: View<'a>,
}

impl RandomGroups {
    /// Borrow the random-groups geometry, parameter names, and stored type.
    pub fn metadata(&self) -> RandomGroupsMetadata<'_> {
        RandomGroupsMetadata {
            parameter_names: &self.parameter_names,
            group_shape: &self.group_shape,
            gcount: self.gcount,
            pcount: self.pcount,
            bitpix: self.samples.bitpix(),
        }
    }

    /// The random groups `header` describes, of the host-endian `samples` its data unit
    /// holds.
    pub(crate) fn from_samples(header: &Header, samples: Data) -> Result<Self> {
        let bitpix = header.bitpix()?;
        debug_assert_eq!(samples.bitpix(), bitpix, "samples of the header's BITPIX");
        let axes = header.axes()?;
        // NAXIS1 is the zero sentinel; the per-group array spans the rest.
        let group_shape: Vec<usize> = axes.iter().skip(1).copied().collect();
        let pcount = usize::try_from(header.pcount()?).map_err(|_| FitsError::DataUnitOverflow)?;
        let gcount = usize::try_from(header.gcount()?).map_err(|_| FitsError::DataUnitOverflow)?;
        // The Eq. 6 product: with no array axis (`NAXIS = 1`) it is the empty product
        // `1`, as the HDU data extent counts it, not `shape_product`'s image rule of 0.
        let array_len = group_shape
            .iter()
            .try_fold(1usize, |len, &axis| len.checked_mul(axis))
            .ok_or(FitsError::DataUnitOverflow)?;
        let expected = pcount
            .checked_add(array_len)
            .and_then(|group_len| group_len.checked_mul(gcount))
            .ok_or(FitsError::DataUnitOverflow)?;
        debug_assert_eq!(
            samples.len(),
            expected,
            "the scan sized the data unit from these keywords"
        );

        let described = pcount.min(LAST_DESCRIBED_PARAMETER);
        let mut parameter_names = Vec::with_capacity(described);
        let mut param_scaling = Vec::with_capacity(described);
        for j in 1..=described {
            parameter_names.push(
                header
                    .get_text(keywords::PTYPE.indexed(j)?)?
                    .filter(|name| !name.is_empty())
                    .map(str::to_string),
            );
            param_scaling.push(ParamScale {
                pscal: header.get_real(keywords::PSCAL.indexed(j)?)?.unwrap_or(1.0),
                pzero: header.get_real(keywords::PZERO.indexed(j)?)?.unwrap_or(0.0),
            });
        }

        Ok(Self {
            parameter_names,
            group_shape,
            gcount,
            pcount,
            array_len,
            array_scaling: Scaling::from_header(header, bitpix)?,
            param_scaling,
            samples,
        })
    }

    /// Elements in one group's array — `Π NAXIS2..NAXISm`, the FITS Eq. 6 product,
    /// so `1` when there is no array axis.
    pub const fn array_len(&self) -> usize {
        self.array_len
    }

    /// Borrow the exact host-endian stored values for group `index`, before
    /// `PSCALn`/`PZEROn` or `BSCALE`/`BZERO` conversion. This preserves all integer
    /// values and floating-point bit patterns that the physical `f64` APIs cannot.
    pub fn group(&self, index: usize) -> Result<RandomGroupView<'_>> {
        let start = self.checked_group_base(index)?;
        let group_len = self.group_len();
        let parameters_end = start + self.pcount;
        let group_end = start + group_len;
        Ok(RandomGroupView {
            parameters: self.samples.view().slice(start..parameters_end),
            array: self.samples.view().slice(parameters_end..group_end),
        })
    }

    /// Append the physical parameter values of group `group` to `out`:
    /// `PZEROn + PSCALn × raw`, one fused multiply-add each.
    pub fn parameters_physical_into(&self, group: usize, out: &mut Vec<f64>) -> Result<()> {
        let base = self.checked_group_base(group)?;
        // Widen the whole parameter run once (identity scaling is a plain widening),
        // then apply each described parameter's own `PSCALn`/`PZEROn`; a later one
        // keeps the identity its absent keywords mean.
        let start = out.len();
        self.samples
            .view()
            .slice(base..base + self.pcount)
            .extend_physical(&Scaling::IDENTITY, out);
        for (value, &ParamScale { pscal, pzero }) in
            out[start..].iter_mut().zip(&self.param_scaling)
        {
            *value = pscal.mul_add(*value, pzero);
        }
        Ok(())
    }

    /// The group parameters that `name` names, resolved once (§6.3): every parameter whose
    /// `PTYPEn` is `name`. Errors with [`FitsError::GroupParameterNotFound`] if none is.
    pub fn parameter(&self, name: &str) -> Result<GroupParameter> {
        let indices: Vec<usize> = self
            .parameter_names
            .iter()
            .enumerate()
            .filter(|(_, parameter)| parameter.as_deref() == Some(name))
            .map(|(index, _)| index)
            .collect();
        if indices.is_empty() {
            return Err(FitsError::GroupParameterNotFound {
                name: name.to_string(),
            });
        }
        Ok(GroupParameter { indices })
    }

    /// The physical value of `parameter` in group `group` (§6.3): when extra precision
    /// splits one logical parameter into two or more group parameters sharing a `PTYPEn`
    /// name, the value is the **sum** of those addends' physical values.
    pub fn parameter_physical(&self, group: usize, parameter: &GroupParameter) -> Result<f64> {
        let base = self.checked_group_base(group)?;
        let parameters = self.samples.view().slice(base..base + self.pcount);
        Ok(parameter
            .indices
            .iter()
            .map(|&index| {
                let ParamScale { pscal, pzero } = self.param_scaling[index];
                pscal.mul_add(parameters.get_f64(index), pzero)
            })
            .sum())
    }

    /// Append the physical array values of group `group` to `out`: `BZERO + BSCALE ×
    /// raw`, with integer samples equal to `BLANK` mapped to `NaN`.
    pub fn array_physical_into<T: PhysicalSample>(
        &self,
        group: usize,
        out: &mut Vec<T>,
    ) -> Result<()> {
        let base = self.checked_group_base(group)? + self.pcount;
        self.samples
            .view()
            .slice(base..base + self.array_len())
            .extend_physical(&self.array_scaling, out);
        Ok(())
    }

    const fn group_len(&self) -> usize {
        self.pcount + self.array_len
    }

    const fn checked_group_base(&self, index: usize) -> Result<usize> {
        if index >= self.gcount {
            return Err(FitsError::IndexOutOfBounds {
                indexed: Indexed::Group,
                index,
                len: self.gcount,
            });
        }
        Ok(index * self.group_len())
    }
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::fits_error::Result;
    use crate::groups::RandomGroups;
    use crate::header_model::Header;
    use crate::image_model::data::Data;

    impl RandomGroups {
        /// The random groups `header` describes, of the stored big-endian `data`.
        pub(crate) fn from_data(header: &Header, data: &[u8]) -> Result<Self> {
            Self::from_samples(header, Data::decode(data, header.bitpix()?))
        }
    }
}

#[cfg(test)]
mod tests;
