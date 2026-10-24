//! The keywords the FITS 4.0 standard defines, as far as fits-well models them, and
//! the roots of its indexed families: `NAXIS` is the keyword and the root of `NAXIS1`,
//! which [`Keyword::indexed`] makes. A hyphen in a name is an underscore in its
//! constant: `DATE_OBS` is `DATE-OBS`.

use crate::keyword::Keyword;

pub const SIMPLE: Keyword = Keyword::new("SIMPLE");
pub const XTENSION: Keyword = Keyword::new("XTENSION");
pub const BITPIX: Keyword = Keyword::new("BITPIX");
pub const NAXIS: Keyword = Keyword::new("NAXIS");
pub const EXTEND: Keyword = Keyword::new("EXTEND");
pub const PCOUNT: Keyword = Keyword::new("PCOUNT");
pub const GCOUNT: Keyword = Keyword::new("GCOUNT");
pub const GROUPS: Keyword = Keyword::new("GROUPS");
pub const BLOCKED: Keyword = Keyword::new("BLOCKED");

pub const DATE: Keyword = Keyword::new("DATE");
pub const ORIGIN: Keyword = Keyword::new("ORIGIN");
pub const AUTHOR: Keyword = Keyword::new("AUTHOR");
pub const REFERENC: Keyword = Keyword::new("REFERENC");
pub const COMMENT: Keyword = Keyword::new("COMMENT");
pub const HISTORY: Keyword = Keyword::new("HISTORY");
pub const DATE_OBS: Keyword = Keyword::new("DATE-OBS");
pub const TELESCOP: Keyword = Keyword::new("TELESCOP");
pub const INSTRUME: Keyword = Keyword::new("INSTRUME");
pub const OBSERVER: Keyword = Keyword::new("OBSERVER");
pub const OBJECT: Keyword = Keyword::new("OBJECT");
pub const EQUINOX: Keyword = Keyword::new("EQUINOX");
pub const EPOCH: Keyword = Keyword::new("EPOCH");
pub const BUNIT: Keyword = Keyword::new("BUNIT");
pub const BSCALE: Keyword = Keyword::new("BSCALE");
pub const BZERO: Keyword = Keyword::new("BZERO");
pub const BLANK: Keyword = Keyword::new("BLANK");
pub const DATAMAX: Keyword = Keyword::new("DATAMAX");
pub const DATAMIN: Keyword = Keyword::new("DATAMIN");
pub const EXTNAME: Keyword = Keyword::new("EXTNAME");
pub const EXTVER: Keyword = Keyword::new("EXTVER");
pub const EXTLEVEL: Keyword = Keyword::new("EXTLEVEL");

pub const CHECKSUM: Keyword = Keyword::new("CHECKSUM");
pub const DATASUM: Keyword = Keyword::new("DATASUM");

pub const TFIELDS: Keyword = Keyword::new("TFIELDS");
pub const TBCOL: Keyword = Keyword::new("TBCOL");
pub const TFORM: Keyword = Keyword::new("TFORM");
pub const TTYPE: Keyword = Keyword::new("TTYPE");
pub const TUNIT: Keyword = Keyword::new("TUNIT");
pub const TSCAL: Keyword = Keyword::new("TSCAL");
pub const TZERO: Keyword = Keyword::new("TZERO");
pub const TNULL: Keyword = Keyword::new("TNULL");
pub const TDISP: Keyword = Keyword::new("TDISP");
pub const TDIM: Keyword = Keyword::new("TDIM");
pub const THEAP: Keyword = Keyword::new("THEAP");

pub const PTYPE: Keyword = Keyword::new("PTYPE");
pub const PSCAL: Keyword = Keyword::new("PSCAL");
pub const PZERO: Keyword = Keyword::new("PZERO");

pub const WCSAXES: Keyword = Keyword::new("WCSAXES");
pub const WCSNAME: Keyword = Keyword::new("WCSNAME");
pub const CTYPE: Keyword = Keyword::new("CTYPE");
pub const CUNIT: Keyword = Keyword::new("CUNIT");
pub const CRVAL: Keyword = Keyword::new("CRVAL");
pub const CDELT: Keyword = Keyword::new("CDELT");
pub const CRPIX: Keyword = Keyword::new("CRPIX");
pub const CROTA: Keyword = Keyword::new("CROTA");
pub const CNAME: Keyword = Keyword::new("CNAME");
pub const CRDER: Keyword = Keyword::new("CRDER");
pub const CSYER: Keyword = Keyword::new("CSYER");
pub const RADESYS: Keyword = Keyword::new("RADESYS");
pub const RADECSYS: Keyword = Keyword::new("RADECSYS");
pub const LONPOLE: Keyword = Keyword::new("LONPOLE");
pub const LATPOLE: Keyword = Keyword::new("LATPOLE");
pub const RESTFRQ: Keyword = Keyword::new("RESTFRQ");
pub const RESTFREQ: Keyword = Keyword::new("RESTFREQ");
pub const RESTWAV: Keyword = Keyword::new("RESTWAV");
pub const SPECSYS: Keyword = Keyword::new("SPECSYS");
pub const SSYSOBS: Keyword = Keyword::new("SSYSOBS");
pub const MJD_OBS: Keyword = Keyword::new("MJD-OBS");

pub const TIMESYS: Keyword = Keyword::new("TIMESYS");
pub const MJDREF: Keyword = Keyword::new("MJDREF");
pub const MJDREFI: Keyword = Keyword::new("MJDREFI");
pub const MJDREFF: Keyword = Keyword::new("MJDREFF");
pub const JDREF: Keyword = Keyword::new("JDREF");
pub const JDREFI: Keyword = Keyword::new("JDREFI");
pub const JDREFF: Keyword = Keyword::new("JDREFF");
pub const DATEREF: Keyword = Keyword::new("DATEREF");
pub const TREFPOS: Keyword = Keyword::new("TREFPOS");
pub const TIMEUNIT: Keyword = Keyword::new("TIMEUNIT");
pub const TIMEOFFS: Keyword = Keyword::new("TIMEOFFS");
pub const DATE_BEG: Keyword = Keyword::new("DATE-BEG");
pub const DATE_END: Keyword = Keyword::new("DATE-END");
pub const DATE_AVG: Keyword = Keyword::new("DATE-AVG");
pub const MJD_BEG: Keyword = Keyword::new("MJD-BEG");
pub const MJD_END: Keyword = Keyword::new("MJD-END");
pub const MJD_AVG: Keyword = Keyword::new("MJD-AVG");
pub const XPOSURE: Keyword = Keyword::new("XPOSURE");
pub const TELAPSE: Keyword = Keyword::new("TELAPSE");
pub const TIMEDEL: Keyword = Keyword::new("TIMEDEL");
pub const TIMEPIXR: Keyword = Keyword::new("TIMEPIXR");
pub const TIMSYER: Keyword = Keyword::new("TIMSYER");
pub const TIMRDER: Keyword = Keyword::new("TIMRDER");
pub const JEPOCH: Keyword = Keyword::new("JEPOCH");
pub const BEPOCH: Keyword = Keyword::new("BEPOCH");
pub const TCTYP: Keyword = Keyword::new("TCTYP");
pub const TCZPH: Keyword = Keyword::new("TCZPH");
pub const TCPER: Keyword = Keyword::new("TCPER");
pub const TRPOS: Keyword = Keyword::new("TRPOS");

pub const ZIMAGE: Keyword = Keyword::new("ZIMAGE");
pub const ZCMPTYPE: Keyword = Keyword::new("ZCMPTYPE");
pub const ZBITPIX: Keyword = Keyword::new("ZBITPIX");
pub const ZNAXIS: Keyword = Keyword::new("ZNAXIS");
pub const ZTILE: Keyword = Keyword::new("ZTILE");
pub const ZNAME: Keyword = Keyword::new("ZNAME");
pub const ZVAL: Keyword = Keyword::new("ZVAL");
pub const ZMASKCMP: Keyword = Keyword::new("ZMASKCMP");
pub const ZSIMPLE: Keyword = Keyword::new("ZSIMPLE");
pub const ZTENSION: Keyword = Keyword::new("ZTENSION");
pub const ZEXTEND: Keyword = Keyword::new("ZEXTEND");
pub const ZBLOCKED: Keyword = Keyword::new("ZBLOCKED");
pub const ZPCOUNT: Keyword = Keyword::new("ZPCOUNT");
pub const ZGCOUNT: Keyword = Keyword::new("ZGCOUNT");
pub const ZHECKSUM: Keyword = Keyword::new("ZHECKSUM");
pub const ZDATASUM: Keyword = Keyword::new("ZDATASUM");
pub const ZQUANTIZ: Keyword = Keyword::new("ZQUANTIZ");
pub const ZDITHER0: Keyword = Keyword::new("ZDITHER0");
pub const ZBLANK: Keyword = Keyword::new("ZBLANK");
pub const ZSCALE: Keyword = Keyword::new("ZSCALE");
pub const ZZERO: Keyword = Keyword::new("ZZERO");
pub const ZHEAPPTR: Keyword = Keyword::new("ZHEAPPTR");
pub const ZTABLE: Keyword = Keyword::new("ZTABLE");
pub const ZTILELEN: Keyword = Keyword::new("ZTILELEN");
pub const ZFORM: Keyword = Keyword::new("ZFORM");
pub const ZCTYP: Keyword = Keyword::new("ZCTYP");
pub const ZTHEAP: Keyword = Keyword::new("ZTHEAP");
