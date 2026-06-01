//! Random-groups primary array (§6) — read only.
//!
//! A legacy structure (radio interferometry `uv` data): `GROUPS = T`, `NAXIS1 =
//! 0`, and the data is `GCOUNT` groups, each `PCOUNT` parameters followed by an
//! array of `NAXIS2 × … × NAXISm` elements. Per "once FITS, always FITS" this is
//! decoded but never written.

use crate::bitpix::Bitpix;
use crate::data::ImageData;
use crate::data::Scaling;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;

/// A decoded random-groups primary array.
#[derive(Debug, Clone)]
pub struct RandomGroups {
    /// `PTYPEn` parameter names, in order (length `pcount`).
    pub parameter_names: Vec<String>,
    /// The per-group array shape (`NAXIS2..NAXISm`; the `NAXIS1` zero sentinel is
    /// dropped).
    pub group_shape: Vec<usize>,
    pub gcount: usize,
    pub pcount: usize,
    bitpix: Bitpix,
    array_scaling: Scaling,
    /// `(PSCALn, PZEROn)` per parameter.
    param_scaling: Vec<(f64, f64)>,
    /// Flat host-endian samples: `gcount` groups of `pcount + array_len` elements.
    samples: ImageData,
}

impl RandomGroups {
    pub(crate) fn from_data(header: &Header, data: &[u8]) -> Result<RandomGroups> {
        let bitpix = header.bitpix()?;
        let axes = header.axes()?;
        // NAXIS1 is the zero sentinel; the per-group array spans the rest.
        let group_shape: Vec<usize> = axes.iter().skip(1).copied().collect();
        let pcount = match header.get_integer("PCOUNT") {
            Some(p) if p < 0 => return Err(FitsError::WrongValueType { name: "PCOUNT" }),
            Some(p) => p as usize,
            None => 0,
        };
        let gcount = match header.get_integer("GCOUNT") {
            Some(g) if g < 1 => return Err(FitsError::WrongValueType { name: "GCOUNT" }),
            Some(g) => g as usize,
            None => 1,
        };

        let mut parameter_names = Vec::with_capacity(pcount);
        let mut param_scaling = Vec::with_capacity(pcount);
        for j in 1..=pcount {
            parameter_names.push(
                header
                    .get_text(&format!("PTYPE{j}"))
                    .unwrap_or("")
                    .to_string(),
            );
            param_scaling.push((
                header.get_real(&format!("PSCAL{j}")).unwrap_or(1.0),
                header.get_real(&format!("PZERO{j}")).unwrap_or(0.0),
            ));
        }

        let samples = ImageData::decode(data, bitpix);
        let groups = RandomGroups {
            parameter_names,
            group_shape,
            gcount,
            pcount,
            bitpix,
            array_scaling: Scaling::from_header(header),
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

    /// `BITPIX` element type of the stored samples.
    pub fn bitpix(&self) -> Bitpix {
        self.bitpix
    }

    /// Elements in one group's array (`Π NAXIS2..NAXISm`; 0 if there is no array).
    pub fn array_len(&self) -> usize {
        if self.group_shape.is_empty() {
            0
        } else {
            self.group_shape.iter().product()
        }
    }

    /// The physical parameter values of group `g`: `PZEROn + PSCALn × raw`.
    pub fn parameters_physical(&self, group: usize) -> Vec<f64> {
        let base = group * self.group_len();
        (0..self.pcount)
            .map(|j| {
                let (pscal, pzero) = self.param_scaling[j];
                pzero + pscal * elem_f64(&self.samples, base + j)
            })
            .collect()
    }

    /// The physical value of the named group parameter (§6.3): when extra
    /// precision splits one logical parameter into two or more group parameters
    /// sharing a `PTYPEn` name, the value is the **sum** of those addends'
    /// physical values. `None` if no parameter has the name. (For the raw
    /// per-addend values, use [`RandomGroups::parameters_physical`].)
    pub fn parameter_physical(&self, group: usize, name: &str) -> Option<f64> {
        let base = group * self.group_len();
        let mut sum = 0.0;
        let mut found = false;
        for j in 0..self.pcount {
            if self.parameter_names[j] == name {
                found = true;
                let (pscal, pzero) = self.param_scaling[j];
                sum += pzero + pscal * elem_f64(&self.samples, base + j);
            }
        }
        found.then_some(sum)
    }

    /// The physical array values of group `g`: `BZERO + BSCALE × raw`.
    pub fn array_physical(&self, group: usize) -> Vec<f64> {
        let base = group * self.group_len() + self.pcount;
        (0..self.array_len())
            .map(|k| {
                self.array_scaling.bzero
                    + self.array_scaling.bscale * elem_f64(&self.samples, base + k)
            })
            .collect()
    }

    fn group_len(&self) -> usize {
        self.pcount + self.array_len()
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

#[cfg(test)]
mod tests;
