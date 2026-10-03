//! Random-groups primary array (§6) — read only.
//!
//! A legacy structure (radio interferometry `uv` data): `GROUPS = T`, `NAXIS1 =
//! 0`, and the data is `GCOUNT` groups, each `PCOUNT` parameters followed by an
//! array of `NAXIS2 × … × NAXISm` elements. Per "once FITS, always FITS" this is
//! decoded but never written.

use crate::bitpix::Bitpix;
use crate::data::image_data::ImageData;
use crate::data::image_view::ImageView;
use crate::data::physical_view;
use crate::data::scaling::Scaling;
use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Result;
use crate::header_model::Header;
use crate::keyword::key;
use std::iter;

/// The last parameter a `PTYPEn`/`PSCALn`/`PZEROn` keyword can describe: each root
/// is five characters, which leaves three digits of an eight-character keyword. A
/// later parameter is legal (§6.1.1 does not bound `PCOUNT`) but has no keyword, so
/// it takes the defaults: no name, `PSCALn = 1`, `PZEROn = 0`.
const LAST_DESCRIBED_PARAMETER: usize = 999;

/// A decoded random-groups primary array.
#[derive(Debug, Clone)]
pub struct RandomGroups {
    /// `PTYPEn` names of the first `min(pcount, 999)` parameters, in order.
    parameter_names: Vec<String>,
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
    samples: ImageData,
}

/// Immutable geometry and representation metadata for a random-groups array.
#[derive(Debug, Clone, Copy)]
pub struct RandomGroupsMetadata<'a> {
    /// `PTYPEn` names in parameter order, `""` where the keyword is absent. Only the
    /// first 999 parameters have a `PTYPEn` keyword, so past that the slice is shorter
    /// than `pcount`.
    pub parameter_names: &'a [String],
    /// Per-group array shape, with the `NAXIS1 = 0` sentinel omitted.
    pub group_shape: &'a [usize],
    /// Number of stored groups.
    pub gcount: usize,
    /// Number of parameters preceding each group's array.
    pub pcount: usize,
    /// Stored sample representation shared by parameters and arrays.
    pub bitpix: Bitpix,
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
    pub parameters: ImageView<'a>,
    pub array: ImageView<'a>,
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

    pub(crate) fn from_data(header: &Header, data: &[u8]) -> Result<RandomGroups> {
        let bitpix = header.bitpix()?;
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
        let got = data.len() / bitpix.elem_size();
        if got != expected {
            return Err(FitsError::DataSizeMismatch { expected, got });
        }

        let described = pcount.min(LAST_DESCRIBED_PARAMETER);
        let mut parameter_names = Vec::with_capacity(described);
        let mut param_scaling = Vec::with_capacity(described);
        for j in 1..=described {
            parameter_names.push(
                header
                    .get_text(key!("PTYPE{j}").as_str())?
                    .unwrap_or("")
                    .to_string(),
            );
            param_scaling.push(ParamScale {
                pscal: header.get_real(key!("PSCAL{j}").as_str())?.unwrap_or(1.0),
                pzero: header.get_real(key!("PZERO{j}").as_str())?.unwrap_or(0.0),
            });
        }

        Ok(RandomGroups {
            parameter_names,
            group_shape,
            gcount,
            pcount,
            array_len,
            array_scaling: header.scaling()?,
            param_scaling,
            samples: ImageData::decode(data, bitpix),
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
    pub fn group_by_idx(&self, index: usize) -> Result<RandomGroupView<'_>> {
        let start = self.checked_group_base(index)?;
        let group_len = self.group_len();
        let parameters_end = start + self.pcount;
        let group_end = start + group_len;
        Ok(RandomGroupView {
            parameters: self.samples.view(start..parameters_end),
            array: self.samples.view(parameters_end..group_end),
        })
    }

    /// The physical parameter values of group `g`: `PZEROn + PSCALn × raw`.
    pub fn parameters_physical(&self, group: usize) -> Result<Vec<f64>> {
        let base = self.checked_group_base(group)?;
        // Widen the whole parameter run once (identity scaling is a plain widening),
        // then apply each described parameter's own `PSCALn`/`PZEROn`; a later one
        // keeps the identity its absent keywords mean.
        let mut values = physical_view(
            self.samples.view(base..base + self.pcount),
            &Scaling::IDENTITY,
        );
        for (value, &ParamScale { pscal, pzero }) in values.iter_mut().zip(&self.param_scaling) {
            *value = pzero + pscal * *value;
        }
        Ok(values)
    }

    /// The physical value of the named group parameter (§6.3): when extra
    /// precision splits one logical parameter into two or more group parameters
    /// sharing a `PTYPEn` name, the value is the **sum** of those addends'
    /// physical values. `None` if no parameter has the name. (For the raw
    /// per-addend values, use [`RandomGroups::parameters_physical`].)
    pub fn parameter_physical(&self, group: usize, name: &str) -> Result<Option<f64>> {
        let values = self.parameters_physical(group)?;
        let mut sum = 0.0;
        let mut found = false;
        let names = self
            .parameter_names
            .iter()
            .map(String::as_str)
            .chain(iter::repeat(""));
        for (value, parameter) in values.iter().zip(names) {
            if parameter == name {
                found = true;
                sum += value;
            }
        }
        Ok(found.then_some(sum))
    }

    /// The physical array values of group `g`: `BZERO + BSCALE × raw`, with integer
    /// samples equal to `BLANK` mapped to `NaN`.
    pub fn array_physical(&self, group: usize) -> Result<Vec<f64>> {
        let base = self.checked_group_base(group)? + self.pcount;
        Ok(physical_view(
            self.samples.view(base..base + self.array_len()),
            &self.array_scaling,
        ))
    }

    const fn group_len(&self) -> usize {
        self.pcount + self.array_len
    }

    fn checked_group_base(&self, index: usize) -> Result<usize> {
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
mod tests;
