//! The files in `test_resources/`, compiled into the test binary: a test reads no file, and
//! a name that is no fixture does not compile.

use std::io::Cursor;

use crate::header_model::Header;
use crate::reader::{FitsReader, SliceReader, StreamReader};
use crate::world_coordinates::Wcs;

macro_rules! fixtures {
    ($($(#[doc = $doc:literal])* $(#[cfg($cfg:meta)])? $variant:ident => $file:literal,)+) => {
        /// A file in `test_resources/`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum Fixture {
            $($(#[doc = $doc])* $(#[cfg($cfg)])? $variant,)+
        }

        impl Fixture {
            /// The file's bytes.
            pub(crate) const fn bytes(self) -> &'static [u8] {
                match self {
                    $($(#[cfg($cfg)])? Self::$variant => include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/test_resources/",
                        $file
                    )),)+
                }
            }
        }
    };
}

fixtures! {
    /// A 16-bit primary of 512 × 512.
    Uit => "UITfuv2582gc.fits",
    /// Real UV random groups.
    Ddts => "DDTSUVDATA.fits",
    /// A primary with no data, then a binary table.
    Iue => "IUElwp25637mxlo.fits",
    /// A primary whose header holds a TAN description.
    WcsTan => "wcs_tan.fits",
    /// astropy's IRAC header with SIP keywords, as bare records (`astropy/wcs/tests/data`,
    /// BSD-3).
    WcsIracSip => "wcs_irac_sip.hdr",
    /// astropy's SIP-only header, as bare records.
    WcsSipOnly => "wcs_siponly.hdr",
    /// astropy's TPV-only header, as bare records.
    WcsTpvOnly => "wcs_tpvonly.hdr",
    CompGzipI16 => "comp_gzip_i16.fits",
    CompTableCfitsio => "comp_table_cfitsio.fits",
    CompTableVla => "comp_table_vla.fits",
    #[cfg(feature = "compression")]
    CompDither2F32 => "comp_dither2_f32.fits",
    #[cfg(feature = "compression")]
    CompGzip2I16 => "comp_gzip2_i16.fits",
    #[cfg(feature = "compression")]
    CompHcompAbsI16 => "comp_hcomp_abs_i16.fits",
    #[cfg(feature = "compression")]
    CompHcompI16 => "comp_hcomp_i16.fits",
    #[cfg(feature = "compression")]
    CompHcompLossy => "comp_hcomp_lossy.fits",
    #[cfg(feature = "compression")]
    CompHcompLossyOdd => "comp_hcomp_lossy_odd.fits",
    #[cfg(feature = "compression")]
    CompHcompOddI16 => "comp_hcomp_odd_i16.fits",
    #[cfg(feature = "compression")]
    CompHcompQuantSeed17F32 => "comp_hcomp_quant_seed17_f32.fits",
    #[cfg(feature = "compression")]
    CompHcompRelI16 => "comp_hcomp_rel_i16.fits",
    #[cfg(feature = "compression")]
    CompHcompSmooth => "comp_hcomp_smooth.fits",
    #[cfg(feature = "compression")]
    CompLosslessGzip1F32 => "comp_lossless_gzip1_f32.fits",
    #[cfg(feature = "compression")]
    CompLosslessGzip1F64 => "comp_lossless_gzip1_f64.fits",
    #[cfg(feature = "compression")]
    CompLosslessGzip2F32 => "comp_lossless_gzip2_f32.fits",
    #[cfg(feature = "compression")]
    CompLosslessGzip2F64 => "comp_lossless_gzip2_f64.fits",
    #[cfg(feature = "compression")]
    CompNanF32 => "comp_nan_f32.fits",
    #[cfg(feature = "compression")]
    CompPlioI32 => "comp_plio_i32.fits",
    #[cfg(feature = "compression")]
    CompQuantAbsSeed17F32 => "comp_quant_abs_seed17_f32.fits",
    #[cfg(feature = "compression")]
    CompQuantRelSeed17F32 => "comp_quant_rel_seed17_f32.fits",
    #[cfg(feature = "compression")]
    CompRefDither2F32 => "comp_ref_dither2_f32.fits",
    #[cfg(feature = "compression")]
    CompRefF32 => "comp_ref_f32.fits",
    #[cfg(feature = "compression")]
    CompRefHcompLossy => "comp_ref_hcomp_lossy.fits",
    #[cfg(feature = "compression")]
    CompRefHcompLossyOdd => "comp_ref_hcomp_lossy_odd.fits",
    #[cfg(feature = "compression")]
    CompRefHcompSmooth => "comp_ref_hcomp_smooth.fits",
    #[cfg(feature = "compression")]
    CompRefNanF32 => "comp_ref_nan_f32.fits",
    #[cfg(feature = "compression")]
    CompRefQuantF32 => "comp_ref_quant_f32.fits",
    #[cfg(feature = "compression")]
    CompRicefNodither => "comp_ricef_nodither.fits",
    #[cfg(feature = "compression")]
    CompRicefQuant => "comp_ricef_quant.fits",
    #[cfg(feature = "compression")]
    CompRiceI16 => "comp_rice_i16.fits",
    #[cfg(feature = "compression")]
    CompTableNocompress => "comp_table_nocompress.fits",
    #[cfg(feature = "compression")]
    CompTableRef => "comp_table_ref.fits",
    #[cfg(feature = "compression")]
    HcompWideI32Raw => "hcomp_wide_i32.raw",
    #[cfg(feature = "compression")]
    HcompWideI32Huf => "hcomp_wide_i32.huf",
}

impl Fixture {
    /// A streaming reader over the file, as a caller with a `File` reads it.
    pub(crate) fn stream(self) -> StreamReader<Cursor<&'static [u8]>> {
        FitsReader::open(Cursor::new(self.bytes()))
            .unwrap_or_else(|error| panic!("open {self:?}: {error}"))
    }

    /// A reader over the file's bytes in memory.
    pub(crate) fn slice(self) -> SliceReader<'static> {
        FitsReader::from_bytes(self.bytes())
            .unwrap_or_else(|error| panic!("open {self:?}: {error}"))
    }

    /// The header a file of bare records (a `.hdr`) holds.
    pub(crate) fn header(self) -> Header {
        Header::parse(self.bytes()).unwrap_or_else(|error| panic!("parse {self:?}: {error}"))
    }

    /// The primary description of the file's primary header.
    pub(crate) fn wcs(self) -> Wcs {
        Wcs::from_header(&self.slice().hdus()[0].header, None).unwrap()
    }
}
