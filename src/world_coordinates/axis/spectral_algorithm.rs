//! [`SpectralAlgorithm`]: the algorithm code of a spectral axis.

use crate::world_coordinates::axis::spectral_kind::Characteristic;

/// The algorithm code of a spectral `CTYPEi` (WCS Paper III, Table 2): `X2P`
/// samples the axis linearly in characteristic X and expresses it as P; `GRI` and
/// `GRA` sample a grism in vacuum or air wavelength.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SpectralAlgorithm {
    Pair {
        sampled: Characteristic,
        expressed: Characteristic,
    },
    Grism {
        sampled: Characteristic,
    },
}

/// Every `X2P` code, indexed by the sampled and the expressed characteristic, built
/// from the one letter table so the names cannot drift from the parse.
static PAIR_CODES: [[[u8; 3]; 4]; 4] = pair_codes();

const fn pair_codes() -> [[[u8; 3]; 4]; 4] {
    let mut codes = [[[0; 3]; 4]; 4];
    let mut sampled = 0;
    while sampled < 4 {
        let mut expressed = 0;
        while expressed < 4 {
            codes[sampled][expressed] = [
                Characteristic::ALL[sampled].letter(),
                b'2',
                Characteristic::ALL[expressed].letter(),
            ];
            expressed += 1;
        }
        sampled += 1;
    }
    codes
}

impl SpectralAlgorithm {
    /// The algorithm `code` spells, or `None` when it is no spectral algorithm.
    pub(super) fn parse(code: &str) -> Option<SpectralAlgorithm> {
        match code {
            "GRI" => Some(SpectralAlgorithm::Grism {
                sampled: Characteristic::Wavelength,
            }),
            "GRA" => Some(SpectralAlgorithm::Grism {
                sampled: Characteristic::AirWavelength,
            }),
            _ => {
                let &[sampled, b'2', expressed] = code.as_bytes() else {
                    return None;
                };
                Some(SpectralAlgorithm::Pair {
                    sampled: Characteristic::from_letter(sampled)?,
                    expressed: Characteristic::from_letter(expressed)?,
                })
            }
        }
    }

    /// The characteristic the axis is sampled linearly in, or through the grism.
    pub(super) const fn sampled(self) -> Characteristic {
        match self {
            SpectralAlgorithm::Pair { sampled, .. } | SpectralAlgorithm::Grism { sampled } => {
                sampled
            }
        }
    }

    /// The code as written in `CTYPEi`, for error messages.
    pub(super) fn name(self) -> &'static str {
        match self {
            SpectralAlgorithm::Grism {
                sampled: Characteristic::AirWavelength,
            } => "GRA",
            SpectralAlgorithm::Grism { .. } => "GRI",
            SpectralAlgorithm::Pair { sampled, expressed } => {
                std::str::from_utf8(&PAIR_CODES[sampled as usize][expressed as usize])
                    .expect("characteristic letters are ASCII")
            }
        }
    }
}
