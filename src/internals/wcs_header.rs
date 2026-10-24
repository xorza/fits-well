//! WCS headers for tests: one celestial description, written into any keyword family.

use crate::alternate::Alternate;
use crate::error::FitsError;
use crate::header_model::{Header, keywords};
use crate::internals::tolerance::{Tolerance, assert_close};
use crate::world_coordinates::Wcs;
use crate::world_coordinates::internals::{to_pixel, to_world};

/// The keyword family a WCS is written in (§8, Table 22): image axes 1 and 2,
/// pixel-list columns 2 and 3, or the axes of vector column 5.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Family {
    Image,
    PixelList,
    VectorCell,
}

/// A two-axis celestial WCS, written into any [`Family`] by [`WcsHeader::header`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct WcsHeader<'a> {
    pub(crate) projection: &'a str,
    pub(crate) crpix: [f64; 2],
    pub(crate) crval: [f64; 2],
    pub(crate) cdelt: [f64; 2],
    pub(crate) degrees: bool,
    /// Row-major.
    pub(crate) pc: Option<[f64; 4]>,
    /// Row-major, in place of `CDELT` and `PC`.
    pub(crate) cd: Option<[f64; 4]>,
    /// On the latitude axis; there is no alternate form.
    pub(crate) crota: Option<f64>,
    /// `PV2_0`, `PV2_1`, …, on the latitude axis.
    pub(crate) pv: &'a [f64],
    /// `LONPOLE` and `LATPOLE`.
    pub(crate) poles: Option<[f64; 2]>,
    /// The order-2 SIP of the distortion tests, which take it apart by hand:
    /// `A_2_0 = 0.001`, `A_1_1 = 0.002` and `B_0_2 = 0.003`. SIP has no alternate or
    /// table form.
    pub(crate) sip: bool,
}

impl<'a> WcsHeader<'a> {
    pub(crate) const fn celestial(
        projection: &'a str,
        crpix: [f64; 2],
        crval: [f64; 2],
        cdelt: [f64; 2],
    ) -> Self {
        Self {
            projection,
            crpix,
            crval,
            cdelt,
            degrees: false,
            pc: None,
            cd: None,
            crota: None,
            pv: &[],
            poles: None,
            sip: false,
        }
    }

    #[must_use]
    pub(crate) const fn pc(mut self, pc: [f64; 4]) -> Self {
        self.pc = Some(pc);
        self
    }

    #[must_use]
    pub(crate) const fn cd(mut self, cd: [f64; 4]) -> Self {
        self.cd = Some(cd);
        self
    }

    #[must_use]
    pub(crate) const fn sip(mut self) -> Self {
        self.sip = true;
        self
    }

    #[must_use]
    pub(crate) const fn pv(mut self, pv: &'a [f64]) -> Self {
        self.pv = pv;
        self
    }

    pub(crate) fn header(
        &self,
        family: Family,
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) -> Header {
        let mut header = Header::new();
        if family == Family::Image {
            header.set_internal(keywords::NAXIS, 2);
        }
        self.write_into(&mut header, family, alt);
        header
    }

    /// Write the description into `header` beside what it holds, as another
    /// description's alternate.
    pub(crate) fn write_into(
        &self,
        header: &mut Header,
        family: Family,
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) {
        let alt = Alternate::of(alt).unwrap();
        let a = alt.to_string();
        let primary = !alt.is_alternate();
        // Both axes' keyword for an image root, a long table root, and the short
        // table root an alternate description takes (Table 22).
        let names = |image_root: &str, long: &str, short: &str| {
            [1, 2].map(|i| match family {
                Family::Image => format!("{image_root}{i}{a}"),
                Family::PixelList if primary => format!("T{long}{}", i + 1),
                Family::PixelList => format!("T{short}{}{a}", i + 1),
                Family::VectorCell if primary => format!("{i}{long}5"),
                Family::VectorCell => format!("{i}{short}5{a}"),
            })
        };
        let matrix = |name: &str, i: usize, j: usize| match family {
            Family::Image => format!("{name}{i}_{j}{a}"),
            Family::PixelList if primary => format!("T{name}{}_{}", i + 1, j + 1),
            Family::PixelList => format!("T{}{}_{}{a}", &name[..1], i + 1, j + 1),
            Family::VectorCell => format!("{i}{j}{name}5{a}"),
        };
        let pole = |name: &str| match family {
            Family::Image => format!("{name}{a}"),
            Family::PixelList => format!("{}P2{a}", &name[..3]),
            Family::VectorCell => format!("{}P5{a}", &name[..3]),
        };

        let ctype = names("CTYPE", "CTYP", "CTY");
        header
            .set_internal(&ctype[0], format!("RA---{}", self.projection))
            .set_internal(&ctype[1], format!("DEC--{}", self.projection));
        let mut vectors = vec![
            (names("CRPIX", "CRPX", "CRP"), self.crpix),
            (names("CRVAL", "CRVL", "CRV"), self.crval),
        ];
        if self.cd.is_none() {
            vectors.push((names("CDELT", "CDLT", "CDE"), self.cdelt));
        }
        for (keys, values) in vectors {
            header
                .set_internal(&keys[0], values[0])
                .set_internal(&keys[1], values[1]);
        }
        if self.degrees {
            let cunit = names("CUNIT", "CUNI", "CUN");
            header
                .set_internal(&cunit[0], "deg")
                .set_internal(&cunit[1], "deg");
        }
        for (name, values) in [("PC", self.pc), ("CD", self.cd)] {
            for (index, value) in values.into_iter().flatten().enumerate() {
                header.set_internal(matrix(name, index / 2 + 1, index % 2 + 1), value);
            }
        }
        if let Some(crota) = self.crota {
            assert!(primary, "CROTAi has no alternate form");
            header.set_internal(&names("CROTA", "CROT", "CROT")[1], crota);
        }
        for (m, &value) in self.pv.iter().enumerate() {
            let key = match family {
                Family::Image => format!("PV2_{m}{a}"),
                Family::PixelList if primary => format!("TPV3_{m}"),
                Family::PixelList => format!("TV3_{m}{a}"),
                Family::VectorCell if primary => format!("2PV5_{m}"),
                Family::VectorCell => format!("2V5_{m}{a}"),
            };
            header.set_internal(&key, value);
        }
        if let Some([lonpole, latpole]) = self.poles {
            header
                .set_internal(pole("LONPOLE"), lonpole)
                .set_internal(pole("LATPOLE"), latpole);
        }
        if self.sip {
            assert!(
                family == Family::Image && primary,
                "SIP has no alternate or table form"
            );
            header
                .set_internal("A_ORDER", 2)
                .set_internal("A_2_0", 1e-3)
                .set_internal("A_1_1", 2e-3)
                .set_internal("B_ORDER", 2)
                .set_internal("B_0_2", 3e-3);
        }
    }

    /// The image-axis form of the primary description.
    pub(crate) fn image_header(&self) -> Header {
        self.header(Family::Image, None)
    }

    pub(crate) fn wcs(
        &self,
        family: Family,
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) -> Wcs {
        let alt = Alternate::of(alt).unwrap();
        let header = self.header(family, alt);
        match family {
            Family::Image => Wcs::from_header(&header, alt),
            Family::PixelList => Wcs::from_pixel_list(&header, &[2, 3], alt),
            Family::VectorCell => Wcs::from_array_column(&header, 5, alt),
        }
        .unwrap()
    }
}

/// Assert that each of `pixels` comes back from its world coordinate within `tolerance`.
#[track_caller]
pub(crate) fn assert_round_trip(
    wcs: &Wcs,
    pixels: &[impl AsRef<[f64]>],
    tolerance: Tolerance,
    context: &str,
) {
    for pixel in pixels {
        let pixel = pixel.as_ref();
        let world = to_world(wcs, pixel).unwrap();
        let back = to_pixel(wcs, &world).unwrap();
        assert_close(&back, pixel, tolerance, &format!("{context} at {pixel:?}"));
    }
}

/// A one-axis image header.
pub(crate) fn axis_header(ctype: &str, cunit: &str, crpix: f64, crval: f64, cdelt: f64) -> Header {
    let mut header = Header::new();
    header
        .set_internal(keywords::NAXIS, 1)
        .set_internal("CTYPE1", ctype)
        .set_internal("CUNIT1", cunit)
        .set_internal("CRPIX1", crpix)
        .set_internal("CRVAL1", crval)
        .set_internal("CDELT1", cdelt);
    header
}
