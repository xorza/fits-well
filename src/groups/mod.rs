//! Random-groups primary array (§6) — read only.
//!
//! A legacy structure (radio interferometry `uv` data): `GROUPS = T`, `NAXIS1 =
//! 0`, and the data is `GCOUNT` groups, each `PCOUNT` parameters followed by an
//! array of `NAXIS2 × … × NAXISm` elements. Per "once FITS, always FITS" this is
//! decoded but never written.

use crate::bitpix::Bitpix;
use crate::data::ImageData;
use crate::data::ImageView;
use crate::data::Scaling;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;

/// A decoded random-groups primary array.
#[derive(Debug, Clone)]
pub struct RandomGroups {
    /// `PTYPEn` parameter names, in order (length `pcount`).
    parameter_names: Vec<String>,
    /// The per-group array shape (`NAXIS2..NAXISm`; the `NAXIS1` zero sentinel is
    /// dropped).
    group_shape: Vec<usize>,
    gcount: usize,
    pcount: usize,
    bitpix: Bitpix,
    array_scaling: Scaling,
    /// `PSCALn`/`PZEROn` per parameter.
    param_scaling: Vec<ParamScale>,
    /// Flat host-endian samples: `gcount` groups of `pcount + array_len` elements.
    samples: ImageData,
}

/// Immutable geometry and representation metadata for a random-groups array.
#[derive(Debug, Clone, Copy)]
pub struct RandomGroupsMetadata<'a> {
    /// `PTYPEn` parameter names in parameter order.
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
            bitpix: self.bitpix,
        }
    }

    pub(crate) fn from_data(header: &Header, data: &[u8]) -> Result<RandomGroups> {
        let bitpix = header.bitpix()?;
        let axes = header.axes()?;
        // NAXIS1 is the zero sentinel; the per-group array spans the rest.
        let group_shape: Vec<usize> = axes.iter().skip(1).copied().collect();
        let pcount = header.pcount()? as usize;
        let gcount = header.gcount()? as usize;

        let mut parameter_names = Vec::with_capacity(pcount);
        let mut param_scaling = Vec::with_capacity(pcount);
        for j in 1..=pcount {
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

        let samples = ImageData::decode(data, bitpix);
        let groups = RandomGroups {
            parameter_names,
            group_shape,
            gcount,
            pcount,
            bitpix,
            array_scaling: header.scaling()?,
            param_scaling,
            samples,
        };
        let expected = groups.gcount * groups.group_len();
        if groups.samples.len() != expected {
            return Err(FitsError::DataSizeMismatch {
                expected,
                got: groups.samples.len(),
            });
        }
        Ok(groups)
    }

    /// Elements in one group's array — `Π NAXIS2..NAXISm`, the FITS Eq. 2 product.
    /// With no array axis (`NAXIS = 1`) this is the empty product `1`, not `0`,
    /// matching how [`crate::hdu`]'s `data_extent` sizes the group (one array element
    /// per group); `shape_product`'s image-specific "empty ⇒ 0" rule must *not* be
    /// used here, or the two disagree and a `NAXIS = 1` group fails the size check.
    pub fn array_len(&self) -> usize {
        self.group_shape.iter().product()
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
        Ok((0..self.pcount)
            .map(|j| {
                let ParamScale { pscal, pzero } = self.param_scaling[j];
                pzero + pscal * elem_f64(&self.samples, base + j)
            })
            .collect())
    }

    /// The physical value of the named group parameter (§6.3): when extra
    /// precision splits one logical parameter into two or more group parameters
    /// sharing a `PTYPEn` name, the value is the **sum** of those addends'
    /// physical values. `None` if no parameter has the name. (For the raw
    /// per-addend values, use [`RandomGroups::parameters_physical`].)
    pub fn parameter_physical(&self, group: usize, name: &str) -> Result<Option<f64>> {
        let base = self.checked_group_base(group)?;
        let mut sum = 0.0;
        let mut found = false;
        for j in 0..self.pcount {
            if self.parameter_names[j] == name {
                found = true;
                let ParamScale { pscal, pzero } = self.param_scaling[j];
                sum += pzero + pscal * elem_f64(&self.samples, base + j);
            }
        }
        Ok(found.then_some(sum))
    }

    /// The physical array values of group `g`: `BZERO + BSCALE × raw`, with integer
    /// samples equal to `BLANK` mapped to `NaN`.
    pub fn array_physical(&self, group: usize) -> Result<Vec<f64>> {
        let base = self.checked_group_base(group)? + self.pcount;
        Ok((0..self.array_len())
            .map(|k| elem_physical(&self.samples, base + k, &self.array_scaling))
            .collect())
    }

    fn group_len(&self) -> usize {
        self.pcount + self.array_len()
    }

    fn checked_group_base(&self, index: usize) -> Result<usize> {
        if index >= self.gcount {
            return Err(FitsError::GroupIndexOutOfBounds {
                index,
                len: self.gcount,
            });
        }
        Ok(index * self.group_len())
    }
}

/// Read sample `i` of a typed buffer as `f64` (widening).
fn elem_f64(samples: &ImageData, i: usize) -> f64 {
    match samples {
        ImageData::U8(v) => v[i] as f64,
        ImageData::I16(v) => v[i] as f64,
        ImageData::I32(v) => v[i] as f64,
        ImageData::I64(v) => v[i] as f64,
        ImageData::F32(v) => v[i] as f64,
        ImageData::F64(v) => v[i],
    }
}

fn elem_physical(samples: &ImageData, i: usize, scaling: &Scaling) -> f64 {
    match samples {
        ImageData::U8(v) => scaling.scale_integer(v[i] as i64),
        ImageData::I16(v) => scaling.scale_integer(v[i] as i64),
        ImageData::I32(v) => scaling.scale_integer(v[i] as i64),
        ImageData::I64(v) => scaling.scale_integer(v[i]),
        ImageData::F32(v) => scaling.scale(v[i] as f64),
        ImageData::F64(v) => scaling.scale(v[i]),
    }
}

#[cfg(test)]
mod tests;
