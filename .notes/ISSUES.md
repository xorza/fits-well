# Issues

- An ASCII-table text column reads back with the blanks that pad each field to its
  width: an `AsciiText` of `"Vega"` written to an 8-wide column reads as `"Vega    "`,
  so a written `AsciiText` does not read back equal. cfitsio and astropy strip the
  trailing blanks of a string field.
