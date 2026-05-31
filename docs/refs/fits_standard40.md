# **Definition of the Flexible Image Transport System** **( FITS )**

> Verbatim conversion of the **FITS 4.0 Standard** (`fits_standard40.pdf`, IAU FITS Working Group, 13 Aug 2018) from PDF to markdown via PyMuPDF. Headings, the table of contents below, and blank-line tidying were reconstructed mechanically; body text, tables, and equations are as extracted. For curated, implementation-oriented notes see [`README.md`](README.md). The PDF remains the normative source.

## Contents

- [1. Introduction](#1-introduction)
  - [1.1. Brief history of FITS](#11-brief-history-of-fits)
  - [1.2. Version history of this document](#12-version-history-of-this-document)
  - [1.3. Acknowledgments](#13-acknowledgments)
- [2. Definitions, acronyms, and symbols](#2-definitions-acronyms-and-symbols)
  - [2.1. Conventions used in this document](#21-conventions-used-in-this-document)
  - [2.2. Defined terms](#22-defined-terms)
- [3. FITS file organization](#3-fits-file-organization)
  - [3.1. Overall file structure](#31-overall-file-structure)
  - [3.2. Individual FITS Structures](#32-individual-fits-structures)
  - [3.3. Primary header and data unit](#33-primary-header-and-data-unit)
  - [3.4. Extensions](#34-extensions)
  - [3.5. Special records (restricted use)](#35-special-records-restricted-use)
  - [3.6. Physical blocking](#36-physical-blocking)
  - [3.7. Restrictions on changes](#37-restrictions-on-changes)
- [4. Headers](#4-headers)
  - [4.1. Keyword records](#41-keyword-records)
  - [4.2. Value](#42-value)
  - [4.3. Units](#43-units)
  - [4.4. Keywords](#44-keywords)
- [5. Data representation](#5-data-representation)
  - [5.1. Characters](#51-characters)
  - [5.2. Integers](#52-integers)
  - [5.3. IEEE-754 floating point](#53-ieee-754-floating-point)
  - [5.4. Time](#54-time)
- [6. Random-groups structure](#6-random-groups-structure)
  - [6.1. Keywords](#61-keywords)
  - [6.2. Data sequence](#62-data-sequence)
  - [6.3. Data representation](#63-data-representation)
- [7. Standard extensions](#7-standard-extensions)
  - [7.1. Image extension](#71-image-extension)
  - [7.2. The ASCII-table extension](#72-the-ascii-table-extension)
  - [7.3. Binary-table extension](#73-binary-table-extension)
- [8. World-coordinate systems](#8-world-coordinate-systems)
  - [8.1. Basic concepts](#81-basic-concepts)
  - [8.2. World-coordinate-system representations](#82-world-coordinate-system-representations)
  - [8.3. Celestial-coordinate-system representations](#83-celestial-coordinate-system-representations)
  - [8.4. Spectral-coordinate-system representations](#84-spectral-coordinate-system-representations)
  - [8.5. Conventional-coordinate types](#85-conventional-coordinate-types)
- [9. Representations of time coordinates](#9-representations-of-time-coordinates)
  - [9.1. Time values](#91-time-values)
  - [9.2. Time coordinate frame](#92-time-coordinate-frame)
  - [9.3. Time unit](#93-time-unit)
  - [9.4. Time offset, binning, and errors](#94-time-offset-binning-and-errors)
  - [9.5. Global time keywords](#95-global-time-keywords)
  - [9.6. Other time-coordinate axes](#96-other-time-coordinate-axes)
  - [9.7. Durations](#97-durations)
  - [9.8. Recommended best practices](#98-recommended-best-practices)
- [10. Representations of compressed data](#10-representations-of-compressed-data)
  - [10.1. Tiled image compression](#101-tiled-image-compression)
  - [10.2. Quantization of floating-point data](#102-quantization-of-floating-point-data)
  - [10.3. Tiled table compression](#103-tiled-table-compression)
  - [10.4. Compression algorithms](#104-compression-algorithms)
- [Appendix A. Syntax of keyword records](#appendix-a-syntax-of-keyword-records)
- [Appendix B. Suggested time-scale specification](#appendix-b-suggested-time-scale-specification)
- [Appendix C. Summary of keywords](#appendix-c-summary-of-keywords)
- [Appendix D. ASCII text](#appendix-d-ascii-text)
- [Appendix E. IEEE floating-point formats](#appendix-e-ieee-floating-point-formats)
- [Appendix F. Reserved extension type names](#appendix-f-reserved-extension-type-names)
- [Appendix G. MIME types](#appendix-g-mime-types)
- [Appendix H. Past changes or clarifications to the formal definition of FITS](#appendix-h-past-changes-or-clarifications-to-the-formal-definition-of-fits)
- [Appendix I. Random-number generator](#appendix-i-random-number-generator)
- [Appendix J. CHECKSUM implementation guidelines](#appendix-j-checksum-implementation-guidelines)
- [Appendix K. Header inheritance convention](#appendix-k-header-inheritance-convention)
- [Appendix L. Green Bank convention](#appendix-l-green-bank-convention)
- [References](#references)
- [Index](#index)

<a id="1-introduction"></a>
## 1. Introduction

_An archival format must be utterly portable and self-_
_describing, on the assumption that, apart from the tran-_
_scription device, neither the software nor the hardware_
_that wrote the data will be available when the data_
_are read._ ‘Preserving Scientific Data on our Physical
Universe,’ p. 60. Steering Committee for the Study
on the Long-Term Retention of Selected Scientific and
Technical Records of the Federal Government, [US]
National Research Council, National Academy Press
1995.

This document, hereafter referred to as the ‘Standard’, describes the Flexible Image Transport System ( _FITS_ ), which is
the standard archival data format for astronomical data sets.
Although _FITS_ was originally designed for transporting image
data on magnetic tape (which accounts for the ‘I’ and ‘T’ in the
name), the capabilities of the _FITS_ format have expanded to accommodate more-complex data structures. The role of _FITS_ has
also grown from simply a way to transport data between different analysis software systems into the preferred format for data
in astronomical archives, as well as the on-line analysis format
used by many software packages.
This standard is intended as a formal codification of the
_FITS_ format, which has been endorsed by the International
Astronomical Union (IAU) for the interchange of astronomical
data (IAU 1983). It is fully consistent with all actions and endorsements of the IAU _FITS_ Working Group (IAUFWG), which
was appointed by Commission 5 of the IAU to oversee further
development of the _FITS_ format. In particular, this standard defines the organization and content of the header and data units for
all standard _FITS_ data structures: the primary array, the randomgroups structure, the image extension, the ASCII-table extension, and the binary-table extension. It also specifies minimum
structural requirements and general principles governing the creation of new extensions. For headers, it specifies the proper syntax for keyword records and defines required and reserved keywords. For data, it specifies character- and numeric-value representations and the ordering of contents within the byte stream.
One important feature of the _FITS_ format is that its structure, down to the bit level, is completely specified in documents
(such as this standard), many of which have been published in
refereed scientific journals. Given these documents, which are
readily available in hard copy form in libraries around the world
as well as in electronic form on the Internet, future researchers
should be able to decode the stream of bytes in any _FITS_ format
data file. In contrast, many other current data formats are only
implicitly defined by the software that reads and writes the files.
If that software is not continually maintained so that it can be
run on future computer systems, then the information encoded
in those data files could be lost.

<a id="11-brief-history-of-fits"></a>
### 1.1. Brief history of FITS

The _FITS_ format evolved out of the recognition that a standard
format was needed for transferring astronomical images from
one research institution to another. The first prototype developments of a universal interchange format that would eventually lead to the definition of the _FITS_ format began in 1976 between Don Wells at KPNO and Ron Harten at the Netherlands
Foundation for Research in Astronomy (NFRA). This need for

an image interchange format was raised at a meeting of the
Astronomy section of the U.S. National Science Foundation in
January 1979, which led to the formation of a task force to work
on the problem. Most of the technical details of the first basic
_FITS_ agreement (with files consisting of only a primary header
followed by a data array) were subsequently developed by Don
Wells and Eric Greisen (NRAO) in March 1979. After further
refinements, and successful image interchange tests between observatories that used widely different types of computer systems,
the first papers that defined the _FITS_ format were published in
1981 (Wells et al. 1981; Greisen & Harten 1981). The _FITS_ format quickly became the de facto standard for data interchange
within the astronomical community (mostly on nine-track magnetic tape at that time), and was officially endorsed by the IAU
in 1982 (IAU 1983). Most national and international astronomical projects and organizations subsequently adopted the _FITS_
format for distribution and archiving of their scientific data products. Some of the highlights in the developmental history of _FITS_
are shown in Table 1.

<a id="12-version-history-of-this-document"></a>
### 1.2. Version history of this document

The fundamental definition of the _FITS_ format was originally
contained in a series of published papers (Wells et al. 1981;
Greisen & Harten 1981; Grosbøl et al. 1988; Harten et al.
1988). As _FITS_ became more widely used, the need for a single document to unambiguously define the requirements of the
_FITS_ format became apparent. In 1990, the NASA Science
Office of Standards and Technology (NOST) at the Goddard
Space Flight Center provided funding for a technical panel to
develop the first version of this Standard document. As shown
in Table 2, the NOST panel produced several draft versions,
culminating in the first NOST standard document, NOST 1001.0, in 1993. Although this document was developed under a
NASA accreditation process, it was subsequently formally approved by the IAUFWG, which is the international control authority for the _FITS_ format. The small update to the Standard in
1995 (NOST 100-1.1) added a recommendation on the physical
units of header keyword values.
The NOST technical panel was convened a second time to
make further updates and clarifications to the Standard, resulting in the NOST 100-2.0 version, which was approved by the
IAUFWG in 1999 and published in 2001 (Hanisch et al. 2001).
In 2005, the IAUFWG formally approved the variable-length
array convention in binary tables, and a short time later approved support for the 64-bit integers data type. New versions
of the Standard were released to reflect both of these changes
(Versions IAUFWG 2.1 and IAUFWG 2.1b).

In early 2007 the IAUFWG appointed its own technical panel
to consider further modifications and updates to the Standard.
The changes proposed by this panel, which were ultimately approved in 2008 by the IAUFWG after a formal public review
process, are shown in the Version 3.0 of the document, published
in Pence et al. (2010).
Since 2006 a Registry for _FITS_ conventions submitted by the
community was established under the care of the IAUFWG at
[http://fits.gsfc.nasa.gov/fits_registry.html. The](http://fits.gsfc.nasa.gov/fits_registry.html)
Registry was intended as a repository of documentation of usages, which, although not endorsed as part of the _FITS_ Standard,
are otherwise perfectly legal usages of _FITS_ . In 2014 a small
team was formed to evaluate the possible incorporation of some

1

Table 1: Significant milestones in the development of _FITS_ .

Date Milestone Section
1979 Initial _FITS_ Agreement and first interchange of files
1981 Published original (single HDU) definition (Wells et al. 1981)
1981 Published random-groups definition (Greisen & Harten 1981) Sect. 6
1982 Formally endorsed by the IAU (IAU 1983)
1988 Defined rules for multiple extensions (Grosbøl et al. 1988)
1988 IAU _FITS_ Working Group (IAUFWG) established
1988 Extended to include ASCII-table extensions (Harten et al. 1988) Sect. 7.2
1988 Formal IAU approval of ASCII tables (IAU 1988) Sect. 7.2
1990 Extended to include IEEE floating-point data (Wells & Grosbøl 1990) Sect. 5.3
1994 Extended to multiple IMAGE-array extensions (Ponz et al. 1994) Sect. 7.1
1995 Extended to binary-table extensions (Cotton et al. 1995) Sect. 7.3
1997 Adopted four-digit-year date format (Bunclark & Rots 1997) Sect. 4.4.2
2002 Adopted proposals for world-coordinate systems (Greisen & Calabretta 2002) Sect. 8
2002 Adopted proposals for celestial coordinates (Calabretta & Greisen 2002) Sect. 8.3
2004 Adopted MIME types for _FITS_ data files (Allen & Wells 2005) App. G
2005 Extended to support variable-length arrays in binary tables Sect. 7.3.5
2005 Adopted proposals for spectral-coordinate systems (Greisen et al. 2006) Sect. 8.4
2005 Extended to include 64-bit integer data type Sect. 5.2.4
2006 Adopted WCS HEALPix projection (Calabretta & Roukema 2007) Sect. 8.3
2006 Established _FITS_ convention registry
2014 Adopted proposals for time coordinates (Rots et al. 2015) Sect. 9
2016 Adopted proposals for compressed data Sect. 10
2016 Adopted various registered conventions App. H.3
2018 General language editing App. H.4

Table 2: Version history of the Standard.

Version Date Status

NOST 100-0.1 1990 December First Draft Standard

NOST 100-0.2 1991 June Second Revised Draft Standard

NOST 100-0.3 1991 December Third Revised Draft Standard

NOST 100-1.0 1993 June NOST Standard
NOST 100-1.1 1995 September NOST Standard
NOST 100-2.0 1999 March NOST Standard
IAUFWG 2.1 2005 April IAUFWG Standard
IAUFWG 2.1b 2005 December IAUFWG Standard
IAUFWG 3.0 2008 July IAUFWG Standard
IAUFWG 4.0 2016 July IAUFWG Standard (approved)
IAUFWG 4.0 2018 August IAUFWG Standard (language-edited)

conventions within the Standard, while another small team was
in charge to update the Standard document with a summary of
the WCS time representation (Rots et al. 2015), which in the
meanwhile had been voted natively as part of the _FITS_ Standard.

Details on the conventions that have been incorporated into
this latest version of the Standard (CONTINUE long-string keywords, blank header space, CHECKSUM, column limits, tiled image and table compression) or only briefly mentioned (keyword inheritance and Green Bank conventions) are described in
Appendix H.3, which also lists the corresponding affected sections of the Standard.

After the approval by the IAUFWG in July 2016 the
Standard was subjected to a thorough language editing (with no
impact on the technical prescriptions) before the final issue in
2018. Details about the language editing changes are provided
in Appendix H.4.

The latest version of the Standard, as well as other information about the _FITS_ format, can be obtained from the _FITS_
[Support Office website at http://fits.gsfc.nasa.gov. This](http://fits.gsfc.nasa.gov)
website also contains the contact information for the Chairman

2

of the IAUFWG, to whom any questions or comments regarding
this Standard should be addressed.

<a id="13-acknowledgments"></a>
### 1.3. Acknowledgments

The members of the three technical panels that produced this
Standard are shown below.

**First technical panel, 1990–1993**
Robert J. Hanisch (Chair) Space Telescope Science Inst.
Lee E. Brotzman Hughes STX
Edward Kemper Hughes STX
Barry M. Schlesinger Raytheon STX
Peter J. Teuben University of Maryland
Michael E. Van SteenbergNASA Goddard SFC
Wayne H. Warren Jr. Hughes STX
Richard A. White NASA Goddard SFC

**Second technical panel, 1994–1999**
Robert J. Hanisch (Chair) Space Telescope Science Inst.
Allen Farris Space Telescope Science Inst.

Eric W. Greisen National Radio Astr. Obs.

William D. Pence NASA Goddard SFC
Barry M. Schlesinger Raytheon STX
Peter J. Teuben University of Maryland
Randall W. Thompson Computer Sciences Corp.
Archibald Warnock A/WWW Enterprises

**Third technical panel, 2007**
William D. Pence (Chair) NASA Goddard SFC
Lucio Chiappetti IASF Milano, INAF, Italy
Clive G. Page University of Leicester, UK
Richard Shaw National Optical Astr. Obs.
Elizabeth Stobie University of Arizona

**Dedicated task forces, 2013-2016**
Lucio Chiappetti IASF Milano, INAF, Italy
Steve Allen UCO Lick Observatory
Adam Dobrzycki European Southern Observatory
William D. Pence NASA Goddard SFC

Arnold Rots Harvard Smithsonian CfA
Richard Shaw National Optical Astr. Obs.
William T. Thompson NASA Goddard SFC

**Language editing, 2016-2018**
Malcolm J. Currie Rutherford Appleton Lab, UK
Lucio Chiappetti IASF Milano, INAF, Italy

<a id="2-definitions-acronyms-and-symbols"></a>
## 2. Definitions, acronyms, and symbols

<a id="21-conventions-used-in-this-document"></a>
### 2.1. Conventions used in this document

Terms or letters set in Courier typeface represent literal
strings that appear in _FITS_ files. In the case of keyword names,
such as ‘NAXIS _n_ ’, the lower-case letter represents a positive integer index number, generally in the range 1 to 999. The emphasized words _must_, _shall_, _should_, _may_, _recommended_, _required_,
and _optional_ in this document are to be interpreted as described
in IETF standard, RFC 2119 (Bradner 1997).

<a id="22-defined-terms"></a>
### 2.2. Defined terms

␣ Used to designate an ASCII space character.
**ANSI** American National Standards Institute.
**Array** A sequence of data values. This sequence corresponds to
the elements in a rectilinear, _n_ -dimensional matrix (1 ≤ _n_ ≤
999, or _n_ = 0 in the case of a null array).
**Array value** The value of an element of an array in a _FITS_ file,
without the application of the associated linear transformation to derive the physical value.
**ASCII** American National Standard Code for Information
Interchange.
**ASCII character** Any member of the seven-bit ASCII charac
ter set.
**ASCII digit** One of the ten ASCII characters ‘0’ through ‘9’,
which are represented by decimal character codes 48 through
57 (hexadecimal 30 through 39).
**ASCII NULL** The ASCII character that has all eight bits set to

zero.
**ASCII space** The ASCII character for space, which is represented by decimal 32 (hexadecimal 20).

**ASCII text** The restricted set of ASCII characters decimal 32
through 126 (hexadecimal 20 through 7E).
**Basic** _**FITS**_ The _FITS_ structure consisting of the primary
header followed by a single primary data array. This is also
known as Single Image _FITS_ (SIF), as opposed to MultiExtension _FITS_ (MEF) files that contain one or more extensions following the primary HDU.
**Big endian** The numerical data format used in _FITS_ files in
which the most-significant byte of the value is stored first
followed by the remaining bytes in order of significance.
**Bit** A single binary digit.
**Byte** An ordered sequence of eight consecutive bits treated as a
single entity.
**Card image** An obsolete term for an 80-character keyword
record derived from the 80-column punched computer cards
that were prevalent in the 1960s and 1970s.
**Character string** A sequence of one or more of the restricted
set of ASCII-text characters, decimal 32 through 126 (hexadecimal 20 through 7E).
**Conforming extension** An extension whose keywords and organization adhere to the requirements for conforming extensions defined in Sect. 3.4.1 of this Standard.
**Data block** A 2880-byte _FITS_ block containing data described
by the keywords in the associated header of that HDU.
**Deprecate** To express disapproval of. This term is used to refer
to obsolete structures that _should not_ be used in new _FITS_
files, but which _shall_ remain valid indefinitely.
**Entry** A single value in an ASCII-table or binary-table standard
extension.
**Extension** A _FITS_ HDU appearing after the primary HDU in a
_FITS_ file.
**Extension type name** The value of the XTENSION keyword,
used to identify the type of the extension.
**Field** A component of a larger entity, such as a keyword record
or a row of an ASCII-table or binary-table standard extension. A field in a table-extension row consists of a set of
zero-or-more table entries collectively described by a single
format.
**File** A sequence of one or more records terminated by an endof-file indicator appropriate to the medium.
_**FITS**_ Flexible Image Transport System.
_**FITS**_ **block** A sequence of 2880 eight-bit bytes aligned on
2880-byte boundaries in the _FITS_ file, most commonly either
a header block or a data block. Special records are another
infrequently used type of _FITS_ block. This block length was
chosen because it is evenly divisible by the byte and word
lengths of all known computer systems at the time _FITS_ was
developed in 1979.
_**FITS**_ **file** A file with a format that conforms to the specifications
in this document.
_**FITS**_ **structure** One of the components of a _FITS_ file: the primary HDU, the random-groups records, an extension, or,
collectively, the special records following the last extension.
_**FITS**_ **Support O** ffi **ce** The _FITS_ information website that is
maintained by the IAUFWG and is currently hosted at
[http://fits.gsfc.nasa.gov.](http://fits.gsfc.nasa.gov)
**Floating point** A computer representation of a real number.
**Fraction** The field of the mantissa (or significand) of a floatingpoint number that lies to the right of its implied binary point.
**Group parameter value** The value of one of the parameters
preceding a group in the random-groups structure, without
the application of the associated linear transformation.

3

**HDU** Header and Data Unit. A data structure consisting of a
header and the data the header describes. Note that an HDU
_may_ consist entirely of a header with no data blocks.
**Header** A series of keyword records organized within one or
more header blocks that describes structures and/or data that
follow it in the _FITS_ file.
**Header block** A 2880-byte _FITS_ block containing a sequence
of thirty-six 80-character keyword records.
**Heap** The supplemental data area following the main data table
in a binary-table standard extension.
**IAU** International Astronomical Union.
**IAUFWG** International Astronomical Union _FITS_ Working
Group.
**IEEE** Institute of Electrical and Electronic Engineers.
**IEEE NaN** IEEE Not-a-Number value; used to represent undefined floating-point values in _FITS_ arrays and binary tables.
**IEEE special values** Floating-point number byte patterns
that have a special, reserved meaning, such as −0, ±∞,
±underflow, ±overflow, ±denormalized, ±NaN. (See
Appendix E).
**Indexed keyword** A keyword name that is of the form of a
fixed root with an appended positive integer index number.
**Keyword name** The first eight bytes of a keyword record,
which contain the ASCII name of a metadata quantity (unless it is blank).
**Keyword record** An 80-character record in a header block consisting of a keyword name in the first eight characters followed by an _optional_ value indicator, value, and comment
string. The keyword record _shall_ be composed only of the
restricted set of ASCII-text characters ranging from decimal
32 to 126 (hexadecimal 20 to 7E).
**Mandatory keyword** A keyword that _must_ be used in all _FITS_
files or a keyword _required_ in conjunction with particular
_FITS_ structures.
**Mantissa** Also known as significand. The component of an
IEEE floating-point number consisting of an explicit or implicit leading bit to the left of its implied binary point and a
fraction field to the right.
**MEF** Multi-Extension _FITS_, i.e., a _FITS_ file containing a primary HDU followed by one or more extension HDUs.
**NOST** NASA/Science Office of Standards and Technology.
**Physical value** The value in physical units represented by an
element of an array and possibly derived from the array value
using the associated, but _optional_, linear transformation.
**Pixel** Short for ‘Picture element’; a single location within an

array.
**Primary data array** The data array contained in the primary
HDU.
**Primary HDU** The first HDU in a _FITS_ file.
**Primary header** The first header in a _FITS_ file, containing information on the overall contents of the file (as well as on the
primary data array, if present).
**Random Group** A _FITS_ structure consisting of a collection of
‘groups’, where a group consists of a subarray of data and a
set of associated parameter values. Random groups are deprecated for any use other than for radio interferometry data.
**Record** A sequence of bits treated as a single logical entity.
**Repeat count** The number of values represented in a field in a
binary-table standard extension.
**Reserved keyword** An _optional_ keyword that _must_ be used
only in the manner defined in this Standard.

4

**SIF** Single Image _FITS_, i.e., a _FITS_ file containing only a primary HDU, without any extension HDUs. Also known as
Basic _FITS_ .
**Special records** A series of one or more _FITS_ blocks following
the last HDU whose internal structure does not otherwise
conform to that for the primary HDU or to that specified for
a conforming extension in this Standard. Any use of special records requires approval from the IAU _FITS_ Working
Group.
**Standard extension** A conforming extension whose header and
data content are completely specified in Sect. 7 of this
Standard, namely, an image extension, an ASCII-table extension, or a binary-table extension.

<a id="3-fits-file-organization"></a>
## 3. FITS file organization

<a id="31-overall-file-structure"></a>
### 3.1. Overall file structure

A _FITS_ file _shall_ be composed of the following _FITS_ structures,
in the order listed:

**–**
Primary header and data unit (HDU).

**–**
Conforming Extensions ( _optional_ ).

**–**
Other special records ( _optional_, restricted).

A _FITS_ file composed of only the primary HDU is sometimes
referred to as a Basic _FITS_ file, or a Single Image _FITS_ (SIF)
file, and a _FITS_ file containing one or more extensions following
the primary HDU is sometimes referred to as a Multi-Extension
_FITS_ (MEF) file.
Each _FITS_ structure _shall_ consist of an integral number of
_FITS_ blocks, which are each 2880 bytes (23040 bits) in length.
The primary HDU _shall_ start with the first _FITS_ block of the
_FITS_ file. The first _FITS_ block of each subsequent _FITS_ structure _shall_ be the _FITS_ block immediately following the last _FITS_
block of the preceding _FITS_ structure.
This Standard neither imposes a limit on the total size of a
_FITS_ file, nor imposes a limit on the size of an individual HDU
within a _FITS_ file. Software packages that read or write data according to this Standard could be limited, however, in the size
of files that are supported. In particular, some software systems
have historically only supported files up to 2 [31] bytes in size (approximately 2.1 × 10 [9] bytes).

<a id="32-individual-fits-structures"></a>
### 3.2. Individual FITS Structures

The primary HDU and every extension HDU _shall_ consist of
one or more 2880-byte header blocks immediately followed by
an _optional_ sequence of associated 2880-byte data blocks. The
header blocks _shall_ contain only the restricted set of ASCII-text
characters, decimal 32 through 126 (hexadecimal 20 through
7E). The ASCII control characters with decimal values less than
32 (including the null, tab, carriage return, and line-feed characters), and the delete character (decimal 127 or hexadecimal 7F)
_must not_ appear anywhere within a header block.

<a id="33-primary-header-and-data-unit"></a>
### 3.3. Primary header and data unit

The first component of a _FITS_ file _shall_ be the primary HDU,
which always contains the primary header and _may_ be followed
by the primary data array. If the primary data array has zero
length, as determined by the values of the NAXIS and NAXIS _n_

...,

...,

...,

...,

Fig. 1: Arrays of more than one dimension _shall_ consist of a sequence such that the index along Axis 1 varies most rapidly and
those along subsequent axes progressively less rapidly.

keywords in the primary header (Sect. 4.4.1), then the primary
HDU _shall_ contain no data blocks.

<a id="331-primary-header"></a>
#### 3.3.1. Primary header

The header of a primary HDU _shall_ consist of one or more
header blocks, each containing a series of 80-character keyword
records containing only the restricted set of ASCII-text characters. Each 2880-byte header block contains 36 keyword records.
The last header block _must_ contain the END keyword (defined in
Sect. 4.4.1), which marks the logical end of the header. Keyword
records without information (e.g., following the END keyword)
_shall_ be filled with ASCII spaces (decimal 32 or hexadecimal
20).

<a id="332-primary-data-array"></a>
#### 3.3.2. Primary data array

The primary data array, if present, _shall_ consist of a single data
array with from 1 to 999 dimensions (as specified by the NAXIS
keyword defined in Sect. 4.4.1). The random-groups convention
in the primary data array is a more-complicated structure and
is discussed separately in Sect. 6. The entire array of data values are represented by a continuous stream of bits starting with
the first bit of the first data block. Each data value _shall_ consist of a fixed number of bits that is determined by the value of
the BITPIX keyword (Sect. 4.4.1). Arrays of more than one dimension _shall_ consist of a sequence such that the index along
Axis 1 varies most rapidly, that along Axis 2 next most rapidly,
and those along subsequent axes progressively less rapidly, with
that along Axis m, where m is the value of NAXIS, varying least
rapidly. There is no space or any other special character between
the last value on a row or plane and the first value on the next
row or plane of a multi-dimensional array. Except for the location of the first element, the array structure is independent of the
_FITS_ block structure. This storage order is shown schematically
in Fig. 1 and is the same order as in multi-dimensional arrays in
the Fortran programming language (ISO 2004). The index count
along each axis _shall_ begin with 1 and increment by 1 up to the
value of the NAXIS _n_ keyword (Sect. 4.4.1).
If the data array does not fill the final data block, the remainder of the data block _shall_ be filled by setting all bits to zero.

The individual data values _shall_ be stored in big-endian byte order such that the byte containing the most-significant bits of the
value appears first in the _FITS_ file, followed by the remaining
bytes, if any, in decreasing order of significance.

<a id="34-extensions"></a>
### 3.4. Extensions

<a id="341-requirements-for-conforming-extensions"></a>
#### 3.4.1. Requirements for conforming extensions

All extensions, whether or not further described in this Standard,
_shall_ fulfill the following requirements to be in conformance
with this _FITS_ Standard. New extension types _should_ be created only when the organization of the information is such that
it cannot be handled by one of the existing extension types. A
_FITS_ file that contains extensions is commonly referred to as a
multi-extension _FITS_ (MEF) file.

<a id="3411-identity"></a>
##### 3.4.1.1. Identity

Each extension type _shall_ have a unique type name, specified in the header by the XTENSION keyword (Sect. 4.4.1). To
preclude conflict, extension type names _must_ be registered with
the IAUFWG. The current list of registered extensions is given
in Appendix F. An up-to-date list is also maintained on the _FITS_
Support Office website.

<a id="3412-size-specification"></a>
##### 3.4.1.2. Size specification

The total number of bits in the data of each extension
_shall_ be specified in the header for that extension, in the manner
prescribed in Sect. 4.4.1.

<a id="342-standard-extensions"></a>
#### 3.4.2. Standard extensions

A standard extension is a conforming extension whose organization and content are completely specified in Sect. 7 of this
Standard. Only one extension format _shall_ be approved for each
type of data organization.

<a id="343-order-of-extensions"></a>
#### 3.4.3. Order of extensions

An extension _may_ follow the primary HDU or another conforming extension. Standard extensions and other conforming extensions _may_ appear in any order in a _FITS_ file.

<a id="35-special-records-restricted-use"></a>
### 3.5. Special records (restricted use)

Special records are 2880-byte _FITS_ blocks following the last
HDU of the _FITS_ file that have an unspecified structure that
does not meet the requirements of a conforming extension. The
first eight bytes of the special records _must not_ contain the string
‘XTENSION’. It is _recommended_ that they do not contain the
string ‘SIMPLE␣␣’. The contents of special records are not otherwise specified by this Standard.
Special records were originally designed as a way for the
_FITS_ format to evolve by allowing new _FITS_ structures to be
implemented. Following the development of conforming extensions, which provide a general mechanism for storing different
types of data structures in _FITS_ format in a well defined manner,
the need for other new types of _FITS_ data structures has been

5

greatly reduced. Consequently, further use of special records is
restricted and requires the approval of the IAU _FITS_ Working
Group.

<a id="36-physical-blocking"></a>
### 3.6. Physical blocking

<a id="361-bit-stream-devices"></a>
#### 3.6.1. Bit-stream devices

For bit-stream devices, including but not restricted to logical file
systems, _FITS_ files _shall_ be interpreted as a sequence of one or
more 2880-byte _FITS_ blocks, regardless of the physical blocking structure of the underlying recording media. When writing
a _FITS_ file on media with a physical block size unequal to the
2880-byte _FITS_ block length, any bytes remaining in the last
physical block following the end of the _F_ ITS file _should_ be set
to zero. Similarly, when reading _FITS_ files on such media, any
bytes remaining in the last physical block following the end of
the _FITS_ file _shall_ be disregarded.

<a id="362-sequential-media"></a>
#### 3.6.2. Sequential media

The _FITS_ format was originally developed for writing files on
sequential magnetic-tape devices. The following rules on how
to write to sequential media (Grosbøl & Wells 1994) are now
irrelevant to most current data-storage devices.
If physically possible, _FITS_ files _shall_ be written on sequential media in blocks that are from one to ten integer multiples of
2880 bytes in length. If this is not possible, the _FITS_ file _shall_ be
written as a bit stream using the native block size of the sequential device. Any bytes remaining in the last block following the
end of the _FITS_ file _shall_ be set to zero.
When reading _FITS_ files on sequential media, any files
shorter than 2880 bytes in length (e.g., ANSI tape labels) are
not considered part of the _FITS_ files and _should_ be disregarded.

<a id="37-restrictions-on-changes"></a>
### 3.7. Restrictions on changes

Any structure that is a valid _FITS_ structure _shall_ remain a valid
_FITS_ structure at all future times. Use of certain valid _FITS_ structures _may_ be deprecated by this or future _FITS_ Standard docu
ments.

<a id="4-headers"></a>
## 4. Headers

The first two sections of this chapter define the structure and
content of header keyword records. Sect. 4.3 offers recommendations on how physical units should be expressed. The final
section defines the mandatory and reserved keywords for primary arrays and conforming extensions.

<a id="41-keyword-records"></a>
### 4.1. Keyword records

<a id="411-syntax"></a>
#### 4.1.1. Syntax

Each 80-character header keyword record _shall_ consist of a keyword name, a value indicator (only _required_ if a value is present),
an _optional_ value, and an _optional_ comment. Keywords _may_ appear in any order except where specifically stated otherwise in
this Standard. It is _recommended_ that the order of the keywords
in _FITS_ files be preserved during data processing operations
because the designers of the _FITS_ file may have used conventions that attach particular significance to the order of certain

6

keywords (e.g., by grouping sequences of COMMENT keywords
at specific locations in the header, or appending HISTORY keywords in chronological order of the data processing steps or using CONTINUE keywords to generate long-string keyword values).
A formal syntax, giving a complete definition of the syntax
of _FITS_ keyword records, is given in Appendix A. It is intended
as an aid in interpreting the text defining the Standard.
In earlier versions of this Standard a _FITS_ keyword, assumed
as an item whose value is to be looked up by name (and presumably assigned to a variable) by a _FITS_ -reading program,
was associated one to one to a single header keyword record.
With the introduction of continued (long-string) keywords (see
Sect. 4.2.1.2), such _FITS_ keywords _may_ span more than one
header keyword record, and the value _shall_ be created by concatenation as explained in Sect. 4.2.1.2.

<a id="412-components"></a>
#### 4.1.2. Components

<a id="4121-keyword-name-bytes-1-through-8"></a>
##### 4.1.2.1. Keyword name (Bytes 1 through 8)

The keyword name _shall_ be a left justified, eight-character,
space-filled, ASCII string with no embedded spaces. All digits
0 through 9 (decimal ASCII codes 48 to 57, or hexadecimal 30
to 39) and upper case Latin alphabetic characters ’A’ through
’Z’ (decimal 65 to 90 or hexadecimal 41 to 5A) are permitted;
lower-case characters _shall not_ be used. The underscore (’ ’,
decimal 95 or hexadecimal 5F) and hyphen (’-’, decimal 45
or hexadecimal 2D) are also permitted. No other characters
are permitted. [3] For indexed keyword names that have a single
positive integer index counter appended to the root name,
the counter _shall not_ have leading zeros (e.g., NAXIS1, not
NAXIS001). Note that keyword names that begin with (or
consist solely of) any combination of hyphens, underscores, and
digits are legal.

<a id="4122-value-indicator-bytes-9-and-10"></a>
##### 4.1.2.2. Value indicator (Bytes 9 and 10)

If the two ASCII characters '=␣' (decimal 61 followed
by decimal 32) are present in Bytes 9 and 10 of the keyword
record, this indicates that the keyword has a value field associated with it, unless it is one of the commentary keywords
defined in Sect. 4.4.2 (i.e., a HISTORY, COMMENT, or completely
blank keyword name), which, by definition, have no value.

<a id="4123-valuecomment-bytes-11-through-80"></a>
##### 4.1.2.3. Value/comment (Bytes 11 through 80)

In keyword records that contain the value indicator in Bytes 9
and 10, the remaining Bytes 11 through 80 of the record _shall_
contain the value, if any, of the keyword, followed by _optional_
comments. In keyword records without a value indicator,
Bytes 9 through 80 _should_ be interpreted as commentary text,
however, this does not preclude conventions that interpret the
content of these bytes in other ways.
The value field, when present, _shall_ contain the ASCII-text
representation of a literal string constant, a logical constant, or
a numerical constant, in the format specified in Sect. 4.2. The
value field _may_ be a null field; i.e., it _may_ consist entirely of

3 This requirement differs from the wording in the original _FITS_ papers. See Appendix H.

spaces, in which case the value associated with the keyword is
undefined.
The mandatory _FITS_ keywords defined in this Standard
_must not_ appear more than once within a header. All other keywords that have a value _should not_ appear more than once. If a
keyword does appear multiple times with different values, then
the value is indeterminate.
If a comment follows the value field, it _must_ be preceded
by a slash (’/’, decimal 47 or hexadecimal 2F). [3] A space between the value and the slash is strongly _recommended_ . The
comment _may_ contain any of the restricted set of ASCII-text
characters, decimal 32 through 126 (hexadecimal 20 through
7E). The ASCII control characters with decimal values less than
32 (including the null, tab, carriage return, and line-feed characters), and the delete character (decimal 127 or hexadecimal 7F)
_must not_ appear anywhere within a keyword record.

<a id="42-value"></a>
### 4.2. Value

The structure of the value field depends on the data type of the
value. The value field represents a single value and not an array
of values. [3] The value field _must_ be in one of two formats: fixed
or free. The fixed-format is _required_ for values of mandatory
keywords and is _recommended_ for values of all other keywords.

<a id="421-character-string"></a>
#### 4.2.1. Character string

4.2.1.1 Single-record string keywords

A character-string value _shall_ be composed only of the set of
restricted ASCII-text characters, decimal 32 through 126 (hexadecimal 20 through 7E) enclosed by single-quote characters
(“’”, decimal 39, hexadecimal 27). A single quote is represented
within a string as two successive single quotes, e.g., O’HARA =
'O''HARA'. Leading spaces are significant; trailing spaces are
not. This Standard imposes no requirements on the case sensitivity of character string values unless explicitly stated in the
definition of specific keywords.
If the value is a fixed-format character string, the starting
single-quote character _must_ be in Byte 11 of the keyword record
and the closing single quote _must_ occur in or before Byte 80.
Earlier versions of this Standard also _required_ that fixed-format
characters strings _must_ be padded with space characters to at
least a length of eight characters so that the closing quote character does not occur before Byte 20. This minimum characterstring length is no longer _required_, except for the value of the
XTENSION keyword (e.g., 'IMAGE␣␣␣' and 'TABLE␣␣␣'; see
Sect. 7), which _must_ be padded to a length of eight characters
for backward compatibility with previous usage.
Free-format character strings follow the same rules as fixedformat character strings except that the starting single-quote
character _may_ occur after Byte 11. Any bytes preceding the starting quote character and after Byte 10 _must_ contain the space
character.
Note that there is a subtle distinction between the following
three keywords.

KEYWORD1= '' / null string keyword
KEYWORD2= ' ' / empty string keyword
KEYWORD3= / undefined keyword

The value of KEYWORD1 is a null, or zero-length string whereas
the value of the KEYWORD2 is an empty string (nominally a single
space character because the first space in the string is significant,
but trailing spaces are not). The value of KEYWORD3 is undefined
and has an indeterminate data type as well, except in cases where
the data type of the specified keyword is explicitly defined in this
Standard.
The maximum length of a string value that can be represented on a single keyword record is 68 characters, with the
opening and closing quote characters in Bytes 11 and 80, respectively. In general, no length limit fewer than 68 is implied
for character-valued keywords.
Whenever a keyword value is declared ‘string’ or said to
‘contain a character string’, the length limits in this section apply. The next section applies when the value is declared ‘longstring’.

4.2.1.2 Continued string (long-string) keywords

Earlier versions of this Standard only defined single-record
string keywords as described in the previous section. The
Standard now incorporates a convention (originally developed
for use in _FITS_ files from high-energy astrophysics missions) for
continuing arbitrarily long string values over a potentially unlimited sequence of multiple consecutive keyword records using the
following procedure.

1. Divide the long-string value into a sequence of smaller substrings, each of which contains fewer than 68 characters.
(Note that if the string contains any literal single-quote characters, then these _must_ be represented as a pair of singlequote characters in the _FITS_ -keyword value, and these two
characters _must_ both be contained within a single substring).
2. Append an ampersand character (’&’) to the end of each
substring, except for the last substring. This character serves
as a flag to _FITS_ -reading software that this string value _may_
be continued on the following keyword in the header.
3. Enclose each substring with single-quote characters. Nonsignificant space characters _may_ occur between the ampersand character and the closing quote character.
4. Write the first substring as the value of the specified keyword.
5. Write each subsequent substring, in order, to a series of keywords that all have the reserved keyword name CONTINUE
(see Sect. 4.4.2) in Bytes 1 through 8, and have space characters in Bytes 9 and 10 of the keyword record. The substring
_may_ be located anywhere in Bytes 11 through 80 of the keyword record and _may_ be preceded by non-significant space
characters starting in Byte 11. A comment string _may_ follow
the substring; if present, the comment string _must_ be separated from the substring by a forward-slash character (’/’).
Also, it is _strongly recommended_ that the slash character be
preceded by a space character.

The CONTINUE keyword _must not_ be used with of any of the
mandatory or reserved keywords defined in this Standard unless
explicitly declared of type long-string.
The following keyword records illustrate a string value that
is continued over multiple keyword records. (Note: the length of
the substrings have been reduced to fit within the page layout.)

WEATHER = 'Partly cloudy during the evening f&'

7

CONTINUE 'ollowed by cloudy skies overnight.&'
CONTINUE ' Low 21C. Winds NNE at 5 to 10 mph.'

If needed, additional space for the keyword comment field
can be generated by continuing the string value with one or more
null strings, as illustrated schematically below.

STRKEY = 'This keyword value is continued &'
CONTINUE ' over multiple keyword records.&'
CONTINUE '&' / The comment field for this
CONTINUE '&' / keyword is also continued
CONTINUE '' / over multiple records.

_FITS_ -reading software can reconstruct the long-string value
by following an inverse procedure of checking if the string value
ends with the ’&’ character and is immediately followed by a
conforming CONTINUE keyword record. If both conditions are
true, then concatenate the substring from the CONTINUE record
onto the previous substring after first deleting the trailing ’&’
character. Repeat these steps until all subsequent CONTINUE
records have been processed.
Note that if a string value ends with the ’&’ character, but
is not immediately followed by a CONTINUE keyword that conforms to all the previously described requirements, then the ’&’
character _should_ be interpreted as the literal last character in the
string. Also, any ‘orphaned’ CONTINUE keyword records (formally not invalidating the _FITS_ file, although likely representing an error with respect to what the author of the file meant)
_should_ be interpreted as containing commentary text in Bytes 9–
80 (similar to a COMMENT keyword).

<a id="422-logical"></a>
#### 4.2.2. Logical

If the value is a fixed-format logical constant, it _shall_ appear as
an upper-case T or F in Byte 30. A logical value is represented
in free-format by a single character consisting of an upper-case
T or F as the first non-space character in Bytes 11 through 80.

<a id="423-integer-number"></a>
#### 4.2.3. Integer number

If the value is a fixed-format integer, the ASCII representation
_shall_ be right-justified in Bytes 11 through 30. An integer consists of a ’+’ (decimal 43 or hexadecimal 2B) or ’-’ (decimal
45 or hexadecimal 2D) sign, followed by one or more contiguous
ASCII digits (decimal 48 to 57 or hexadecimal 30 to 39), with
no embedded spaces. The leading ’+’ sign is _optional_ . Leading
zeros are permitted, but are not significant. The integer representation _shall_ always be interpreted as a signed, decimal number.
This Standard does not limit the range of an integer keyword
value, however, software packages that read or write data according to this Standard could be limited in the range of values
that are supported (e.g., to the range that can be represented by
a 32-bit or 64-bit signed binary integer).
A free-format integer value follows the same rules as fixedformat integers except that the ASCII representation _may_ occur
anywhere within Bytes 11 through 80.

<a id="424-real-floating-point-number"></a>
#### 4.2.4. Real floating-point number

If the value is a fixed-format real floating-point number, the
ASCII representation _shall_ be right-justified in Bytes 11 through
30.

8

Table 3: IAU-recommended basic units.

Quantity Unit Meaning Notes

_SI base_ & _supplementary units_
length m meter
mass kg kilogram g gram allowed
time s second
plane angle rad radian
solid angle sr steradian
temperature K kelvin
electric current A ampere
amount of substance mol mole
luminous intensity cd candela

_IAU-recognized derived units_
frequency Hz hertz s [−][1]
energy J joule N m
power W watt J s [−][1]
electric potential V volt J C [−][1]

force N newton kg m s [−][2]

pressure, stress Pa pascal N m [−][2]
electric charge C coulomb A s
electric resistance Ohm ohm V A [−][1]

electric conductance S siemens A V [−][1]
electric capacitance F farad C V [−][1]
magnetic flux Wb weber V s
magnetic flux density T tesla Wb m [−][2]

inductance H henry Wb A [−][1]
luminous flux lm lumen cd sr
illuminance lx lux lm m [−][2]

A floating-point number is represented by a decimal number
followed by an _optional_ exponent, with no embedded spaces. A
decimal number _shall_ consist of a ’+’ (decimal 43 or hexadecimal 2B) or ’-’ (decimal 45 or hexadecimal 2D) sign, followed
by a sequence of ASCII digits containing a single decimal point
(’.’), representing an integer part and a fractional part of the
floating-point number. The leading ’+’ sign is _optional_ . At least
one of the integer part or fractional part _must_ be present. If the
fractional part is present, the decimal point _must_ also be present.
If only the integer part is present, the decimal point _may_ be omitted, in which case the floating-point number is indistinguishable
from an integer. The exponent, if present, consists of an exponent letter followed by an integer. Letters in the exponential form
(’E’ or ’D’) [4] _shall_ be upper case. The full precision of 64-bit
values cannot be expressed over the whole range of values using
the fixed-format. This Standard neither imposes an upper limit
on the number of digits of precision, nor any limit on the range
of floating-point keyword values. Software packages that read or
write data according to this Standard could be limited, however,
in the range of values and exponents that are supported (e.g., to
the range that can be represented by a 32-bit or 64-bit floatingpoint number).

A free-format floating-point value follows the same rules as
a fixed-format floating-point value except that the ASCII representation _may_ occur anywhere within Bytes 11 through 80.

4 The ’D’ exponent form is traditionally used when representing values that have more decimals of precision or a larger magnitude than can
be represented by a single-precision 32-bit floating-point number, but
otherwise there is no distinction between ’E’ or ’D’.

Table 4: Additional allowed units.

Quantity Unit Meaning Notes

plane angle deg degree of arc π/180 rad
arcmin minute of arc 1/60 deg
arcsec second of arc 1/3600 deg
mas milli-second of arc 1/3 600 000 deg
time min minute 60 s
h hour 60 min = 3600 s
d day 86 400 s

             - a year (Julian) 31 557 600 s (365.25 d), peta a(Pa) forbidden

             - yr year (Julian) a is IAU-style
energy [∗]        - eV electron volt 1.6021765 × 10 [−][19] J

             - erg erg 10 [−][7] J

2

1 2π _e_ [2] 2
Ry rydberg 2 � _hc_ � _m_ _e_ _c_ = 13.605692 eV

mass [∗] solMass solar mass 1.9891 × 10 [30] kg
u unified atomic mass unit 1.6605387 × 10 [−][27] kg
luminosity solLum Solar luminosity 3.8268 × 10 [26] W
length        - Angstrom angstrom 10 [−][10] m
solRad Solar radius 6.9599 × 10 [8] m
AU astronomical unit 1.49598 × 10 [11] m
lyr light year 9.460730 × 10 [15] m

             - pc parsec 3.0857 × 10 [16] m
events count count
ct count
photon photon
ph photon
flux density         - Jy jansky 10 [−][26] W m [−][2] Hz [−][1]

             - mag (stellar) magnitude

             - R rayleigh 10 [10] /(4π) photons m [−][2] s [−][1] sr [−][1]
magnetic field †‡ G gauss 10 [−][4] T
area pixel (image/detector) pixel
pix (image/detector) pixel

            - ‡ barn barn 10 [−][28] m [2]

_Miscellaneous units_

D debye 31 [×][ 10] [−][29] [ C.m]
Sun relative to Sun e.g., abundances
chan (detector) channel
bin numerous applications (including the one-dimensional analog of pixel)
voxel three-dimensional analog of pixel

             - bit binary information unit

             - byte (computer) byte eight bits
adu Analog-to-digital converter
beam beam area of observation as in Jy/beam

**Notes.** [(][†][)] Addition of prefixes for decimal multiples and submultiples are allowed. [(][‡][)] Deprecated in IAU Style Manual (McNally 1988) but still in
use. [(][∗][)] [Conversion factors from CODATA Internationally recommended values of the fundamental physical constants 2002 (http://physics.](http://physics.nist.gov/cuu/Constants/)
[nist.gov/cuu/Constants/).](http://physics.nist.gov/cuu/Constants/)

<a id="425-complex-integer-number"></a>
#### 4.2.5. Complex integer number

There is no fixed-format for complex integer numbers. [5]

If the value is a complex integer number, the value _must_ be
represented as a real part and an imaginary part, separated by
a comma and enclosed in parentheses e.g., (123, 45). Spaces
_may_ precede and follow the real and imaginary parts. The real
and imaginary parts are represented in the same way as integers
(Sect. 4.2.3). Such a representation is regarded as a single value
for the complex integer number. This representation _may_ be located anywhere within Bytes 11 through 80.

5 This requirement differs from the wording in the original _FITS_ papers. See Appendix H.

<a id="426-complex-floating-point-number"></a>
#### 4.2.6. Complex floating-point number

There is no fixed-format for complex floating-point numbers. [5]

If the value is a complex floating-point number, the value
_must_ be represented as a real part and an imaginary part,
separated by a comma and enclosed in parentheses, e.g.,
(123.23, -45.7). Spaces _may_ precede and follow the real
and imaginary parts. The real and imaginary parts are represented in the same way as floating-point values (Sect. 4.2.4).
Such a representation is regarded as a single value for the complex floating-point number. This representation _may_ be located
anywhere within Bytes 11 through 80.

9

<a id="427-date"></a>
#### 4.2.7. Date

There is strictly no such thing as a data type for _date valued_
keywords, however a pseudo data type of _datetime_ is defined in
Sect. 9.1.1 and _must_ be used to write ISO-8601 _datetime_ strings
as character strings.
If a keyword needs to express a _time_ in JD or MJD (see
Sect. 9), this can be formatted as an arbitrary precision number,
optionally separating the integer and fractional part as specified
in Sect. 9.2.2.

<a id="43-units"></a>
### 4.3. Units

When a numerical keyword value represents a physical quantity,
it is _recommended_ that units be provided. Units _shall_ be represented with a string of characters composed of the restricted
ASCII-text character set. Unit strings can be used as values of
keywords (e.g., for the reserved keywords BUNIT, and TUNIT _n_ ),
as an entry in a character-string column of an ASCII-table or
binary-table extension, or as part of a keyword comment string
(see Sect. 4.3.2, below).
The units of all _FITS_ header keyword values, with the exception of measurements of angles, _should_ conform with the recommendations in the IAU Style Manual (McNally 1988). For
angular measurements given as floating-point values and specified with reserved keywords, the units _should_ be degrees (i.e.,
deg). If a requirement exists within this Standard for the units of
a keyword, then those units _must_ be used.
The units for fundamental physical quantities recommended
by the IAU are given in Table 3. Table 4 lists additional units
that are commonly used in astronomy. Further specifications for
time units are given in Sect. 9.3. The recommended plain-text
form for the IAU-recognized _base units_ are given in Column 2
of both tables. [6] All base units strings _may_ be preceded, with no
intervening spaces, by a single character (two for deca) taken
from Table 5 and representing scale factors mostly in steps of
10 [3] . Compound prefixes (e.g., ZYeV for 10 [45] eV) _must not_ be
used.

<a id="431-construction-of-units-strings"></a>
#### 4.3.1. Construction of units strings

Compound units strings _may_ be formed by combining strings
of base units (including prefixes, if any) with the recommended
syntax described in Table 6. Two or more base units strings
(called str1 and str2 in Table 6) _may_ be combined using
the restricted set of (explicit or implicit) operators that provide
for multiplication, division, exponentiation, raising arguments to
powers, or taking the logarithm or square-root of an argument.
Note that functions such as log actually require dimensionless arguments, so that log(Hz), for example, actually means
log( x /1 Hz). The final units string is the compound string, or
a compound of compounds, preceded by an _optional_ numeric
multiplier of the form 10** k, 10ˆ _k_, or 10± _k_ where _k_ is an integer, _optionally_ surrounded by parentheses with the sign character
_required_ in the third form in the absence of parentheses. Creators
of _FITS_ files are encouraged to use the numeric multiplier only
when the available standard scale factors of Table 5 will not suffice. Parentheses are used for symbol grouping and are strongly

6 These tables are reproduced from the first in a series of papers
on world-coordinate systems (Greisen & Calabretta 2002), which provides examples and expanded discussion.

10

Table 5: Prefixes for multiples and submultiples.

Submult Prefix Char Mult Prefix Char

10 [−][1] deci d 10 deca da
10 [−][2] centi c 10 [2] hecto h
10 [−][3] milli m 10 [3] kilo k
10 [−][6] micro u 10 [6] mega M
10 [−][9] nano n 10 [9] giga G
10 [−][12] pico p 10 [12] tera T
10 [−][15] femto f 10 [15] peta P
10 [−][18] atto a 10 [18] exa E
10 [−][21] zepto z 10 [21] zetta Z
10 [−][24] yocto y 10 [24] yotta Y

_recommended_ whenever the order of operations might be subject to misinterpretation. A space character implies multiplication, which can also be conveyed explicitly with an asterisk or a
period. Therefore, although spaces are allowed as symbol separators, their use is discouraged. Note that, per IAU convention,
case is significant throughout. The IAU style manual forbids the
use of more than one slash (’/’) character in a units string.
However, since normal mathematical precedence rules apply in
this context, more than one slash _may_ be used but is discouraged.
A unit raised to a power is indicated by the unit string followed, with no intervening spaces, by the _optional_ symbols ** or
ˆ followed by the power given as a numeric expression, called
expr in Table 6. The power _may_ be a simple integer, with or
without sign, _optionally_ surrounded by parentheses. It _may_ also
be a decimal number (e.g., 1.5, 0.5) or a ratio of two integers
(e.g., 7/9), with or without sign, which _must_ be surrounded by
parentheses. Thus meters squared _may_ be indicated by m**(2),
m**+2, m+2, m2, mˆ2, mˆ(+2), etc. and per meter cubed _may_ be
indicated by m**-3, m-3, mˆ(-3), /m3, and so forth. Meters to
the three-halves power _may_ be indicated by m(1.5), mˆ(1.5),
m**(1.5), m(3/2), m**(3/2), and mˆ(3/2), but _not_ by ms/2
or m1.5.

<a id="432-units-in-comment-fields"></a>
#### 4.3.2. Units in comment fields

If the units of the keyword value are specified in the comment of
the header keyword, it is _recommended_ that the units string be
enclosed in square brackets (i.e., enclosed by ‘[’ and ‘]’) at the
beginning of the comment field, separated from the slash (’/’)
comment field delimiter by a single space character. An example, using a non-standard keyword, is
EXPTIME = 1200. / [s] exposure time in seconds
This widespread, but _optional_, practice suggests that square
brackets _should_ be used in comment fields only for this purpose. Nonetheless, software _should not_ depend on units being
expressed in this fashion within a keyword comment, and software _should not_ depend on any string within square brackets in
a comment field containing a proper units string.

<a id="44-keywords"></a>
### 4.4. Keywords

<a id="441-mandatory-keywords"></a>
#### 4.4.1. Mandatory keywords

Mandatory keywords are _required_ in every HDU as described
in the remainder of this subsection. They _must_ be used only as
described in this Standard. Values of the mandatory keywords
_must_ be written in fixed-format.

Table 6: Characters and strings allowed to denote mathematical
operations.

String Meaning

str1 str2 Multiplication
str1*str2 Multiplication
str1.str2 Multiplication
str1/str2 Division
str1**expr Raised to the power expr
str1ˆexpr Raised to the power expr
str1expr Raised to the power expr
log(str1) Common Logarithm (to base 10)
ln(str1) Natural Logarithm
exp(str1) Exponential ( _e_ [str1] )
sqrt(str1) Square root

Table 7: Mandatory keywords for primary header.

Position Keyword

1 SIMPLE = T

2 BITPIX

3 NAXIS

...
(other keywords)

...

last END

Table 8: Interpretation of valid BITPIX value.

Value Data represented

8 Character or unsigned binary integer
16 16-bit two’s complement binary integer
32 32-bit two’s complement binary integer
64 64-bit two’s complement binary integer
−32 IEEE single-precision floating point
−64 IEEE double-precision floating point

<a id="4411-primary-header"></a>
##### 4.4.1.1. Primary header

The SIMPLE keyword is _required_ to be the first keyword
in the primary header of all _FITS_ files. The primary header _must_
contain the other mandatory keywords shown in Table 7 in the
order given. Other keywords _must not_ intervene between the
SIMPLE keyword and the last NAXIS _n_ keyword.

SIMPLE keyword. The value field _shall_ contain a logical constant with the value T if the file conforms to this Standard. This
keyword is mandatory for the primary header and _must not_ appear in extension headers. [7] A value of F signifies that the file
does not conform to this Standard.

BITPIX keyword. The value field _shall_ contain an integer. The
absolute value is used in computing the sizes of data structures.
It _shall_ specify the number of bits that represent a data value in
the associated data array. The only valid values of BITPIX are

7 This requirement differs from the wording in the original _FITS_ papers. See Appendix H.

given in Table 8. Writers of _FITS_ arrays _should_ select a BITPIX
data type appropriate to the form, range of values, and accuracy
of the data in the array.

NAXIS keyword. The value field _shall_ contain a non-negative
integer no greater than 999 representing the number of axes in
the associated data array. A value of zero signifies that no data
follow the header in the HDU.

NAXIS n keywords. The NAXIS _n_ keywords _must_ be present for
all values _n_ = 1, ..., NAXIS, in increasing order of _n_, and for
no other values of _n_ . The value field of this indexed keyword
_shall_ contain a non-negative integer representing the number of
elements along Axis _n_ of a data array. A value of zero for any of
the NAXIS _n_ signifies that no data follow the header in the HDU
(however, the random-groups structure described in Sect. 6 has
NAXIS1 = 0, but will have data following the header if the other
NAXIS _n_ keywords are non-zero). If NAXIS is equal to 0, there
_shall not_ be any NAXIS _n_ keywords.

END keyword. This keyword has no associated value. Bytes 9
through 80 _shall_ be filled with ASCII spaces (decimal 32 or hexadecimal 20). The END keyword marks the logical end of the
header and _must_ occur in the last 2880-byte _FITS_ block of the
header.

The total number of bits in the primary data array, exclusive
of fill that is needed after the data to complete the last 2880-byte
data block (Sect. 3.3.2), is given by the following expression:

_N_ bits = |BITPIX| × (NAXIS1 × NAXIS2 × · · · × NAXIS _m_ ), (1)

where _N_ bits _must_ be non-negative and is the number of bits excluding fill, m is the value of NAXIS, and BITPIX and the NAXIS _n_
represent the values associated with those keywords. Note that
the random-groups convention in the primary array has a morecomplicated structure whose size is given by Eq. 4. The header
of the first _FITS_ extension in the file, if present, _shall_ start with
the first _FITS_ block following the data block that contains the
last bit of the primary data array.
An example of a primary array header is shown in Table 9.
In addition to the required keywords, it includes a few of the
reserved keywords that are discussed in Sect. 4.4.2.

<a id="4412-conforming-extensions"></a>
##### 4.4.1.2. Conforming extensions

All conforming extensions, whether or not further specified in this Standard, _must_ use the keywords defined in Table 10
in the order specified. Other keywords _must not_ intervene
between the XTENSION keyword and the GCOUNT keyword.
The BITPIX, NAXIS, NAXIS _n_, and END keywords are defined in
Sect. 4.4.1.

XTENSION keyword. The value field _shall_ contain a character
string giving the name of the extension type. This keyword is
mandatory for an extension header and _must not_ appear in the
primary header. [7] To preclude conflict, extension type names
_must_ be registered with the IAUFWG. The current list of registered extensions is given in Appendix F. An up-to-date list is
also maintained on the _FITS_ Support Office website.

11

Table 9: Example of a primary array header.

Keyword records

SIMPLE = T / file does conform to FITS Standard
BITPIX = 16 / number of bits per data pixel
NAXIS = 2 / number of data axes
NAXIS1 = 250 / length of data axis 1
NAXIS2 = 300 / length of data axis 2
OBJECT = 'Cygnus X-1'
DATE = '2006-10-22'

END

PCOUNT keyword. The value field _shall_ contain an integer that
_shall_ be used in any way appropriate to define the data structure,
consistent with Eq. 2. In IMAGE (Sect. 7.1) and TABLE (Sect. 7.2)
extensions this keyword _must_ have the value 0; in BINTABLE
extensions (Sect. 7.3) it is used to specify the number of bytes
that follow the main data table in the supplemental data area
called the heap. This keyword is also used in the random-groups
structure (Sect. 6) to specify the number of parameters preceding
each array in a group.

GCOUNT keyword. The value field _shall_ contain an integer that
_shall_ be used in any way appropriate to define the data structure, consistent with Eq. 2. This keyword _must_ have the value 1
in the IMAGE, TABLE, and BINTABLE standard extensions defined
in Sect. 7. This keyword is also used in the random-groups structure (Sect. 6) to specify the number of random groups present.
The total number of bits in the extension data array (exclusive of fill that is needed after the data to complete the last 2880byte data block) is given by the following expression:

_N_ bits = |BITPIX| × GCOUNT ×

(PCOUNT + NAXIS1 × NAXIS2 × · · · × NAXIS _m_ ), (2)

where _N_ bits _must_ be non-negative and is the number of bits excluding fill; m is the value of NAXIS; and BITPIX, GCOUNT,
PCOUNT, and the NAXIS _n_ represent the values associated with
those keywords. If _N_ bits - 0, then the data array _shall_ be contained in an integral number of 2880-byte _FITS_ data blocks. The
header of the next _FITS_ extension in the file, if any, _shall_ start
with the first _FITS_ block following the data block that contains
the last bit of the current extension data array.

<a id="442-other-reserved-keywords"></a>
#### 4.4.2. Other reserved keywords

The reserved keywords described below are _optional_, but if
present in the header they _must_ be used only as defined in this
Standard. They apply to any _FITS_ structure with the meanings
and restrictions defined below. Any _FITS_ structure _may_ further
restrict the use of these keywords.

<a id="4421-general-descriptive-keywords"></a>
##### 4.4.2.1. General descriptive keywords

DATE keyword. The value field _shall_ contain a character string
giving the date on which the HDU was created, in the form
YYYY-MM-DD, or the date and time when the HDU was created, in
the form YYYY-MM-DDThh:mm:ss[.sss...], where YYYY _shall_
be the four-digit calendar year number, MM the two-digit month
number with January given by 01 and December by 12, and DD

12

Table 10: Mandatory keywords in conforming extensions.

Position Keyword

1 XTENSION

2 BITPIX

3 NAXIS

5 PCOUNT

6 GCOUNT

...
(other keywords)

...

last END

the two-digit day of the month. When both date and time are
given, the literal T _shall_ separate the date and time, hh _shall_
be the two-digit hour in the day, mm the two-digit number of
minutes after the hour, and ss[.sss...] the number of seconds
(two digits followed by an _optional_ fraction) after the minute.
Default values _must not_ be given to any portion of the date/time
string, and leading zeros _must not_ be omitted. The decimal part
of the seconds field is _optional_ and _may_ be arbitrarily long,
so long as it is consistent with the rules for value formats of
Sect. 4.2. Otherwise said, the format for DATE keywords written after January 1, 2000 _shall_ be the ISO-8601 _datetime_ form
described in Sect. 9.1.1. See also Sect. 9.5.

The value of the DATE keyword _shall_ always be expressed in
UTC when in this format, for all data sets created on Earth.
The following format _may_ appear on files written before
January 1, 2000. The value field contains a character string
giving the date on which the HDU was created, in the form
DD/MM/YY, where DD is the day of the month, MM the month
number with January given by 01 and December by 12, and YY
the last two digits of the year, the first two digits being understood to be 19. Specification of the date using Universal Time is
_recommended_ but not assumed.

When a newly created HDU is substantially a verbatim copy
of another HDU, the value of the DATE keyword in the original
HDU _may_ be retained in the new HDU instead of updating the
value to the current date and time.

ORIGIN keyword. The value field _shall_ contain a character
string identifying the organization or institution responsible for
creating the _FITS_ file.

EXTEND keyword. The value field _shall_ contain a logical value
indicating whether the _FITS_ file is allowed to contain conform

ing extensions following the primary HDU. This keyword _may_
only appear in the primary header and _must not_ appear in an extension header. If the value field is T then there _may_ be conforming extensions in the _FITS_ file following the primary HDU. This
keyword is only advisory, so its presence with a value T does
not require that the _FITS_ file contains extensions, nor does the
absence of this keyword necessarily imply that the file does not
contain extensions. Earlier versions of this Standard stated that
the EXTEND keyword _must_ be present in the primary header if the
file contained extensions, but this is no longer _required_ .

BLOCKED keyword. This keyword is deprecated and _should not_
be used in new _FITS_ files. It is reserved primarily to prevent its
use with other meanings. As previously defined, this keyword, if
used, was _required_ to appear only within the first 36 keywords in
the primary header. Its presence with the _required_ logical value
of T advised that the physical block size of the _FITS_ file on which
it appears _may_ be an integral multiple of the _FITS_ block length
and not necessarily equal to it.

<a id="4422-keywords-describing-observations"></a>
##### 4.4.2.2. Keywords describing observations

DATE-OBS keyword. The format of the value field for
DATE-OBS keywords _shall_ follow the prescriptions for the DATE
keyword (Sect. 4.4.2 and Sect. 9.1.1 Either the four-digit year
format or the two-digit year format _may_ be used for observation
dates from 1900 through 1999, although the four-digit format is
_recommended_ .

When the format with a four-digit year is used, the default interpretations for time _should_ be UTC for dates beginning 197201-01 and UT before. Other date and time scales are permissible.
The value of the DATE-OBS keyword _shall_ be expressed in the
principal time system or time scale of the HDU to which it belongs; if there is any chance of ambiguity, the choice _should_ be
clarified in comments. The value of DATE-OBS _shall_ be assumed
to refer to the start of an observation, unless another interpretation is clearly explained in the comment field. Explicit specification of the time scale is _recommended_ . By default, times for
TAI and times that run simultaneously with TAI, e.g., UTC and
TT, will be assumed to be as measured at the detector (or, in
practical cases, at the observatory). For coordinate times such as
TCG, TCB, and TDB, the default _shall_ be to include light-time
corrections to the associated spatial origin, namely the geocenter for TCG and the Solar System barycenter for the other two.
Conventions _may_ be developed that use other time systems. Time
scales are now discussed in detail in Sect. 9.2.1 and Table 30.

When the value of DATE-OBS is expressed in the two-digit
year form, allowed for files written before January 1, 2000 with
a year in the range 1900–1999, there is no default assumption as
to whether it refers to the start, middle or end of an observation.

DATExxxx keywords. The value fields for all keywords beginning with the string DATE whose value contains date, and _op-_
_tionally_ time, information _shall_ follow the prescriptions for the
DATE-OBS keyword. See also Sect. 9.1.1 for the _datetime_ format,
and Sect. 9.5 for further global time keywords specified by the
Standard.

TELESCOP keyword. The value field _shall_ contain a character
string identifying the telescope used to acquire the data associated with the header.

INSTRUME keyword. The value field _shall_ contain a character
string identifying the instrument used to acquire the data associated with the header.

OBSERVER keyword. The value field _shall_ contain a character string identifying who acquired the data associated with the
header.

OBJECT keyword. The value field _shall_ contain a character
string giving a name for the object observed.

<a id="4423-bibliographic-keywords"></a>
##### 4.4.2.3. Bibliographic keywords

AUTHOR keyword. The value field _shall_ contain a character
string identifying who compiled the information in the data associated with the header. This keyword is appropriate when the
data originate in a published paper or are compiled from many

sources.

REFERENC keyword. The value field _shall_ contain a character string citing a reference where the data associated with the header are published. It is _recommended_
that either the 19-digit bibliographic identifier [8] used in
the Astrophysics Data System bibliographic databases
[(http://adswww.harvard.edu/)](http://adswww.harvard.edu/) or the Digital Object
Identifier [(http://doi.org)](http://doi.org) be included in the value
string, when available (e.g., ’1994A&AS..103..135A’ or
’doi:10.1006/jmbi.1998.2354’).

<a id="4424-commentary-keywords"></a>
##### 4.4.2.4. Commentary keywords

These keywords provide commentary information about the
contents or history of the _FITS_ file and _may_ occur any number of
times in a header. These keywords _shall_ have no associated value
even if the value indicator characters �=␣' appear in Bytes 9 and
10 (hence it is _recommended_ that these keywords not contain the
value indicator). Bytes 9 through 80 _may_ contain any of the restricted set of ASCII-text characters, decimal 32 through 126
(hexadecimal 20 through 7E).
In earlier versions of this Standard continued string keywords (see Sect. 4.2.1.2) could be handled as commentary keywords if the relevant convention was not supported.
Now CONTINUE keywords _shall_ be honoured as specified in
Sect. 4.2.1.2.

COMMENT keyword. This keyword _may_ be used to supply any
comments regarding the _FITS_ file.

8 This bibliographic convention (Schmitz et al. 1995) was initially
developed for use within NED (NASA/IPAC Extragalactic Database)
and SIMBAD (operated at CDS, Strasbourg, France).

13

HISTORY keyword. This keyword _should_ be used to describe
the history of steps and procedures associated with the processing of the associated data.

Keyword field is blank. This keyword _may_ be used to supply
any comments regarding the _FITS_ file. It is frequently used for
aesthetic purposes to provide a break between groups of related
keywords in the header.

A sequence of one or more entirely blank keyword records
(consisting of 80 ASCII space characters) that immediately precede the END keyword _may_ be interpreted as non-significant fill
space that _may_ be overwritten when new keywords are appended
to the header. This usage convention enables an arbitrarily large
amount of header space to be preallocated when the _FITS_ HDU
is first created, which can help mitigate the potentially timeconsuming alternative of having to shift all the following data
in the file by 2880 bytes to make room for a new _FITS_ header
block each time space is needed for a new keyword.

<a id="4425-keywords-that-describe-arrays"></a>
##### 4.4.2.5. Keywords that describe arrays

These keywords are used to describe the contents of an
array, either in the primary array, in an IMAGE extension
(Sect. 7.1), or in a series of random groups (Sect. 6). They are
_optional_, but if they appear in the header describing an array
or groups, they _must_ be used as defined in this section of this
Standard. They _shall not_ be used in headers describing other
structures unless the meaning is the same as defined here.

BSCALE keyword. This keyword _shall_ be used, along with the
BZERO keyword, to linearly scale the array pixel values (i.e., the
actual values stored in the _FITS_ file) to transform them into the
physical values that they represent using Eq. 3.

physical value = BZERO + BSCALE × array value. (3)

The value field _shall_ contain a floating-point number representing the coefficient of the linear term in the scaling equation, the
ratio of physical value to array value at zero offset. The default
value for this keyword is 1.0. Before support for IEEE floatingpoint data types was added to _FITS_ (Wells & Grosbøl 1990), this
technique of linearly scaling integer values was the only way to
represent the full range of floating-point values in a _FITS_ array.
This linear scaling technique is still commonly used to reduce
the size of the data array by a factor of two by representing 32bit floating-point physical values as 16-bit scaled integers.

BZERO keyword. This keyword _shall_ be used, along with the
BSCALE keyword, to linearly scale the array pixel values (i.e., the
actual values stored in the _FITS_ file) to transform them into the
physical values that they represent using Eq. 3. The value field
_shall_ contain a floating-point number representing the physical
value corresponding to an array value of zero. The default value
for this keyword is 0.0.
Besides its use in representing floating-point values as scaled
integers (see the description of the BSCALE keyword), the BZERO
keyword is also used when storing unsigned-integer values in the
_FITS_ array. In this special case the BSCALE keyword _shall_ have

14

the default value of 1.0, and the BZERO keyword _shall_ have one
of the integer values shown in Table 11.
Since the _FITS_ format does not support a native unsigned
integer data type (except for the unsigned eight-bit byte data
type), the unsigned values are stored in the _FITS_ array as native signed integers with the appropriate integer offset specified
by the BZERO keyword value shown in the table. For the byte
data type, the converse technique can be used to store signed
byte values as native unsigned values with the negative BZERO
offset. In each case, the physical value is computed by adding
the offset specified by the BZERO keyword to the native data type
value that is stored in the _FITS_ file. [9]

BUNIT keyword. The value field _shall_ contain a character string
describing the physical units in which the quantities in the array, after application of BSCALE and BZERO, are expressed. These
units _must_ follow the prescriptions of Sect. 4.3.

BLANK keyword. This keyword _shall_ be used only in headers
with positive values of BITPIX (i.e., in arrays with integer data).
Bytes 1 through 8 contain the string �BLANK␣␣␣' (ASCII spaces
in Bytes 6 through 8). The value field _shall_ contain an integer
that specifies the value that is used within the integer array to
represent pixels that have an undefined physical value.
If the BSCALE and BZERO keywords do not have the default
values of 1.0 and 0.0, respectively, then the value of the BLANK
keyword _must_ equal the actual value in the _FITS_ data array
that is used to represent an undefined pixel and not the corresponding physical value (computed from Eq. 3). To cite a specific, common example, _unsigned_ 16-bit integers are represented
in a _signed_ integer _FITS_ array (with BITPIX = 16) by setting
BZERO = 32768 and BSCALE = 1. If it is desired to use pixels
that have an _unsigned_ value (i.e., the physical value) equal to 0
to represent undefined pixels in the array, then the BLANK keyword _must_ be set to the value -32768 because that is the actual
value of the undefined pixels in the _FITS_ array.

DATAMAX keyword. The value field _shall_ always contain a
floating-point number, regardless of the value of BITPIX. This
number _shall_ give the maximum valid physical value represented
by the array (from Eq. 3), exclusive of any IEEE special values.

DATAMIN keyword. The value field _shall_ always contain a
floating-point number, regardless of the value of BITPIX. This
number _shall_ give the minimum valid physical value represented
by the array (from Eq. 3), exclusive of any IEEE special values.

WCS keywords. An extensive set of keywords have been defined to describe the world coordinates associated with an array.
These keywords are discussed separately in Sect. 8.

9 A more computationally efficient method of adding or subtracting
the BZERO values is to simply flip the most-significant bit of the binary value. For example, using eight-bit integers, the decimal value 248
minus the BZERO value of 128 equals 120. The binary representation
of 248 is 11111000. Flipping the most-significant bit gives the binary
value 01111000, which is equal to decimal 120.

Table 11: Usage of BZERO to represent non-default integer data types.

BITPIX Native Physical BZERO
data type data type

8 unsigned signed byte -128 (−2 [7] )
16 signed unsigned 16-bit 32768 (2 [15] )
32 signed unsigned 32-bit 2147483648 (2 [31] )
64 signed unsigned 64-bit 9223372036854775808 (2 [63] )

<a id="4426-extension-keywords"></a>
##### 4.4.2.6. Extension keywords

The next three keywords were originally defined for use
within the header of a conforming extension, however they also
_may_ appear in the primary header with an analogous meaning.
If these keywords are present, it is _recommended_ that they have
a unique combination of values in each HDU of the _FITS_ file.

EXTNAME keyword. The value field _shall_ contain a character
string to be used to distinguish among different extensions of
the same type, i.e., with the same value of XTENSION, in a _FITS_
file. Within this context, the primary array _should_ be considered
as equivalent to an IMAGE extension.

EXTVER keyword. The value field _shall_ contain an integer to be
used to distinguish among different extensions in a _FITS_ file with
the same type and name, i.e., the same values for XTENSION and
EXTNAME. The values need not start with 1 for the first extension
with a particular value of EXTNAME and need not be in sequence
for subsequent values. If the EXTVER keyword is absent, the file
_should_ be treated as if the value were 1.

EXTLEVEL keyword. The value field _shall_ contain an integer
specifying the level in a hierarchy of extension levels of the extension header containing it. The value _shall_ be 1 for the highest
level; levels with a higher value of this keyword _shall_ be subordinate to levels with a lower value. If the EXTLEVEL keyword is
absent, the file _should_ be treated as if the value were 1.

The following keyword is _optional_, but is _reserved_ for use
by the convention described in Appendix K. If present it _shall_
appear in the extension header immediately after the mandatory
keywords, and be used as described in the appendix.

INHERIT keyword. The value field _shall_ contain a logical value
of T or F to indicate whether or not the current extension should
inherit the keywords in the primary header of the _FITS_ file.

4.4.2.7 Data-integrity keywords

The two keywords described here provide an integrity check
on the information contained in _FITS_ HDUs.

DATASUM keyword. The value field of the DATASUM keyword
_shall_ consist of a character string that _should_ contain the
unsigned-integer value of the 32-bit ones’ complement checksum of the data records in the HDU (i.e., excluding the header

records). For this purpose, each 2880-byte _FITS_ logical record
_should_ be interpreted as consisting of 720 32-bit unsigned integers. The four bytes in each integer _must_ be interpreted in order of decreasing significance where the most-significant byte is
first, and the least-significant byte is last. Accumulate the sum of
these integers using ones’ complement arithmetic in which any
overflow of the most-significant bit is propagated back into the
least-significant bit of the sum.

The DATASUM value is expressed as a character string (i.e.,
enclosed in single-quote characters) because support for the full
range of 32-bit unsigned-integer keyword values is problematic
in some software systems. This string _may_ be padded with nonsignificant leading or trailing blank characters or leading zeros.
A string containing only one or more consecutive ASCII blanks
_may_ be used to represent an undefined or unknown value for
the DATASUM keyword. The DATASUM keyword _may_ be omitted in HDUs that have no data records, but it is preferable to
include the keyword with a value of 0. Otherwise, a missing
DATASUM keyword asserts no knowledge of the checksum of
the data records. Recording in the comment field the ISO-8601formatted Datetime (ISO 2004b) when the value of this keyword
record is created or updated is _recommended_ .

CHECKSUM keyword. The value field of the CHECKSUM keyword
_shall_ consist of an ASCII character string whose value forces the
32-bit ones’ complement checksum accumulated over the entire
_FITS_ HDU to equal negative 0. (Note that ones’s complement
arithmetic has both positive and negative zero elements). It is
_recommended_ that the particular 16-character string generated
by the algorithm described in Appendix J be used. A string containing only one or more consecutive ASCII blanks _may_ be used
to represent an undefined or unknown value for the CHECKSUM
keyword.

The CHECKSUM keyword value _must_ be expressed in fixed
format, when the algorithm in Appendix J is used, otherwise the
usage of fixed format is _recommended_ . Recording in the comment field the ISO-8601-formatted Datetime when the value of
this keyword record is created or updated is _recommended_ .

If the CHECKSUM keyword exists in the header of the HDU
and the accumulated checksum is not equal to −0, or if the
DATASUM keyword exists in the header of the HDU and its value
does not match the data checksum, then this provides a strong indication that the content of the HDU has changed subsequent to
the time that the respective keyword value was computed. Such
an invalid checksum may indicate corruption during a prior file
copy or transfer operation, or a corruption of the physical media
on which the file was stored. It may alternatively reflect an intentional change to the data file by subsequent data processing if
the CHECKSUM value was not also updated.

15

Normally both keywords will be present in the header if either
is present, but this is not _required_ . These keywords apply _only_
to the HDU in which they are contained. If these keywords are
written in one HDU of a multi-HDU _FITS_ file then it is _strongly_
_recommended_ that they also be written to every other HDU in
the file with values appropriate to each HDU in turn; in that
case the checksum accumulated over the entire file will equal
−0 as well. The DATASUM keyword _must_ be updated before the
CHECKSUM keyword. In general updating the two checksum keywords _should_ be the final step of any update to either data or
header records in a _FITS_ HDU. It is highly _recommended_ that if
a _FITS_ file is intended for public distribution, then the checksum
keywords, if present, _should_ contain valid values.

<a id="443-additional-keywords"></a>
#### 4.4.3. Additional keywords

New keywords _may_ be devised in addition to those described
in this Standard, so long as they are consistent with the generalized rules for keywords and do not conflict with mandatory
or reserved keywords. Any keyword that refers to or depends
upon the existence of other specific HDUs in the same or other
files _should_ be used with caution because the persistence of those
HDUs cannot always be guaranteed.

<a id="5-data-representation"></a>
## 5. Data representation

Primary and extension data _shall_ be represented in one of the
formats described in this section. _FITS_ data _shall_ be interpreted
to be a byte stream. Bytes are in big-endian order of decreasing
significance. The byte that includes the sign bit _shall_ be first, and
the byte that has the ones bit _shall_ be last.

<a id="51-characters"></a>
### 5.1. Characters

Each character _shall_ be represented by one byte. A character
_shall_ be represented by its seven-bit ASCII (ANSI 1977) code
in the low-order seven bits in the byte. The high-order bit _shall_
be zero.

<a id="52-integers"></a>
### 5.2. Integers

<a id="521-eight-bit"></a>
#### 5.2.1. Eight-bit

Eight-bit integers _shall_ be unsigned binary integers, contained in
one byte with decimal values ranging from 0 to 255.

<a id="522-sixteen-bit"></a>
#### 5.2.2. Sixteen-bit

Sixteen-bit integers _shall_ be two’s complement signed binary integers, contained in two bytes with decimal values ranging from
−32768 to +32767.

<a id="523-thirty-two-bit"></a>
#### 5.2.3. Thirty-two-bit

Thirty-two-bit integers _shall_ be two’s complement signed binary
integers, contained in four bytes with decimal values ranging
from −2147483648 to +2147483647.

16

<a id="524-sixty-four-bit"></a>
#### 5.2.4. Sixty-four-bit

Sixty-four-bit integers _shall_ be two’s complement signed binary
integers, contained in eight bytes with decimal values ranging
from −9223372036854775808 to +9223372036854775807.

<a id="525-unsigned-integers"></a>
#### 5.2.5. Unsigned integers

The _FITS_ format does not support a native unsigned integer data
type (except for the unsigned 8-bit byte data type) therefore unsigned 16-bit, 32-bit, or 64-bit binary integers cannot be stored
directly in a _FITS_ data array. Instead, the appropriate offset _must_
be applied to the unsigned integer to shift the value into the range
of the corresponding signed integer, which is then stored in the
_FITS_ file. The BZERO keyword _shall_ record the amount of the
offset needed to restore the original unsigned value. The BSCALE
keyword _shall_ have the default value of 1.0 in this case, and the
appropriate BZERO value, as a function of BITPIX, is specified
in Table 11.

This same technique _must_ be used when storing unsigned
integers in a binary-table column of signed integers (Sect. 7.3.2).
In this case the TSCAL _n_ keyword (analogous to BSCALE) _shall_
have the default value of 1.0, and the appropriate TZERO _n_ value
(analogous to BZERO) is specified in Table 19.

<a id="53-ieee-754-floating-point"></a>
### 5.3. IEEE-754 floating point

Transmission of 32- and 64-bit floating-point data within the
_FITS_ format _shall_ use the ANSI/IEEE-754 standard (IEEE
1985). BITPIX = -32 and BITPIX = -64 signify 32- and 64-bit
IEEE floating-point numbers, respectively; the absolute value of
BITPIX is used for computing the sizes of data structures. The
full IEEE set of number forms is allowed for _FITS_ interchange,
including all special values.
The BLANK keyword _should not_ be used when BITPIX = -32
or -64; rather, the IEEE NaN _should_ be used to represent an
undefined value. Use of the BSCALE and BZERO keywords is
_not recommended_ .

Appendix E has additional details on the IEEE format.

<a id="54-time"></a>
### 5.4. Time

There is strictly no such thing as a data type for _time valued_ data,
but rules to encode time values are given in Sect. 9 and in more
detail in Rots et al. (2015).

<a id="6-random-groups-structure"></a>
## 6. Random-groups structure

The random-groups structure allows a collection of ‘groups’,
where a group consists of a subarray of data and a set of associated parameter values, to be stored within the _FITS_ primary
data array. Random groups have been used almost exclusively
for applications in radio interferometry; outside this field, there
is little support for reading or writing data in this format. Other
than the existing use for radio interferometry data, the randomgroups structure is deprecated and _should not_ be further used.
For other applications, the binary-table extension (Sect. 7.3) provides a more extensible and better documented way of associating groups of data within a single data structure.

Table 12: Mandatory keywords in primary header preceding random groups.

Position Keyword

1 SIMPLE = T

2 BITPIX

3 NAXIS

4 NAXIS1 = 0

...
GROUPS = T

PCOUNT

GCOUNT

...

last END

<a id="61-keywords"></a>
### 6.1. Keywords

<a id="611-mandatory-keywords"></a>
#### 6.1.1. Mandatory keywords

The SIMPLE keyword is _required_ to be the first keyword in the
primary header of all _FITS_ files, including those with randomgroups records. If the random-groups format records follow the
primary header, the keyword records of the primary header _must_
use the keywords defined in Table 12 in the order specified. No
other keywords _may_ intervene between the SIMPLE keyword and
the last NAXIS _n_ keyword.

SIMPLE keyword. The keyword record containing this keyword
is structured in the same way as if a primary data array were
present (Sect. 4.4.1).

BITPIX keyword. The keyword record containing this keyword
is structured as prescribed in Sect. 4.4.1.

NAXIS keyword. The value field _shall_ contain an integer ranging from 1 to 999, representing one more than the number of
axes in each data array.

NAXIS1 keyword. The value field _shall_ contain the integer 0,
a signature of random-groups format indicating that there is no
primary data array.

keywords _must_ be present for all values _n_ = 2, ..., NAXIS, in
increasing order of _n_, and for no larger values of _n_ . The value
field _shall_ contain an integer, representing the number of positions along Axis _n_ − 1 of the data array in each group.

GROUPS keyword. The value field _shall_ contain the logical constant T. The value T associated with this keyword implies that
random-groups records are present.

PCOUNT keyword. The value field _shall_ contain an integer equal
to the number of parameters preceding each array in a group.

GCOUNT keyword. The value field _shall_ contain an integer equal
to the number of random groups present.

END keyword. This keyword has no associated value. Bytes 9
through 80 _shall_ contain ASCII spaces (decimal 32 or hexadecimal 20).

The total number of bits in the random-groups records exclusive of the fill described in Sect. 6.2 is given by the following
expression:

_N_ bits = |BITPIX| × GCOUNT ×

(PCOUNT + NAXIS2 × NAXIS3 × · · · × NAXIS _m_ ), (4)

where _N_ bits is non-negative and the number of bits excluding fill;
m is the value of NAXIS; and BITPIX, GCOUNT, PCOUNT, and the
NAXIS _n_ represent the values associated with those keywords.

<a id="612-reserved-keywords"></a>
#### 6.1.2. Reserved keywords

PTYPE n keywords. The value field _shall_ contain a character
string giving the name of Parameter _n_ . If the PTYPE _n_ keywords
for more than one value of _n_ have the same associated name in
the value field, then the data value for the parameter of that name
is to be obtained by adding the derived data values of the corresponding parameters. This rule provides a mechanism by which
a random parameter _may_ have more precision than the accompanying data array elements; for example, by summing two 16-bit
values with the first scaled relative to the other such that the sum
forms a number of up to 32-bit precision.

PSCAL n keywords. This keyword _shall_ be used, along with the
PZERO _n_ keyword, when the _n_ [th] _FITS_ group parameter value is
not the true physical value, to transform the group parameter
value to the true physical values it represents, using Eq. 5. The
value field _shall_ contain a floating-point number representing the
coefficient of the linear term in Eq. 5, the scaling factor between
true values and group parameter values at zero offset. The default
value for this keyword is 1.0.

PZERO n keywords. This keyword _shall_ be used, along with the
PSCAL _n_ keyword, when the _n_ [th] _FITS_ group parameter value
is not the true physical value, to transform the group parameter value to the physical value. The value field _shall_ contain a
floating-point number, representing the true value corresponding to a group parameter value of zero. The default value for this
keyword is 0.0. The transformation equation is as follows:

physical value = PZERO _n_ + PSCAL _n_ × group param value. (5)

<a id="62-data-sequence"></a>
### 6.2. Data sequence

Random-groups data _shall_ consist of a set of groups. The number of groups _shall_ be specified by the GCOUNT keyword in the
associated header. Each group _shall_ consist of the number of parameters specified by the PCOUNT keyword followed by an array
with the number of elements _N_ elem given by the following expression:

17

_N_ elem = (NAXIS2 × NAXIS3 × · · · × NAXIS _m_ ), (6)

where _N_ elem is the number of elements in the data array in a
group, m is the value of NAXIS, and the NAXIS _n_ represent the
values associated with those keywords.
The first parameter of the first group _shall_ appear in the first
location of the first data block. The first element of each array
_shall_ immediately follow the last parameter associated with that
group. The first parameter of any subsequent group _shall_ immediately follow the last element of the array of the previous group.
The arrays _shall_ be organized internally in the same way as an
ordinary primary data array. If the groups data do not fill the
final data block, the remainder of the block _shall_ be filled with
zero values in the same way as a primary data array (Sect. 3.3.2).
If random-groups records are present, there _shall_ be no primary
data array.

<a id="63-data-representation"></a>
### 6.3. Data representation

Permissible data representations are those listed in Sect. 5.
Parameters and elements of associated data arrays _shall_ have the
same representation. If more precision is required for an associated parameter than for an element of a data array, the parameter
_shall_ be divided into two or more addends, represented by the
same value for the PTYPE _n_ keyword. The value _shall_ be the sum
of the physical values, which _may_ have been obtained from the
group parameter values using the PSCAL _n_ and PZERO _n_ keywords.

<a id="7-standard-extensions"></a>
## 7. Standard extensions

A standard extension is a conforming extension whose organization and content are completely specified in this Standard. The
specifications for the three currently defined standard extensions,
namely,

1. IMAGE extensions;
2. TABLE ASCII-table extensions; and
3. BINTABLE binary-table extensions

are given in the following sections. A list of other conforming
extensions is given in Appendix F.

<a id="71-image-extension"></a>
### 7.1. Image extension

The _FITS_ IMAGE extension is nearly identical in structure to the
the primary HDU and is used to store an array of data. Multiple
IMAGE extensions can be used to store any number of arrays in a
single _FITS_ file. The first keyword record in an IMAGE extension
_shall_ be XTENSION=␣'IMAGE␣␣␣'.

<a id="711-mandatory-keywords"></a>
#### 7.1.1. Mandatory keywords

The XTENSION keyword is _required_ to be the first keyword of
all IMAGE extensions. The keyword records in the header of an
image extension _must_ use the keywords defined in Table 13 in
the order specified. No other keywords _may_ intervene between
the XTENSION and GCOUNT keywords.

XTENSION keyword. The value field _shall_ contain the character
string 'IMAGE␣␣␣'.

18

Table 13: Mandatory keywords in IMAGE extensions.

Position Keyword

1 XTENSION=␣'IMAGE␣␣␣'

2 BITPIX

3 NAXIS

5 PCOUNT = 0

6 GCOUNT = 1

...

...

last END

BITPIX keyword. The value field _shall_ contain an integer. The
absolute value is used in computing the sizes of data structures.
It _shall_ specify the number of bits that represent a data value.
The only valid values of BITPIX are given in Table 8. Writers of
IMAGE extensions _should_ select a BITPIX data type appropriate
to the form, range of values, and accuracy of the data in the array.

NAXIS keyword. The value field _shall_ contain a non-negative
integer no greater than 999, representing the number of axes in
the associated data array. If the value is zero then the IMAGE
extension _shall not_ have any data blocks following the header.

NAXIS n keywords. The NAXIS _n_ keywords _must_ be present for
all values _n_ = 1, ..., NAXIS, in increasing order of _n_, and for
no other values of _n_ . The value field of this indexed keyword
_shall_ contain a non-negative integer, representing the number of
elements along Axis _n_ of a data array. If the value of any of the
NAXIS _n_ keywords is zero, then the IMAGE extension _shall not_
have any data blocks following the header. If NAXIS is equal to
0 there _should not_ be any NAXIS _n_ keywords.

PCOUNT keyword. The value field _shall_ contain the integer 0.

GCOUNT keyword. The value field _shall_ contain the integer 1;
each IMAGE extension contains a single array.

END keyword. This keyword has no associated value. Bytes 9
through 80 _shall_ be filled with ASCII spaces (decimal 32 or hexadecimal 20).

<a id="712-other-reserved-keywords"></a>
#### 7.1.2. Other reserved keywords

The reserved keywords defined in Sect. 4.4.2 (except for EXTEND
and BLOCKED) _may_ appear in an image-extension header. The
keywords _must_ be used as defined in that section.

<a id="713-data-sequence"></a>
#### 7.1.3. Data sequence

The data format _shall_ be identical to that of a primary data array
as described in Sect. 3.3.2.

Table 14: Mandatory keywords in ASCII-table extensions.

Position Keyword

1 XTENSION=␣'TABLE␣␣␣'

2 BITPIX = 8

3 NAXIS = 2

4 NAXIS1

5 NAXIS2

6 PCOUNT = 0

7 GCOUNT = 1

8 TFIELDS

...

...

last END

<a id="72-the-ascii-table-extension"></a>
### 7.2. The ASCII-table extension

The ASCII-table extension provides a means of storing catalogs
and tables of astronomical data in _FITS_ format. Each row of the
table consists of a fixed-length sequence of ASCII characters
divided into fields that correspond to the columns in the table.
The first keyword record in an ASCII-table extension _shall_ be

' '
XTENSION=␣ TABLE␣␣␣ .

<a id="721-mandatory-keywords"></a>
#### 7.2.1. Mandatory keywords

The header of an ASCII-table extension _must_ use the keywords
defined in Table 14. The first keyword _must_ be XTENSION;
the seven keywords following XTENSION (BITPIX ... TFIELDS)
_must_ be in the order specified with no intervening keywords.

XTENSION keyword. The value field _shall_ contain the character
string 'TABLE␣␣␣'.

BITPIX keyword. The value field _shall_ contain the integer 8,
denoting that the array contains ASCII characters.

NAXIS keyword. The value field _shall_ contain the integer 2, denoting that the included data array is two-dimensional: rows and
columns.

NAXIS1 keyword. The value field _shall_ contain a non-negative
integer, giving the number of ASCII characters in each row of
the table. This includes all the characters in the defined fields
plus any characters that are not included in any field.

NAXIS2 keyword. The value field _shall_ contain a non-negative
integer, giving the number of rows in the table.

PCOUNT keyword. The value field _shall_ contain the integer 0.

GCOUNT keyword. The value field _shall_ contain the integer 1;
the data blocks contain a single table.

Table 15: Valid TFORM _n_ format values in TABLE extensions.

Field value Data type

Aw Character
Iw Decimal integer
Fw.d Floating-point, fixed decimal notation
Ew.d Floating-point, exponential notation
Dw.d Floating-point, exponential notation

**Notes.** w is the width in characters of the field and d is the number of
digits to the right of the decimal.

TFIELDS keyword. The value field _shall_ contain a non-negative
integer representing the number of fields in each row. The maximum permissible value is 999.

TBCOL n keywords. The TBCOL _n_ keywords _must_ be present for
all values _n_ = 1, ..., TFIELDS and for no other values of _n_ .
The value field of this indexed keyword _shall_ contain an integer
specifying the column in which Field _n_ starts. The first column
of a row is numbered 1.

TFORM n keywords. The TFORM _n_ keywords _must_ be present for
all values _n_ = 1, ..., TFIELDS and for no other values of _n_ .
The value field of this indexed keyword _shall_ contain a character string describing the format in which Field _n_ is encoded.
Only the formats in Table 15, interpreted as Fortran (ISO 2004)
input formats and discussed in more detail in Sect. 7.2.5, are
permitted for encoding. Format codes _must_ be specified in upper
case. Other format editing codes common to Fortran such as repetition, positional editing, scaling, and field termination are not
permitted. All values in numeric fields have a number base of
ten (i.e., they are decimal); binary, octal, hexadecimal, and other
representations are not permitted. The TDISP _n_ keyword, defined
in Sect. 7.2.2, _may_ be used to _recommend_ that a decimal integer
value in an ASCII table be displayed as the equivalent binary,
octal, or hexadecimal value.

19

Table 16: Valid TDISP _n_ format values in TABLE extensions.

Field value Data type

Aw Character
Iw.m Integer
Bw.m Binary, integers only
Ow.m Octal, integers only
Zw.m Hexadecimal, integers only
Fw.d Floating-point, fixed decimal notation
Ew.dEe Floating-point, exponential notation
ENw.d Engineering; E format with exponent multiple of three
ESw.d Scientific; same as EN but non-zero leading digit if not zero
Gw.dEe General; appears as F if significance not lost, else E.
Dw.dEe Floating-point, exponential notation

**Notes.** w is the width in characters of displayed values, m is the minimum number of digits displayed, d is the number of digits to right of decimal,
and e is number of digits in exponent. The .m and Ee fields are _optional_ .

END keyword. This keyword has no associated value. Bytes 9
through 80 _shall_ contain ASCII spaces (decimal 32 or hexadecimal 20).

<a id="722-other-reserved-keywords"></a>
#### 7.2.2. Other reserved keywords

In addition to the reserved keywords defined in Sect. 4.4.2 (except for EXTEND and BLOCKED), the following other reserved
keywords _may_ be used to describe the structure of an ASCIItable data array. They are _optional_, but if they appear within an
ASCII-table extension header, they _must_ be used as defined in
this section of this Standard.

TTYPE n keywords. The value field for this indexed keyword
_shall_ contain a character string giving the name of Field _n_ . It
is _strongly recommended_ that every field of the table be assigned a unique, case-insensitive name with this keyword, and
it is _recommended_ that the character string be composed only
of upper- and lower-case letters, digits, and the underscore (’ ’,
decimal 95, hexadecimal 5F) character. Use of other characters is
_not recommended_ because it may be difficult to map the column
names into variables in some languages (e.g., any hyphens, ’*’
or ’+’ characters in the name may be confused with mathematical operators). String comparisons with the TTYPE _n_ keyword
values _should not_ be case sensitive (e.g., ’TIME’ and ’Time’
_should_ be interpreted as the same name).

TUNIT n keywords. The value field _shall_ contain a character
string describing the physical units in which the quantity in
Field _n_, after any application of TSCAL _n_ and TZERO _n_, is expressed. Units _must_ follow the prescriptions in Sect. 4.3.

TSCAL n keywords. This indexed keyword _shall_ be used, along
with the TZERO _n_ keyword, to linearly scale the values in the table Field _n_ to transform them into the physical values that they
represent using Eq. 7. The value field _shall_ contain a floatingpoint number representing the coefficient of the linear term in
the scaling equation. The default value for this keyword is 1.0.
This keyword _must not_ be used for A-format fields.
The transformation equation used to compute a true physical
value from the quantity in Field _n_ is

physical value = TZERO _n_ + TSCAL _n_ × field value (7)

20

where field value is the value that is actually stored in that
table field in the _FITS_ file.

TZERO n keywords. This indexed keyword _shall_ be used, along
with the TSCAL _n_ keyword, to linearly scale the values in the table
Field _n_ to transform them into the physical values that they represent using Eq. 7. The value field _shall_ contain a floating-point
number representing the physical value corresponding to an array value of zero. The default value for this keyword is 0.0. This
keyword _must not_ be used for A-format fields.

TNULL n keywords. The value field for this indexed keyword
_shall_ contain the character string that represents an undefined
value for Field _n_ . The string is implicitly space filled to the width
of the field.

TDISP n keywords. The value field of this indexed keyword
_shall_ contain a character string describing the format recommended for displaying an ASCII-text representation of of the
contents of Field _n_ . This keyword overrides the default display
format given by the TFORM _n_ keyword. If the table value has been
scaled, the physical value, derived using Eq. 7, _shall_ be displayed. All elements in a field _shall_ be displayed with a single,
repeated format. Only the format codes in Table 16, interpreted
as Fortran (ISO 2004) output formats, and discussed in more detail in Sect. 7.3.4, are permitted for encoding. The format codes
_must_ be specified in upper case. If the Bw.m, Ow.m, and Zw.m
formats are not readily available to the reader, the Iw.m display
format _may_ be used instead, and if the ENw.d and ESw.d formats
are not available, Ew.d _may_ be used.

The following four keywords _may_ be used to specify minimum
and maximum values in numerical columns of a _FITS_ ASCII or
binary table. These keywords _must_ have the same data type as
the physical values in the associated column (either an integer or
a floating-point number). Any undefined elements in the column
_shall_ be excluded when determining the value of these keywords.

TDMIN n keywords. The value field _shall_ contain a number giving the minimum physical value contained in Column _n_ of the
table. This keyword is analogous to the DATAMIN keyword that
is defined for arrays in Sect. 4.4.2.5.

TDMAX n keywords. The value field _shall_ contain a number giving the maximum physical value contained in Column _n_ of the
table. This keyword is analogous to the DATAMAX keyword that
is defined for arrays in Sect. 4.4.2.5.

TLMIN n keywords. The value field _shall_ contain a number that
specifies the minimum physical value in Column _n_ that has
a valid meaning or interpretation. The column is not _required_
to actually contain any elements that have this value, and the
column _may_ contain elements with physical values less than
TLMIN _n_, however, the interpretation of any such out-of-range
column elements is not defined.

TLMAX n keywords. The value field _shall_ contain a number that
specifies the maximum physical value in Column _n_ that has a
valid meaning or interpretation. The column is not _required_ to
actually contain any elements that have this value, and the column _may_ contain elements with physical values greater than
TLMAX _n_, however, the interpretation of any such out-of-range
column elements is not defined.

The TLMIN _n_ and TLMAX _n_ keywords are commonly used when
constructing histograms of the data values in a column. For example, if a table contains columns that give the _X_ and _Y_ pixel
location of a list of photons that were detected by a photoncounting device, then the TLMIN _n_ and TLMAX _n_ keywords could
be used respectively to specify the minimum and maximum values that the detector is capable of assigning to the _X_ and _Y_
columns.

<a id="723-data-sequence"></a>
#### 7.2.3. Data sequence

The table is constructed from a two-dimensional array of ASCII
characters. The row length and the number of rows _shall_ be those
specified, respectively, by the NAXIS1 and NAXIS2 keywords of
the associated header. The number of characters in a row and the

number of rows in the table _shall_ determine the size of the character array. Every row in the array _shall_ have the same number of
characters. The first character of the first row _shall_ be at the start
of the data block immediately following the last header block.
The first character of subsequent rows _shall_ follow immediately
the character at the end of the previous row, independent of the
_FITS_ block structure. The positions in the last data block after
the last character of the last row of the table _shall_ be filled with
ASCII spaces.

<a id="724-fields"></a>
#### 7.2.4. Fields

Each row in the array _shall_ consist of a sequence of from 0 to
999 fields, as specified by the TFIELDS keyword, with one entry
in each field. For every field, the Fortran (ISO 2004) format of
the information contained (given by the TFORM _n_ keyword), the
location in the row of the beginning of the field (given by the
TBCOL _n_ keyword), and ( _optionally_, but _strongly recommended_ )
the field name (given by the TTYPE _n_ keyword), _shall_ be specified in the associated header. The location and format of fields
_shall_ be the same for every row. Fields _may_ overlap, but this usage is _not recommended_ . Only a limited set of ASCII character
values _may_ appear within any field, depending on the field type
as specified below. There _may_ be characters in a table row that

are not included in any field, (e.g., between fields, or before the
first field or after the last field). Any seven-bit ASCII character
_may_ occur in characters of a table row that are not included in a
defined field. A common convention is to include a space character between each field for added legibility if the table row is
displayed verbatim. It is also permissible to add control characters, such as a carriage return or line-feed character, following
the last field in each row as a way of formatting the table if it is
printed or displayed by a text-editing program.

<a id="725-entries"></a>
#### 7.2.5. Entries

All data in an ASCII-table extension field _shall_ be ASCII text in
a format that conforms to the rules for fixed field input in Fortran
(ISO 2004) format, as described below. The only possible formats _shall_ be those specified in Table 15. If values of −0 and
+0 need to be distinguished, then the sign character _should_ appear in a separate field in character format. TNULL _n_ keywords
_may_ be used to specify a character string that represents an undefined value in each field. The characters representing an undefined value _may_ differ from field to field but _must_ be the same
within a field. Writers of ASCII tables _should_ select a format for
each field that is appropriate to the form, range of values, and
accuracy of the data in that field. This Standard does not impose
an upper limit on the number of digits of precision, nor any limit
on the range of numeric values. Software packages that read or
write data according to this Standard could be limited, however,
in the range of values and exponents that are supported (e.g.,
to the range that can be represented by 32-bit or 64-bit binary
numbers).
The value of each entry _shall_ be interpreted as described in
the following paragraphs.

Character fields. The value of a character-formatted (Aw) field
is a character string of width w containing the characters in
columns TBCOL _n_ through TBCOL _n_ + w − 1. The character string
_shall_ be composed of the restricted set of ASCII-text characters
with decimal values in the range 32 through 126 (hexadecimal
20 through 7E).

Integer fields. The value of an integer-formatted (Iw) field is a
signed decimal integer contained in Columns TBCOL _n_ through
TBCOL _n_ + w − 1 consisting of a single _optional_ sign (’+’ or ’-’)
followed by one or more decimal digits (’0’ through ’9’). Nonsignificant space characters _may_ precede and/or follow the integer value within the field. A blank field has value 0. All characters other than leading and trailing spaces, a contiguous string of
decimal digits, and a single leading sign character are forbidden.

Real fields. The value of a real-formatted field (Fw.d, Ew.d,
Dw.d) is a real number determined from the w characters from
Columns TBCOL _n_ through TBCOL _n_ + w − 1. The value is formed
by

1. discarding any trailing space characters in the field and rightjustifying the remaining characters,
2. interpreting the first non-space characters as a numeric string
consisting of a single _optional_ sign (’+’ or ’-’) followed
by one or more decimal digits (’0’ through ’9’) _optionally_
containing a single decimal point (’.’). The numeric string

21

is terminated by the end of the right-justified field or by the
occurrence of any character other than a decimal point (’.’)
and the decimal integers (’0’ through ’9’). If the string
contains no explicit decimal point, then the implicit decimal point is taken as immediately preceding the rightmost
_d_ digits of the string, with leading zeros assumed if necessary. The use of implicit decimal points is _deprecated_ and
is strongly discouraged because of the possibility that _FITS_   reading programs will misinterpret the data value. Therefore,
real-formatted fields _should_ always contain an explicit decimal point.
3. If the numeric string is terminated by a
(a) ’+’ or ’-’, interpreting the following string as an exponent in the form of a signed decimal integer, or
(b) ’E’, or ’D’, interpreting the following string as an exponent of the form E or D followed by an _optionally_ signed
decimal integer constant.
4. The exponent string, if present, is terminated by the end of
the right-justified string.
5. Characters other than those specified above, including embedded space characters, are forbidden.

The numeric value of the table field is then the value of the numeric string multiplied by ten (10) to the power of the exponent
string, i.e., value = numeric string × 10 [(exponent string)] . The default
exponent is zero and a blank field has value zero. There is no difference between the F, D, and E formats; the content of the string
determines its interpretation. Numbers requiring more precision
and/or range than the local computer can support _may_ be represented. It is good form to specify a D format in TFORM _n_ for a
column of an ASCII table when that column will contain numbers that cannot be accurately represented in 32-bit IEEE binary
format (see Appendix E).

<a id="73-binary-table-extension"></a>
### 7.3. Binary-table extension

The binary-table extension is similar to the ASCII table in that it
provides a means of storing catalogs and tables of astronomical
data in _FITS_ format, however, it offers more features and provides more-efficient data storage than ASCII tables. The numerical values in binary tables are stored in more-compact binary
formats rather than coded into ASCII, and each field of a binary
table can contain an array of values rather than a simple scalar
as in ASCII tables. The first keyword record in a binary-table
extension _shall_ be XTENSION=␣'BINTABLE'.

<a id="731-mandatory-keywords"></a>
#### 7.3.1. Mandatory keywords

The XTENSION keyword is the first keyword of all binarytable extensions. The seven keywords following (BITPIX
... TFIELDS) _must_ be in the order specified in Table 17, with
no intervening keywords.

XTENSION keyword. The value field _shall_ contain the character
string 'BINTABLE'.

BITPIX keyword. The value field _shall_ contain the integer 8,
denoting that the array is an array of eight-bit bytes.

22

NAXIS keyword. The value field _shall_ contain the integer 2, denoting that the included data array is two-dimensional: rows and
columns.

NAXIS1 keyword. The value field _shall_ contain a non-negative
integer, giving the number of eight-bit bytes in each row of the
table.

NAXIS2 keyword. The value field _shall_ contain a non-negative
integer, giving the number of rows in the table.

PCOUNT keyword. The value field _shall_ contain the number of
bytes that follow the table in the supplemental data area called
the heap.

GCOUNT keyword. The value field _shall_ contain the integer 1;
the data blocks contain a single table.

TFIELDS keyword. The value field _shall_ contain a non-negative
integer representing the number of fields in each row. The maximum permissible value is 999.

TFORM n keywords. The TFORM _n_ keywords _must_ be present for
all values _n_ = 1, ..., TFIELDS and for no other values of _n_ .
The value field of this indexed keyword _shall_ contain a character string of the form _r_ T _a_ . The repeat count _r_ is the ASCII
representation of a non-negative integer specifying the number
of elements in Field _n_ . The default value of _r_ is 1; the repeat
count need not be present if it has the default value. A zero element count, indicating an empty field, is permitted. The data
type T specifies the data type of the contents of Field _n_ . Only the
data types in Table 18 are permitted. The format codes _must_ be
specified in upper case. For fields of type P or Q, the only permitted repeat counts are 0 and 1. The additional characters _a_ are
_optional_ and are not further defined in this Standard. Table 18
lists the number of bytes each data type occupies in a table row.
The first field of a row is numbered 1. The total number of bytes
_n_ row in a table row is given by

TFIELDS

_n_ row = _r_ _i_ _b_ _i_ (8)
�

_i_ =1

where _r_ _i_ is the repeat count for Field _i_, _b_ _i_ is the number of bytes
for the data type in Field _i_, and TFIELDS is the value of that
keyword, _must_ equal the value of NAXIS1.

END keyword. This keyword has no associated value. Bytes 9
through 80 _shall_ contain ASCII spaces (decimal 32 or hexadecimal 20).

<a id="732-other-reserved-keywords"></a>
#### 7.3.2. Other reserved keywords

In addition to the reserved keywords defined in Sect. 4.4.2 (except for EXTEND and BLOCKED), the following other reserved
keywords _may_ be used to describe the structure of a binary-table

Table 17: Mandatory keywords in binary-table extensions.

Position Keyword

1 XTENSION=␣'BINTABLE'

2 BITPIX = 8

3 NAXIS = 2

4 NAXIS1

5 NAXIS2

6 PCOUNT

7 GCOUNT = 1

8 TFIELDS

...

...

last END

Table 18: Valid TFORM _n_ data types in BINTABLE extensions.

TFORM _n_ value Description Eight-bit Bytes

’L’ Logical 1
’X’ Bit †
’B’ Unsigned byte 1
’I’ 16-bit integer 2
’J’ 32-bit integer 4
’K’ 64-bit integer 8
’A’ Character 1
’E’ Single-precision floating point 4
’D’ Double-precision floating point 8
’C’ Single-precision complex 8
’M’ Double-precision complex 16
’P’ Array Descriptor (32-bit) 8
’Q’ Array Descriptor (64-bit) 16

**Notes.** [(][†][)] Number of eight-bit bytes needed to contain all bits.

data array. They are _optional_, but if they appear within a binarytable extension header, they _must_ be used as defined in this section of this Standard.

TTYPE n keywords. The value field for this indexed keyword
_shall_ contain a character string giving the name of Field _n_ . It
is _strongly recommended_ that every field of the table be assigned a unique, case-insensitive name with this keyword, and
it is _recommended_ that the character string be composed only
of upper- and lower-case letters, digits, and the underscore (’ ’,
decimal 95, hexadecimal 5F) character. Use of other characters is
_not recommended_ because it may be difficult to map the column
names into variables in some languages (e.g., any hyphens, ’*’
or ’+’ characters in the name may be confused with mathematical operators). String comparisons with the TTYPE _n_ keyword
values _should not_ be case sensitive (e.g., ’TIME’ and ’Time’
_should_ be interpreted as the same name).

TUNIT n keywords. The value field _shall_ contain a character
string describing the physical units in which the quantity in
Field _n_, after any application of TSCAL _n_ and TZERO _n_, is expressed. Units _must_ follow the prescriptions in Sect. 4.3.

TSCAL n keywords. This indexed keyword _shall_ be used, along
with the TZERO _n_ keyword, to linearly scale the values in the table Field _n_ to transform them into the physical values that they
represent using Eq. 7. It _must not_ be used if the format of Field _n_
is ’A’, ’L’, or ’X’. For fields with all other data types, the value
field _shall_ contain a floating-point number representing the coefficient of the linear term in Eq. 7, which is used to compute
the true physical value of the field, or, in the case of the complex
data types ’C’ and ’M’, of the real part of the field, with the
imaginary part of the scaling factor set to zero. The default value
for this keyword is 1.0. For fields of type ’P’ or ’Q’, the values
of TSCAL _n_ and TZERO _n_ are to be applied to the values in the data
array in the heap area, not the values of the array descriptor (see
Sect. 7.3.5).

TZERO n keywords. This indexed keyword _shall_ be used, along
with the TSCAL _n_ keyword, to linearly scale the values in the table Field _n_ to transform them into the physical values that they
represent using Eq. 7. It _must not_ be used if the format of Field _n_
is ’A’, ’L’, or ’X’. For fields with all other data types, the value
field _shall_ contain a floating-point number representing the true
physical value corresponding to a value of zero in Field _n_ of the
_FITS_ file, or, in the case of the complex data types ’C’ and ’M,
in the real part of the field, with the imaginary part set to zero.
The default value for this keyword is 0.0. Equation 7 is used to
compute a true physical value from the quantity in Field _n_ . For
fields of type ’P’ or ’Q’, the values of TSCAL _n_ and TZERO _n_ are
to be applied to the values in the data array in the heap area, not
the values of the array descriptor (see Sect. 7.3.5).
In addition to its use in representing floating-point values
as scaled integers, the TZERO _n_ keyword is also used when storing unsigned integer values in the field. In this special case the
TSCAL _n_ keyword _shall_ have the default value of 1.0 and the
TZERO _n_ keyword _shall_ have one of the integer values shown in
Table 19.

Since the binary-table format does not support a native unsigned integer data type (except for the unsigned eight-bit ’B’
column type), the unsigned values are stored in the field as native signed integers with the appropriate integer offset specified
by the TZERO _n_ keyword value shown in the table. For the byte
column type, the converse technique can be used to store signed
byte values as native unsigned values with the negative TZERO _n_

23

Table 19: Usage of TZERO _n_ to represent non-default integer data types.

TFORM _n_ Native Physical TZERO _n_
data type data type

’B’ unsigned signed byte -128 (−2 [7] )
’I’ signed unsigned 16-bit 32768 (2 [15] )
’J’ signed unsigned 32-bit 2147483648 (2 [31] )
’K’ signed unsigned 64-bit 9223372036854775808 (2 [63] )

offset. In each case, the physical value is computed by adding the
offset specified by the TZERO _n_ keyword to the native data type
value that is stored in the table field.

TNULL n keywords. The value field for this indexed keyword
_shall_ contain the integer that represents an undefined value for
Field _n_ of Data Type B, I, J or K, or P or Q array-descriptor
fields (Sect. 7.3.5) that point to B, I, J, or K integer arrays. The
keyword _must not_ be used if Field _n_ is of any other data type.
The value of this keyword corresponds to the table column values before applying any transformation indicated by the TSCAL _n_
and TZERO _n_ keywords.

If the TSCAL _n_ and TZERO _n_ keywords do not have the default values of 1.0 and 0.0, respectively, then the value of the
TNULL _n_ keyword _must_ equal the actual value in the _FITS_ file
that is used to represent an undefined element and not the corresponding physical value (computed from Eq. 7). To cite a
specific, common example, _unsigned_ 16-bit integers are represented in a _signed_ integer column (with TFORM _n_ = ’I’) by setting TZERO _n_ = 32768 and TSCAL _n_ = 1. If it is desired to use
elements that have an _unsigned_ value (i.e., the physical value)
equal to 0 to represent undefined elements in the field, then the
TNULL _n_ keyword _must_ be set to the value -32768 because that is
the actual value stored in the _FITS_ file for those elements in the
field.

TDISP n keywords. The value field of this indexed keyword
_shall_ contain a character string describing the format recommended for displaying an ASCII-text representation of the contents of Field _n_ . If the table value has been scaled, the physical
value, derived using Eq. 7, _shall_ be displayed. All elements in a
field _shall_ be displayed with a single, repeated format. For purposes of display, each byte of bit (Type X) and byte (Type B)
arrays is treated as an unsigned integer. Arrays of Type A _may_ be
terminated with a zero byte. Only the format codes in Table 20,
interpreted as Fortran (ISO 2004) output formats, and discussed
in more detail in Sect. 7.3.4, are permitted for encoding. The
format codes _must_ be specified in upper case. If the Bw.m, Ow.m,
and Zw.m formats are not readily available to the reader, the Iw.m
display format _may_ be used instead, and if the ENw.d and ESw.d
formats are not available, Ew.d _may_ be used. In the case of fields
of Type P or Q, the TDISP _n_ value applies to the data array pointed
to by the array descriptor (Sect. 7.3.5), not the values in the array
descriptor itself.

THEAP keyword. The value field of this keyword _shall_ contain
an integer providing the separation, in bytes, between the start
of the main data table and the start of a supplemental data area
called the heap. The default value, which is also the minimum
allowed value, _shall_ be the product of the values of NAXIS1 and

24

NAXIS2. This keyword _shall not_ be used if the value of PCOUNT
is 0. The use of this keyword is described in in Sect. 7.3.5.

TDIM n keywords. The value field of this indexed keyword _shall_
contain a character string describing how to interpret the contents of Field _n_ as a multi-dimensional array with a format of
’( _l_, _m_, _n_ ...)’, where _l_, _m_, _n_, ...are the dimensions of the array. The data are ordered such that the array index of the first
dimension given ( _l_ ) is the most rapidly varying, and that of the
last dimension given is the least rapidly varying. The total number of elements in the array equals the product of the dimensions
specified in the TDIM _n_ keyword. The size _must_ be less than or
equal to the repeat count in the TFORM _n_ keyword, or, in the case
of columns that have a ’P’ or ’Q’ TFORM _n_ data type, less than or
equal to the array length specified in the variable-length array descriptor (see Sect. 7.3.5). In the special case where the variablelength array descriptor has a size of zero, then the TDIM _n_ keyword is not applicable. If the number of elements in the array
implied by the TDIM _n_ is fewer than the allocated size of the array in the _FITS_ file, then the unused trailing elements _should_ be
interpreted as containing undefined fill values.

A character string is represented in a binary table by a onedimensional character array, as described under ‘Character’ in
the list of data types in Sect. 7.3.3. For example, a Fortran
CHARACTER*20 variable could be represented in a binary table as
a character array declared as TFORM _n_ = ’20A’. Arrays of strings,
i.e., multi-dimensional character arrays, _may_ be represented using the TDIM _n_ notation. For example, if TFORM _n_ = ’60A’ and
TDIM _n_ = ’(5,4,3)’, then the entry consists of a 4 × 3 array of
strings each comprising five characters.

The following four keywords _may_ be used to specify minimum
and maximum values in numerical columns of a _FITS_ ASCII or
binary table. These keywords _must_ have the same data type as
the physical values in the associated column (either an integer or
a floating-point number). Any undefined elements in the column
or any other IEEE special values in the case of floating-point
columns _shall_ be excluded when determining the value of these
keywords.

TDMIN n keywords. The value field _shall_ contain a number giving the minimum physical value contained in Column _n_ of the
table. This keyword is analogous to the DATAMIN keyword that
is defined for arrays in Sect. 4.4.2.5.

TDMAX n keywords. The value field _shall_ contain a number giving the maximum physical value contained in Column _n_ of the
table. This keyword is analogous to the DATAMAX keyword that
is defined for arrays in Sect. 4.4.2.5.

Table 20: Valid TDISP _n_ format values in BINTABLE extensions.

Field Value Data type

Aw Character
Lw Logical
Iw.m Integer
Bw.m Binary, integers only
Ow.m Octal, integers only
Zw.m Hexadecimal, integers only
Fw.d Floating-point, fixed decimal notation
Ew.dEe Floating-point, exponential notation
ENw.d Engineering; E format with exponent multiple of three
ESw.d Scientific; same as EN but non-zero leading digit if not zero
Gw.dEe General; appears as F if significance not lost, else E.
Dw.dEe Floating-point, exponential notation

**Notes.** w is the width in characters of displayed values, m is the minimum number of digits displayed, d is the number of digits to right of decimal,
and e is number of digits in exponent. The .m and Ee fields are _optional_ .

TLMIN n keywords. The value field _shall_ contain a number that
specifies the minimum physical value in Column _n_ that has
a valid meaning or interpretation. The column is not _required_
to actually contain any elements that have this value, and the
column _may_ contain elements with physical values less than
TLMIN _n_, however, the interpretation of any such out-of-range
column elements is not defined.

TLMAX n keywords. The value field _shall_ contain a number that
specifies the maximum physical value in Column _n_ that has a
valid meaning or interpretation. The column is not _required_ to
actually contain any elements that have this value, and the column _may_ contain elements with physical values greater than
TLMAX _n_, however, the interpretation of any such out-of-range
column elements is not defined.

The TLMIN _n_ and TLMAX _n_ keywords are commonly used when
constructing histograms of the data values in a column. For example, if a table contains columns that give the _X_ and _Y_ pixel
location of a list of photons that were detected by a photoncounting device, then the TLMIN _n_ and TLMAX _n_ keywords could
be used respectively to specify the minimum and maximum values that the detector is capable of assigning to the _X_ and _Y_
columns.

<a id="733-data-sequence"></a>
#### 7.3.3. Data sequence

The data in a binary-table extension _shall_ consist of a main data
table, which _may_, but need not, be followed by additional bytes
in the supplemental data area. The positions in the last data block
after the last additional byte, or, if there are no additional bytes,
the last character of the last row of the main data table, _shall_ be
filled by setting all bits to zero.

<a id="7331-main-data-table"></a>
##### 7.3.3.1. Main data table

The table is constructed from a two-dimensional byte array. The number of bytes in a row _shall_ be specified by the
value of the NAXIS1 keyword and the number of rows _shall_
be specified by the NAXIS2 keyword of the associated header.
Within a row, fields _shall_ be stored in order of increasing column

number, as determined from the _n_ of the TFORM _n_ keywords. The
number of bytes in a row and the number of rows in the table
_shall_ determine the size of the byte array. Every row in the array
_shall_ have the same number of bytes. The first row _shall_ begin at
the start of the data block immediately following the last header
block. Subsequent rows _shall_ begin immediately following the
end of the previous row, with no intervening bytes, independent
of the _FITS_ block structure. Words need not be aligned along
word boundaries.

Each row in the array _shall_ consist of a sequence of from 0
to 999 fields as specified by the TFIELDS keyword. The number
of elements in each field and their data type _shall_ be specified
by the TFORM _n_ keyword in the associated header. A separate format keyword _must_ be provided for each field. The location and
format of fields _shall_ be the same for every row. Fields _may_ be
empty, if the repeat count specified in the value of the TFORM _n_
keyword of the header is 0. Writers of binary tables _should_ select
a format appropriate to the form, range of values, and accuracy
of the data in the table. The following data types, and no others,
are permitted.

Logical. If the value of the TFORM _n_ keyword specifies Data
Type ’L’, the contents of Field _n shall_ consist of ASCII T indicating true or ASCII F, indicating false. A 0 byte (hexadecimal
00) indicates a NULL value.

Bit array. If the value of the TFORM _n_ keyword specifies data type
’X’, the contents of Field _n shall_ consist of a sequence of bits
starting with the most-significant bit; the bits following _shall_ be
in order of decreasing significance, ending with the least significant bit. A bit array _shall_ be composed of an integral number of
bytes, with those bits following the end of the data set to zero.
No null value is defined for bit arrays.

Character. If the value of the TFORM _n_ keyword specifies Data
Type ’A’, Field _n shall_ contain a character string of zero-ormore members, composed of the restricted set of ASCII-text
characters. This character string _may_ be terminated before the
length specified by the repeat count by an ASCII NULL (hexadecimal code 00). Characters after the first ASCII NULL are
not defined. A string with the number of characters specified by

25

the repeat count is not NULL terminated. Null strings are defined
by the presence of an ASCII NULL as the first character.

Unsigned 8-Bit integer. If the value of the TFORM _n_ keyword
specifies Data Type ’B’, the data in Field _n shall_ consist of unsigned eight-bit integers, with the most-significant bit first, and
subsequent bits in order of decreasing significance. Null values
are given by the value of the associated TNULL _n_ keyword. Signed
integers can be represented using the convention described in
Sect. 5.2.5.

16-Bit integer. If the value of the TFORM _n_ keyword specifies
Data Type ’I’, the data in Field _n shall_ consist of two’s complement signed 16-bit integers, contained in two bytes. The mostsignificant byte _shall_ be first (big-endian byte order). Within
each byte the most-significant bit _shall_ be first, and subsequent
bits _shall_ be in order of decreasing significance. Null values are
given by the value of the associated TNULL _n_ keyword. Unsigned
integers can be represented using the convention described in
Sect. 5.2.5.

32-Bit integer. If the value of the TFORM _n_ keyword specifies
Data Type ’J’, the data in Field _n shall_ consist of two’s complement signed 32-bit integers, contained in four bytes. The mostsignificant byte _shall_ be first, and subsequent bytes _shall_ be in
order of decreasing significance (big-endian byte order). Within
each byte, the most-significant bit _shall_ be first, and subsequent
bits _shall_ be in order of decreasing significance. Null values are
given by the value of the associated TNULL _n_ keyword. Unsigned
integers can be represented using the convention described in
Sect. 5.2.5.

64-Bit integer. If the value of the TFORM _n_ keyword specifies
Data Type ’K’, the data in Field _n shall_ consist of two’s complement signed 64-bit integers, contained in eight bytes. The
most-significant byte _shall_ be first, and subsequent bytes _shall_
be in order of decreasing significance. Within each byte, the
most-significant bit _shall_ be first, and subsequent bits _shall_ be
in order of decreasing significance (big-endian byte order). Null
values are given by the value of the associated TNULL _n_ keyword.
Unsigned integers can be represented using the convention described in Sect. 5.2.5.

Single-precision floating point. If the value of the TFORM _n_ keyword specifies Data Type ’E’, the data in Field _n shall_ consist
of ANSI/IEEE-754 (IEEE 1985) 32-bit floating-point numbers,
in big-endian byte order, as described in Appendix E. All IEEE
special values are recognized. The IEEE NaN is used to represent null values.

Double-precision floating point. If the value of the TFORM _n_
keyword specifies Data Type ’D’, the data in Field _n shall_ consist of ANSI/IEEE-754 (IEEE 1985) 64-bit double-precision
floating-point numbers, in big-endian byte order, as described in
Appendix E. All IEEE special values are recognized. The IEEE
NaN is used to represent null values.

26

Single precision complex. If the value of the TFORM _n_ keyword
specifies Data Type ’C’, the data in Field _n shall_ consist of a
sequence of pairs of 32-bit single-precision floating-point numbers. The first member of each pair _shall_ represent the real part
of a complex number, and the second member _shall_ represent
the imaginary part of that complex number. If either member
contains an IEEE NaN, the entire complex value is null.

Double-precision complex. If the value of the TFORM _n_ keyword
specifies Data Type ’M’, the data in Field _n shall_ consist of a
sequence of pairs of 64-bit double-precision floating-point numbers. The first member of each pair _shall_ represent the real part
of a complex number, and the second member of the pair _shall_
represent the imaginary part of that complex number. If either
member contains an IEEE NaN, the entire complex value is null.

Array descriptor. The repeat count on the P and Q arraydescriptor fields _must_ either have a value of 0 (denoting an empty
field of zero bytes) or 1. If the value of the TFORM _n_ keyword
specifies Data Type ’1P’, the data in Field _n shall_ consist of one
pair of 32-bit integers. If the value of the TFORM _n_ keyword specifies Data Type ’1Q’, the data in Field _n shall_ consist of one pair
of 64-bit integers. The meaning of these integers is defined in
Sect. 7.3.5.

<a id="7332-bytes-following-main-table"></a>
##### 7.3.3.2. Bytes following main table

The main data table _may_ be followed by a supplemental
data area called the heap. The size of the supplemental data
area, in bytes, is specified by the value of the PCOUNT keyword.
The use of this data area is described in Sect. 7.3.5.

<a id="734-data-display"></a>
#### 7.3.4. Data display

The indexed TDISP _n_ keyword _may_ be used to describe the recommended format for displaying an ASCII-text representation
of the contents of Field _n_ . The permitted display format codes
for each type of data (i.e., character strings, logical, integer, or
real) are given in Table 20 and described below.

Character data. If the table column contains a character string
(with TFORM _n_ = ’rA’) then the TDISP _n_ format code _must_ be
Aw, where w is the number of characters to display. If the character datum has length less than or equal to w, it is represented
on output right-justified in a string of w characters. If the character datum has length greater than w, the first w characters of
the datum are represented on output in a string of w characters.
Character data are not surrounded by single- or double-quotation
marks unless those marks are themselves part of the data value.

Logical data. If the table column contains logical data (with
TFORM _n_ = ’rL’) then the TDISP _n_ format code _must_ be Lw, where
w is the width in characters of the display field. Logical data are
represented on output with the character T for true or F for false
right-justified in a space-filled string of w characters. A null value
_may_ be represented by a string of w space characters.

Integer data. If the table column contains integer data (with
TFORM _n_ = ’rX’, ’rB’, ’rI’, ’rJ’, or ’rK’) then the TDISP _n_
format code _may_ have any of these forms: Iw.m, Bw.m, Ow.m, or
Zw.m. The default value of m is one and the ’.m’ is _optional_ . The
first letter of the code specifies the number base for the encoding
with I for decimal (10), B for binary (2), O for octal (8), and Z
for hexadecimal (16). Hexadecimal format uses the upper-case
letters A through F to represent decimal values 10 through 15.
The output field consists of w characters containing zero-or-more
leading spaces followed by a minus sign if the internal datum is
negative (only in the case of decimal encoding with the I format code), followed by the magnitude of the internal datum in
the form of an unsigned-integer constant in the specified number
base, with only as many leading zeros as are needed to have at
least m numeric digits. Note that m ≤ w is allowed if all values
are positive, but m < w is _required_ if any values are negative. If
the number of digits required to represent the integer datum exceeds w, then the output field consists of a string of w asterisk (*)
characters.

Real data. If the table column contains real data (with
TFORM _n_ = ’rE’, or ’rD’) or contains integer data (with any
of the TFORM _n_ format codes listed in the previous paragraph),
which are _recommended_ to be displayed as real values (i.e., especially in cases where the integer values represent scaled physical values using Eq. 7), then the TDISP _n_ format code _may_ have
any of these forms: Fw.d, Ew.dEe, Dw.dEe, ENw.d, or ESw.d.
In all cases, the output is a string of w characters including the
decimal point, any sign characters, and any exponent including
the exponent’s indicators, signs, and values. If the number of
digits required to represent the real datum exceeds w, then the
output field consists of a string of w asterisk (*) characters. In all
cases, d specifies the number of digits to appear to the right of
the decimal point.
The F format code output field consists of w − d − 1 characters containing zero-or-more leading spaces, followed by a minus sign if the internal datum is negative, followed by the absolute magnitude of the internal datum in the form of an unsignedinteger constant. These characters are followed by a decimal
point (’.’) and d characters giving the fractional part of the
internal datum, rounded by the normal rules of arithmetic to d
fractional digits.

For the E and D format codes, an exponent is taken such that
the fraction 0.1 ≤|datum|/10 [exponent] < 1.0. The fraction (with
appropriate sign) is output with an F format of width w − e − 2
characters with d characters after the decimal followed by an E
or D followed by the exponent as a signed e + 1 character integer
with leading zeros as needed. The default value of e is 2 when
the Ee portion of the format code is omitted. If the exponent
value will not fit in e + 1 characters but will fit in e + 2 then the E
(or D) is omitted and the wider field used. If the exponent value
will not fit (with a sign character) in e + 2 characters, then the
entire w-character output field is filled with asterisks (*).

The ES format code is processed in the same manner as the
E format code except that the exponent is taken so that 1.0 ≤
fraction < 10.

The EN format code is processed in the same manner as the
E format code except that the exponent is taken to be an integer
multiple of three and so that 1.0 ≤ fraction < 1000.0. All real
format codes have number base 10. There is no difference be

tween E and D format codes on input other than an implication
with the latter of greater precision in the internal datum.
The Gw.dEe format code _may_ be used with data of any type.
For data of type integer, logical, or character, it is equivalent to
Iw, Lw, or Aw, respectively. For data of type real, it is equivalent
to an F format (with different numbers of characters after the
decimal) when that format will accurately represent the value
and is equivalent to an E format when the number (in absolute
value) is either very small or very large. Specifically, for real
values outside the range 0.1 − 0.5 × 10 [−] [d] [−][1] ≤ value < 10 [d] −
0.5, it is equivalent to Ew.dEe. For real values within the above
range, it is equivalent to Fw [′] .d [′] followed by 2 + e spaces, where
datum value lies in the range 10 _[k]_ [−][1] [ �] 1 − 0.5×10 [−] [d][�] ≤ value ≤

10 _[k]_ [ �] 1 − 0.5×10 [−] [d][�] .

Complex data. If the table column contains complex data (with
TFORM _n_ = ’rC’, or ’rM’) then they _may_ be displayed with any
of the real data formats as described above. The same format is
used for the real and imaginary parts. It is _recommended_ that the
two values be separated by a comma and enclosed in parentheses
with a total field width of 2w + 3.

<a id="735-variable-length-arrays"></a>
#### 7.3.5. Variable-length arrays

One of the most attractive features of binary tables is that any
field of the table can be an array. In the standard case this is
a fixed-size array, i.e., a fixed amount of storage is allocated in
each row for the array data—whether it is used or not. This is fine
so long as the arrays are small or a fixed amount of array data
will be stored in each field, but if the stored array length varies
for different rows, it is necessary to impose a fixed upper limit on
the size of the array that can be stored. If this upper limit is made
too large excessive wasted space can result and the binary-table
mechanism becomes seriously inefficient. If the limit is set too
low then storing certain types of data in the table could become
impossible.
The variable-length array construct presented here was devised to deal with this problem. Variable-length arrays are implemented in such a way that, even if a table contains such arrays, a simple reader program that does not understand variablelength arrays will still be able to read the main data table (in
other words a table containing variable-length arrays conforms
to the basic binary-table standard). The implementation chosen
is such that the rows in the main data table remain fixed in size
even if the table contains a variable-length array field, allowing
efficient random access to the main data table.
Variable-length arrays are logically equivalent to regular
static arrays, the only differences being 1) the length of the stored
array can differ for different rows, and 2) the array data are not
stored directly in the main data table. Since a field of any data
type can be a static array, a field of any data type can also be
a variable-length array (excluding the Type P and Q variablelength array descriptors themselves, which are not a data type so
much as a storage-class specifier). Other established _FITS_ conventions that apply to static arrays will generally apply as well
to variable-length arrays.
A variable-length array is declared in the table header with
one of the following two special field data-type specifiers

_r_ P _t_ ( _e_ max )

27

_r_ Q _t_ ( _e_ max )

where the ’P’ or ’Q’ indicates the presence of an array descriptor (described below), the element count _r should_ be 0, 1, or
absent, _t_ is a character denoting the data type of the array data
(L, X, B, I, J, K, etc., but not P or Q), and _e_ max is a quantity guaranteed to be equal to or greater than the maximum number of
elements of type _t_ actually stored in any row of the table. There
is no built-in upper limit on the size of a stored array (other than
the fundamental limit imposed by the range of the array descriptor, defined below); _e_ max merely reflects the size of the largest
array actually stored in the table, and is provided to avoid the
need to preview the table when, for example, reading a table
containing variable-length elements into a database that supports
only fixed-size arrays. There _may_ be additional characters in the
TFORM _n_ keyword following the _e_ max .
For example,

TFORM8 = ’PB(1800)’ / Variable byte array

indicates that Field 8 of the table is a variable-length array of
type byte, with a maximum stored array length not to exceed
1800 array elements (bytes in this case).
The data for the variable-length arrays in a table are not
stored in the main data table; they are stored in a supplemental
data area, the heap, following the main data table. What is stored
in the main data table field is an _array descriptor_ . This consists
of two 32-bit signed integer values in the case of ’P’ array descriptors, or two 64-bit signed integer values in the case of ’Q’
array descriptors: the number of elements (array length) of the
stored array, followed by the zero-indexed byte offset of the first
element of the array, measured from the start of the heap area.
The meaning of a negative value for either of these integers is
not defined by this Standard. Storage for the array is contiguous.
The array descriptor for Field _N_ as it would appear embedded in
a table row is illustrated symbolically below.

If the stored array length is zero, there is no array data, and
the offset value is undefined (it _should_ be set to zero). The storage referenced by an array descriptor _must_ lie entirely within the
heap area; negative offsets are not permitted.
A binary table containing variable-length arrays consists of
three principal segments, as follows.

[table header] [main data table] (optional gap) [heap area]

The table header consists of one or more 2880-byte header
blocks with the last block indicated by the keyword END somewhere in the block. The main data table begins with the first data
block following the last header block and is NAXIS1 × NAXIS2
bytes in length. The zero-indexed byte offset to the start of
the heap, measured from the start of the main data table, _may_
be given by the THEAP keyword in the header. If this keyword is missing then the heap begins with the byte immediately
following main data table (i.e., the default value of THEAP is
NAXIS1 × NAXIS2). This default value is the minimum allowed
value for the THEAP keyword, because any smaller value would
imply that the heap and the main data table overlap. If the THEAP
keyword has a value larger than this default value, then there is
a gap between the end of the main data table and the start of
the heap. The total length in bytes of the supplemental data area

28

following the main data table (gap plus heap) is given by the
PCOUNT keyword in the table header.
For example, suppose a table contains five rows that are each
168 bytes long, with a heap area 3000 bytes long, beginning at
an offset of 2880, thereby aligning the main data table and heap
areas on data block boundaries (this alignment is not necessarily
recommended but is useful for this example). The data portion of
the table consists of three 2880-byte data blocks: the first block
contains the 840 bytes from the five rows of the main data table
followed by 2040 fill bytes; the heap completely fills the second
block; the third block contains the remaining 120 bytes of the
heap followed by 2760 fill bytes. PCOUNT gives the total number
of bytes from the end of the main data table to the end of the
heap, and in this example has a value of 2040 + 2880 + 120 =
5040. This is expressed in the table header as shown below.

NAXIS1 = 168 / Width of table row in bytes
NAXIS2 = 5 / Number of rows in table
PCOUNT = 5040 / Random parameter count

...
THEAP = 2880 / Byte offset of heap area

The values of TSCAL _n_ and TZERO _n_ for variable-length array
column entries are to be applied to the values in the data array in
the heap area, not the values of the array descriptor. These keywords can be used to scale data values in either static or variablelength arrays.

<a id="736-variable-length-array-guidelines"></a>
#### 7.3.6. Variable-length-array guidelines

While the above description is sufficient to define the required
features of the variable-length array implementation, some hints
regarding usage of the variable-length array facility might also
be useful.
Programs that read binary tables should take care to not assume more about the physical layout of the table than is _required_
by the specification. For example, there are no requirements on
the alignment of data within the heap. If efficient runtime access is a concern one might want to design the table so that data
arrays are aligned to the size of an array element. In another
case one might want to minimize storage and forgo any efforts
at alignment (by careful design it is often possible to achieve
both goals). Variable-length array data _may_ be stored in the heap
in any order, i.e., the data for row _N_ +1 are not necessarily stored
at a larger offset than that for row _N_ . There _may_ be gaps in the
heap where no data are stored. Pointer aliasing is permitted, i.e.,
the array descriptors for two or more arrays _may_ point to the
same storage location (this could be used to save storage if two
or more arrays are identical).
Byte arrays are a special case because they can be used
to store a ‘typeless’ data sequence. Since _FITS_ is a machineindependent storage format, some form of machine-specific data
conversion (byte swapping, floating-point format conversion) is
implied when accessing stored data with types such as integer
and floating, but byte arrays are copied to and from external storage without any form of conversion.
An important feature of variable-length arrays is that it is
possible that the stored array length _may_ be zero. This makes
it possible to have a column of the table for which, typically,
no data are present in each stored row. When data are present,
the stored array can be as large as necessary. This can be useful
when storing complex objects as rows in a table.

Accessing a binary table stored on a random-access storage
medium is straightforward. Since the rows of data in the main
data table are fixed in size they can be randomly accessed given
the row number, by computing the offset. Once the row has been
read in, any variable-length array data can be directly accessed
using the element count and offset given by the array descriptor
stored in that row.

Reading a binary table stored on a sequential-access storage
medium requires that a table of array descriptors be built up as
the main data table rows are read in. Once all the table rows
have been read, the array descriptors are sorted by the offset of
the array data in the heap. As the heap data are read, arrays are
extracted sequentially from the heap and stored in the affected
rows using the back pointers to the row and field from the table
of array descriptors. Since array aliasing is permitted, it might
be necessary to store a given array in more than one field or row.
Variable-length arrays are more complicated than regular
static arrays and might not be supported by some software systems. The producers of _FITS_ data products should consider the
capabilities of the likely recipients of their files when deciding
whether or not to use this format, and as a general rule should
use it only in cases where it provides significant advantages over
the simpler fixed-length array format. In particular, the use of
variable-length arrays might present difficulties for applications
that ingest the _FITS_ file via a sequential input stream, because
the application cannot fully process any rows in the table until
after the entire fixed-length table, and potentially the entire heap
has been transmitted as outlined in the previous paragraph.

<a id="8-world-coordinate-systems"></a>
## 8. World-coordinate systems

Representations of the mapping between image coordinates and
physical (i.e., world) coordinate systems (WCSs) _may_ be represented within _FITS_ HDUs. The keywords that are used to express
these mappings are now rigorously defined in a series of papers on world-coordinate systems (Greisen & Calabretta 2002),
celestial-coordinate systems (Calabretta & Greisen 2002),
spectral-coordinate systems (Greisen et al. 2006), and timecoordinate systems (Rots et al. 2015). An additional spherical projection, called HEALPix, is defined in reference
(Calabretta & Roukema 2007). These WCS papers have been
formally approved by the IAUFWG and therefore are _incor-_
_porated by reference_ as an official part of this Standard. The
reader should refer to these papers for additional details and
background information that cannot be included here. Various
updates and corrections to the primary WCS papers have been
compiled by the authors, and are reflected in this section.
Therefore, where conflicts exist, the description in this Standard
will prevail.

<a id="81-basic-concepts"></a>
### 8.1. Basic concepts

Rather than store world coordinates separately for each datum,
the regular lattice structure of a _FITS_ image offers the possibility of defining rules for computing world coordinates at each
point. As stated in Sect. 3.3.2 and depicted in Fig. 1, image array data are addressed via _integral array indices_ that range in
value from 1 to NAXIS _j_ on Axis _j_ . Recognizing that image data
values may have an extent, for example an angular separation,
spectral channel width or time span, and thus that it may make
sense to interpolate between them, these integral array indices

|Pixel<br>Coordinates|Col2|
|---|---|
|||

|Intermediate Pixel<br>Coordinates|Col2|
|---|---|
|||

|Intermediate World<br>Coordinates|Col2|
|---|---|
|||

where _r_ _j_ are the pixel-coordinateelements of the reference point,
_j_ indexes the pixel axis, and _i_ the world axis. The _m_ _i j_ matrix

29

**`CRPIX`** _**j**_,
**`PC`** _**i_j**_ or
**`CD`** _**i_j**_

**`CDELT`** _**i**_

**`CTYPE`** _**i**_,
**`CRVAL`** _**i**_

**`PV`** _**i_m**_

Fig. 2: A schematic view of converting pixel coordinates to
world coordinates.

_may_ be generalized to floating-point _pixel coordinates_ . Integral
pixel-coordinate values coincide with the corresponding array
indices, while fractional pixel-coordinate values lie between array indices and thus imply interpolation. Pixel-coordinate values are defined at all points within the image lattice and outside
it (except along _conventional_ axes, see Sect. 8.5). They form
the basis of the world-coordinate formalism in _FITS_ depicted
schematically in Fig. 2.
The essence of representing world-coordinate systems in
_FITS_ is the association of various reserved keywords with elements of a transformation (or a series of transformations), or
with parameters of a projection function. The conversion from
pixel coordinates in the data array to world coordinates is simply a matter of applying the specified transformations (in order)
via the appropriate keyword values; conversely, defining a WCS
for an image amounts to solving for the elements of the transformation matrix(es) or coefficients of the function(s) of interest
and recording them in the form of WCS keyword values. The
description of the WCS systems and their expression in _FITS_
HDUs is quite extensive and detailed, but is aided by a careful
choice of notation. Key elements of the notation are summarized
in Table 21, and are used throughout this section. The formal
definitions of the keywords appear in the following subsections.
The conversion of image pixel coordinates to world coordinates is a multi-step process, as illustrated in Fig. 2.
For all coordinate types, the first step is a linear transformation applied via matrix multiplication of the vector of pixelcoordinate elements, _p_ _j_ :

_q_ _i_ =

_N_
� _m_ _i j_ ( _p_ _j_ − _r_ _j_ ) (9)

_j_ =1

Table 21: WCS and celestial coordinates notation.

Variable(s) Meaning Related _FITS_ keywords

_i_ Index variable for world coordinates
_j_ Index variable for pixel coordinates
_a_ Alternative WCS version code
_p_ _j_ Pixel coordinates
_r_ _j_ Reference pixel coordinates CRPIX _ja_
_m_ _ij_ Linear-transformation matrix CD _i ja_ or PC _i ja_
_s_ _i_ Coordinate scales CDELT _ia_
( _x_, _y_ ) Projection plane coordinates
(φ, θ) Native longitude and latitude
(α, δ) Celestial longitude and latitude
(φ 0, θ 0 ) Native longitude and latitude of the fiducial point PV i 1 _a_ [†], PV i 2 _a_ [†]
(α 0, δ 0 ) Celestial longitude and latitude of the fiducial point CRVAL _ia_
(α _p_, δ _p_ ) Celestial longitude and latitude of the native pole
(φ _p_, θ _p_ ) Native longitude and latitude of the celestial pole LONPOLE _a_ (=PV i 3 _a_ [†] ),
LATPOLE _a_ (=PV i 4 _a_ [†] )

**Notes.** - Associated with _Longitude_ Axis _i_ .

is a non-singular, square matrix of dimension _N_ × _N_, where _N_
is the number of world-coordinate axes. The elements _q_ _i_ of the
resulting _intermediate pixel coordinate_ vector are offsets, in dimensionless pixel units, from the reference point along axes coincident with those of the _intermediate world coordinates_ . Thus,
the conversion of _q_ _i_ to the corresponding Intermediate-worldcoordinate Element _x_ _i_ is a simple scale:

_x_ _i_ = _s_ _i_ _q_ _i_ . (10)

There are three conventions for associating _FITS_ keywords
with the above transformations. In the first formalism, the matrix
elements _m_ _i j_ are encoded in the PC _i j_ keywords and the scale
factors _s_ _i_ are encoded in the CDELT _i_ keywords, which _must_ have
non-zero values. In the second formalism Eqs. (9) and (10) are
combined as

In other cases it is more complicated, and may require the application of some non-linear algorithm (e.g., a projection, as for
celestial coordinates), which may require the specification of additional parameters. Where necessary, numeric parameter values
for non-linear algorithms _must_ be specified via PV _i m_ keywords
and character-valued parameters will be specified via PS _i m_ keywords, where _m_ is the parameter number.

The application of these formalisms to coordinate systems of
interest is discussed in the following sub-sections: Sect. 8.2 describes general WCS representations (see Greisen & Calabretta
2002), Sect. 8.3 describes celestial-coordinate systems (see
Calabretta & Greisen 2002)), Sect. 8.4 describes spectralcoordinate systems (see Greisen et al. 2006), and Sect. 9 describes the representation of time coordinates (see Rots et al.
2015).

<a id="82-world-coordinate-system-representations"></a>
### 8.2. World-coordinate-system representations

A variety of keywords have been reserved for computing the
coordinate values that are to be associated with any pixel location within an array. The full set is given in Table 22; those in
most common usage are defined in detail below for convenience.
Coordinate-system specifications _may_ appear in HDUs that contain simple images in the primary array or in an IMAGE extension.
Images _may_ also be stored in a multi-dimensional vector cell of
a binary table, or as a tabulated list of pixel locations (and optionally, the pixel value) in a table. In these last two types of image representations, the WCS keywords have a different naming
convention, which reflects the needs of the tabular data structure
and the eight-character limit for keyword lengths, but otherwise
follow exactly the same rules for type, usage, and default values.
See reference Calabretta & Greisen (2002) for example usage of
these keywords. All forms of these reserved keywords _must_ be
used only as specified in this Standard.

In the case of the binary-table vector representation, it is
possible that the images contained in a given column of the
table have different coordinate transformation values. Table 9
of Calabretta & Greisen (2002) illustrates a technique (com

_x_ _i_ =

_N_
�( _s_ _i_ _m_ _i j_ )( _p_ _j_ − _r_ _j_ ) (11)

_j_ =1

and the CD _i j_ keywords encode the product _s_ _i_ _m_ _i j_ . The third convention was widely used before the development of the two previously described conventions and uses the CDELT _i_ keywords
to define the image scale and the CROTA2 keyword to specify
a bulk rotation of the image plane. Use of the CROTA2 keyword
is now deprecated, and instead the newer PC _i j_ or CD _i j_ keywords
are _recommended_ because they allow for skewed axes and fully
general rotation of multi-dimensional arrays. The CDELT _i_ and
CROTA2 keywords _may_ co-exist with the CD _i j_ keywords (but
the CROTA2 _must not_ occur with the PC _i j_ keywords) as an aid
to old _FITS_ interpreters, but these keywords _must_ be ignored
by software that supports the CD _i j_ keyword convention. In all
these formalisms the reference pixel coordinates _r_ _j_ are encoded
in the CRPIX _i_ keywords, and the world coordinates at the reference point are encoded in the CRVAL _i_ keywords. For additional
details, see Greisen & Calabretta (2002).
The third step of the process, computing the final world coordinates, depends on the type of coordinate system, which is
indicated with the value of the CTYPE _i_ keyword. For some simple, linear cases an appropriate choice of normalization for the
scale factors allows the world coordinates to be taken directly (or
by applying a constant offset) from the _x_ _i_ (e.g., some spectra).

30

monly known as the “Green Bank Convention [10] ”), which utilizes additional columns in the table to record the coordinatetransformation values that apply to the corresponding image
in each row of the table. More information is provided in
Appendix L.
The keywords given below constitute a complete set of fundamental attributes for a WCS description. Although their inclusion in an HDU is optional, _FITS_ writers _should_ include a complete set of keywords when describing a WCS. In the event that
some keywords are missing, default values _must_ be assumed, as
specified below.

WCSAXES – [integer; default: NAXIS, or larger of WCS indices _i_
or _j_ ]. Number of axes in the WCS description. This keyword,
if present, _must_ precede all WCS keywords except NAXIS in
the HDU. The value of WCSAXES _may_ exceed the number of
pixel axes for the HDU.
CTYPE _i_ – [string; indexed; default: '␣' (i.e. a linear, undefined
axis)]. Type for the Intermediate-coordinate Axis _i_ . Any co
                                      ordinate type that is not covered by this Standard or an offi
cially recognized _FITS_ convention _shall_ be taken to be linear.
All non-linear coordinate system names _must_ be expressed
in ‘4–3’ form: the first four characters specify the coordinate

                 type, the fifth character is a hyphen (‘ ’), and the remaining three characters specify an algorithm code for computing
the world coordinate value. Coordinate types with names of
fewer than four characters are padded on the right with hyphens, and algorithm codes with fewer than three characters are padded on the right with blanks [11] . Algorithm codes
_should_ be three characters.
CUNIT _i_ – [string; indexed; default: '␣' (i.e., undefined)].
Physical units of CRVAL and CDELT for Axis _i_ . Note that units
_should_ always be specified (see Sect. 4.3). Units for celestial
coordinate systems defined in this Standard _must_ be degrees.

CRPIX _j_ – [floating point; indexed; default: 0.0]. Location of
the reference point in the image for Axis _j_ corresponding to
_r_ _j_ in Eq. (9). Note that the reference point _may_ lie outside the
CRVAL _i_ – [floating point; indexed; default: 0.0]. Worldcoordinate value at the reference point of Axis _i_ .
CDELT _i_ – [floating point; indexed; default: 1.0]. Increment of
the world coordinate at the reference point for Axis _i_ . The
value _must not_ be zero.
CROTA _i_ – [floating point; indexed; default: 0.0]. The amount
of rotation from the standard coordinate system to a different
coordinate system. Further use of this of this keyword is deprecated, in favor of the newer formalisms that use the CD _i j_
or PC _i j_ keywords to define the rotation.
PC _i j_ – [floating point; defaults: 1.0 when _i_ = _j_, 0.0 otherwise]. Linear transformation matrix between Pixel Axes
_j_ and Intermediate-coordinate Axes _i_ . The PC _i j_ matrix
_must not_ be singular.
CD _i j_ – [floating point; defaults: 0.0, but see below]. Linear
transformation matrix (with scale) between Pixel Axes _j_
and Intermediate-coordinate Axes _i_ . This nomenclature is

10 Named after a meeting held in Green Bank, West Virginia, USA
in 1989 to develop standards for the interchange of single-dish radioastronomy data.
11
Example: ’RA---UV ’.

equivalent to PC _i j_ when CDELT _i_ is unity. The CD _i j_ matrix
_must not_ be singular. Note that the CD _i j_ formalism is an exclusive alternative to PC _i j_, and the CD _i j_ and PC _i j_ keywords
_must not_ appear together within an HDU.

In addition to the restrictions noted above, if any CD _i j_ keywords
are present in the HDU, all other unspecified CD _i j_ keywords
_shall_ default to zero. If no CD _i j_ keywords are present then the
header _shall_ be interpreted as being in PC _i j_ form whether or not
any PC _i j_ keywords are actually present in the HDU.
Some non-linear algorithms that describe the transformation
between pixel and intermediate-coordinate axes require parameter values. A few non-linear algorithms also require charactervalued parameters, e.g., table lookups require the names of the
table extension and the columns to be used. Where necessary
parameter values _must_ be specified via the following keywords.

PV _i m_ – [floating point]. Numeric parameter values for
Intermediate-world-coordinateAxis _i_, where _m_ is the parameter number. Leading zeros _must not_ be used, and _m may_
have only values in the range 0 through 99, and that are defined for the particular non-linear algorithm.
PS _i m_ – [string]. Character-valued parameters for Intermediateworld-coordinate Axis _i_, where _m_ is the parameter number.
Leading zeros _must not_ be used, and _m may_ have only values in the range 0 through 99, and that are defined for the
particular non-linear algorithm.

The following keywords, while not essential for a complete
specification of an image WCS, can be extremely useful for readers to interpret the accuracy of the WCS representation of the
image.

CRDER _i_ – [floating point; default: 0.0]. Random error in
Coordinate _i_, which _must_ be non-negative.
CSYER _i_ – [floating point; default: 0.0]. Systematic error in
Coordinate _i_, which _must_ be non-negative.

These values _should_ give a representative average value of the
error over the range of the coordinate in the HDU. The total error
in the coordinates would be given by summing the individual
errors in quadrature.

<a id="821-alternative-wcs-axis-descriptions"></a>
#### 8.2.1. Alternative WCS axis descriptions

In some cases it is useful to describe an image with more than
one coordinate type [12] . Alternative WCS descriptions _may_ be
added to the header by adding the appropriate sets of WCS keywords, and appending to all keywords in each set an alphabetic
code in the range A through Z. Keywords that may be used in
this way to specify a coordinate system version are indicated in
Table 22 with the suffix _a_ . All implied keywords with this encoding are _reserved keywords_, and _must only_ be used in _FITS_ HDUs
as specified in this Standard. The axis numbers _must_ lie in the
range 1 through 99, and the coordinate parameter _m must_ lie in
the range 0 through 99, both with no leading zeros.
The _primary_ version of the WCS description is that specified
with _a_ as the blank character [13] . Alternative axis descriptions are

12 Examples include the frequency, velocity, and wavelength along a
spectral axis (only one of which, of course, could be linear), or the position along an imaging detector in both meters and degrees on the sky.
13 There are a number of keywords (e.g. _ij_ PC _na_ ) where the _a_ could be
pushed off the eight-character keyword name for plausible values of _i_,

31

Table 22: Reserved WCS keywords (continues on next page)

BINTABLE vector Pixel list
Keyword Description Global Image Primary Alternative Primary Alternative

Axis type CTYPE _ia_ _i_ CTYP _n_ _i_ CTY _na_ TCTYP _n_ TCTY _na_
Axis units CUNIT _ia_ _i_ CUNI _n_ _i_ CUN _na_ TCUNI _n_ TCUN _na_

Reference value CRVAL _ia_ _i_ CRVL _n_ _i_ CRV _na_ TCRVL _n_ TCRV _na_

Coordinate increment CDELT _ia_ _i_ CDLT _n_ _i_ CDE _na_ TCDLT _n_ TCDE _na_
Reference point CRPIX _ja_ _j_ CRPX _n_ _j_ CRP _na_ TCRPX _n_ TCRP _na_
Coordinate rotation [1] CROTA _i_ _i_ CROT _n_ TCROT _n_
Transformation matrix [2] PC _i ja_ _ij_ PC _na_ TPC _n ka_ or TP _n ka_
Transformation matrix [2] CD _i ja_ _ij_ CD _na_ TCD _n ka_ or TC _n ka_
Coordinate parameter PV _i ma_ _i_ PV _n ma_ or _i_ V _n ma_ TPV _n ma_ or TV _n ma_
Coordinate parameter PS _i ma_ _i_ PS _n ma_ or _i_ S _n ma_ TPS _n ma_ or TS _n ma_
Coordinate name WCSNAME _a_ WCSN _na_ WCS _na_ or TWCS _na_

Coordinate axis name CNAME _ia_ _i_ CNA _na_ TCNA _na_

Random error CRDER _ia_ _i_ CRD _na_ TCRD _na_
Systematic error CSYER _ia_ _i_ CSY _na_ TCSY _na_

Coordinate rotation LONPOLE _a_ LONP _na_ LONP _na_

Coordinate rotation LATPOLE _a_ LATP _na_ LATP _na_
Coordinate epoch EQUINOX _a_ EQUI _na_ EQUI _na_
Coordinate epoch [3] EPOCH EPOCH EPOCH
Reference frame RADECSYS [4] RADESYS _a_ RADE _na_ RADE _na_
Line rest frequency (Hz) RESTFREQ [4] RESTFRQ _a_ RFRQ _na_ RFRQ _na_
Line rest vacuum wavelength (m) RESTWAV _a_ RWAV _na_ RWAV _na_
Spectral reference frame SPECSYS _a_ SPEC _na_ SPEC _na_
Spectral reference frame SSYSOBS _a_ SOBS _na_ SOBS _na_
Spectral reference frame SSYSSRC _a_ SSRC _na_ SSRC _na_
Observation X (m) OBSGEO-X [5] OBSGX _n_ OBSGX _n_
Observation Y (m) OBSGEO-Y [5] OBSGY _n_ OBSGY _n_
Observation Z (m) OBSGEO-Z [5] OBSGZ _n_ OBSGZ _n_
Radial velocity (m s [−][1] ) VELOSYS _a_ VSYS _na_ VSYS _na_
Redshift of source ZSOURCE _a_ ZSOU _na_ ZSOU _na_
Angle of true velocity VELANGL _a_ VANG _na_ VANG _na_
Date-time related keywords (see Sect.9)

Date of HDU creation DATE
Date/time of observation DATE-OBS DOBS _n_ DOBS _n_
MJD-OBS MJDOB _n_ MJDOB _n_

BEPOCH

JEPOCH
Average date/time of observation DATE-AVG DAVG _n_ DAVG _n_
MJD-AVG MJDA _n_ MJDA _n_
Start date/time of observation DATE-BEG
MJD-BEG

TSTART
End date/time of observation DATE-END

MJD-END

TSTOP
Net exposure duration XPOSURE
Wall-clock exposure duration TELAPSE
Time scale TIMESYS CTYPE _ia_ _i_ CTYP _n_ _i_ CTY _na_ TCTYP _n_ TCTY _na_
Time zero point (MJD) MJDREF [6]

Time zero point (JD) JDREF [6]
Time zero point (ISO) DATEREF
Reference position TREFPOS TRPOS _n_ TRPOS _n_
Reference direction TREFDIR TRDIR _n_ TRDIR _n_
Solar System ephemeris PLEPHEM
Time unit TIMEUNIT CUNIT _ia_ _i_ CUNI _n_ _i_ CUN _na_ TCUNI _n_ TCUN _na_
Time offset TIMEOFFS
Time absolute error TIMSYER CSYER _ia_ _i_ CSYE _n_ _i_ CSY _na_ TCSY _n_ TCSY _na_

Time relative error TIMRDER CRDER _ia_ _i_ CRDE _n_ _i_ CRD _na_ TCRD _n_ TCRD _na_

Time resolution TIMEDEL
Time location in pixel TIMEPIXR
Phase-axis zero point CZPHS _ia_ _i_ CZPH _n_ _i_ CZP _na_ TCZPH _n_ TCZP _na_
Phase-axis period CPERI _ia_ _i_ CPER _n_ _i_ CPR _na_ TCPER _n_ TCPR _na_
32

Table 22 (continued)

**Notes.** The indices _j_ and _i_ are pixel and intermediate-world-coordinate axis numbers, respectively. Within a table, the index _n_ refers to a column
number, and _m_ refers to a coordinate parameter number. The index _k_ also refers to a column number. The indicator _a_ is either blank (for the primary
coordinate description) or a character A through Z that specifies the coordinate version. See the text.
(1) CROTA _i_ form is deprecated but still in use. It _must not_ be used with PC _i j_, PV _i m_, and PS _i m_ . (2) PC _i j_ and CD _i j_ forms of the transformation matrix
are mutually exclusive, and _must not_ appear together in the same HDU. [(3)] EPOCH is deprecated. Use EQUINOX instead. [(4)] These eight-character
keywords are deprecated; the seven-character forms, which can include an alternate version code letter at the end, _should_ be used instead. [(5)] For
the purpose of time reference position, geodetic latitude/longitude/elevation OBSGEO-B, OBSGEO-L, OBSGEO-H or an orbital-ephemeris keyword
OBSORBIT can be also used (see Sect. 9.2.3). [(6)] [M]JDREF can be split in integer and fractional values [M]JDREFI and [M]JDREFF as explained
in Sect. 9.2.2.

Table 23: Reserved celestial-coordinate-algorithm codes.

Default
Code φ 0 θ 0 Properties [1] Projection name

Zenithal (azimuthal) projections
AZP 0 [◦] 90 [◦] Sect. 5.1.1 Zenithal perspective
SZP 0 [◦] 90 [◦] Sect. 5.1.2 Slant zenithal perspective
TAN 0 [◦] 90 [◦] Sect. 5.1.3 Gnomonic
STG 0 [◦] 90 [◦] Sect. 5.1.4 Stereographic
SIN 0 [◦] 90 [◦] Sect. 5.1.5 Slant orthographic
ARC 0 [◦] 90 [◦] Sect. 5.1.6 Zenithal equidistant
ZPN 0 [◦] 90 [◦] Sect. 5.1.7 Zenithal polynomial
ZEA 0 [◦] 90 [◦] Sect. 5.1.8 Zenithal equal-area
AIR 0 [◦] 90 [◦] Sect. 5.1.9 Airy

Cylindrical projections
CYP 0 [◦] 0 [◦] Sect. 5.2.1 Cylindrical perspective
CEA 0 [◦] 0 [◦] Sect. 5.2.2 Cylindrical equal area
CAR 0 [◦] 0 [◦] Sect. 5.2.3 Plate carr´ee

MER 0 [◦] 0 [◦] Sect. 5.2.4 Mercator

Pseudo-cylindrical and related projections
SFL 0 [◦] 0 [◦] Sect. 5.3.1 Samson-Flamsteed

PAR 0 [◦] 0 [◦] Sect. 5.3.2 Parabolic

MOL 0 [◦] 0 [◦] Sect. 5.3.3 Mollweide
AIT 0 [◦] 0 [◦] Sect. 5.3.4 Hammer-Aitoff

Conic projections
COP 0 [◦] θ _a_ Sect. 5.4.1 Conic perspective
COE 0 [◦] θ _a_ Sect. 5.4.2 Conic equal-area
COD 0 [◦] θ _a_ Sect. 5.4.3 Conic equidistant
COO 0 [◦] θ _a_ Sect. 5.4.4 Conic orthomorphic

Polyconic and pseudoconic projections
BON 0 [◦] 0 [◦] Sect. 5.5.1 Bonne’s equal area
PCO 0 [◦] 0 [◦] Sect. 5.5.2 Polyconic

Quad-cube projections
TSC 0 [◦] 0 [◦] Sect. 5.6.1 Tangential spherical cube
CSC 0 [◦] 0 [◦] Sect. 5.6.2 COBE quadrilateralized spherical cube
QSC 0 [◦] 0 [◦] Sect. 5.6.3 Quadrilateralized spherical cube

HEALPix grid projection
HPX 0 [◦] 0 [◦] Sect. 6 [2] HEALPix grid

(1) Refer to the indicated section in Calabretta & Greisen (2002) for a detailed description. (2) This projection is defined in Calabretta & Roukema
(2007).

optional, but _must not_ be specified unless the primary WCS description is also specified. If an alternative WCS description is
specified, all coordinate keywords for that version _must_ be given
even if the values do not differ from those of the primary version.
Rules for the default values of alternative coordinate descriptions

_j_, _k_, _n_, and _m_ . In such cases _a_ is still said to be ‘blank’ although it is not
the blank character.

are the same as those for the primary description. The alternative descriptions are computed in the same fashion as the primary coordinates. The type of coordinate depends on the value
of CTYPE _ia_, and may be linear in one of the alternative descriptions and non-linear in another.

The alternative version codes are selected by the _FITS_ writer;
there is no requirement that the codes be used in alphabetic se

33

quence, nor that one coordinate version differ in its parameter
values from another. An optional keyword WCSNAME _a_ is also defined to name, and otherwise document, the various versions of
WCS descriptions.

WCSNAME _a_ – [string; default for _a_ : '␣' (i.e., blank, for the primary WCS, else a character A through Z that specifies the
coordinate version]. Name of the world-coordinate system
represented by the WCS keywords with the suffix _a_ . Its primary function is to provide a means by which to specify a
particular WCS if multiple versions are defined in the HDU.

<a id="83-celestial-coordinate-system-representations"></a>
### 8.3. Celestial-coordinate-system representations

The conversion from intermediate world coordinates ( _x_, _y_ ) in the
plane of projection to celestial coordinates involves two steps: a
spherical projection to native longitude and latitude (φ, θ), defined in terms of a convenient coordinate system (i.e., _native_
_spherical coordinates_ ), followed by a spherical rotation of these
native coordinates to the required celestial coordinate system
(α, δ). The algorithm to be used to define the spherical projection _must_ be encoded in the CTYPE _i_ keyword as the threeletter algorithm code, the allowed values for which are specified in Table 23 and defined in references Calabretta & Greisen
(2002) and Calabretta & Roukema (2007). The target celestialcoordinate system is also encoded into the left-most portion of
the CTYPE _i_ keyword as the coordinate type.
For the final step, the parameter LONPOLE _a must_ be specified,
which is the native longitude of the celestial pole, φ _p_ . For certain
projections (such as cylindricals and conics, which are less commonly used in astronomy), the additional keyword LATPOLE _a_
_must_ be used to specify the native latitude of the celestial pole.
See Calabretta & Greisen (2002) for the transformation equations and other details.
The accepted celestial-coordinate systems are: the standard
equatorial (RA-- and DEC-), and others of the form _x_ LON and
_x_ LAT for longitude-latitude pairs, where _x_ is G for Galactic, E
for ecliptic, H for helioecliptic and S for supergalactic coordinates. Since the representation of planetary-, lunar-, and solarcoordinate systems could exceed the 26 possibilities afforded by
the single character _x_, pairs of the form _yz_ LN and _yz_ LT _may_ be
used as well.

RADESYS _a_ – [string; default: ’FK4’, ’FK5’, or ’ICRS’: see
below]. Name of the reference frame of equatorial or ecliptic coordinates, whose value _must_ be one of those specified in Table 24. The default value is ’FK4’ if the value of
EQUINOX _a_ < 1984.0, ’FK5’ if ’EQUINOX’ _a_ ≥ 1984.0, or
’ICRS’ if ’EQUINOX’ _a_ is not given.
EQUINOX _a_ – [floating point; default: see below]. Epoch of the
mean equator and equinox in years, whose value _must_ be
non-negative. The interpretation of epoch depends upon the
value of RADESYS _a_ if present: _Besselian_ if the value is ’FK4’
or ’FK4-NO-E’, _Julian_ if the value is ’FK5’; and _not appli-_
_cable_ if the value is ’ICRS’ or ’GAPPT’.
EPOCH – [floating point]. This keyword is deprecated and
_should not_ be used in new _FITS_ files. It is reserved primarily
to prevent its use with other meanings. The EQUINOX keyword _shall_ be used instead. The value field of this keyword
was previously defined to contain a floating-point number
giving the equinox in years for the celestial-coordinate system in which positions are expressed.

34

DATE-OBS – [floating point]. This reserved keyword is defined
in Sect. 4.4.2.
MJD-OBS – [floating point; default: DATE-OBS if given, otherwise no default]. Modified Julian Date (JD − 2,400,000.5) of
the observation, whose value corresponds (by default) to the
_start_ of the observation, unless another interpretation is explained in the comment field. No specific time system (e.g.
UTC, TAI, etc.) is defined for this or any of the other timerelated keywords. It is _recommended_ that the TIMESYS keyword, as defined in Sect. 9.2.1 be used to specify the time
system. See also Sect. 9.5.
LONPOLE _a_ – [floating point; default: φ 0 if δ 0 ≥ θ 0, φ 0 + 180 [◦]
otherwise]. Longitude in the native coordinate system of the
celestial system’s north pole. Normally, φ 0 is zero unless a
non-zero value has been set for PV i 1 _a_, which is associated
with the _longitude_ axis. This default applies for all values of
θ 0, including θ 0 = 90 [◦], although the use of non-zero values
of θ 0 are discouraged in that case.
LATPOLE _a_ – [floating point; default: 90 [◦], or no default if
(θ 0, δ 0, φ _p_ − φ 0 ) = (0, 0, ±90 [◦] )]. Latitude in the native coordinate system of the celestial system’s north pole, or equivalently, the latitude in the celestial-coordinate system of the
native system’s north pole. This keyword _may_ be ignored or
omitted in cases where LONPOLE _a_ completely specifies the
rotation to the target celestial system.

<a id="84-spectral-coordinate-system-representations"></a>
### 8.4. Spectral-coordinate-system representations

This section discusses the conversion of intermediate world coordinates to spectral coordinates with common axes such as frequency, wavelength, and apparent radial velocity (represented
here with the coordinate variables ν, λ, or _v_ ). The key point for
constructing spectral WCS in _FITS_ is that one of these coordinates _must_ be sampled linearly in the dispersion axis; the others
are derived from prescribed, usually non-linear transformations.
Frequency and wavelength axes _may_ also be sampled linearly in
their logarithm.

Following the convention for the CTYPE _ia_ keyword, when _i_ is
the spectral axis the first four characters _must_ specify a code for
the coordinate type; for non-linear algorithms the fifth character
_must_ be a hyphen, and the next three characters _must_ specify a
predefined algorithm for computing the world coordinates from
the intermediate physical coordinates. The coordinate type _must_
be one of those specified in Table 25. When the algorithm is linear, the remainder of the CTYPE _ia_ keyword _must_ be blank. When
the algorithm is non-linear, the three-letter algorithm code _must_
be one of those specified in Table 26. The relationships between
the basic physical quantities ν, λ, and _v_, as well as the relationships between various derived quantities are given in reference
Greisen et al. (2006).

The generality of the algorithm for specifying the spectralcoordinate system and its representation suggests that some additional description of the coordinate may be helpful beyond
what can be encoded in the first four characters of the CTYPE _ia_
keyword; CNAME _ia_ is reserved for this purpose. Note that this
keyword provides a name for an axis in a particular WCS, while
the WCSNAME _a_ keyword names the particular WCS as a whole.
In order to convert between some form of radial velocity and
either frequency or wavelength, the keywords RESTFRQ _a_ and
RESTWAV _a_, respectively, are reserved.

Table 24: Allowed values of RADESYS _a_ .

Value Definition

’ICRS’ International Celestial Reference System
’FK5’ Mean place, new (IAU 1984) system
’FK4’ [1] Mean place, old (Bessel-Newcomb) system
’FK4-NO-E’ [1] Mean place: but without eccentricity terms
’GAPPT’ Geocentric apparent place, IAU 1984 system

(1) New _FITS_ files _should_ avoid using these older reference systems.

CNAME _ia_ – [string; default: default: '␣' (i.e. a linear, undefined
axis)]. Spectral-coordinate description that _must not_ exceed
68 characters in length.
RESTFRQ _a_ – [floating point; default: none]. Rest frequency of
the of the spectral feature of interest. The physical unit _must_
be Hz.
RESTWAV _a_ – [floating point; default: none]. Vacuum rest wavelength of the of the spectral feature of interest. The physical
unit _must_ be m.

One or the other of RESTFRQ _a_ or RESTWAV _a should_ be given
when it is meaningful to do so.

<a id="841-spectral-coordinate-reference-frames"></a>
#### 8.4.1. Spectral-coordinate reference frames

Frequencies, wavelengths, and apparent radial velocities are always referred to some selected standard of rest (i.e., reference
frame). While the spectra are obtained they are, of necessity, in
the observer’s rest frame. The velocity correction from topocentric (the frame in which the measurements are usually made) to
standard reference frames (which _must_ be one of those given in
Table 27) are dependent on the dot product with time-variable
velocity vectors. That is, the velocity with respect to a standard
reference frame depends upon direction, and the velocity (and
frequency and wavelength) with respect to the local standard
of rest is a function of the celestial coordinate within the image. The keywords SPECSYS _a_ and SSYSOBS _a_ are reserved and,
if used, _must_ describe the reference frame in use for the spectralaxis coordinate(s) and the spectral reference frame that was held
constant during the observation, respectively. In order to compute the velocities it is necessary to have the date and time of the
observation; the keywords DATE-AVG and MJD-AVG are reserved
for this purpose. See also Sect. 9.5.

DATE-AVG – [string; default: none]. Calendar date of the midpoint of the observation, expressed in the same way as the
DATE-OBS keyword.
MJD-AVG – [floating point; default: none]. Modified Julian Date
(JD − 2,400,000.5) of the mid-point of the observation.

SPECSYS _a_ – [string; default: none]. The reference frame in use
for the spectral-axis coordinate(s). Valid values are given in
Table 27.

SSYSOBS _a_ – [string; default: ’TOPOCENT’]. The spectral reference frame that is constant over the range of the non-spectral
world coordinates. Valid values are given in Table 27.

The transformation from the rest frame of the observer to a
standard reference frame requires a specification of the location
on Earth [14] of the instrument used for the observation in order to

14 The specification of location for an instrument on a spacecraft in
flight requires an ephemeris; keywords that might be required in this
circumstance are not defined here.

Table 26: Non-linear spectral algorithm codes.

Code [1] Regularly sampled in Expressed as

F2W Frequency Wavelength
F2V Apparent radial velocity
F2A Air wavelength
W2F Wavelength Frequency
W2V Apparent radial velocity
W2A Air wavelength
V2F Apparent radial vel. Frequency
V2W Wavelength
V2A Air wavelength
A2F Air wavelength Frequency
A2W Wavelength
A2V Apparent radial velocity

LOG Logarithm Any four-letter type code
GRI Detector Any type code from Table 25
GRA Detector Any type code from Table 25
TAB Not regular Any four-letter type code

(1) Characters 6 through 8 of the value of the keyword CTYPE _ia_ .

calculate the diurnal Doppler correction due to the Earth’s rotation. The location, if specified, _shall_ be represented as a geocentric Cartesian triple with respect to a standard ellipsoidal geoid
at the time of the observation. While the position can often be
specified with an accuracy of a meter or better, for most purposes positional errors of several kilometers will have negligible
impact on the computed velocity correction. For details, see reference Greisen et al. (2006).

OBSGEO-X – [floating point; default: none]. _X_ −coordinate (in
meters) of a Cartesian triplet that specifies the location, with
respect to a standard, geocentric terrestrial reference frame,
where the observation took place. The coordinate _must_ be
valid at the epoch MJD-AVG or DATE-AVG.
OBSGEO-Y – [floating point; default: none]. _Y_ −coordinate (in
meters) of a Cartesian triplet that specifies the location, with
respect to a standard, geocentric terrestrial reference frame,
where the observation took place. The coordinate _must_ be
valid at the epoch MJD-AVG or DATE-AVG.
OBSGEO-Z – [floating point; default: none]. _Z_ −coordinate (in
meters) of a Cartesian triplet that specifies the location, with
respect to a standard, geocentric terrestrial reference frame,
where the observation took place. The coordinate _must_ be
valid at the epoch MJD-AVG or DATE-AVG.

Information on the relative radial velocity between the observer and the selected standard of rest in the direction of the
celestial reference coordinate _may_ be provided, and if so _shall_
be given by the VELOSYS _a_ keyword. The frame of rest defined
with respect to the emitting source may be represented in _FITS_ ;
for this reference frame it is necessary to define the velocity with
respect to some other frame of rest. The keywords SPECSYS _a_ and
ZSOURCE _a_ are used to document the choice of reference frame
and the value of the systemic velocity of the source, respectively.

SSYSSRC _a_ – [string; default: none]. Reference frame for the
value expressed in the ZSOURCE _a_ keyword to document the
systemic velocity of the observed source. Value _must_ be one
of those given in Table 27 _except_ for ’SOURCE’.
VELOSYS _a_ – [floating point; default: none]. Relative radial velocity between the observer and the selected standard of rest

35

Table 25: Reserved spectral-coordinate type codes.

Code [1] Type Symbol Associated Default units
variable

FREQ Frequency ν ν Hz
ENER Energy _E_ ν J
WAVN Wavenumber κ ν m [−][1]

VRAD Radio velocity [2] _V_ ν m s [−][1]
WAVE Vacuum wavelength λ λ m
VOPT Optical velocity [2] _Z_ λ m s [−][1]
ZOPT Redshift _z_ λ ...
AWAV Air wavelength λ _a_ λ _a_ m
VELO Apparent radial velocity _v_ _v_ m s [−][1]
BETA Beta factor ( _v_ / _c_ ) β _v_ ...

(1) Characters 1 through 4 of the value of the keyword CTYPE _ia_ . (2) By convention, the ‘radio’ velocity is given by _c_ (ν 0 − ν)/ν 0 and the ‘optical’
velocity is given by _c_ (λ − λ 0 )/λ 0 .

Table 27: Spectral reference systems.

Value Definition

’TOPOCENT’ Topocentric
’GEOCENTR’ Geocentric
’BARYCENT’ Barycentric
’HELIOCEN’ Heliocentric
’LSRK’ Local standard of rest (kinematic)
’LSRD’ Local standard of rest (dynamic)
’GALACTOC’ Galactocentric
’LOCALGRP’ Local Group
’CMBDIPOL’ Cosmic-microwave-background dipole
’SOURCE’ Source rest frame

**Notes.** These are the allowed values of the SPECSYS _a_, SSYSOBS _a_, and
SSYSSRC _a_ keywords.

in the direction of the celestial reference coordinate. Units
_must_ be m s [−][1] . The CUNIT _ia_ keyword is not used for this
purpose since the WCS Version _a_ might not be expressed in
velocity units.
ZSOURCE _a_ – [floating point; default: none]. Radial velocity
with respect to an alternative frame of rest, expressed as a
unitless redshift (i.e., velocity as a fraction of the speed of
light in vacuum). Used in conjunction with SSYSSRC _a_ to
document the systemic velocity of the observed source.
VELANGL _a_ – [floating point; default:+90.]. In the case of relativistic velocities (e.g., a beamed astrophysical jet) the transverse velocity component is important. This keyword _may_
be used to express the orientation of the space velocity vector with respect to the plane of the sky. See Appendix A of
reference Greisen et al. (2006) for further details.

<a id="85-conventional-coordinate-types"></a>
### 8.5. Conventional-coordinate types

The first _FITS_ paper (Wells et al. 1981) listed a number of
‘suggested values’ for the CTYPE _i_ keyword. Two of these have
the attribute the associated world coordinates can assume only
integer values and that the meaning of these integers is only
defined by convention. The first ‘conventional’ coordinate is
CTYPE _ia_ = ’COMPLEX’ to specify that complex values (i.e., pairs
of real and imaginary components) are stored in the data array
(along with an optional weight factor). Thus, the complex axis
of the data array will contain two values (or three if the weight is

36

Table 28: Example keyword records for a 100-element array of
complex values.

Keyword records

SIMPLE = T

BITPIX = -32

NAXIS = 2

NAXIS1 = 2

NAXIS2 = 100

CTYPE1 = 'COMPLEX'

CRVAL1 = 0.

CRPIX1 = 0.

CDELT1 = 1.

END

Table 29: Conventional Stokes values.

Value Symbol Polarization

1 ’I’ Standard Stokes unpolarized
2 ’Q’ Standard Stokes linear
3 ’U’ Standard Stokes linear

4 ’V’ Standard Stokes circular
−1 ’RR’ Right-right circular
−2 ’LL’ Left-left circular
−3 ’RL’ Right-left cross-circular
−4 ’LR’ Left-right cross-circular
−5 ’XX’ _X_ parallel linear
−6 ’YY’ _Y_ parallel linear
−7 ’XY’ _XY_ cross linear

−8 ’YX’ _YX_ cross linear

specified). By convention, the real component has a coordinate
value of 1, the imaginary component has a coordinate value of 2,
and the weight, if any, has a coordinate value of 3. Table 28 illustrates the required keywords for an array of 100 complex values
(without weights).
The second conventional coordinate is CTYPE _ia_ = ’STOKES’
to specify the polarization of the data. Conventional values, their
symbols, and polarizations are given in Table 29.

<a id="9-representations-of-time-coordinates"></a>
## 9. Representations of time coordinates

Time as a dimension in astronomical data presents challenges
for its representation in _FITS_ files. This section formulates the
representation of the time axis, or possibly multiple time axes,

into the world-coordinate system (WCS) described in Sect. 8.
Much of the basic structure is employed, while extensions are
developed to cope with the differences between time and spatial
dimensions; notable amongst these differences is the huge dynamic range, covering the highest resolution timing relative to
the age of the universe.
The precision with which any time stamp conforms to any
conventional time scale is highly dependent on the characteristics of the acquiring system. The definitions of many conventional time scales vary over their history along with the precision
that can be attributed to any time stamp. The meaning of any
time stamp may be ambiguous if a time scale is used for dates
prior to its definition by a recognized authority, or for dates after that definition is abandoned. However, common sense should
prevail: the precision in the description of the time coordinate
_should_ be appropriate to the accuracy of the temporal information in the data.

<a id="91-time-values"></a>
### 9.1. Time values

The three most common ways to specify time are: ISO-8601
(ISO 2004b), Julian Date (JD), or Modified Julian Date (MJD
= JD − 2, 400, 000.5; see IAU 1997). Julian Dates are counted
from Julian proleptic calendar date 1 January 4713 BCE at noon,
or Gregorian proleptic calendar date 24 November 4714 BCE,
also at noon. For an explanation of the calendars, see Rots et al.
(2015). Even though it is common to think of certain representations of time as absolute, time values in _FITS_ files _shall_ all
be considered relative: elapsed time since a particular reference
point in time. It may help to view the “absolute” values as merely
relative to a globally accepted zero point. For a discussion of
the precision required to represent time values in floating-point
numbers, see Rots et al. (2015).

<a id="911-iso-8601-datetime-strings"></a>
#### 9.1.1. ISO-8601 datetime strings

_FITS_ datetime strings conform to a subset of ISO-8601 (which
in itself does not imply a particular time scale) for several timerelated keywords (Bunclark & Rots 1997), such as DATE-xxxx.
Here _datetime_ will be used as a pseudo data type to indicate its
use, although its values _must_ be written as a character string in
’A’ format. The full specification for the format of the _datetime_
string has been:

CCYY-MM-DD[ **T** hh:mm:ss[.s...]]

in which all of the time part _may_ be omitted (just leaving
the date) or the decimal seconds _may_ be omitted. Leading zeros _must not_ be omitted and timezone designators are _not al-_
_lowed_ . This definition is extended to allow five-digit years with a
_mandatory_ sign, in accordance with ISO-8601. That is, one _shall_
use either the _unsigned_ four-digit year format, or the _signed_ fivedigit year format shown below.

[±C]CCYY-MM-DD[ **T** hh:mm:ss[.s...]]

Note the following.

**–**
In counting years, ISO-8601 follows the convention of including Year Zero. Consequently, for negative year numbers
there is an offset of one from BCE dates, which do not recognize a Year Zero. Thus Year 1 corresponds to 1 CE, Year 0
to 1 BCE, Year −1 to 2 BCE, and so on.

**–**
The earliest date that may be represented in the four-digit
year format is ’0000-01-01T00:00:00’ (in the year 1
BCE); the latest date is ’9999-12-31T23:59:59’. This
representation of time is tied to the Gregorian calendar. In
conformance with the present ISO-8601:2004(E) standard
(ISO 2004b) dates prior to 1582 _must_ be interpreted according to the proleptic application of the rules of Gregorius
XIII. For dates not covered by that range the use of Modified
Julian Date (MJD) or Julian Date (JD) numbers or the use of
the signed five-digit year format is _recommended_ .

**–**
In the five-digit year format the earliest and latest dates are
’-99999-01-01T00:00:00’ (i.e., −100 000 BCE) and
’+99999-12-31T23:59:59’.

**–**
The origin of JD can be written as:
’-04713-11-24T12:00:00’.

**–**
In the UTC time scale the integer part of the seconds field
runs from 00 to 60 (in order to accommodate leap seconds);
in all other time scales the range is 00 to 59.
**–** The ISO-8601 _datetime_ data type is _not allowed_ in imageaxis descriptions since CRVAL is _required_ to be a floatingpoint value.
**–** ISO-8601 _datetime_ does not imply the use of any particular
time scale (see Sect. 9.2.1).

**–**
As specified by Bunclark & Rots (1997), time zones are explicitly not supported in _FITS_ and, consequently, appending
the letter ’Z’ to a _FITS_ ISO-8601 string is _not allowed_ . The
rationale for this rule is that its role in the ISO standard is
that of a time-zone indicator, not a time-scale indicator. As
the concept of a time zone is not supported in _FITS_, the use
of time-zone indicator is inappropriate.

<a id="912-julian-and-besselian-epochs"></a>
#### 9.1.2. Julian and Besselian epochs

In a variety of contexts _epochs_ are provided with astronomical
data. Until 1976 these were commonly based on the Besselian
year (see Sect. 9.3), with standard epochs B1900.0 and B1950.0.
After 1976 the transition was made to Julian epochs based on
the Julian year of 365.25 days, with the standard epoch J2000.0.
They are tied to the ET and TDB time scales, respectively. Note
that the Besselian epochs are scaled by the variable length of the
Besselian year (see Sect. 9.3 and its cautionary note, which also
applies to this context). The Julian epochs are easier to calculate,
as long as one keeps track of leap days.

<a id="92-time-coordinate-frame"></a>
### 9.2. Time coordinate frame

<a id="921-time-scale"></a>
#### 9.2.1. Time scale

The _time scale_ defines the temporal reference frame, and is specified in the header in one of a few ways, depending upon the context. When recorded as a global keyword, the time scale _shall_ be
specified by the following keyword.

TIMESYS – [string; default: ’UTC’]. The value field of this keyword _shall_ contain a character-string code for the time scale
of the time-related keywords. The _recommended_ values for
this keyword in Table 30 have well-defined meanings, but
other values _may_ be used. If this keyword is absent, ’UTC’
_must_ be assumed.

In relevant contexts (e.g., time axes in image arrays, table
columns, or random groups) TIMESYS _may_ be overridden by a

37

time scale recorded in CTYPE _ia_, its binary-table equivalents, or
PTYPE _i_ (see Table 22).

The keywords TIMESYS, CTYPE _ia_, TCTYP _n_, and TCTY _na_ or
binary-table equivalent _may_ assume the values listed in Table 30.
In addition, for backward compatibility, all except TIMESYS and
PTYPE _i may_ also assume the value ’TIME’ (case-insensitive),
whereupon the time scale _shall_ be that recorded in TIMESYS or,
in its absence, its default value, ’UTC’. As noted above, local
time scales other than those listed in Table 30 _may_ be used, but
their use _should_ be restricted to alternate coordinates in order
that the primary coordinates will always refer to a properly recognized time scale.

See Rots et al. (2015), Appendix A, for a detailed discussion
of the various time scales. In cases where high-precision timing
is important one _may_ append a specific realization, in parentheses, to the values in the table; e.g., ’TT(TAI)’, ’TT(BIPM08)’,
’UTC(NIST)’. Note that linearity is not preserved across all
time scales. Specifically, if the reference position remains unchanged (see Sect. 9.2.3), the first ten, with the exception of
’UT1’, are linear transformations of each other (excepting leap
seconds), as are ’TDB’ and ’TCB’. On average ’TCB’ runs
faster than ’TCG’ by approximately 1.6 × 10 [−][8], but the transformation from ’TT’ or ’TCG’ (which are linearly related)
is to be achieved through a time ephemeris as provided by
Irwin & Fukushima (1999).

The relations between coordinate time scales and their dynamical equivalents have been defined as:

_T_ (TCG) = _T_ (TT) + _L_ G × 86400 × ( _JD_ (TT) − _JD_ 0 )
_T_ (TDB) = _T_ (TCB)− _L_ B ×86400×( _JD_ (TCB)− _JD_ 0 )+ _TDB_ 0,

where:

_T_ is in seconds
_L_ G = 6.969290134 × 10 [−][10]
_L_ B = 1.550519768 × 10 [−][8]
_JD_ 0 = 2443144.5003725
_TDB_ 0 = −6.55 × 10 [−][5] s.

Linearity is virtually guaranteed since images and individual table columns do not allow more than one reference position to
be associated with them, and since there is no overlap between
reference positions that are meaningful for the first nine time
scales on the one hand, and for the barycentric ones on the other.
All use of the GMT time scale in _FITS_ files _shall_ be taken to
have its zero point at midnight, conformant with UT, including
dates prior to 1925. For high-precision timing prior to 1972, see
Rots et al. (2015), Appendix A.

Some time scales in use are not listed in Table 30 because
they are intrinsically unreliable or ill-defined. When used, they
_should_ be tied to one of the existing scales with appropriate specification of the uncertainties; the same is true for free-running
clocks. However, a local time scale such as MET (Mission
Elapsed Time) or OET (Observation Elapsed Time) _may_ be defined for practical reasons. In those cases the time reference
value (see Sect. 9.2.2) _shall not_ be applied to the values, and
it is strongly _recommended_ that such time scales be provided as
alternate time scales, with a defined conversion to a recognized
time scale.

It is useful to note that while UT1 is, in essence, an angle (of
the Earth’s rotation – _i.e.,_ a _clock_ ), the others are SI-second counters ( _chronometers_ ); UTC, by employing leap seconds, serves as
a bridge between the two types of time scales.

38

Table 30: Recognized Time Scale Values

Value Meaning

’TAI’ (International Atomic Time): atomic-time standard
maintained on the rotating geoid
’TT’ (Terrestrial Time; IAU standard): defined on the rotating geoid, usually derived as TAI + 32.184 s
’TDT’ (Terrestrial Dynamical Time): synonym for TT (deprecated)
’ET’ (Ephemeris Time): continuous with TT; _should not_
be used for data taken after 1984-01-01
’IAT’ synonym for TAI (deprecated)
’UT1’ (Universal Time): Earth rotation time
’UTC’ (Universal Time, Coordinated; default): runs synchronously with TAI, except for the occasional insertion of leap seconds intended to keep UTC within
0.9 s of UT1; as of 2015-07-01 UTC = TAI − 36 s
’GMT’ (Greenwich Mean Time): continuous with UTC; its
use is deprecated for dates after 1972-01-01
UT() [1] (Universal Time, with qualifier): for high-precision
use of radio-signal distributions between 1955 and
1972; see Rots et al. (2015), Appendix A
’GPS’ (Global Positioning System): runs (approximately)
synchronously with TAI; GPS ≈ TAI − 19 s
’TCG’ (Geocentric Coordinate Time): TT reduced to the
geocenter, corrected for the relativistic effects of
the Earth’s rotation and gravitational potential; TCG
runs faster than TT at a constant rate
’TCB’ (Barycentric Coordinate Time): derived from TCG
by a four-dimensional transformation, taking into account the relativistic effects of the gravitational potential at the barycenter (relative to that on the rotating geoid) as well as velocity time-dilation variations due to the eccentricity of the Earth’s orbit,
thus ensuring consistency with fundamental physical constants; Irwin & Fukushima (1999) provide a
time ephemeris
’TDB’ (Barycentric Dynamical Time): runs slower than
TCB at a constant rate so as to remain approximately
in step with TT; runs therefore quasi-synchronously
with TT, except for the relativistic effects introduced by variations in the Earth’s velocity relative
to the barycenter. When referring to celestial observations, a pathlength correction to the barycenter
may be needed, which requires the Time Reference
Direction used in calculating the pathlength correction.
’LOCAL’ for simulation data and for free-running clocks.

1
Specific realization codes _may_ be appended to these values, in
parentheses; see the text. For a more-detailed discussion of time
scales, see Rots et al. (2015), Appendix A.

<a id="922-time-reference-value"></a>
#### 9.2.2. Time reference value

The time reference value is _not required_ to be present in an
HDU. However, if the time reference point is specified explicitly it _must_ be expressed in one of ISO-8601, JD, or MJD. These
reference values _must only_ be applied to time values associated
with one of the recognized time scales listed in Table 30, and that
time scale _must_ be specified explicitly or implicitly as explained
in Sect. 9.2.1.

The reference point in time, to which all times in the HDU
are relative, _shall_ be specified through one of three keywords
specified below.

MJDREF – [floating-point]; default: 0.0] The value field of this
keyword _shall_ contain the value of the reference time in
MJD.
JDREF – [floating-point; default: none] The value field of this
keyword _shall_ contain the value of the reference time in JD.
DATEREF – [datetime; default: none] The value field of this keyword _shall_ contain a character-string representation of the
reference time in ISO-8601 format.

MJDREF and JDREF _may_, for clarity or precision reasons, be split
into two keywords holding the integer and fractional parts separately.

MJDREFI – [integer; default: 0] The value field of this keyword
_shall_ contain the integer part of reference time in MJD.
MJDREFF – [floating-point; default: 0.0] The value field of this
keyword _shall_ contain the fractional part of reference time in
MJD.
JDREFI – [integer; default: none] The value field of this keyword _shall_ contain the integer part of reference time in JD.
JDREFF – [floating-point; default: none] The value field of this
keyword _shall_ contain the fractional part of reference time in
JD.

If [M]JDREF and both [M]JDREFI and [M]JDREFF are
present, the integer and fractional values _shall_ have precedence
over the single value. If the single value is present with one of
the two parts, the single value _shall_ have precedence. In the following, MJDREF and JDREF refer to their literal meaning or the
combination of their integer and fractional parts. If a header
contains more than one of these keywords, JDREF _shall_ have
precedence over DATEREF and MJDREF _shall_ have precedence
over both the others. If none of the three keywords is present,
there is no problem as long as all times in the HDU are expressed in ISO-8601; otherwise MJDREF = 0.0 _must_ be assumed.
If TREFPOS = ’CUSTOM’ (Sect. 9.2.3), it is legitimate for none of
the reference-time keywords to be present, as one may assume
the data are from a simulation. Note that the _value_ of the reference time has global validity for all time values, but it does not
have a particular time scale associated with it.

<a id="923-time-reference-position"></a>
#### 9.2.3. Time reference position

An observation is an event in space-time. The reference position
specifies the spatial location at which the time is valid, either
where the observation was made or the point in space for which
light-time corrections have been applied. When recorded as a
global keyword, the time reference position _shall_ be specified
by the following keyword.

TREFPOS – [string; default: ’TOPOCENTER’]. The value field
of this keyword _shall_ contain a character-string code for the
spatial location at which the observation time is valid. The
value _should_ be one of those given in Table 31. This keyword
_shall_ apply to time-coordinate axes in images as well.

In binary tables, different columns _may_ represent completely
different Time Coordinate Frames. However, each column can
have only one time reference position, thus guaranteeing linearity (see Sect. 9.2.1).

TRPOS _n_ – [string; default: ’TOPOCENTER’] The value field of
this keyword _shall_ contain a character-string code for the
spatial location at which the observation time is valid. This
table keyword _shall_ override TREFPOS.

The reference position value _may_ be a standard location
(such as ’GEOCENTER’ or ’TOPOCENTER’) or a point in space
defined by specific coordinates. In the latter case one should be
aware that a (three-dimensional) spatial-coordinate frame needs
to be defined that is likely to be different from the frame(s) with
which the data are associated. Note that ’TOPOCENTER’ is only
moderately informative if no observatory location is provided
or indicated. The commonly allowed standard values are shown
in Table 31. Note that for the gaseous planets the barycenters of
their planetary systems, including satellites, are used for obvious
reasons. While it is preferable to spell the location names out in
full, in order to be consistent with the practice of Greisen et al.
(2006) the values are allowed to be truncated to eight characters.
Furthermore, in order to allow for alternative spellings, only the
first three characters of all these values _shall_ be considered significant. The value of the keyword _shall_ be case-sensitive.

Table 31: Standard Time Reference Position Values

Value [1] Meaning

’TOPOCENTER’ Topocenter: the location from where the observation was made (default)
’GEOCENTER’ Geocenter
’BARYCENTER’ Barycenter of the Solar System
’RELOCATABLE’ Relocatable: to be used for simulation data
only
’CUSTOM’ A position specified by coordinates that is
not the observatory location

Less-common, but allowed standard values

’HELIOCENTER’ Heliocenter

’GALACTIC’ Galactic center
’EMBARYCENTER’ Earth-Moon barycenter
’MERCURY’ Center of Mercury
’VENUS’ Center of Venus

’MARS’ Center of Mars
’JUPITER’ Barycenter of the Jupiter system
’SATURN’ Barycenter of the Saturn system
’URANUS’ Barycenter of the Uranus system
’NEPTUNE’ Barycenter of the Neptune system

**Notes.** [(1)] Recognized values for TREFPOS, TRPOS _n_ ; only the first three
characters of the values are significant and Solar System locations are
as specified in the ephemerides.

The reader is cautioned that time scales and reference positions cannot be combined arbitrarily if one wants a clock
that runs linearly at TREFPOS. Table 32 provides a summary
of compatible combinations. ’BARYCENTER’ _should_ only be
used in conjunction with time scales ’TDB’ and ’TCB’, and
_should_ be the only reference position used with these time
scales. With proper care ’GEOCENTER’, ’TOPOCENTER’, and
’EMBARYCENTER’ are appropriate for the first ten time scales
in Table 30. However, relativistic effects introduce a (generally
linear) scaling in certain combinations; highly eccentric spacecraft orbits are the exceptions. Problems will arise when using
a reference position on another Solar System body (including
’HELIOCENTER’). Therefore, it is _recommended_ to synchronize

39

Table 32: Compatibility of Time Scales and Reference Positions

Reference Time scale [1]

Position TT, TDT TCG TDB TCB LOCAL
TAI, IAT
GPS

UTC, GMT

’TOPOCENTER’ t ls

’GEOCENTER’ ls c

’BARYCENTER’ ls c

’RELOCATABLE’ c
Other [2] re re

**Notes.** [(1)] Legend (combination is _not recommended_ if there is no entry);
**c** : correct match; reference position coincides with the spatial origin of
the space-time coordinates; **t** : correct match on Earth’s surface, otherwise usually linear scaling; **ls** : linear relativistic scaling; **re** : non-linear
relativistic scaling. [(2)] All other locations in the Solar System.

the local clock with one of the time scales defined on the Earth’s
surface, ’TT’, ’TAI’, ’GPS’, or ’UTC’ (in the last case: beware
of leap seconds). This is common practice for spacecraft clocks.
Locally, such a clock will not appear to run at a constant rate, because of variations in the gravitational potential and in motions
with respect to Earth, but the effects can be calculated and are
probably small compared with errors introduced by the alternative: establishing a local time standard.
In order to provide a complete description, ’TOPOCENTER’
requires the observatory’s coordinates to be specified. There
are three options: _(a)_ the ITRS Cartesian coordinates defined
in Sect. 8.4.1 (OBSGEO-X, OBSGEO-Y, OBSGEO-Z), which are
_strongly preferred_ ; _(b)_ a geodetic latitude/longitude/elevation
triplet (defined below); or _(c)_ a reference to an orbit-ephemeris
file. A set of geodetic coordinates is recognized by the following
keywords.

OBSGEO-B – [floating-point] The value field of this keyword
_shall_ contain the latitude of the observation in deg, with
North positive.
OBSGEO-L – [floating-point] The value field of this keyword
_shall_ contain the longitude of the observation in deg, with
East positive.
OBSGEO-H – [floating-point] The value field of this keyword
_shall_ contain the altitude of the observation in meters.

An orbital-ephemeris file can instead be specified.

OBSORBIT – [string] The value field of this keyword _shall_ contain the character-string URI, URL, or the name of an orbitephemeris file.

Beware that only one set of coordinates is allowed in a given
HDU. Cartesian ITRS coordinates are the preferred coordinate
system; however, when using these in an environment requiring nanosecond accuracy, one should take care to distinguish
between meters consistent with TCG or with TT. If one uses
geodetic coordinates, the geodetic altitude OBSGEO-H is measured with respect to the IAU 1976 ellipsoid, which is defined
as having a semi-major axis of 6 378 140 m and an inverse flattening of 298.2577.
A non-standard location indicated by ’CUSTOM’ _must_ be
specified in a manner similar to the specification of the observatory location (indicated by ’TOPOCENTER’). One should be

40

careful with the use of the ’CUSTOM’ value and not confuse it
with ’TOPOCENTER’, as use of the latter imparts additional information on the provenance of the data.
ITRS coordinates ( _X_, _Y_, _Z_ ) may be derived from geodetic coordinates ( _L_, _B_, _H_ ) through:

_X_ = ( _N_ ( _B_ ) + _H_ ) cos( _L_ ) cos( _B_ )

_Y_ = ( _N_ ( _B_ ) + _H_ ) sin( _L_ ) cos( _B_ )

_Z_ = ( _N_ ( _B_ )(1 − _e_ [2] ) + _H_ ) sin( _B_ )

where:

_a_
_N_ ( _B_ ) =

1 − _e_ [2] sin [2] ( _B_ )

�

_e_ [2] = 2 _f_ − _f_ [2]

_a_ is the semi-major axis, and _f_ is the inverse of the inverse flattening. Nanosecond precision in timing requires that
OBSGEO-[BLH] be expressed in a geodetic reference frame defined after 1984 in order to be sufficiently accurate.

<a id="924-time-reference-direction"></a>
#### 9.2.4. Time reference direction

If any pathlength corrections have been applied to the time
stamps (i.e., if the reference position is not ’TOPOCENTER’ for
observational data), the reference direction that is used in calculating the pathlength delay _should_ be provided in order to maintain a proper analysis trail of the data. However, this is useful
only if there is also information available on the location from
where the observation was made (the observatory location). The
direction will usually be provided in a spatial-coordinate frame
that is already being used for the spatial metadata, although it is
conceivable that multiple spatial frames are involved, e.g., spherical ICRS coordinates for celestial positions, and Cartesian FK5
for spacecraft ephemeris. The time reference direction does not
by itself provide sufficient information to perform a fully correct
transformation; however, within the context of a specific analysis
environment it should suffice.
The uncertainty in the reference direction affects the errors
in the time stamps. A typical example is provided by barycentric
corrections where the time error is related to the position error:

_t_ err (ms) ≤ 2.4 _pos_ err (arcsec).

The reference direction is indicated through a reference to specific keywords. These keywords _may_ hold the reference direction
explicitly or (for data in BINTABLE extensions) indicate columns
holding the coordinates. In event lists where the individual photons are tagged with a spatial position, those coordinates _may_
have been used for the reference direction and the reference will
point to the columns containing these coordinate values. The
time reference direction _shall_ be specified by the following keyword.

TREFDIR – [string] The value field of this keyword _shall_ contain a character string composed of: the name of the keyword containing the longitudinal coordinate, followed by a
comma, followed by the name of the keyword containing the
latitudinal coordinate. This reference direction _shall_ apply to
time-coordinate axes in images as well.

In binary tables, different columns _may_ represent completely
different Time Coordinate Frames. However, also in that situation the condition holds that each column can have only one
Time Reference Direction. Hence, the following keyword _may_
override TREFDIR.

TRDIR _n_ – [string] The value field of this keyword _shall_ contain
a character string consisting of the name of the keyword or
column containing the longitudinal coordinate, followed by
a comma, followed by the name of the keyword or column
containing the latitudinal coordinate. This reference direction _shall_ apply to time-coordinate axes in images as well.

<a id="925-solar-system-ephemeris"></a>
#### 9.2.5. Solar System ephemeris

If applicable, the Solar System ephemeris used for calculating
pathlength delays _should_ be identified. This is particularly pertinent when the time scale is ’TCB’ or ’TDB’. The ephemerides
that are currently most often used are those from JPL (2014a,b).
The Solar System ephemeris used for the data (if required)
_shall_ be indicated by the following keyword.

PLEPHEM – [string; default: ’DE405’] The value field of
this keyword _shall_ contain a character string that _should_
represent a recognized designation for the Solar System
ephemeris. Recognized designations for JPL Solar System
ephemerides that are often used are listed in Table 33.

Table 33: Valid Solar System ephemerides

Value Reference

’DE200’ Standish (1990); considered obsolete, but still in use
’DE405’ Standish (1998); default
’DE421’ Folkner, et al. (2009)
’DE430’ Folkner, et al. (2014)
’DE431’ Folkner, et al. (2014)
’DE432’ Folkner, et al. (2014)

Future ephemerides in this series _shall_ be accepted and recognized as they are released. Additional ephemerides designations _may_ be recognized by the IAUFWG upon request.

<a id="93-time-unit"></a>
### 9.3. Time unit

When recorded as a global keyword, the unit used to express
time _shall_ be specified by the following keyword.

TIMEUNIT – [string; default: ’s’] The value field of this keyword _shall_ contain a character string that specifies the time
unit; the value _should_ be one of those given in Table 34. This
time unit _shall_ apply to all time instances and durations that
do not have an implied time unit (such as is the case for JD,
MJD, ISO-8601, J and B epochs). If this keyword is absent,
’s’ _shall_ be assumed.

In an appropriate context, e.g., when an image has a time axis,
TIMEUNIT _may_ be overridden by the CUNIT _ia_ keywords and
their binary-table equivalents (see Table 22).
The specification of the time unit allows the values defined
in Greisen & Calabretta (2002), shown in Table 34, with the addition of the century. See also Sect. 4.3 for generalities about
units.

Table 34: Recommended time units

Value Definition

’s’ second (default)
’d’ day (= 86,400 s)
’a’ (Julian) year (= 365.25 d)
’cy’ (Julian) century (= 100 a)

The following values are also acceptable.

’min’ minute (= 60 s)
’h’ day (= 86,400 s)
’yr’ (Julian) year (= ’a’ = 365.25
d)
’ta’ tropical year
’Ba’ Besselian year

The use of ’ta’ and ’Ba’ is not encouraged, but there are
data and applications that require the use of tropical years or
Besselian epochs (see Sect. 9.1.2). The length of the tropical
year, ’ta’, in days is:

1 ta = 365.24219040211236 − 0.00000615251349 _T_
−6.0921 × 10 [−][10] _T_ [2] + 2.6525 × 10 [−][10] _T_ [3] (d)

where _T_ is in Julian centuries since J2000, using time scale TDB.
The length of the Besselian year in days is:

1Ba = 365.2421987817 − 0.00000785423 _T_ (d)

where _T_ is in Julian centuries since J1900, using time scale ET,
although for these purposes the difference with TDB is negligible.

Readers are cautioned that the subject of tropical and
Besselian years presents a particular quandary for the specification of standards. The expressions presented here are the most
accurate available, but are applicable for use when creating data
files (which is strongly discouraged), rather than for interpreting
existing data that are based upon these units. However, there is
no guarantee that the authors of the data applied these particular definitions. Users are therefore advised to pay close attention
and attempt to ascertain what the authors of the data really used.

<a id="94-time-offset-binning-and-errors"></a>
### 9.4. Time offset, binning, and errors

<a id="941-time-offset"></a>
#### 9.4.1. Time offset

A uniform clock correction _may_ be applied in bulk with the following single keyword.

TIMEOFFS – [floating-point; default: 0.0] The value field of
this keyword _shall_ contain the value of the offset in time
that _shall_ be added to the reference time, given by one of:
MJDREF, JDREF, or DATEREF.

The time offset may serve to set a zero-point offset to a relative time series, allowing zero-relative times, or just higher precision, in the time stamps. Its default value is zero. The value of
this keyword affects the values of TSTART, and TSTOP, as well as
any time pixel values in a binary table. However, this construct
_may_ only be used in tables and _must not_ be used in images.

41

<a id="942-time-resolution-and-binning"></a>
#### 9.4.2. Time resolution and binning

The resolution of the time stamps (the width of the time sampling function) _shall_ be specified by the following keyword.

TIMEDEL – [floating-point] The value field of this keyword
_shall_ contain the value of the time resolution in the units of
TIMEUNIT. This construct, when present, _shall only_ be used
in tables and _must not_ be used in images.

In tables this may, for instance, be the size of the bins for timeseries data or the bit precision of the time-stamp values.
When data are binned in time bins (or, as a special case,
events are tagged with a time stamp of finite precision) it is important to know to the position within the bin (or pixel) to which
the time stamp refers. Coordinate values normally correspond
to the center of all pixels (see Sect. 8.2); yet clock readings are
effectively truncations, not rounded values, and therefore correspond to the lower bound of the pixel.

TIMEPIXR – [floating-point; default: 0.5] The value field of
this keyword _shall_ contain the value of the position within
the pixel, from 0.0 to 1.0, to which the time-stamp refers.
This construct, when present, _shall only_ be used in tables
and _must not_ be used in images.

A value of 0.0 may be more common in certain contexts, e.g.
when truncated clock readings are recorded, as is the case for
almost all event lists.

<a id="943-time-errors"></a>
#### 9.4.3. Time errors

The absolute time error is the equivalent of a systematic error,
_shall_ be given by the following keyword.

TIMSYER – [floating-point; default: 0.] The value field of this
keyword _shall_ contain the value of the absolute time error, in
units of TIMESYS.

This keyword _may_ be overridden, in appropriate context (e.g.,
time axes in image arrays or table columns; by the CSYER _ia_ keywords and their binary-table equivalents (see Table 22).
The relative time error specifies accuracy of the time stamps
relative to each other. This error will usually be much smaller
than the absolute time error. This error is equivalent to a random
error, and _shall_ be given by the following keyword.

TIMRDER – [floating-point; default: 0.] The value field of this
keyword _shall_ contain the value of the relative time error, i.e.
the random error between time stamps, in units of TIMESYS.

This keyword _may_ be overridden, in appropriate context (e.g.,
time axes in image arrays or table columns; by the CRDER _ia_ keywords and their binary-table equivalents (see Table 22).

<a id="95-global-time-keywords"></a>
### 9.5. Global time keywords

The time keywords in Table 35 are likely to occur in headers
even when there are no time axes in the data. Except for DATE,
they provide the top-level temporal bounds of the data in the
HDU. As noted before, they may also be implemented as table
columns. Keywords not previously described are defined below;
all are included in the summary Table 22.

42

Table 35: Keywords for global time values

Keyword Notes
DATE Defined in Sect. 4.4.2.
DATE-OBS Defined in Sect. 4.4.2. Keyword value was not restricted to mean the start time of an observation, and
has historically also been used to indicate some form
of mean observing date and time. To avoid ambiguity
use DATE-BEG instead.
DATE-BEG Defined in this section.
DATE-AVG Defined in Sect. 8.4.1. The method by which average times should be calculated is not defined by this
Standard.
DATE-END Defined in this section.
MJD-OBS Defined in Sect. 8.3.
MJD-BEG Defined in this section.
MJD-AVG Defined in Sect. 8.4.1. The method by which average times should be calculated is not defined by this
Standard.
MJD-END Defined in this section.
TSTART Defined in this section.
TSTOP Defined in this section.

DATE-BEG – [datetime] The value field of this keyword _shall_
contain a character string in ISO-8601 format that specifies
the start time of data acquisition in the time system specified
by the TIMESYS keyword.
DATE-END – [datetime] The value field of this keyword _shall_
contain a character string in ISO-8601 format that specifies
the stop time of data acquisition in the time system specified
by the TIMESYS keyword.
MJD-BEG – [floating-point] The value field of this keyword
_shall_ contain the value of the MJD start time of data acquisition in the time system specified by the TIMESYS keyword.
MJD-END – [floating-point] The value field of this keyword
_shall_ contain the value of the MJD stop time of data acquisition in the time system specified by the TIMESYS keyword.
TSTART – [floating-point] The value field of this keyword _shall_
contain the value of the start time of data acquisition in units
of TIMEUNIT, relative to MJDREF, JDREF, or DATEREF and
TIMEOFFS, in the time system specified by the TIMESYS keyword.
TSTOP – [floating-point] The value field of this keyword _shall_
contain the value of the stop time of data acquisition in units
of TIMEUNIT, relative to MJDREF, JDREF, or DATEREF and
TIMEOFFS, in the time system specified by the TIMESYS keyword.

The alternate-axis equivalent keywords for BINTABLE extensions, DOBS _n_, MJDOB _n_, DAVG _n_, and MJDA _n_, as defined in
Table 22, are also allowed. Note that of the above only TSTART
and TSTOP are relative to the time reference value. As in the case
of the time reference value (see Sect. 9.2.2), the JD values supersede DATE values, and MJD values supersede both, in cases
where conflicting values are present.
It should be noted that, although they do not represent global
time values within an HDU, the CRVAL _ia_ and CDELT _ia_ keywords,
and their binary-table equivalents (see Table 22), also represent
(binary) time values. They should be handled with the same care
regarding precision when combining them with the time reference value, as any other time value.

Finally, Julian and Besselian epochs (see
Sects. 9.1.2 and 9.3) _may_ be expressed by these two keywords – to be used with great caution, as their definitions are
more complicated and hence their use more prone to confusion.

JEPOCH – [floating-point] The value field of this keyword _shall_
contain the value of the Julian epoch, with an implied time
scale of ’TDB’.
BEPOCH – [floating-point] The value field of this keyword _shall_
contain the value of the Besselian epoch, with an implied
time scale of ’ET’.

When these epochs are used as time stamps in a table column, their interpretation will be clear from the context. When
the keywords appear in the header without obvious context, they
_must_ be regarded as equivalents of DATE-OBS and MJD-OBS, i.e.,
with no fixed definition as to what part of the dataset they refer.

<a id="96-other-time-coordinate-axes"></a>
### 9.6. Other time-coordinate axes

There are a few coordinate axes that are related to time and that
are accommodated in this Standard: (temporal) _phase_, _timelag_,
and _frequency_ . Phase results from folding a time series on a
given period, and can appear in parallel with _time_ as an alternate description of the same axis. Timelag is the coordinate of
cross- and auto-correlation spectra. The temporal _frequency_ is
the Fourier transform equivalent of time and, particularly, the
coordinate axis of power spectra; spectra where the dependent
variable is the electromagnetic field are excluded here, but see
Greisen et al. (2006). These coordinate axes _shall_ be specified
by giving CTYPE _i_ and its binary-table equivalents one of the values: ’PHASE’, ’TIMELAG’, or ’FREQUENCY’.
Timelag units are the regular time units, and the basic unit
for frequency is ’Hz’. Neither of these two coordinates is a linear or scaled transformation of time, and therefore cannot appear
in parallel with time as an alternate description. That is, a given
vector of values for an observable can be paired with a coordinate vector of time, or timelag, or frequency, but not with more
than one of these; the three coordinates are orthogonal.
Phase can appear in parallel with time as an alternate description of the same axis. Phase _shall_ be recorded in the following
keywords.

CZPHS _ia_ – [floating-point] The value field of this keyword _shall_
contain the value of the time at the zero point of a phase axis.
Its units _may_ be ’deg’, ’rad’, or ’turn’.
CPERI _ia_ – [floating-point] The value field of this keyword, if
present _shall_ contain the value of the period of a phase axis.
This keyword can be used only if the period is a constant; if
that is not the case, this keyword _should_ either be absent or

set to zero.

CZPHS _ia may_ instead appear in binary-table forms TCZPH _n_,
TCZP _na_, _i_ CZPH _n_, and _i_ CZP _na_ . CPERI _ia may_ instead appear in
binary-table forms TCPER _n_, TCPR _na_, _i_ CPER _n_, and _i_ CPR _na_ . The
phase, period, and zero point _shall_ be expressed in the globally
valid time reference frame and unit as defined by the global keywords (or their defaults) in the header.

<a id="97-durations"></a>
### 9.7. Durations

There is an extensive collection of header keywords that indicate time durations, such as exposure times, but there are many

pitfalls and subtleties that make this seemingly simple concept
treacherous. Because of their crucial role and common use, keywords are defined below to record exposure and elapsed time.

XPOSURE – [floating-point] The value field of this keyword
_shall_ contain the value for the effective exposure duration
for the data, corrected for dead time and lost time in the units
of TIMEUNIT. If the HDU contains multiple time slices, this
value _shall_ be the total accumulated exposure time over all
slices.
TELAPSE – [floating-point] The value field of this keyword
_shall_ contain the value for the amount of time elapsed, in
the units of TIMEUNIT, between the start and the end of the
observation or data stream.

Durations _must not_ be expressed in ISO-8601 format, but
only as actual durations (i.e., numerical values) in the units of
the specified time unit.
Good-Time-Interval (GTI) tables are common for exposures
with gaps in them, particularly photon-event files, as they make
it possible to distinguish time intervals with “no signal detected”
from “no data taken.” GTI tables in BINTABLE extensions _must_
contain two mandatory columns, START and STOP, and _may_ contain one _optional_ column, WEIGHT. The first two define the interval, the third, with a value between 0 and 1, the quality of the interval; _i.e.,_ a weight of 0 indicates a _Bad_ -Time-Interval. WEIGHT
has a default value of 1. Any time interval not covered in the
table _shall_ be considered to have a weight of zero.

<a id="98-recommended-best-practices"></a>
### 9.8. Recommended best practices

The following guidelines should be helpful in creating data products with a complete and correct time representation.

**–** The presence of the informational DATE keyword is _strongly_
_recommended_ in all HDUs.
**–** One or more of the informational keywords DATE-xxxx
and/or MJD-xxxx _should_ be present in all HDUs whenever a
meaningful value can be determined. This also applies, e.g.,
to catalogs derived from data collected over a well-defined
time range.
**–** The global keyword TIMESYS is _strongly recommended_ .
**–** The global keywords MJDREF or JDREF or DATEREF are
_recommended_ .
**–** The remaining informational and global keywords _should_ be
present whenever applicable.
**–** All context-specific keywords _shall_ be present as needed and
_required_ by the context of the data.

<a id="981-global-keywords-and-overrides"></a>
#### 9.8.1. Global keywords and overrides

For reference to the keywords that are discussed here, see
Table 22. The globally applicable keywords listed in Sect. B of
the table serve as default values for the corresponding C* and
TC* keywords in that same section, but only when axis and column specifications (including alternate coordinate definitions)
use a time scale listed in Table 30, or when the corresponding
CTYPE or TTYPE keywords are set to the value ’TIME’. Any alternate coordinate specified in a non-recognized time scale assumes the value of the axis pixels or the column cells, optionally
modified by applicable scaling and/or reference value keywords;
see also Sect. 9.2.1.

43

<a id="982-restrictions-on-alternate-descriptions"></a>
#### 9.8.2. Restrictions on alternate descriptions

An image will have at most one time axis as identified by having the CTYPE _i_ value of ’TIME’ or one of the values listed in
Table 30. Consequently, as long as the axis is identified through
CTYPE _i_, there is no need to have axis-number identification
on the global time-related keywords. It is expressly prohibited
to specify more than one time reference position on this axis
for alternate time-coordinate frames, since this would give rise
to complicated model-dependent non-linear relations between
these frames. Hence, time scales ’TDB’ and ’TCB’ (or ’ET’, to
its precision) _may_ be specified in the same image, but cannot be
combined with any of the first nine time scales in Table 30; those
first nine can be expressed as linear transformations of each
other, too, provided the reference position remains unchanged.
Time scale ’LOCAL’ is by itself, intended for simulations, and
_should not_ be mixed with any of the others.

<a id="983-image-time-axes"></a>
#### 9.8.3. Image time axes

Sect. 8.2 requires keywords CRVAL _ia_ to be numeric and they cannot be expressed in ISO-8601 format. Therefore it is _required_
that CRVAL _ia_ contain the elapsed time in units of TIMEUNIT or
CUNIT _ia_, even if the zero point of time is specified by DATEREF.
If the image does not use a matrix for scaling, rotation, and
shear (Greisen & Calabretta 2002), CDELT _ia_ provides the numeric value for the time interval. If the PC form of scaling, rotation, and shear (Greisen & Calabretta 2002) is used, CDELT _ia_
provides the numeric value for the time interval, and PC _i j_, where
_i_ = _j_ = the index of the time axis (in the typical case of an image cube with Axis 3 being time, _i_ = _j_ = 3) would take the
exact value 1, the default (Greisen & Calabretta 2002). When
the CD _i j_ form of mapping is used, CD _i j_ provides the numeric
value for the time interval. If one of the axes is time and the
matrix form is used, then the treatment of the PC _i ja_ (or CD _i ja_ )
matrices involves at least a Minkowsky metric and Lorentz transformations (as contrasted with Euclidean and Galilean).

<a id="10-representations-of-compressed-data"></a>
## 10. Representations of compressed data

Minimizing data volume is important in many contexts, particularly for publishers of large astronomical data collections. The
following sections describe compressed representations of data
in _FITS_ images and BINTABLE extensions that preserve metadata and allow for full or partial extraction of the original data as
necessary. The resulting _FITS_ file structure is independent of the
specific data-compression algorithm employed. The implementation details for some compression algorithms that are widely
used in astronomy are defined in Sect. 10.4, but other compression techniques could also be supported. See the _FITS_ convention by White et al. (2013) for details of the compression techniques, but beware that the specifications in this Standard _shall_
supersede those in the registered convention.

Compression of _FITS_ files can be beneficial for sites that
store or distribute large quantities of data; the present section
provides a standard framework that addresses such needs. As implementation of compression/decompression codes can be quite
complex, not all software for reading and writing _FITS_ is nec

44

essarily expected to support these capabilities. External utilities
are available to compress and decompress _FITS_ files [15] .

<a id="101-tiled-image-compression"></a>
### 10.1. Tiled image compression

The following describes the process for compressing
_n_ −dimensional _FITS_ images and storing the resulting byte
stream in a variable-length column in a _FITS_ binary table, and
for preserving the image header keywords in the table header.
The general principle is to first divide the _n_ −dimensional image
into a rectangular grid of subimages or “tiles.” Each tile is then
compressed as a block of data, and the resulting compressed
byte stream is stored in a row of a variable-length column
in a _FITS_ binary table (see Sect. 7.3). By dividing the image
into tiles it is possible to extract and decompress subsections
of the image without having to decompress the whole image.
The default tiling pattern treats each row of a two-dimensional
image (or higher-dimensional cube) as a tile, such that each
tile contains NAXIS1 pixels. This default may not be optimal
for some applications or compression algorithms, so any other
rectangular tiling pattern _may_ be defined using keywords that
are defined below. In the case of relatively small images it may
suffice to compress the entire image as a single tile, resulting
in an output binary table containing a single row. In the case of
three-dimensional data cubes, it may be advantageous to treat
each plane of the cube as a separate tile if application software
typically needs to access the cube on a plane-by-plane basis.

<a id="1011-required-keywords"></a>
#### 10.1.1. Required keywords

In addition to the mandatory keywords for BINTABLE extensions
(see Sect. 7.3.1) the following keywords are reserved for use in
the header of a _FITS_ binary-table extension to describe the structure of a valid compressed _FITS_ image. All are mandatory.

ZIMAGE – [logical; value T] The value field of this keyword
_shall_ contain the logical value T to indicate that the _FITS_
binary-table extension contains a compressed image, and
that logically this extension _should_ be interpreted as an image rather than a table.
ZCMPTYPE – [string; default: none] The value field of this keyword _shall_ contain a character string giving the name of the
algorithm that was used to compress the image. Only the values given in Table 36 are permitted; the corresponding algorithms are described in Sect. 10.4. Other algorithms may be
added in the future.
ZBITPIX – [integer; default: none] The value field of this keyword _shall_ contain an integer that gives the value of the
BITPIX keyword in the uncompressed _FITS_ image.
ZNAXIS – [integer; default: none] The value field of this keyword _shall_ contain an integer that gives the value of the
NAXIS keyword (i.e., the number of axes) in the uncompressed _FITS_ image.
ZNAXIS _n_ – [integer; indexed; default: none) The value field of
these keywords _shall_ contain a positive integer that gives the
value of the corresponding NAXIS _n_ keywords (i.e., the size
of Axis _n_ ) in the uncompressed _FITS_ image.

15
[e.g. fpack/funpack, see https://heasarc.gsfc.nasa.gov/](https://heasarc.gsfc.nasa.gov/fitsio/fpack/)
[fitsio/fpack/](https://heasarc.gsfc.nasa.gov/fitsio/fpack/)

The comment fields for the BITPIX, NAXIS, and NAXIS _n_
keywords in the uncompressed image _should_ be copied to the
corresponding fields in the ZBITPIX, ZNAXIS, and ZNAXIS _n_
keywords.

<a id="1012-other-reserved-keywords"></a>
#### 10.1.2. Other reserved keywords

The compressed image tiles _must_ be stored in the binary table
in the same order that the first pixel in each tile appears in the
_FITS_ image; the tile containing the first pixel in the image _must_
appear in the first row of the table, and the tile containing the
last pixel in the image _must_ appear in the last row of the binary
table. The following keywords are reserved for use in describing
compressed images stored in BINTABLE extensions; they _may_ be
present in the header, and their values depend upon the type of
image compression employed.

ZTILE _n_ – [integer; indexed; default: 1 for _n_ - 1] The value
field of these keywords (where _n_ is a positive integer index
that ranges from 1 to ZNAXIS) _shall_ contain a positive integer
representing the number of pixels along Axis _n_ of the compressed tiles. Each tile of pixels _must_ be compressed separately and stored in a row of a variable-length vector column
in the binary table. The size of each image dimension (given
by ZNAXIS _n_ ) need not be an integer multiple of ZTILE _n_, and
if it is not, then the last tile along that dimension of the image will contain fewer image pixels than the other tiles. If the
ZTILE _n_ keywords are not present then the default “row-byrow” tiling will be assumed, i.e., ZTILE1 = ZNAXIS1, and
the value of all the other ZTILE _n_ keywords _must_ equal 1.
ZNAME _i_ – [string; indexed; default: none] The value field of
these keywords (where _i_ is a positive integer index starting with 1) _shall_ supply the names of up to 999 algorithmspecific parameters that are needed to compress or decompress the image. The order of the compression parameters
_may_ be significant, and _may_ be defined as part of the description of the specific decompression algorithm.
ZVAL _i_ – [string; indexed; default: none] The value field of these
keywords (where _i_ is a positive integer index starting with
1) _shall_ contain the values of up to 999 algorithm-specific
parameters with the same index _i_ . The value of ZVAL _i may_
have any valid _FITS_ data type.
ZMASKCMP – [string; default: none] The value field of this keyword _shall_ contain the name of the image compression algorithm that was used to compress the optional null-pixel
data mask. This keyword _may_ be omitted if no null-pixel data
masks appear in the table. See Sect. 10.2.2 for details.
ZQUANTIZ – [string; default: ’NO DITHER’] The value field of
this keyword _shall_ contain the name of the algorithm that was
used to quantize floating-point image pixels into integer values, which were then passed to the compression algorithm as
discussed further in Sect. 10.2. If this keyword is not present,
the default is to assume that no dithering was applied during
quantization.
ZDITHER0 – [integer; default: none] The value field of this keyword _shall_ contain a positive integer (that may range from 1
to 10000 inclusive) that gives the seed value for the random
dithering pattern that was used when quantizing the floatingpoint pixel values. This keyword _may_ be absent if no dithering was applied. See Sect. 10.2 for further discussion.

The following keywords are reserved to preserve a verbatim
copy of the _value and comment fields_ for keywords in the original uncompressed _FITS_ image that were used to describe its
structure. These optional keywords, when present, _shall_ be used
when reconstructing an identical copy of the original _FITS_ HDU
of the uncompressed image. They _should not_ appear in the compressed image header unless the corresponding keywords were
present in the uncompressed image.

ZSIMPLE – [logical; value T] The value field of this keyword
_must_ contain the value of the original SIMPLE keyword in
the uncompressed image.
ZEXTEND – [string] The value field of this keyword _must_ contain the value of the original EXTEND keyword in the uncompressed image.
ZBLOCKED – [logical] The value field of this keyword _must_ contain the value of the original BLOCKED keyword in the uncompressed image.
ZTENSION – [string] The value field of this keyword _must_ contain the original XTENSION keyword in the uncompressed
image.
ZPCOUNT – [integer] The value field of this keyword _must_ contain the original PCOUNT keyword in the uncompressed im
age.
ZGCOUNT – [integer] The value field of this keyword _must_ contain the original GCOUNT keyword in the uncompressed im
age.
ZHECKSUM – [string] The value field of this keyword _must_ contain the original CHECKSUM keyword (see Sect. 4.4.2.7) in the
uncompressed image.
ZDATASUM – [string] The value field of this keyword _must_ contain the original DATASUM keyword (see Sect. 4.4.2.7) in the
uncompressed image.

The ZSIMPLE, ZEXTEND, and ZBLOCKED keywords _must not_
be used unless the original uncompressed image was contained
in the primary array of a _FITS_ file. The ZTENSION, ZPCOUNT,
and ZGCOUNT keywords _must not_ be used unless the original uncompressed image was contained in an IMAGE extension.
The _FITS_ header of the compressed image _may_ contain other
keywords. If a _FITS_ primary array or IMAGE extension is compressed using the procedure described here, it is _strongly rec-_
_ommended_ that all the keywords (including comment fields) in
the header of the original image, except for the mandatory keywords mentioned above, be copied verbatim and in the same order into the header of the binary-table extension that contains
the compressed image. All these keywords will have the same
meaning and interpretation as they did in the original image,
even in cases where the keyword is not normally expected to
occur in the header of a binary-table extension (e.g., the BSCALE
and BZERO keywords, or the world-coordinate-system keywords
such as CTYPE _n_, CRPIX _n_, and CRVAL _n_ ).

<a id="1013-table-columns"></a>
#### 10.1.3. Table columns

Two columns in the _FITS_ binary table are defined below to contain the compressed image tiles; the order of the columns in the
table is not significant. One of the table columns describes optional content; but when this column appears it _must_ be used as
defined in this section. The column names (given by the TTYPE _n_

45

keyword) are reserved; they are shown here in upper-case letters,
but case is not significant.

COMPRESSED DATA – [variable-length; _required_ ] Each row of
this column _must_ contain the byte stream that is generated
as a result of compressing the corresponding image tile. The
data type of the column (as given by the TFORM _n_ keyword)
_must_ be one of ’1PB’, ’1PI’, or ’1PJ’ (or the equivalent
’1QB’, ’1QI’, or ’1QJ’), depending on whether the compression algorithm generates an output stream of 8-bit bytes,
or integers of 16, or 32 bits respectively.

When using the quantization method to compress floatingpoint images that is described in Sect. 10.2, it sometimes may
not be possible to quantize some of the tiles (e.g., if the range of
pixels values is too large or if most of the pixels have the same
value and hence the calculated RMS noise level in the tile is
close to zero). There also may be other rare cases where the nominal compression algorithm cannot be applied to certain tiles. In
these cases, an alternate technique _may_ be used in which the raw
pixel values are losslessly compressed with the Gzip algorithm.

GZIP COMPRESSED DATA [variable-length; _optional_ ] If the raw
pixel values in an image tile are losslessly compressed with
the Gzip algorithm, the resulting byte stream _must_ be stored
in this column (with a ’1PB’ or ’1QB’ variable-length arraycolumn format). The corresponding COMPRESSED DATA column for these tiles _must_ contain a null pointer (i.e., the pair
of integers that constitute the descriptor for the column _must_
both have the value zero: see Sect. 7.3.5).

The compressed data columns described above _may_ use either the ’1P’ or ’1Q’ variable-length array _FITS_ column format
if the size of the heap in the compressed _FITS_ file is < 2.1 GB. If
the the heap is larger than 2.1 GB, then the ’1Q’ format (which
uses 64-bit pointers) _must_ be used.
When using the _optional_ quantization method described in
Sect. 10.2 to compress floating-point images, the following
columns are _required_ .

ZSCALE – [floating-point; _optional_ ] This column _shall_ be used
to contain linear scale factors that, along with ZZERO, transform the floating-point pixel values in each tile to integers
via,

_F_ _i_ − ZZERO
_I_ _i_ = round
� ZSCALE

(12)
�

value. However, these pixel values will be altered when using
the quantization method described in Sect. 10.2 to compress
floating-point images. The value of the undefined pixels _may_ be
preserved in the following way.

ZBLANK – [integer; _optional_ ] When present, this column _shall_
be used to store the integer value that represents undefined
pixels in the scaled integer array. The _recommended_ value
for ZBLANK is −2147483648, the largest negative 32-bit integer. If the same null value is used in every tile of the image, then ZBLANK _may_ be given in a header keyword instead
of a table column; if both a keyword and a table column
named ZBLANK are present, the values in the table column
_must_ be used. If there are no undefined pixels in the image
then ZBLANK is _not required_ to be present either as a table
column or a keyword.

If the uncompressed image has an integer data type
(ZBITPIX > 0) then the value of undefined pixels is given by the
BLANK keyword (see Sect. 5.3), which _should_ be used instead of
ZBLANK.
When using some compression techniques that do not exactly preserve integer pixel values, it may be necessary to store
the location of the undefined pixels prior to compressing the image. The locations _may_ be stored in an image mask, which _must_
itself be compressed and stored in a table column with the following definition. See Sect. 10.2.2 for more details.

NULL PIXEL MASK – [integer array; _optional_ ] When present,
this column _shall_ be used to store, in compressed form, an
image mask with the same original dimensions as the uncompressed image, that records the location of the undefined
pixels. The process defined in Sect. 10.2.2 _shall_ be used to
construct the compressed pixel mask.

Additional columns _may_ be present in the table to supply
other parameters that relate to each image tile. However, these
parameters _should not_ be recorded in the image HDU when the
uncompressed image is restored.

<a id="102-quantization-of-floating-point-data"></a>
### 10.2. Quantization of floating-point data

While floating-point format images may be losslessly compressed, noisy images often do not compress very well. Higher
compression can only be achieved by removing some of this
noise without losing the useful information content. One commonly used technique for reducing the noise is to scale the
floating-point values into quantized integers using Eq. 12, and
using the ZSCALE and ZZERO columns to record the two scaling coefficients that are used for each tile. Note that the absence
of these two columns in a tile-compressed floating-point image
is an indication that the image was not scaled, and was instead
losslessly compressed.
An effective scaling algorithm for preserving a specified amount of noise in each pixel value is described by
White & Greenfield (1999) and by Pence et al. (2009). With this
method, the ZSCALE value (which is numerically equal to the
spacing between adjacent quantization levels) is calculated to be
some fraction, _Q_, of the RMS noise as measured in background
regions of the image. Pence et al. (2009) shows that the number of binary bits of noise that are preserved in each pixel value
is given by _log_ 2 ( _Q_ ) + 1.792. The _Q_ value directly affects the

where _I_ _i_ and _F_ _i_ are the integer and (original) floating-point
values of the image pixels, respectively, and the round function rounds the result to the nearest integer value.
ZZERO – [floating-point; optional] This column _shall_ be used to
contain zero-point offsets that are used to scale the floatingpoint pixel values in each tile to integers via Eq. 12.

Do not confuse the ZSCALE and ZZERO columns with the
BSCALE and BZERO keywords (defined in Sect. 4.4.2) that may be
present in integer _FITS_ images. Any such integer images _should_
normally be compressed without any further scaling, and the
BSCALE and BZERO keywords _should_ be copied verbatim into
the header of the binary table containing the compressed image.
Some images contain undefined pixel values; in uncompressed floating-point images these pixels have an IEEE NaN

46

compressed file size: decreasing _Q_ by a factor of two will decrease the file size by about one bit per pixel. In order to achieve
the greatest amount of compression, one should use the smallest
value of _Q_ that still preserves the required amount of photometric and astrometric precision in the image.
A potential problem when applying this scaling method to
astronomical images, in particular, is that it can lead to a systematic bias in the measured intensities in faint parts of the image.
As the image is quantized more coarsely, the measured intensity
of the background regions of the sky will tend to be biased towards the nearest quantize level. One very effective technique
for minimizing this potential bias is to dither the quantized pixel
values by introducing random noise during the quantization process. So instead of simply scaling every pixel value in the same
way using Eq. 12, the quantized levels are randomized by using
this slightly modified equation:

_F_ _i_ − ZZERO
_I_ _i_ = round + _R_ _i_ − 0.5 (13)
� ZSCALE �

where _R_ _i_ is a random number between 0.0 and 1.0, and 0.5 is
subtracted so that the mean quantity equals 0. Then restoring the
floating-point value, the same _R_ _i_ is used with the inverse formula:

_F_ _i_ = (( _I_ _i_ − _R_ _i_ + 0.5) ∗ ZSCALE) + ZZERO. (14)

This “subtractive dithering” technique has the effect of dithering
the zero point of the quantization grid on a pixel-by-pixel basis
without adding any actual noise to the image. The net effect of
this is that the mean (and median) pixel value in faint regions
of the image more closely approximate the value in the original unquantized image than if all the pixels are scaled without
dithering.
The key requirement when using this subtractive-dithering
technique is that _the exact same random-number sequence must_
be used when quantizing the pixel values to integers, and when
restoring them to floating-point values. While most computer
languages supply a function for generating random numbers,
these functions are not guaranteed to generate the same sequence
of numbers every time. An algorithm for generating a repeatable
sequence of pseudo-randomnumbers is given in Appendix I; this
algorithm _must_ be used when applying a subtractive dither.

<a id="1021-dithering-algorithms"></a>
#### 10.2.1. Dithering algorithms

The ZQUANTIZ keyword, if present, _must_ have one of the following values to indicate the type of quantization, if any, that
was applied to the floating-point image for compression.

’NO DITHER’ – No dithering was performed; the floating-point
pixels were simply quantized using Eq. 12. This option _shall_
be assumed if the ZQUANTIZ keyword is not present in the
header of the compressed floating-point image.

’SUBTRACTIVE DITHER 1’ – The basic subtractive dithering
was performed, the algorithm for which is described below.
Note that an image quantized using this technique can still
be unquantized using the simple linear scaling function given
by Eq. 12, at the cost of introducing slightly more noise in the
image than if the full subtractive-dithering algorithm were
applied.

’SUBTRACTIVE DITHER 2’ – This dithering algorithm is identical to that for ’SUBTRACTIVE DITHER 1’, except that any

pixels in the floating-point image that are exactly equal to
0.0 are represented by the reserved value −2147483647 in
the quantized integer array. When the image is subsequently
decompressed and unscaled, these pixels _must_ be restored to
their original value of 0.0. This dithering option is useful if
the zero-valued pixels have special significance to the data
analysis software, so that the value of these pixels _must not_
be dithered.

The process for generating a subtractive dither for a floatingpoint image is the following.

1. Generate a sequence of 10000 single-precision floating-point
random numbers, RN, with a value between 0.0 and 1.0.
Since it could be computationally expensive to generate a
unique random number for every pixel of large images, simply cycle through this look-up table of random numbers.
2. Choose an integer in the range 1 to 10000 to serve as an
initial seed value for creating a unique sequence of random
numbers from the array that was calculated in the previous
step. The purpose of this is to reduce the chances of applying the same dithering pattern to two images that are subsequently subtracted from each other (or co-added), because
the benefits of randomized dithering are lost if all the pixels
are dithered in phase with each other. The exact method for
computing this seed integer is not important as long as the
value is chosen more or less randomly.
3. Write the integer seed value that was selected in the previous
step as the value of the ZDITHER0 keyword in the header of
the compressed image. This value is required to recompute
the same dithering pattern when decompressing the image.
4. Before quantizing each tile of the floating-point image, calculate an initial value for two offset parameters, _I_ 0 and _I_ 1,
with the following formulae:

_I_ 0 = mod( _N_ tile − 1 + ZDITHER0, 10000) (15)

_I_ 1 = INT(RN( _I_ 0 ) ∗ 500.) (16)

where _N_ tile is the row number in the binary table that is used
to store the compressed bytes for that tile, ZDITHER0 is that
value of that keyword, and RN(dom number in the sequence that was computed in the first _I_ 0 ) is the value of the _I_ 0 [th] [ran-]
step. Note that _I_ 0 has a value in the range 0 to 9999 and _I_ 1
has a value in the range 0 to 499. This method for computing
_I_ 0 and _I_ 1 was chosen so that a different sequence of random
numbers is used to compress successive tiles in the image,
and so that the sequence of _I_ 1 values has a length of order
100-million elements before repeating.
5. Now quantize each floating-point pixel in the tile using
Eq. 13 and using random number RN( _I_ 1 ) for the first pixel.
Increment the value of _I_ 1 for each subsequent pixel in the
tile. If _I_ 1 reaches the upper limit of 500, then increment the
value of _I_ 0 and recompute _I_ 1 from Eq. 16. If _I_ 0 also reaches
the upper limit of 10000, then reset _I_ 0 to 0.
If the floating-point pixel has an IEEE NaN value, then it is
not quantized or dithered but instead is set to the reserved
integer value specified by the ZBLANK keyword. For consistency, the value of _I_ 1 _should_ also be incremented in this case
even though it is not used.
6. Compress the array of quantized integers using the lossless
algorithm that is specified by the ZCMPTYPE keyword (use
’RICE 1’ by default).

47

7. Write the compressed byte stream into the
COMPRESSED DATA column in the appropriate row of
the binary table corresponding to that tile.
8. Write the linear scaling and zero-point values that were used
in Eq. 13 for that tile into the ZSCALE and ZZERO columns,
respectively, in the same row of the binary table.
9. Repeat Steps 4 through 8 for each tile of the image.

<a id="1022-preserving-undefined-pixels-with-lossy-compression"></a>
#### 10.2.2. Preserving undefined pixels with lossy compression

The undefined pixels in integer images are flagged by a reserved
BLANK value and will be preserved if a lossless compression algorithm is used. (ZBLANK is used for undefined pixels in floatingpoint images.) If the image is compressed with a lossy algorithm,
then some other technique _must_ be used to identify the undefined
pixels in the image. In this case it is _recommended_ that the undefined pixels be recorded with the following procedure.

1. Create an integer data mask with the same dimensions as the
image tile.
2. For each undefined pixel in the image, set the corresponding
mask pixels to 1 and all the other pixels to 0.
3. Compress the mask array using a lossless algorithm such as
PLIO or Gzip, and record the name of that algorithm with
the keyword ZMASKCMP.
4. Store the compressed byte stream in a variable-length-array
column called NULL PIXEL MASK in the table row corresponding to that image tile.

The data mask array pixels _should_ have the shortest integer
data type that is supported by the compression algorithm (i.e.,
usually eight-bit bytes). When decompressing the image tile, the
software _must_ check if the corresponding compressed data mask
exists with a length greater than 0, and if so, decompress the
mask and set the corresponding undefined pixels in the image
array to the value given by the BLANK keyword.

<a id="103-tiled-table-compression"></a>
### 10.3. Tiled table compression

The following section describes the process for compressing
the content of BINTABLE columns. Some additional details of
BINTABLE compression may be found in Pence et al. (2013), but
the specifications in this Standard _shall_ supersede those in the
registered convention. The uncompressed table _may_ be subdivided into tiles, each containing a subset of rows, then each column of data within each tile is extracted, compressed, and stored
as a variable-length array of bytes in the output compressed table. The header keywords from the uncompressed table, with
only a few limited exceptions, _shall_ be copied verbatim to the
header of the compressed table. The compressed table _must_ itself be a valid _FITS_ binary table (albeit one where the contents
cannot be interpreted without decompressing the contents) that
contains the same number and order of columns as in the uncompressed table, and that contains one row for each tile of rows in
the uncompressed table. Only the compression algorithms specified in Sect. 10.3.5 are permitted.

<a id="1031-required-keywords"></a>
#### 10.3.1. Required keywords

With only a few exceptions noted below, all the keywords and
corresponding comment fields from the uncompressed table
_must_ be copied verbatim, in order, into the header of the com

48

pressed table. Note in particular that the values of the reserved
column descriptor keywords TTYPEn, TUNIT _n_, TSCAL _n_, TZERO _n_,
TNULL _n_, TDISP _n_, and TDIM _n_, as well as all the column-specific
WCS keywords defined in the _FITS_ Standard, _must_ have the
same values and data types in both the original and in the compressed table, with the understanding that these keywords apply
to the uncompressed data values.

The only keywords that _must not_ be copied verbatim from
the uncompressed table header to the compressed table header
are the mandatory NAXIS1, NAXIS2, PCOUNT, and TFORM _n_ keywords, and the optional CHECKSUM, DATASUM (see Sect. 4.4.2.7),
and THEAP keywords. These keywords must necessarily describe
the contents and structure of the compressed table itself. The
original values of these keywords in the uncompressed table
_must_ be stored in a new set of reserved keywords in the compressed table header. Note that there is no need to preserve a
copy of the GCOUNT keyword because the value is always equal
to 1 for BINTABLE extensions. The complete set of keywords
that have a reserved meaning within a tile-compressed binary
table are given below.

ZTABLE – [logical; value: T] The value field of this keyword
_shall_ be T to indicate that the _FITS_ binary-table extension
contains a compressed BINTABLE, and that logically this extension _should_ be interpreted as a tile-compressed binary table.
ZNAXIS1 – [integer; default: none] The value field of this keyword _shall_ contain an integer that gives the value of the
NAXIS1 keyword in the original uncompressed _FITS_ table
header. This represents the width in bytes of each row in the
uncompressed table.
ZNAXIS2 – [integer; default: none] The value field of this keyword _shall_ contain an integer that gives the value of the
NAXIS2 keyword in the original uncompressed _FITS_ table
header. This represents the number of rows in the uncompressed table.
ZPCOUNT – [integer; default: none] The value field of this keyword _shall_ contain an integer that gives the value of the
PCOUNT keyword in the original uncompressed _FITS_ table
header.
ZFORM _n_ – [string; indexed; default: none] The value field of
these keywords _shall_ contain the character-string values of
the corresponding TFORM _n_ keywords that defines the data
type of Column _n_ in the original uncompressed _FITS_ table.
ZCTYP _n_ – [string; indexed; default: none] The value field
of these keywords _shall_ contain the character-string value
mnemonic name of the algorithm that was used to compress
Column _n_ of the table. The only permitted values are given in
Sect. 10.3.5, and the corresponding algorithms are described
in Sect. 10.4.
ZTILELEN – [integer; default: none] The value field of this keyword _shall_ contain an integer representing the number of
rows of data from the original binary table that are contained
in each tile of the compressed table. The number of rows
in the last tile may be fewer than in the previous tiles. Note
that if the entire table is compressed as a single tile, then
the compressed table will only contains a single row, and the
ZTILELEN and ZNAXIS2 keywords will have the same value.

<a id="1032-procedure-for-table-compression"></a>
#### 10.3.2. Procedure for table compression

The procedure for compressing a _FITS_ binary table consists of
the following sequence of steps.

1. Divide table into tiles ( _optional_ )
In order to limit the amount of data that must be managed at
one time, large _FITS_ tables _may_ be divided into tiles, each
containing the same number of rows (except for the last tile,
which _may_ contain fewer rows). Each tile of the table is compressed in order, and each is stored in a single row in the output compressed table. There is no fixed upper limit on the allowed tile size, but for practical purposes it is _recommended_
that it not exceed 100 MB.

2. Decompose each tile into the component columns
_FITS_ binary tables are physically stored in row-by-row sequential order, such that the data values for the first row in
each column are followed by the values in the second row,
and so on (see Sect. 7.3.3). Because adjacent columns in
binary tables can contain very non-homogeneous types of
data, it can be challenging to efficiently compress the native
stream of bytes in the _FITS_ tables. For this reason, the table is
first decomposed into its component columns, and then each
column of data is compressed separately. This also allows
one to choose the most-efficient compression algorithm for
each column.

3. Compress each column of data
Each column of data _must_ be compressed with one of the
lossless compression algorithms described in Sect. 10.4. If
the table is divided into tiles, then the same compression algorithm _must_ be applied to a given column in every tile. In
the case of variable-length array columns (where the data
are stored in the table heap: see Sect. 7.3.5), each individual
variable-length vector _must_ be compressed separately.

4. Store the compressed bytes
The compressed stream of bytes for each column _must_ be
written into the corresponding column in the output table.
The compressed table _must_ have exactly the same number and order of columns as the input table, however, the
data type of the columns in the output table will all have a
variable-length byte data type, with TFORM _n_ = ’1QB’. Each
row in the compressed table corresponds to a tile of rows in
the uncompressed table.
In the case of variable-length array columns, the array of
descriptors that point to each compressed variable-length
array, as well as the array of descriptors from the input
uncompressed table, _must_ also be compressed and written
into the corresponding column in the compressed table. See
Sect. 10.3.6 for more details.

<a id="1033-compression-directive-keywords"></a>
#### 10.3.3. Compression directive keywords

The following compression-directive keywords, if present in the
header of the table to be compressed, are reserved to provide
guidance to the compression software on how the table should be
compressed. The compression software _should_ attempt to obey
these directives, but if that is not possible the software _may_ disregard them and use an appropriate alternative. These keywords
are optional, but _must_ be used as specified below.

**–** FZTILELN – [integer] The value field of this keyword _shall_
contain an integer that specifies the requested number of table rows in each tile that are to be compressed as a group.
**–** FZALGOR – [string] The value field of this keyword _shall_ contain a character string giving the mnemonic name of the algorithm that is requested to be used by default to compress
every column in the table. The permitted values are given in
Sect. 10.3.5.
**–** FZALG _n_ – [string; indexed] The value fields of these keywords _shall_ contain a character string giving the mnemonic
name of the algorithm that is requested to compress
Column _n_ of the table. The current allowed values are the
same as for the FZALGOR keyword. The FZALGn keyword
takes precedence over FZALGOR in determining which algorithm to use for a particular column if both keywords are
present.

<a id="1034-other-reserved-keywords"></a>
#### 10.3.4. Other reserved keywords

The following keywords are reserved to store a verbatim copy
of the value and comment fields for specific keywords in the
original uncompressed BINTABLE. These keywords, if present,
_should_ be used to reconstruct an identical copy of the uncompressed BINTABLE, and _should not_ appear in the compressed table header unless the corresponding keywords were present in
the uncompressed BINTABLE.

ZTHEAP – [integer; default: none] The value field of this keyword _shall_ contain an integer that gives the value of the
THEAP keyword if present in the original uncompressed _FITS_
table header.
ZHECKSUM – [string; default: none] The value field of this keyword _shall_ contain a character string that gives the value of
the CHECKSUM keyword (see Sect. 4.4.2.7) in the original uncompressed _FITS_ HDU.
ZDATASUM – [string; default: none] The value field of this keyword _shall_ contain a character string that gives the value of
the DATASUM keyword (see Sect. 4.4.2.7) in the original uncompressed _FITS_ HDU.

<a id="1035-supported-compression-algorithms-for-tables"></a>
#### 10.3.5. Supported compression algorithms for tables

The permitted algorithms for compressing BINTABLE columns
are ’RICE 1’, ’GZIP 1’, and ’GZIP 2’ (plus ’NOCOMPRESS’),
which are lossless and are described in Sect. 10.4. Lossy compression could be allowed in the future once a process is defined
to preserve the details of the compression.

<a id="1036-compressing-variable-length-array-columns"></a>
#### 10.3.6. Compressing variable-length array columns

Compression of BINTABLE tiles that contain variable-length array (VLA) columns requires special consideration because the
array values in these columns are not stored directly in the table, but are instead stored in a data heap, which follows the main
table (see Sect. 7.3.5). The VLA column in the original, uncompressed table only contains descriptors, which comprise two integers that give the size and location of the arrays in the heap.
When decompressing, these descriptor values will be needed to
write the decompressed VLAs back into the same location in the
heap as in the original uncompressed table. Thus, the following

49

process _must_ be followed, in order, when compressing a VLA
column within a tile. Refer to Pence et al. (2013) for additional
details.

1. For each VLA in the column:

**–**
read the array from the input table, and compress it using
the algorithm specified by ZCTYP for this VLA column;

**–**
write the resulting byte stream to the heap of the compressed table; and

**–**
store (or append) the descriptors to the compressed byte
stream (which _must_ be 64-bit Q-type) in a temporary ar
ray.

2. Append the VLA descriptors from the uncompressed table
(which _may_ be either Q-type or P-type) to the temporary array of VLA descriptors for the compressed table.

3. Compress the combined array of descriptors using
’GZIP 1’, and write that byte stream into the corresponding VLA column in the output table, so that the
compressed array is appended to the heap.

When decompressing a VLA column, two stages of decompression _must_ be performed in order.

1. Decompress the combined array of descriptors using the
Gzip algorithm.

2. For each descriptor to a compressed array:

**–**
read the compressed VLA from the compressed table, and decompress it using the algorithm specified by
ZCTYP for this VLA column; and

**–**
write it to the correct location in the decompressed table.

<a id="104-compression-algorithms"></a>
### 10.4. Compression algorithms

Table 36: Valid mnemonic values for the ZCMPTYPE and ZCTYP _n_
keywords

Value Sect. Compression Type

’RICE 1’ 10.4.1 Rice algorithm for integer data
’GZIP 1’ 10.4.2 Combination of the LZ77 algorithm
and Huffman coding, used in GNU
Gzip
’GZIP 2’ 10.4.2 Like ’GZIP 1’, but with reshuffled
byte values
’PLIO 1’ 10.4.3 IRAF PLIO algorithm for integer data
’HCOMPRESS 1’ 10.4.4 H-compress algorithm for twodimensional images
’NOCOMPRESS’ The HDU remains uncompressed

The name of the permitted algorithms for compressing _FITS_
HDUs, as recorded in the ZCMPTYPE keyword, are listed in
Table 36; if other types are later supported, they _must_ be
registered with the IAUFWG to reserve the keyword values.
Keywords for the parameters of supported compression algorithms have also been reserved, and are described with each
algorithm in the subsections below. If alternative compression
algorithms require keywords beyond those defined below, they
_must_ also be registered with the IAUFWG to reserve the associated keyword names.

50

<a id="1041-rice-compression"></a>
#### 10.4.1. Rice compression

When ZCMPTYPE = ’RICE 1’, the Rice algorithm (Rice et al.
1993) _shall_ be used for data (de)compression. When selected,
the keywords in Table 37 _should_ also appear in the header with
one of the values indicated. If these keywords are absent, then
their default values _must_ be used. The Rice algorithm is lossless, but can only be applied to integer-valued arrays. It offers
a significant performance advantage over the other compression
techniques (see White et al. 2013).

Table 37: Keyword parameters for Rice compression

Values
Keyword Permitted Default Meaning

ZNAME1 ’BLOCKSIZE’ − Size of block in pixels
ZVAL1 16, 32 32 No. of pixels in a block
ZNAME2 ’BYTEPIX’ − Size of pixel value in bytes
ZVAL2 1, 2, 4, 8 4 No. of eight-bit bytes per original pixel value

<a id="1042-gzip-compression"></a>
#### 10.4.2. Gzip compression

When ZCMPTYPE = ’GZIP 1’, the Gzip algorithm _shall_ be used
for data (de)compression. There are no algorithm parameters,
so the keywords ZNAME _n_ and ZVAL _n should not_ appear in the
header. The Gzip algorithm is used in the free GNU software
compression utility of the same name. It was created by J.L. Gailly and M. Adler, based on the DEFLATE algorithm
(Deutsch 1996), which is a combination of LZ77 (Ziv & Lempel
1977) and Huffman coding. The Unix gzip program accepts an
integer parameter that provides a trade between optimization for
speed (1) and compression ratio (9), which does not affect the
format of the resultant data stream. The selection of this parameter is an implementation detail that is not covered by this
Standard.
When ZCMPTYPE = ’GZIP 2’, the gzip2 algorithm _shall_ be
used for data (de)compression. The gzip2 algorithm is a variation on ’GZIP 1’. There are no algorithm parameters, so the
keywords ZNAME _n_ and ZVAL _n should not_ appear in the header.
In this case the bytes in the array of data values are shuffled so
that they are arranged in order of decreasing significance before
being compressed. For example, a five-element contiguous array
of two-byte (16-bit) integer values, with an original big-endian
byte order of:

_A_ 1 _A_ 2 _B_ 1 _B_ 2 _C_ 1 _C_ 2 _D_ 1 _D_ 2 _E_ 1 _E_ 2

will have the following byte order after shuffling:

_A_ 1 _B_ 1 _C_ 1 _D_ 1 _E_ 1 _A_ 2 _B_ 2 _C_ 2 _D_ 2 _E_ 2,

where _A_ 1, _B_ 1, _C_ 1, _D_ 1, and _E_ 1 are the most-significant bytes from
each of the integer values. Byte shuffling _shall only_ be performed
for integer or floating-point numeric data types; logical, bit, and
character types _must not_ be shuffled.

<a id="1043-irafplio-compression"></a>
#### 10.4.3. IRAF/PLIO compression

When ZCMPTYPE = ’PLIO 1’. the IRAF PLIO algorithm _shall_
be used for data (de)compression. There are no algorithm parameters, so the keywords ZNAME _n_ and ZVAL _n should not_ appear in

the header. The PLIO algorithm was developed to store integervalued image masks in a compressed form. The compression algorithm used is based on run-length encoding, with the ability
to dynamically follow level changes in the image, in principle
allowing a 16-bit encoding to be used regardless of the image
depth. However, this algorithm has only been implemented in a
way that supports image depths of no more than 12 bits; therefore ’PLIO 1’ _must only_ be used for integer image types with
values between 0 and 2 [24] .
The compressed line lists are stored as variable-length arrays
of type short integer (16 bits per list element), regardless of the
mask depth. A line list consists of a series of simple instructions,
which are executed in sequence to reconstruct a line of the mask.
Each 16-bit instruction consists of the sign bit (not used), a threebit opcode, and twelve bits of data, as depicted below.

+--+--------+-------------------+

|16|15 13|12 1|

+--+--------+-------------------+

| | opcode | data |

+--+----------------------------+

The significance of the data depends upon the instruction.
In order to reconstruct a mask line, the application executing
these instructions is _required_ to keep track of two values, the
current high value and the current position in the output line.
The detailed operation of each instruction is given in Table 38.

Table 38: PLIO Instructions

Instr. Opcode Meaning

’ZN’ ’00’ Zero the next _N_ output pixels.
’HN’ ’04’ Set the next _N_ output pixels to the current
high value.
’PN’ ’05’ Zero the next _N_ − 1 output pixels, and set
Pixel _N_ to the current high value.
’SH’ ’05’ Set the high value (absolute rather than incremental), taking the high 15 bits from the
next word in the instruction stream, and the
low 12 bits from the current data value.
’IH,DH’ ’02,03’ Increment (’IH’) or decrement (’DH’) the
current high value by the data value. The
current position is not affected.
’IS,DS’ ’06,07’ Increment (’IS’) or decrement (’DS’) the
current high value by the data value, and
step, i.e., output one high value.

The high value _must_ be set to 1 at the beginning of a line,
hence the ’IH,DH’ and ’IS,DS’ instructions are not normally
needed for Boolean masks.

<a id="1044-h-compress-algorithm"></a>
#### 10.4.4. H-Compress algorithm

When ZCMPTYPE = ’HCOMPRESS 1’, the H-compress algorithm
_shall_ be used for data (de)compression. The algorithm was described by White (1992), and can be applied only to images with
two dimensions. Briefly, the compression method is to apply, in
order:

1. a wavelet transform called the H-transform (a Haar transform generalized to two dimensions), followed by

2. a quantization that discards noise in the image while retaining the signal on all scales, and finally

3. a quadtree coding of the quantized coefficients.

The H-transform is a two-dimensional generalization of the
Haar transform. The H-transform is calculated for an image of
size 2 _[N]_ × 2 _[N]_ as follows.

1. Divide the image up into blocks of 2 × 2 pixels. Call the four
pixel values in a block _a_ 00, _a_ 10, _a_ 01, and _a_ 11 .

2. For each block compute four coefficients:
_h_ 0 = ( _a_ 11 + _a_ 10 + _a_ 01 + _a_ 00 )/(SCALE ∗ σ)
_h_ _x_ = ( _a_ 11 + _a_ 10 − _a_ 01 − _a_ 00 )/(SCALE ∗ σ)
_h_ _y_ = ( _a_ 11 − _a_ 10 + _a_ 01 − _a_ 00 )/(SCALE ∗ σ)
_h_ _c_ = ( _a_ 11 − _a_ 10 − _a_ 01 + _a_ 00 )/(SCALE ∗ σ)
where SCALE is an algorithm parameter defined below, and
σ characterizes the RMS noise in the uncompressed image.
3. Construct a 2 _[N]_ [−][1] × 2 _[N]_ [−][1] image from the _h_ 0 values for each
2 × 2 block. Divide that image up into 2 × 2 blocks and repeat
the above calculation. Repeat this process _N_ times, reducing
the image in size by a factor of two at each step, until only
one _h_ 0 value remains.

This calculation can be easily inverted to recover the original image from its transform. The transform is exactly reversible using
integer arithmetic. Consequently, the program can be used for
either lossy or lossless compression, with no special approach
needed for the lossless case.
Noise in the original image is still present in the H-transform,
however. To compress noisy images, each coefficient can be divided by SCALE ∗ σ, where SCALE ∼ 1 is chosen according to
how much loss is acceptable. This reduces the noise in the transform to 0.5/SCALE, so that large portions of the transform are
zero (or nearly zero) and the transform is highly compressible.
There is one user-defined parameter associated with the HCompress algorithm: a scale factor to the RMS noise in the
image that determines the amount of compression that can be
achieved. It is not necessary to know what scale factor was used
when compressing the image in order to decompress it, but it
is still useful to record it. The keywords in Table 39 _should_ be
recorded in the header for this purpose.

Table 39: Keyword parameters for H-compression

Values
Keyword Permitted Default Meaning

ZNAME1 ’SCALE’ ’-’ Scale factor
ZVAL1 0.0 or larger 0.0 Scaling of the RMS noise; 0.0
yields lossless compression

_Scale Factor_ – The floating-point scale parameter (whose value
is stored in Keyword ZVAL1) determines the amount of compression; higher values result in higher compression, but
with greater loss of information. SCALE = 0.0 is a special
case that yields lossless compression, i.e. the decompressed
image has exactly the same pixel values as the original image. SCALE > 0.0 leads to lossy compression, where SCALE
determines how much of the noise is discarded.

51

<a id="appendix-a-syntax-of-keyword-records"></a>
## Appendix A. Syntax of keyword records

_This appendix is not part of the FITS Standard but is included_
_for convenient reference._

:= means ‘is defined to be’
X | Y means one of _X_ or _Y_
(no ordering relation is implied)

[X] means that _X_ is _optional_
X... means _X_ is repeated one or more times
‘B’ means the ASCII character B

‘A’–‘Z’ means one of the ASCII characters A
through Z in the ASCII collating
sequence, as shown in Appendix D
\0xnn means the ASCII character associated
with the hexadecimal code nn
{...} expresses a constraint or a comment
(it immediately follows the syntax rule)

The following statements define the formal syntax used in
_FITS_ free-format keyword records, as well as for long-string
keywords spanning more than one keyword record).

FITS keyword :=
single record keyword |
long string keyword

single record keyword :=
FITS keyword record

FITS keyword record :=
FITS commentary keyword record |
FITS value keyword record

FITS commentary keyword record :=
COMMENT keyword [ascii text char...] |
HISTORY keyword [ascii text char...] |
BLANKFIELD keyword [ascii text char...] |
keyword field anychar but equal

[ascii text char...] |
keyword field ‘=’ anychar but space

[ascii text char...]
{Constraint: The total number of characters in a
FITS commentary keyword record _must_ be exactly equal
to 80.}

FITS value keyword record :=
keyword field value indicator [space...] [value]

[space...] [comment]
{Constraint: The total number of characters in a
FITS value keyword record _must_ be exactly equal to 80.}
{Comment: If the value field is not present, the value of the _FITS_
keyword is not defined.}

long string keyword :=
initial kwd record [continuation kwd record...]
last continuation record
{Comment: the value of a long string keyword is reconstructed by concatenating the partial string values of the
initial kwd record and of any continuation kwd records in
the order they occur, and the character string value of the

52

last continuation record.}

initial kwd record :=
keyword field value indicator [space...]

[partial string value] [space...] [comment]
{Constraint: The total number of characters in an initial kwd record _must_ be exactly equal to 80.}

continuation kwd record :=
CONTINUE keyword [space...]

[partial string value] [space...] [comment]
{Constraint: The total number of characters in a continuation kwd record _must_ be exactly equal to 80.}

last continuation record :=
CONTINUE keyword [space...]

[character string value] [space...] [comment]
{Constraint: The total number of characters in a
last continuation record _must_ be exactly equal to 80.}

=
keyword field :

[keyword char...] [space...]
{Constraint: The total number of characters in the keyword field
_must_ be exactly equal to 8.}

keyword char :=
‘A’–‘Z’ | ‘0’–‘9’ | ‘ ’ | ‘-’

COMMENT keyword :=
‘C’ ‘O’ ‘M’ ‘M’ ‘E’ ‘N’ ‘T’ space

HISTORY keyword :=
‘H’ ‘I’ ‘S’ ‘T’ ‘O’ ‘R’ ‘Y’ space

BLANKFIELD keyword :=

space space space space space space space space

CONTINUE keyword :=
‘C’ ‘O’ ‘N’ ‘T’ ‘I’ ‘N’ ‘U’ ‘E’

value indicator :=
‘=’ space

space :=

‘ ’

␣

comment :=
‘/’ [ascii text char...]

ascii text char :=
space–‘˜’

anychar but equal :=
space–‘<’ | ‘>’–‘˜’

anychar but space :=
‘!’–‘˜’

value :=
character string value | logical value |
integer value | floating value |

complex integer value | complex floating value

character string value :=
begin quote [string text char...] end quote
{Constraint: The begin quote and end quote are not part of the
character-string value but only serve as delimiters. Leading
spaces are significant; trailing spaces are not.}

partial string value :=
begin quote [string text char...] ampersand end quote
{Constraint: The begin quote, end quote, and ampersand are not
part of the character-string value but only serve respectively as
delimiters or continuation indicator. }

begin quote :=
quote

end quote :=
quote
{Constraint: The ending quote _must not_ be immediately followed by a second quote.}

quote :=
\0x27

ampersand :=
‘&’

string text char :=
ascii text char
{Constraint: A string text char is identical to an ascii text char
except for the quote char; a quote char is represented by two
successive quote chars.}

logical value :=
‘T’ | ‘F’

integer value :=

[sign] digit [digit...]
{Comment: Such an integer value is interpreted as a signed
decimal number. It _may_ contain leading zeros.}

sign :=
‘-’ | ‘+’

digit :=
‘0’–‘9’

=
floating value :
decimal number [exponent]
decimal number :=

[sign] [integer part] [‘.’ [fraction part]]
{Constraint: At least one of the integer part and fraction part
_must_ be present.}

integer part :=
digit | [digit...]

fraction part :=
digit | [digit...]

exponent :=
exponent letter [sign] digit [digit...]

exponent letter :=
‘E’ | ‘D’

complex integer value :=
‘(’ [space...] real integer part [space...] ‘,’ [space...]
imaginary integer part [space...] ‘)’

real integer part :=
integer value

imaginary integer part :=
integer value

=
complex floating value :
‘(’ [space...] real floating part [space...] ‘,’ [space...]
imaginary floating part [space...] ‘)’

=
real floating part :
floating value

=
imaginary floating part :
floating value

<a id="appendix-b-suggested-time-scale-specification"></a>
## Appendix B. Suggested time-scale specification

_The content of this appendix has been superseded by Sect. 9 of_
_the formal Standard, which derives from Rots et al. (2015)._

53

<a id="appendix-c-summary-of-keywords"></a>
## Appendix C. Summary of keywords

_This appendix is not part of the FITS Standard, but is included for convenient reference._
All of the mandatory and reserved keywords that are defined in the Standard, except for the reserved WCS keywords that are
discussed separately in Sect. 8, are listed in Tables C.1, C.2, and C.3. An alphabetical list of these keywords and their definitions is
[available online: http://heasarc.gsfc.nasa.gov/docs/fcg/standard_dict.html.](http://heasarc.gsfc.nasa.gov/docs/fcg/standard_dict.html)

Table C.1: Mandatory _FITS_ keywords for the structures described in this document.

Primary Conforming Image ASCII-table Binary-table Compressed Compressed Random-groups
HDU extension extension extension extension images [6] tables [6] records

SIMPLE XTENSION XTENSION [1] XTENSION [2] XTENSION [3] ZIMAGE =T ZTABLE =T SIMPLE

BITPIX BITPIX BITPIX BITPIX = 8 BITPIX = 8 ZBITPIX ZNAXIS1 BITPIX

NAXIS NAXIS NAXIS NAXIS = 2 NAXIS = 2 ZNAXIS ZNAXIS2 NAXIS
NAXIS _n_ [4] NAXIS _n_ [4] NAXIS _n_ [4] NAXIS1 NAXIS1 ZNAXIS _n_ ZPCOUNT NAXIS1 = 0
END PCOUNT PCOUNT = 0 NAXIS2 NAXIS2 ZCMPTYPE ZFORM _n_ NAXIS _n_ [4]

GCOUNT GCOUNT = 1 PCOUNT = 0 PCOUNT ZCTYP _n_ GROUPS = T

END END GCOUNT = 1 GCOUNT = 1 ZTILELEN PCOUNT

TFIELDS TFIELDS GCOUNT
TFORM _n_ [5] TFORM _n_ [5] END
TBCOL _n_ [5] END

END

(1) XTENSION=␣'IMAGE␣␣␣' for the image extension. (2) XTENSION=␣'TABLE␣␣␣' for the ASCII-table extension. (3) XTENSION=␣'BINTABLE' for
the binary-table extension. [(4)] Runs from 1 through the value of NAXIS. [(5)] Runs from 1 through the value of TFIELDS. [(6)] Required in addition to the
mandatory keywords for binary tables.

Table C.2: Reserved _FITS_ keywords for the structures described in this document.

All [1] Array [2] ASCII-table Binary-table Compressed Compressed Random-groups
HDUs HDUs extension extension images tables records

DATE EXTNAME BSCALE TSCAL _n_ TSCAL _n_ ZTILE _n_ FZTILELN PTYPE _n_

DATE-OBS EXTVER BZERO TZERO _n_ TZERO _n_ ZNAME _i_ FZALGOR PSCAL _n_

ORIGIN EXTLEVEL BUNIT TNULL _n_ TNULL _n_ ZVAL _i_ FZALG _n_ PZERO _n_

AUTHOR EQUINOX BLANK TTYPE _n_ TTYPE _n_ ZMASKCMP
REFERENC EPOCH [3] DATAMAX TUNIT _n_ TUNIT _n_ ZQUANTIZ
COMMENT BLOCKED [3] DATAMIN TDISP _n_ TDISP _n_ ZDITHER0
HISTORY EXTEND [4] TDMAX _n_ TDIM _n_ ZSIMPLE ZTHEAP

␣␣␣␣␣␣␣␣ TELESCOP TDMIN _n_ THEAP ZEXTEND

OBJECT INSTRUME TLMAX _n_ TDMAX _n_ ZBLOCKED

OBSERVER TLMIN _n_ TDMIN _n_ ZTENSION

CONTINUE TLMAX _n_ ZPCOUNT
INHERIT [5] TLMIN _n_ ZGCOUNT

CHECKSUM ZHECKSUM ZHECKSUM

DATASUM ZDATASUM ZDATASUM

(1) These keywords are further categorized in Table C.3. (2) Primary HDU, IMAGE extension, user-defined HDUs with same array structure.
(3) Deprecated. (4) Only permitted in the primary HDU. (5) Only permitted in extension HDUs, immediately following the mandatory keywords.

54

Table C.3: General reserved _FITS_ keywords described in this document.

Production Bibliographic Commentary Observation

DATE AUTHOR COMMENT DATE-OBS

ORIGIN REFERENC HISTORY TELESCOP
BLOCKED [1] ␣␣␣␣␣␣␣␣ INSTRUME

OBSERVER

OBJECT

EQUINOX
EPOCH [1]

(1) Deprecated.

55

Table D.1: ASCII character set.

ASCII control ASCII text

dec hex char dec hex char dec hex char dec hex char

0 00 NUL 32 20 SP 64 40 @ 96 60 �

1 01 SOH 33 21 ! 65 41 A 97 61 a

2 02 STX 34 22 " 66 42 B 98 62 b

3 03 ETX 35 23 # 67 43 C 99 63 c

4 04 EOT 36 24 $ 68 44 D 100 64 d
5 05 ENQ 37 25 % 69 45 E 101 65 e
6 06 ACK 38 26 & 70 46 F 102 66 f
7 07 BEL 39 27 ' 71 47 G 103 67 g
8 08 BS 40 28 ( 72 48 H 104 68 h
9 09 HT 41 29 ) 73 49 I 105 69 i
10 0A LF 42 2A             - 74 4A J 106 6A j
11 0B VT 43 2B + 75 4B K 107 6B k

12 0C FF 44 2C, 76 4C L 108 6C l
13 0D CR 45 2D             - 77 4D M 109 6D m

14 0E SO 46 2E . 78 4E N 110 6E n

15 0F SI 47 2F / 79 4F O 111 6F o
16 10 DLE 48 30 0 80 50 P 112 70 p
17 11 DC1 49 31 1 81 51 Q 113 71 q
18 12 DC2 50 32 2 82 52 R 114 72 r

19 13 DC3 51 33 3 83 53 S 115 73 s

20 14 DC4 52 34 4 84 54 T 116 74 t

21 15 NAK 53 35 5 85 55 U 117 75 u

22 16 SYN 54 36 6 86 56 V 118 76 v

23 17 ETB 55 37 7 87 57 W 119 77 w

24 18 CAN 56 38 8 88 58 X 120 78 x
25 19 EM 57 39 9 89 59 Y 121 79 y
26 1A SUB 58 3A : 90 5A Z 122 7A z
27 1B ESC 59 3B ; 91 5B [ 123 7B {
28 1C FS 60 3C < 92 5C \ 124 7C |
29 1D GS 61 3D = 93 5D ] 125 7D }
30 1E RS 62 3E             - 94 5E ˆ 126 7E ˜
31 1F US 63 3F ? 95 5F _ 127 7F DEL [1]

1 Not ASCII Text

<a id="appendix-d-ascii-text"></a>
## Appendix D. ASCII text

_This appendix is not part of the FITS Standard_ ; the material in it is based on the ANSI standard for ASCII (ANSI 1977) and is
included here for informational purposes.)
In Table D.1, the first column is the decimal and the second column the hexadecimal value for the character in the third column.
The characters hexadecimal 20 to 7E (decimal 32 to 126) constitute the subset referred to in this document as the restricted set of
ASCII-text characters.

<a id="appendix-e-ieee-floating-point-formats"></a>
## Appendix E. IEEE floating-point formats

_The material in this appendix is not part of this Standard_ ; it is adapted from the IEEE-754 floating-point standard (IEEE 1985) and
provided for informational purposes. It is not intended to be a comprehensive description of the IEEE formats; readers should refer
to the IEEE standard.)
_FITS_ recognizes all IEEE basic formats, including the special values.

E.1. Basic formats

Numbers in the single and double formats are composed of the following three fields:

1. a one-bit sign _s_,
2. a biased exponent _e_ = _E_ + _bias_, and
3. a fraction _f_ = • _b_ 1 _b_ 2 - · · _b_ _p_ −1 .

The range of the unbiased exponent _E shall_ include every integer between two values _E_ _min_ and _E_ _max_, inclusive, and also two other
reserved values _E_ _min_ − 1 to encode ±0 and denormalized numbers, and _E_ _max_ +1 to encode ±∞ and NaNs. The foregoing parameters
are given in Table E.1. Each nonzero numerical value has just one encoding. The fields are interpreted as follows.

56

Table E.1: Summary of format parameters.

Format
Parameter Single Double
Single extended Double extended

_p_ 24 ≥ 32 53 ≥ 64
_E_ _max_ +127 ≥ +1023 +1023 ≥ +16383
_E_ _min_ −126 ≤−1022 −1022 ≤−16382
Exponent _bias_ +127 unspecified +1023 unspecified
Exponent width in bits 8 ≥ 11 11 ≥ 15
Format width in bits 32 ≥ 43 64 ≥ 79

Fig. E.1: Single Format. msb means _most-significant bit_, lsb means _least-significant bit_

1 8 23 ....widths

_s_ _e_ _f_

msb lsb msb lsb ....order

Fig. E.2: Double Format. msb means _most-significant bit_, lsb means _least-significant bit_

1 11 52 ....widths

_s_ _e_ _f_

msb lsb msb lsb ....order

E.1.1. Single

A 32-bit single-format number _X_ is divided as shown in Fig. E.1. The value _v_ of _X_ is inferred from its constituent fields.

1. If _e_ = 255 and _f_ � 0, then _v_ is NaN regardless of _s_ .
2. If _e_ = 255 and _f_ = 0, then _v_ = (−1) _[s]_ ∞.
3. If 0 < _e_ < 255, then _v_ = (−1) _[s]_ 2 _[e]_ [−][127] (1 • _f_ ).
4. If _e_ = 0 and _f_ � 0, then _v_ = (−1) _[s]_ 2 _[e]_ [−][126] (0 • _f_ ) (denormalized numbers).
5. If _e_ = 0 and _f_ = 0, then _v_ = (−1) _[s]_ 0 (zero).

E.1.2. Double

A 64-bit double-format number _X_ is divided as shown in Fig. E.2. The value _v_ of _X_ is inferred from its constituent fields.

1. If _e_ = 2047 and _f_ � 0, then _v_ is NaN regardless of _s_ .
2. If _e_ = 2047 and _f_ = 0, then _v_ = (−1) _[s]_ ∞.
3. If 0 < _e_ < 2047, then _v_ = (−1) _[s]_ 2 _[e]_ [−][1023] (1 • _f_ ).
4. If _e_ = 0 and _f_ � 0, then _v_ = (−1) _[s]_ 2 _[e]_ [−][1022] (0 • _f_ ) (denormalized numbers).
5. If _e_ = 0 and _f_ = 0, then _v_ = (−1) _[s]_ 0 (zero).

E.2. Byte patterns

Table E.2 shows the types of IEEE floating-point value, whether regular or special, corresponding to all double- and single-precision
hexadecimal byte patterns.

57

Table E.2: IEEE floating-point formats.

IEEE value Double precision Single precision

+0 0000000000000000 00000000

denormalized 0000000000000001 00000001

to to

000FFFFFFFFFFFFF 007FFFFF
positive underflow 0010000000000000 00800000
positive numbers 0010000000000001 00800001

to to

7FEFFFFFFFFFFFFE 7F7FFFFE
positive overflow 7FEFFFFFFFFFFFFF 7F7FFFFF
+∞ 7FF0000000000000 7F800000
NaN [1] 7FF0000000000001 7F800001

to to

7FFFFFFFFFFFFFFF 7FFFFFFF

−0 8000000000000000 80000000
negative 8000000000000001 80000001
denormalized to to

800FFFFFFFFFFFFF 807FFFFF
negative underflow 8010000000000000 80800000
negative numbers 8010000000000001 80800001

to to

FFEFFFFFFFFFFFFE FF7FFFFE
negative overflow FFEFFFFFFFFFFFFF FF7FFFFF
−∞ FFF0000000000000 FF800000
NaN [1] FFF0000000000001 FF800001

to to

FFFFFFFFFFFFFFFF FFFFFFFF

1 Certain values _may_ be designated as _quiet_ NaN (no diagnostic when used) or _signaling_ (produces diagnostic when used) by particular implementations.

58

<a id="appendix-f-reserved-extension-type-names"></a>
## Appendix F. Reserved extension type names

_This appendix is not part of the FITS Standard, but is in-_
_cluded for informational purposes_ . It describes the extension
type names registered as of the date this Standard was issued.) A
current list is available from the _FITS_ Support Office website at
[http://fits.gsfc.nasa.gov.](http://fits.gsfc.nasa.gov)

F.1. Standard extensions

These three extension types have been approved by the
IAUFWG and are defined in Sect. 7 of this Standard document
as well as in the indicated _Astronomy and Astrophysics_ journal
articles.

**–** 'IMAGE␣␣␣' – This extension type provides a means of storing a multi-dimensional array similar to that of the _FITS_ primary header and data unit. Approved as a standard extension
in 1994 (Ponz et al. 1994).
**–** 'TABLE␣␣␣' – This ASCII-table extension type contains
rows and columns of data entries expressed as ASCII characters. Approved as a standard extension in 1988 (Harten et al.
1988).
**–** 'BINTABLE' – This binary-table extension type provides a
more-flexible and efficient means of storing data structures
than is provided by the TABLE extension type. The table
rows can contain a mixture of numerical, logical, and character data entries. In addition, each entry is allowed to be
a single-dimensioned array. Numeric data are kept in binary formats. Approved as a standard extension in 1994
(Cotton et al. 1995).

F.2. Conforming extensions

These conventions meet the requirements for a conforming extension as defined in in Sect. 3.4.1 of this Standard, however they
have not been formally approved or endorsed by the IAUFWG.

**–** 'IUEIMAGE' – This name was given to the prototype of
the IMAGE extension type and was primarily used in the
IUE project data archive from approximately 1992 to 1994.
Except for the name, the format is identical to the IMAGE
extension.
**–** 'A3DTABLE' – This name was given to the prototype of
the BINTABLE extension type and was primarily used in
the AIPS data processing system developed at NRAO from
about 1987 until it was replaced by BINTABLE in the early
1990s. The format is defined in the ‘Going AIPS’ manual
(Cotton et al. 1990), Chapter 14. It is very similar to the
BINTABLE type except that it does not support the variablelength-array convention.
**–** 'FOREIGN␣' – This extension type is used to put a _FITS_
wrapper about an arbitrary file, allowing a file or tree of files
to be wrapped up in _FITS_ and later restored to disk. A full
description of this extension type is given in the Registry of
_FITS_ conventions on the _FITS_ Support Office website.
**–** 'DUMP␣␣␣␣' – This extension type can be used to store a
stream of binary-data values. The only known use of this extension type is to record telemetry header packets for data
from the Hinode mission. The more-general FOREIGN extension type could also be used to store this type of data.

F.3. Other suggested extension names

There have been occasional suggestions for other extension
names that might be used for other specific purposes. These include a COMPRESS extension for storing compressed images, a
_FITS_ extension for hierarchically embedding entire _FITS_ files
within other _FITS_ files, and a FILEMARK extension for representing the equivalent of an end-of-file mark on magnetic-tape
media. None of these extension types have been implemented or
used in practice, therefore these names are not reserved. These
extension names (or any other extension name not specifically
mentioned in the previous sections of this appendix) _should not_
be used in any _FITS_ file without first registering the name with
the IAU _FITS_ Working Group.

<a id="appendix-g-mime-types"></a>
## Appendix G. MIME types

_This appendix is not part of the FITS Standard, but is included_
_for informational purposes_ .
RFC 4047 (Allen & Wells 2005) describes the registration
of the Multipurpose Internet Mail Extensions (MIME) sub-types
‘application/fits’ and ‘image/fits’ to be used by the international astronomical community for the interchange of _FITS_
files. The MIME type serves as a electronic tag or label that is
transmitted along with the _FITS_ file that tells the receiving application what type of file is being transmitted. The remainder of
this appendix has been extracted verbatim from the RFC 4047
document.

The general nature of the full _FITS_ Standard requires the
use of the media type ‘application/fits’. Nevertheless, the
principal intent for a great many _FITS_ files is to convey a single data array in the primary HDU, and such arrays are very
often two-dimensional images. Several common image viewing applications already display single-HDU _FITS_ files, and the
prototypes for virtual-observatory projects specify that data provided by web services be conveyed by the data array in the primary HDU. These uses justify the registration of a second media type, namely ‘image/fits’, for files that use the subset of
the Standard described by the original _FITS_ Standard paper. The
MIME type ‘image/fits’ _may_ be used to describe _FITS_ primary HDUs that have other than two dimensions, however it is
expected that most files described as ‘image/fits’ will have
two-dimensional (NAXIS = 2) primary HDUs.

G.1. MIME type ‘ application/fits ’

A _FITS_ file described with the media type
‘application/fits’ _should_ conform to the published
standards for _FITS_ files as determined by convention and
agreement within the international _FITS_ community. No other
constraints are placed on the content of a file described as
‘application/fits’.
A _FITS_ file described with the media type
‘application/fits’ _may_ have an arbitrary number of
conforming extension HDUs that follow its mandatory primary
header and data unit. The extension HDUs _may_ be one of the
standard types (IMAGE, TABLE, and BINTABLE) or any other
type that satisfies the ‘Requirements for conforming extensions’
(Sect. 3.4.1). The primary HDU or any IMAGE extension _may_
contain zero to 999 dimensions with zero-or-more pixels along
each dimension.

59

The primary HDU _may_ use the random-groups convention,
in which the dimension of the first axis is zero and the keywords
GROUPS. PCOUNT and GCOUNT appear in the header. NAXIS1 = 0
and GROUPS = T is the signature of random groups; see Sect. 6.

G.1.1. Recommendations for application writers

An application intended to handle ‘application/fits’ _should_
be able to provide a user with a manifest of all of the HDUs that
are present in the file and with all of the keyword/value pairs
from each of the HDUs.

An application intended to handle ‘application/fits’
_should_ be prepared to encounter extension HDUs that contain
either ASCII or binary tables, and to provide a user with access
to their elements.
An application that can modify _FITS_ files or retrieve _FITS_
files from an external service _should_ be capable of writing such
files to a local storage medium.
Complete interpretation of the meaning and intended use of
the data in each of the HDUs typically requires the use of heuristics that attempt to ascertain which local conventions were used
by the author of the _FITS_ file.
As examples, files with media type ‘application/fits’
might contain any of the following contents.

**–**
An empty primary HDU (containing zero data elements) followed by a table HDU that contains a catalog of celestial
objects.
**–** An empty primary HDU followed by a TABLE HDU that encodes a series of time-tagged photon events from an exposure using an X-ray detector.
**–** An empty primary HDU followed by a series of IMAGE
HDUs containing data from an exposure taken by a mosaic
of CCD detectors.
**–** An empty primary HDU followed by a series of TABLE
HDUs that contain a snapshot of the state of a relational
database.

**–**
A primary HDU containing a single image along with keyword/value pairs of metadata.
**–** A primary HDU with NAXIS1 = 0 and GROUPS = T followed
by random-groups data records of complex fringe visibilities.

G.2. MIME type ‘ image/fits ’

A _FITS_ file described with the media type ‘image/fits’ _should_
have a primary HDU with positive integer values for the NAXIS
and NAXIS _n_ keywords, and hence _should_ contain at least one
pixel. Files with four or more non-degenerate axes (NAXIS _n_

- 1) _should_ be described as ‘application/fits’, not as
‘image/fits’. (In rare cases it may be appropriate to describe
a NULL image – a dataless container for _FITS_ keywords, with
NAXIS = 0 or NAXIS _n_ = 0 – or an image with four or more nondegenerate axes as ‘image/fits’ but this usage is discouraged
because such files may confuse simple image-viewer applications.)
_FITS_ files declared as ‘image/fits’ _may_ also have one
or more conforming extension HDUs following their primary
HDUs. These extension HDUs _may_ contain standard, non-linear,
world-coordinate system (WCS) information in the form of tables or images. The extension HDUs _may_ also contain other,

60

non-standard metadata pertaining to the image in the primary
HDU in the forms of keywords and tables.
A _FITS_ file described with the media type ‘image/fits’
_should_ be principally intended to communicate the single data
array in the primary HDU. This means that ‘image/fits’
_should not_ be applied to _FITS_ files containing multi-exposureframe mosaic images. Also, random-groups files _must_ be described as ‘application/fits’ and not as ‘image/fits’.
A _FITS_ file described with the media type ‘image/fits’
is also valid as a file of media type ‘application/fits’. The
choice of classification depends on the context and intended us
age.

G.2.1. Recommendations for application writers

An application that is intended to handle ‘image/fits’ _should_
be able to provide a user with a manifest of all of the HDUs that
are present in the file and with all of the keyword/value pairs
from each of the HDUs. An application writer _may_ choose to
ignore HDUs beyond the primary HDU, but even in this case
the application _should_ be able to present the user with the keyword/value pairs from the primary HDU.
Note that an application intended to render ‘image/fits’
for viewing by a user has significantly more responsibility
than an application intended to handle, e.g., ’image/tiff’ or
’image/gif’. _FITS_ data arrays contain elements that typically
represent the values of a physical quantity at some coordinate
location. Consequently they need not contain any pixel rendering information in the form of transfer functions, and there is
no mechanism for color look-up tables. An application _should_
provide this functionality, either statically using a more- or lesssophisticated algorithm, or interactively allowing a user various
degrees of choice.
Furthermore, the elements in a _FITS_ data array _may_ be integers or floating-point numbers. The dynamic range of the
data-array values may exceed that of the display medium and
the eye, and their distribution may be highly non-uniform.
Logarithmic, square-root, and quadratic transfer functions along
with histogram-equalization techniques have proved helpful
for rendering _FITS_ data arrays. Some elements of the array may have values that indicate that their data are undefined or invalid; these _should_ be rendered distinctly. Via WCS
Paper I (Greisen & Calabretta 2002) the Standard permits
CTYPE _n_ = ’COMPLEX’ to assert that a data array contains complex numbers (future revisions might admit other elements such
as quaternions or general tensors).
Three-dimensional data arrays (NAXIS = 3 with NAXIS1,
NAXIS2, and NAXIS3 all greater than 1) are of special interest. Applications intended to handle ‘image/fits’ _may_ default
to displaying the first two-dimensional plane of such an image
cube, or they _may_ default to presenting such an image in a fashion akin to that used for an animated GIF, or they _may_ present
the data cube as a mosaic of ‘thumbnail’ images. The time-lapse
movie-looping display technique can be effective in many instances, and application writers _should_ consider offering it for
all three-dimensional arrays.
An ‘image/fits’ primary HDU with NAXIS = 1 is describing a one-dimensional entity such as a spectrum or a time series.
Applications intended to handle ‘image/fits’ _may_ default to
displaying such an image as a graphical plot rather than as a
two-dimensional picture with a single row.

An application that cannot handle an image with dimensionality other than two _should_ gracefully indicate its limitations to
its users when it encounters NAXIS = 1 or NAXIS = 3 cases, while
still providing access to the keyword/value pairs.
_FITS_ files with degenerate axes (i.e., one or more
NAXIS _n_ = 1) _may_ be described as ‘image/fits’, but the first
axes _should_ be non-degenerate (i.e., the degenerate axes _should_
be the highest dimensions). An algorithm designed to render
only two-dimensional images will be capable of displaying such
an NAXIS = 3 or NAXIS = 4 _FITS_ array that has one or two of the
axes consisting of a single pixel, and an application writer _should_
consider coding this capability into the application. Writers of
new applications that generate _FITS_ files intended to be described as ‘image/fits’ _should_ consider using the WCSAXES
keyword (Greisen et al. 2006) to declare the dimensionality of
such degenerate axes, so that NAXIS can be used to convey the
number of non-degenerate axes.

G.3. File extensions

The _FITS_ Standard originated in the era when files were stored
and exchanged via magnetic tape; it does not prescribe any
nomenclature for files on disk. Various sites within the _FITS_
community have long-established practices where files are presumed to be _FITS_ by context. File extensions used at such sites
commonly indicate content of the file instead of the data format.
In the absence of other information it is reasonably safe to
presume that a file name ending in ‘.fits’ is intended to be
a _FITS_ file. Nevertheless, there are other commonly used extensions; e.g., ‘.fit’, ‘.fts’, and many others not suitable for
listing in a media type registration.

<a id="appendix-h-past-changes-or-clarifications-to-the-formal-definition-of-fits"></a>
## Appendix H. Past changes or clarifications to the formal definition of FITS

_This appendix is not part of the FITS Standard, but is included_
_for informational purposes_ .

H.1. Differences between the requirements in this Standard
and the requirements in the original FITS papers.

1. Sect. 4.1.2: The original _FITS_ definition paper (Wells et al.
1981) disallows lower-case letters in the keyword name, but
does not specify what other characters may or may not appear in the name.
2. Sect. 4.1.2: The slash between the value and comment is
‘recommended’ in the original paper (Wells et al. 1981)
whereas the Standard requires that it be present, which is
consistent with the prescription of Fortran list-directed input.
3. Sect. 4.2: The original paper (Wells et al. 1981) speculated
that _FITS_ would eventually support the full range of flexibility that is allowed by Fortran list-directed input, including dimensioned parameters. The Standard restricts the value field
to a single value, not an array.
4. Sect. 4.2.5 and Sect. 4.2.6: The original paper (Wells et al.
1981) defined a fixed format for complex keyword values,
with the real part right justified in Bytes 11 through 30 and
the imaginary part right justified in Bytes 31 through 50.
There are no known _FITS_ files that use this fixed format.
The Standard does not define a fixed format for complex
keyword values. Instead, complex values are represented in

conformance with the rules for Fortran list-directed input,
namely, with the real and imaginary parts separated by a
comma and enclosed in parentheses.
5. Sect. 4.4.1.1 and Sect. 4.4.1.2: The paper that defines generalized extensions (Grosbøl et al. 1988) does not prohibit
the appearance of the SIMPLE keyword in extensions nor the
XTENSION keyword in the primary header.

H.2. List of modification to the FITS Standard, Version 3.0

After the IAUFWG officially approved Version 3.0 of the _FITS_
Standard in 2008, the following additional corrections, clarifications, or format modifications have been made to the document.

1. Two typographical errors in Table 21 (previously Table 8.1)
were corrected. The last two lines of the third column should
read ‘LONPOLE _a_ (= PV i 3 _a_ )’ and ‘LATPOLE _a_ (= PV i 4 _a_ )’),
instead of PV i 1 _a_ and PV i 2 _a_, respectively. (October 2008)
2. The L [A] TEX text source document was reformatted to conform
to the Astronomy & Astrophysics journal page style (June
2010). The visible changes include the following.

**–**
The tables, figures, equations, and footnotes are numbered sequentially throughout the entire the document,
instead of sequentially within each chapter.

**–**
The citations use the standard ‘Author (year)’ format instead of being referenced by a sequential number. Also,
the ‘Bibliography’ section at the end of the document has
been replaced by a ‘References’ section in which the citations are listed alphabetically by author.
3. The following minor corrections or clarifications were made
during the refereeing process after submitting Version 3.0
of the _FITS_ Standard for publication in the Astronomy &
Astrophysics journal (July 2010).

**–** A sentence was added to the end of Sect. 1.2: ‘This

website also contains the contact information for the
Chairman of the IAUFWG, to whom any questions or
comments regarding this Standard should be addressed.’
**–** A ‘Section’ column was added to Table 1 to reference the

relevant section of the document.

**–**
The wording of the second sentence in Sect. 4.1.1 was
revised from ‘Except where specifically stated otherwise
in this standard, keywords may appear in any order.’ to
‘Keywords may appear in any order except where specifically stated otherwise in this Standard.’

**–**
A sentence was added to the end of the ‘Keyword name’
subsection in Sect. 4.1.2: ‘Note that keyword names that
begin with (or consist solely of) any combination of hyphens, underscores, and digits are legal.’
**–** A footnote to the description of the REFERENC keyword in Sect. 4.4.2 was added: ‘This bibliographic convention (Schmitz 1995) was initially developed for use
within NED (NASA/IPAC Extragalactic Database) and
SIMBAD (operated at CDS, Strasbourg, France).’
**–** In Sect. 7.3.4, the phrase ‘TFORM _n_ format code’ was corrected to read ‘TDISP _n_ format code’ (in four places).

**–**
The wording in the ‘Expressed as’ column in Table 26
for the LOG, GRI, GRA, and TAB spectral algorithm codes
was clarified.
**–** In Table C.2 the EXTNAME, EXTVER, and EXTLEVEL keywords were moved under the ‘All HDUs’ column because they are now allowed in the primary array header.

61

**–**
The last paragraph of Sect. 4.1.2.3 was corrected to state
that the ASCII-text characters have hexadecimal values
20 through 7E, not 41 through 7E.

H.3. List of modifications to the latest FITS Standard

1. The representation of time coordinates has been incorporated by reference from Rots et al. (2015) and is summarized in Sect. 9. Cross-references have been inserted in preexisting sections of the Standard (namely in Sects. 4.2.7,
4.3, 4.4.2.1, 4.4.2.2 and 5.4, as well as in various places of
Sect. 8, such as Sect. 8.3 and Sect. 8.4.1). New keywords are
listed in a rearranged Table 22. Contextually an erratum was
applied in Sect. 8.4.1: keywords OBSGEO-[XYZ] were incorrectly marked as OBSGEO-[XYZ] _a_ ; the TAI-UTC difference
in Table 30 was updated with respect to Rots et al. (2015)
taking into account the latest leap second; the possibility of
introducing more sources for the Solar System ephemerides
was re-worded (at the end of Sect.9.2.5 and in Table 31).
2. The continued string keywords described in Sect. 4.2.1.2
were originally introduced as a _FITS_ convention during
1994, and registered in 2007. The text of the original
[convention is reported at http://fits.gsfc.nasa.gov/](http://fits.gsfc.nasa.gov/registry/continue_keyword.html)
[registry/continue_keyword.html.](http://fits.gsfc.nasa.gov/registry/continue_keyword.html) The differences
with this Standard concern the following.

**–** In the convention, the LONGSTRN keyword was used
to signal the possible presence of long strings in the
HDU. The use of this keyword is no longer _required_ or
_recommended_ .
**–** Usage of the convention was _not recommended_ for reserved or mandatory keywords. Now it is _explicitly for-_
_bidden_ unless keywords are explicitly declared longstring.

**–**
To avoid ambiguities in the application of the previous
clause, the declaration of string keywords in Sects. 8,
9 and 10 has been reset from the generic ‘character’ to
‘string’.

**–**
It is also explicitly clarified there is no limit to the number of continuation records.

**–**
The description of continued comment field is new.

3. The blank header space convention described in Sect. 4.4.2.4
was used from 1996, and registered in 2014. The text of
[the original convention is reported at http://fits.gsfc.](http://fits.gsfc.nasa.gov/registry/headerspace.html)
[nasa.gov/registry/headerspace.html. It included a](http://fits.gsfc.nasa.gov/registry/headerspace.html)
_recommendation_ about using the convention in a controlled
environment, which does not appear in this Standard.
4. The INHERIT keyword described in Sect. 4.4.2.6 was
originally introduced as a _FITS_ convention in 1995, and
registered in 2007. The text of the original convention
[is reported at http://fits.gsfc.nasa.gov/registry/](http://fits.gsfc.nasa.gov/registry/inherit.html)
[inherit.html. See also references and practical consider-](http://fits.gsfc.nasa.gov/registry/inherit.html)
ations therein. The differences with the present document
concern a more-precise RFC 2119 compliant wording in a
couple of sentences in Appendix K.
5. The checksum keywords described in Sect. 4.4.2.7 were
originally introduced as a _FITS_ convention during 1994,
and registered in 2007. The text of the original convention
[is reported at http://fits.gsfc.nasa.gov/registry/](http://fits.gsfc.nasa.gov/registry/checksum.html)
[checksum.html. The differences with this Standard con-](http://fits.gsfc.nasa.gov/registry/checksum.html)

cern:

62

**–**
the omission of some additional implementation guidelines, and

**–**
the omission of a discussion on alternate algorithms and
relevant additional references.

6. The table keywords described in Sect. 7.2.2 and 7.3.2 were
originally introduced as a _FITS_ convention during 1993,
and registered in 2006. The text of the original convention
[is reported at http://fits.gsfc.nasa.gov/registry/](http://fits.gsfc.nasa.gov/registry/colminmax.html)
[colminmax.html. The differences with this Standard are as](http://fits.gsfc.nasa.gov/registry/colminmax.html)
follows.

**–**
The exclusion of undefined or IEEE special values when
computing maximum and minimum is now _mandatory_
while it was _optional_ .

**–**
The original text included the possibility of using the fact
TDMIN _n_ were greater than TDMAX _n_ (or TLMIN _n_ greater
than TLMAX _n_ ) as an indication the values were undefined.
This clause has been removed

**–**
The original text contained usage examples and additional minor explanatory details.

7. The Green Bank convention, mentioned in Sect. 8.2 and described in Appendix L, has been in use since 1989, and
was registered in 2010. The text of the registered convention
[is reported at http://fits.gsfc.nasa.gov/registry/](http://fits.gsfc.nasa.gov/registry/greenbank/greenbank.pdf)
[greenbank/greenbank.pdf and contains some additional](http://fits.gsfc.nasa.gov/registry/greenbank/greenbank.pdf)
details about the history of the convention.
8. The conventions for compressed data described in Sect. 10.
were originally introduced as a couple of _FITS_ conventions registered in 2007 and 2013. The text of the origi[nal conventions is reported at http://fits.gsfc.nasa.](http://fits.gsfc.nasa.gov/registry/tilecompression.html)
[gov/registry/tilecompression.html for compressed](http://fits.gsfc.nasa.gov/registry/tilecompression.html)
[images and at http://fits.gsfc.nasa.gov/registry/](http://fits.gsfc.nasa.gov/registry/tiletablecompression.html)
[tiletablecompression.html for compressed binary ta-](http://fits.gsfc.nasa.gov/registry/tiletablecompression.html)
bles. The differences with this Standard are listed below.

**–** In Sect. 10.3.3 the original text for FZALG _n_ mentioned
the possibility that, ‘If the column cannot be compressed
with the requested algorithm (e.g., if it has an inappropriate data type), then a default compression algorithm will
be used instead.’ But there is no default algorithm. This
is irrelevant for the Standard.
**–** In Sect. 10.4 the alias ’RICE ONE’ is _not_ adopted in the
Standard as a synonym for ’RICE 1’.

**–**
In Sect. 10.4.3 a sentence was left out about requiring
additional instructions in PLIO to make it work for more
then 2 [12] bits, since we aren’t allowing this possibility in
the Standard.

**–**
In Sect. 10.4.4 the reference to a ‘smoothing flag’ was
dropped.
**–** Also in Sect. 10.4.4 the _scale factor_ is now floating point,
while it was originally integer.
**–** In Table 36 (and Sect. 10.3.5) the ’NOCOMPRESS’ algorithm is explicitly mentioned.

H.4. List of modifications for language editing

1. Apply systematically L [A] TEX macros for keyword names and
values, and for RFC 2119 expressions, according to instructions reported in the L [A] TEX source preamble (for future editors of the Standard).
2. The acronym _FITS_ is always indicated in italics.
3. Use italics systematically for RFC 2119 obligations and recommendations.

4. Apply consistent use of italic and typewriter fonts, and ’
quotation marks around literal keyword values. Correct other
minor L [A] TEX issues.
5. Apply systematic capitalization of the names of specific entities, where appropriate. These include Standard (when referring to the _FITS_ Standard document), Version (where numbered), Byte, Column, Parameter, Field, and Axis. Start some
words with a lower-case letter that previously began with a
capital letter.
6. Address other typographical issues, such as the insertion of
commas in several places, adding a few non-breaking spaces,
and better handling of references to sections, etc.
7. Several cases of minor rewording.
8. Express small numbers in letter form (one to nine), not in numerals (1 to 9), wherever sensible. However, there is the customary exception for normalization in sentences and headings that also contain numbers greater than nine.
9. Compound nouns are systematically hyphenated to highlight the correct grouping (and hence meaning) of the components. This includes the attributive references to ASCIItable, binary-table, and random-groups.
10. Improve the aesthetics of some tables.

<a id="appendix-i-random-number-generator"></a>
## Appendix I. Random-number generator

_This appendix is not part of the FITS Standard, but is included_
_for informational purposes_ .
The portable random-number generator algorithm below is
from Park & Miller (1988). This algorithm repeatedly evaluates
the function
_seed_ = ( _a_ ∗ _seed_ ) mod _m_

where the values of _a_ and _m_ are shown below, but it is implemented in a way to avoid integer overflow problems.

int random_generator(void) {

/* initialize an array of random numbers */

int ii;
double a = 16807.0;
double m = 2147483647.0;
double temp, seed;
float rand_value[10000];

/* initialize the random numbers */
seed = 1;
for (ii = 0; ii < N_RANDOM; ii++) {
temp = a * seed;
seed = temp -m * ((int) (temp / m) );
/* divide by m for value between 0 and 1 */
rand_value[ii] = seed / m;
}
}

If implemented correctly, the 10 000 [th] value of seed will
equal 1 043 618 065.

<a id="appendix-j-checksum-implementation-guidelines"></a>
## Appendix J. CHECKSUM implementation guidelines

_This appendix is not part of the FITS Standard, but is included_
_for informational purposes_ .

J.1. Recommended CHECKSUM keyword implementation

The _recommended_ CHECKSUM keyword algorithm described here
generates a 16-character ASCII string that forces the 32-bit ones’
complement checksum accumulated over the entire _FITS_ HDU
to equal negative 0 (all 32 bits equal to 1). In addition, this string
will only contain alphanumeric characters within the ranges 0–9,
A–Z, and a–z to promote human readability and transcription.
If the present algorithm is used, the CHECKSUM keyword value
_must_ be expressed in fixed format, with the starting single-quote
character in Column 11 and the ending single-quote character
in Column 28 of the _FITS_ keyword record, because the relative
placement of the value string within the keyword record affects
the computed HDU checksum. The steps in the algorithm are as
follows.

1. Write the CHECKSUM keyword into the HDU header
with an initial value consisting of 16 ASCII zeros
(’0000000000000000’) where the first single-quote character is in Column 11 of the _FITS_ keyword record. This
specific initialization string is _required_ by the encoding algorithm described in Sect. J.2. The final comment field of
the keyword, if any, _must_ also be written at this time. It
is _recommended_ that the current date and time be recorded
in the comment field to document when the checksum was
computed.
2. Accumulate the 32-bit ones’ complement checksum over the
_FITS_ logical records that make up the HDU header in the
same manner as was done for the data records by interpreting each 2880-byte logical record as 720 32-bit unsigned integers.
3. Calculate the checksum for the entire HDU by adding (using ones’ complement arithmetic) the checksum accumulated over the header records to the checksum accumulated
over the data records (i.e., the previously computed DATASUM
keyword value).
4. Compute the bit-wise complement of the 32-bit total HDU
checksum value by replacing all 0 bits with 1 and all 1 bits
with 0.
5. Encode the complement of the HDU checksum into a 16character ASCII string using the algorithm described in
Sect. J.2
6. Replace the initial CHECKSUM keyword value with this 16character encoded string. The checksum for the entire HDU
will now be equal to negative 0.

J.2. Recommended ASCII encoding algorithm

The algorithm described here is used to generate an ASCII
string, which, when substituted for the value of the CHECKSUM
keyword, will force the checksum for the entire HDU to equal
negative 0. It is based on a fundamental property of ones’ complement arithmetic that the sum of an integer and the negation
of that integer (i.e, the bitwise complement formed by replacing
all 0 bits with 1s and all 1 bits with 0s) will equal negative 0
(all bits set to 1). This principle is applied here by constructing a
16-character string, which, when interpreted as a byte stream of
four 32-bit integers, has a sum that is equal to the complement of
the sum accumulated over the rest of the HDU. This algorithm
also ensures that the 16 bytes that make up the four integers all
have values that correspond to ASCII alpha-numeric characters
in the range 0–9, A–Z, and a–z.

63

Fig. J.1: Only ASCII alpha-numeric characters are used to encode the checksum – punctuation is excluded

**2** ~~2222222222222222222222222222222222222222222222222222222222222222222222~~ 222222222222222222222222222222222222222222222222222222222222222222222

0 _30_ 1 _31_ 2 _32_ 3 _33_ 4 _34_ 5 _35_ 6 _36_ 7 _37_ 8 _38_ 9 _39_

~~:~~ _3a_ ~~;~~ _3b_ ~~<~~ _3c_ ~~=~~ _3d_ ~~>~~ _3e_ ~~?~~ _3f_ ~~@~~ _40_ A _41_ B _42_ C _43_

D _44_ E _45_ F _46_ G _47_ H _48_ I _49_ J _4a_ K _4b_ L _4c_ M _4d_

N _4e_ O _4f_ P _50_ Q _51_ R _52_ S _53_ T _54_ U _55_ V _56_ W _57_

X _58_ Y _59_ Z _5a_ ~~[~~ ~~_5b_~~ ~~\~~ ~~_5c_~~ ~~]~~ ~~_5d_~~ ~~^~~ ~~_5e_~~ ~~_~~ ~~_5f_~~ ~~‘~~ ~~_60_~~ a _61_

b _62_ c _63_ d _64_ e _65_ f _66_ g _67_ h _68_ i _69_ j _6a_ k _6b_

l _6c_ m _6d_ n _6e_ o _6f_ p _70_ q _71_ r _72_

~~**2**~~ 2222222222222222222222222222222222222222222222222222222222222222222222 Figure 1. Only ASCII alpha-numerics are used to encode the checksum — punctuation is excluded. ~~222222222222222222222222222222222222222222222222222222222222222222222~~

1. Begin with the ones’ complement (replace 0s with 1s and
1s with 0s) of the 32-bit checksum accumulated over all the
_FITS_ records in the HDU after first initializing the CHECKSUM
keyword with a fixed-format string consisting of 16 ASCII
zeros (’0000000000000000’).
2. Interpret this complemented 32-bit value as a sequence of
four unsigned eight-bit integers, _A_, _B_, _C_, and _D_, where _A_ is
the most-significant byte and _D_ is the least-significant byte.
Generate a sequence of four integers, _A_ 1, _A_ 2, _A_ 3, _A_ 4, that
are all equal to _A_ divided by 4 (truncated to an integer if necessary). If _A_ is not evenly divisible by 4, add the remainder
to _A_ 1. The key property to note here is that the sum of the
four new integers is equal to the original byte value (e.g.,
_A_ = _A_ 1 + _A_ 2 + _A_ 3 + _A_ 4). Perform a similar operation on _B_,
_C_, and _D_, resulting in a total of 16 integer values, four from
each of the original bytes, which _should_ be rearranged in the
following order:

A1 B1 C1 D1 A2 B2 C2 D2 A3 B3 C3 D3 A4 B4 C4 D4.

Each of these integers represents one of the 16 characters
in the final CHECKSUM keyword value. Note that if this byte
stream is interpreted as four 32-bit integers, the sum of the
integers is equal to the original complemented checksum
value.
3. Add 48 (hex 30), which is the value of an ASCII zero character, to each of the 16 integers generated in the previous
step. This places the values in the range of ASCII alphanumeric characters ’0’ (ASCII zero) to ’r’. This offset is effectively subtracted back out of the checksum when the initial
CHECKSUM keyword value string of 16 ASCII 0s is replaced
with the final encoded checksum value.
4. To improve human readability and transcription of the string,
eliminate any non-alphanumeric characters by considering
the bytes a pair at a time (e.g., _A_ 1 + _A_ 2, _A_ 3 + _A_ 4, _B_ 1 + _B_ 2,
etc.) and repeatedly increment the first byte in the pair by 1
and decrement the second byte by 1 as necessary until they
both correspond to the ASCII value of the allowed alphanumeric characters 0–9, A–Z, and a–z shown in Figure J.1.
Note that this operation conserves the value of the sum of
the four equivalent 32-bit integers, which is required for use
in this checksum application.
5. Cyclically shift all 16 characters in the string one place to the
right, rotating the last character ( _D_ 4) to the beginning of the
string. This rotation compensates for the fact that the fixed
format _FITS_ character-string values are not aligned on fourbyte word boundaries in the _FITS_ file. (The first character
of the string starts in Column 12 of the header card image,
rather than Column 13).

64

6. Write this string of 16 characters to the value of the
CHECKSUM keyword, replacing the initial string of 16 ASCII

zeros.

To invert the ASCII encoding, cyclically shift the 16 characters in the encoded string one place to the left, subtract the
hex 30 offset from each character, and calculate the checksum
by interpreting the string as four 32-bit unsigned integers. This
can be used, for instance, to read the value of CHECKSUM into the
software when verifying or updating a HDU.

J.3. Encoding example

This example illustrates the encoding algorithm given in
Sect. J.2 Consider a _FITS_ HDU whose ones’ complement checksum is 868229149, which is equivalent to hex 33C0201D. This
number was obtained by accumulating the 32-bit checksum
over the header and data records using ones’ complement arithmetic after first initializing the CHECKSUM keyword value to
’0000000000000000’. The complement of the accumulated
checksum is 3426738146, which is equivalent to hex CC3FDFE2.
The steps needed to encode this hex value into ASCII are shown
schematically below.

Byte Preserve byte alignment

A B C D A1 B1 C1 D1 A2 B2 C2 D2 A3 B3 C3 D3 A4 B4 C4 D4

CC 3F DF E2 -> 33 0F 37 38 33 0F 37 38 33 0F 37 38 33 0F 37 38

+ remainder 0 3 3 2

= hex 33 12 3A 3A 33 0F 37 38 33 0F 37 38 33 0F 37 38

+ 0 offset 30 30 30 30 30 30 30 30 30 30 30 30 30 30 30 30

= hex 63 42 6A 6A 63 3F 67 68 63 3F 67 68 63 3F 67 68

ASCII c B j j c ? g h c ? g h c ? g h

Eliminate punctuation characters
initial values c B j j c ? g h c ? g h c ? g h
. c C j j c   - g h c @ g h c   - g h
. c D j j c = g h c A g h c = g h
. c E j j c < g h c B g h c < g h
. c F j j c ; g h c C g h c ; g h
. c G j j c : g h c D g h c : g h
final values c H j j c 9 g h c E g h c 9 g h

final string "hcHjjc9ghcEghc9g" (rotate 1 place to the right)

In this example Byte _B_ 1 (originally ASCII B) is shifted
higher (to ASCII H) to balance Byte _B_ 2 (originally ASCII ?)
being shifted lower (to ASCII 9). Similarly, Bytes _B_ 3 and _B_ 4
are shifted by opposing amounts. This is possible because the
two sequences of ASCII punctuation characters that can occur
in encoded checksums are both preceded and followed by longer
sequences of ASCII alphanumeric characters. This operation is
purely for cosmetic reasons to improve readability of the final
string.
This is how these CHECKSUM and DATASUM keywords would
appear in a _FITS_ header (with the recommended time stamp in
the comment field).

DATASUM = '2503531142' / 2015-06-28T18:30:45
CHECKSUM= 'hcHjjc9ghcEghc9g' / 2015-06-28T18:30:45

J.4. Incremental updating of the checksum

The symmetry of ones’ complement arithmetic also means that
after modifying a _FITS_ HDU, the checksum _may_ be incrementally updated using simple arithmetic without accumulating the
checksum for portions of the HDU that have not changed. The

new checksum is equal to the old total checksum plus the checksum accumulated over the modified records, minus the original
checksum for the modified records.
An incremental update provides the mechanism for end-toend checksum verification through any number of intermediate
processing steps. By calculating rather than accumulating the intermediate checksums, the original checksum test is propagated
through to the final data file. On the other hand, if a new checksum is accumulated with each change to the HDU, no information is preserved about the HDU’s original state.
The recipe for updating the CHECKSUM keyword following
some change to the HDU is: _C_ [′] = _C_ − _m_ + _m_ [′], where _C_
and _C_ [′] represent the HDU’s checksum (that is, the complement
of the CHECKSUM keyword) before and after the modification
and _m_ and _m_ [′] are the corresponding checksums for the modified _FITS_ records or keywords only. Since the CHECKSUM keyword contains the complement of the checksum, the correspondingly complemented form of the recipe is more directly useful:
˜ _C_ [′] = ˜( _C_ + ˜ _m_ + _m_ [′] ), where ˜ (tilde) denotes the (ones’) complement operation. See Braden et al. (1988); Mallory & Kullberg
(1990); Rijsinghani (1994). Note that the tilde on the right hand
side of the equation cannot be distributed over the contents of the
parentheses due to the dual nature of zero in ones’ complement
arithmetic (Rijsinghani 1994).

J.5. Example C code for accumulating the checksum

The ones’ complement checksum is simple and fast to compute. This routine assumes that the input records are a multiple of four bytes long (as is the case for _FITS_ logical records ),
but it is not difficult to allow for odd length records if necessary. To use this routine, first initialize the CHECKSUM keyword
to ’0000000000000000’ and initialize sum32 = 0, then step
through all the _FITS_ logical records in the _FITS_ HDU.

void checksum (
unsigned char *buf, /* Input array of bytes to be checksummed */
/* (interpret as 4-byte unsigned ints) */
int length, /* Length of buf array, in bytes */
/* (must be multiple of 4) */
unsigned int *sum32) /* 32-bit checksum */
{
/*
Increment the input value of sum32 with the 1's complement sum
accumulated over the input buf array.
*/
unsigned int hi, lo, hicarry, locarry, i;

/* Accumulate the sum of the high-order 16 bits and the */
/* low-order 16 bits of each 32-bit word, separately. */
/* The first byte in each pair is the most significant. */
/* This algorithm works on both big and little endian machines.*/
hi = (*sum32 >> 16);
lo = *sum32 & 0xFFFF;
for (i=0; i < length; i+=4) {
hi += ((buf[i] << 8) + buf[i+1]);
lo += ((buf[i+2] << 8) + buf[i+3]);
}

/* fold carry bits from each 16 bit sum into the other sum */
hicarry = hi >> 16;
locarry = lo >> 16;
while (hicarry || locarry) {
hi = (hi & 0xFFFF) + locarry;
lo = (lo & 0xFFFF) + hicarry;
hicarry = hi >> 16;
locarry = lo >> 16;
}

/* concatenate the full 32-bit value from the 2 halves */
*sum32 = (hi << 16) + lo;
}

J.6. Example C code for ASCII encoding

This routine encodes the complement of the 32-bit HDU checksum value into a 16-character string. The byte alignment of the
string is permuted one place to the right for _FITS_ to left justify
the string value starting in Column 12.

unsigned int exclude[13] = {0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x40,
0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60 };

int offset = 0x30; /* ASCII 0 (zero) */
unsigned long mask[4] = { 0xff000000, 0xff0000, 0xff00, 0xff };

void char_encode (
unsigned int value, /* 1's complement of the checksum */
/* value to be encoded */
char *ascii) /* Output 16-character encoded string */
{
int byte, quotient, remainder, ch[4], check, i, j, k;
char asc[32];

for (i=0; i < 4; i++) {
/* each byte becomes four */
byte = (value & mask[i]) >> ((3 - i) * 8);
quotient = byte / 4 + offset;
remainder = byte % 4;
for (j=0; j < 4; j++)
ch[j] = quotient;

ch[0] += remainder;

for (check=1; check;) /* avoid ASCII punctuation */
for (check=0, k=0; k < 13; k++)
for (j=0; j < 4; j+=2)
if (ch[j]==exclude[k] || ch[j+1]==exclude[k]) {
ch[j]++;
ch[j+1]--;
check++;
}

for (j=0; j < 4; j++) /* assign the bytes */
asc[4*j+i] = ch[j];
}

for (i=0; i < 16; i++) /* permute the bytes for FITS */
ascii[i] = asc[(i+15)%16];

ascii[16] = 0; /* terminate the string */
}

<a id="appendix-k-header-inheritance-convention"></a>
## Appendix K. Header inheritance convention

_This appendix is not part of the FITS Standard, but is included_
_for informational purposes_ .
The reserved Boolean INHERIT keyword described in
Sect. 4.4.2.6 is _optional_, but if present it _shall_ appear in the
extension header immediately after the mandatory keywords.
The INHERIT keyword _must not_ appear in the primary header.
Keyword inheritance provides a mechanism to store keywords in
the primary HDU, and have them be shared by one or more extensions in the file. This mechanism minimizes duplication (and
maintenance) of metadata in multi-extension _FITS_ files.
It _should_ only be used in _FITS_ files that have a null primary
array (e.g., with NAXIS = 0). to avoid possible confusion if arrayspecific keywords (e.g., BSCALE and BZERO) were to be inherited.
When an application reads an extension header with
INHERIT = T, it _should_ merge the keywords in the current extension with the primary header keywords. The exact merging
mechanism is left up to the application. The mandatory primary
array keywords (e.g., BITPIX, NAXIS, and NAXIS _n_ ) and any
COMMENT, HISTORY, and blank keywords in the primary header
_must not_ be inherited. It is assumed also that the table-specific
keywords described in Sect. 7.2 and 7.3, and the table-specific

65

WCS keywords described in Sect. 8, cannot be inherited since
they will never appear in the primary header. If the same keyword is present in both the primary header and the extension
header, the value in the extension header _shall_ take precedence.
If INHERIT = F in an extension header, the keywords from the
primary header _should not_ be inherited.
An application that merely reads a _FITS_ file is authorized by
INHERIT = T to look in the primary HDU for an expected keyword not found in the current HDU. However if the application
writes out a modified file, it has to be very careful to avoid unwanted duplication of keywords, and preserve the separation of
primary and extension headers. If an application modifies the
value of an inherited keyword while processing an extension
HDU, then it is _recommended_ to write the modified value of
that keyword into the extension header, leaving the value of the
keyword in the primary header unchanged. The primary array
keywords _should_ only be modified when the intent is to explicitly change the value that will subsequently be inherited in the
extensions.
Also if the _FITS_ file is read in sequentially (e.g., from tape or
Internet downloads), the reader would need to cache the primary
header in case it turns out that a later extension in the file uses
the INHERIT keyword.

<a id="appendix-l-green-bank-convention"></a>
## Appendix L. Green Bank convention

_This appendix is not part of the FITS Standard, but is included_
_for informational purposes_ .
The Green Bank convention was developed at a meeting in
October 1989 at the US National Radio Astronomy Observatory
in Green Bank, West Virginia, to discuss the use of _FITS_ for
single-dish radio-astronomy data, and has since been widely
used in conjunction with the SDFITS convention [16] . It was devised primarily to record WCS keywords independently for each
row of a table containing an image array column, but subsequently it has found more-general application.
The basic idea is that of expanding header keywords into
binary-table columns, and vice versa, of collapsing unvarying
binary-table columns into header keywords.
For example, the standard header keyword DATE-OBS, which
records the date and time of observation, could be expanded into
a column with TTYPE _n_ = ’DATE-OBS’ to record the date and
time independently for each row of a binary table. Conversely,
a binary-table column with TTYPE _n_ = ’HUMIDITY’ containing
the same value in each row, could be collapsed into a keyword,
HUMIDITY, that recorded the constant value.
When the Green Bank convention is used (and arguably otherwise), a keyword _should not_ coexist with a column of the same
name within a single binary table. Should this situation occur,
the column value takes precedence over the keyword.
When expanding keywords into columns, the Green Bank
convention applies to all _FITS_ keywords that may appear in a binary table except for the following, most of
which describe the structure or identity of a binary table
HDU: XTENSION, BITPIX, NAXIS, NAXIS _n_, PCOUNT, GCOUNT,
TFIELDS, EXTNAME, EXTVER, EXTLEVEL, TTYPE _n_, TFORM _n_,
TUNIT _n_, TSCAL _n_, TZERO _n_, TNULL _n_, TDISP _n_, THEAP, TDIM _n_,
DATE, ORIGIN, COMMENT, HISTORY, CONTINUE, and END.
In order to collapse a column into a keyword, the name of the
column (given by TTYPE _n_ ) _must_ be a valid keyword name, and

16 [http://fits.gsfc.nasa.gov/registry/sdfits.html](http://fits.gsfc.nasa.gov/registry/sdfits.html)

66

the column’s constant value _must_ be amenable to representation
as a valid keyvalue.
Software that implements the Green Bank convention _must_
take into account the possibility that any “keyword” (apart from
those on the proscribed list), such as DATE-OBS, may change
value from row to row of a table. Moreover, when searching the
header for a particular keyword, it must first consider the values of the TTYPE _n_ keywords in case the desired keyword has
been expanded as a column. Likewise, it must consider each
header keyword potentially as a collapsed column, so that a
request for the value in a particular row or rows of the nonexistent HUMIDITY column would be satisfied by the value of
the HUMIDITY keyword.

<a id="references"></a>
## References

**Note:** Many of these _FITS_ references are available electronically from the
NASA Astrophysics Data System (ADS) and/or the _FITS_ Support Office
websites at

[http://adswww.harvard.edu and](http://adswww.harvard.edu)
[http://fits.gsfc.nasa.gov/fits_documentation.html.](http://fits.gsfc.nasa.gov/fits_documentation.html)

Allen, S. & Wells, D. 2005, IETF RFC 4047,
[http://www.ietf.org/rfc/rfc4047.txt](http://www.ietf.org/rfc/rfc4047.txt)
ANSI 1977, _American National Standard for Information Processing: Code for_
_Information Interchange_, ANSI X3.4–1977 (ISO 646) New York: American
National Standards Institute, Inc.
Braden, R. T., Borman, D.A., and Partridge, C. 1988 ACM Computer
Communication Review, 19, no. 2, 86, IETF RFC 1071,
[https://tools.ietf.org/html/rfc1071](https://tools.ietf.org/html/rfc1071)
[Bradner, S. 1997, IETF RFC 2119, http://www.ietf.org/rfc/rfc2119.](http://www.ietf.org/rfc/rfc2119.txt)

[txt](http://www.ietf.org/rfc/rfc2119.txt)
Bunclark, P. & Rots, A. 1997, _Precise re-definition of_ DATE-OBS _Keyword en-_
_compassing the millennium_,
[http://fits.gsfc.nasa.gov/year2000.html](http://fits.gsfc.nasa.gov/year2000.html)
Calabretta, M. R. & Greisen, E. W. 2002, A&A, 395, 1077
Calabretta, M. R. & Roukema, B. F. 2007, MNRAS, 381, 865
Cotton, W. D., Tody, D. B., & Pence, W. D. 1995, A&AS, 113, 159
Cotton, W. D., et al. 1990, _Going AIPS: A Programmer’s Guide to the NRAO_
_Astronomical Image Processing System_, Charlottesville: NRAO
Deutsch P. 1996, RFC 1951, Network Working Group; available online:
[http://tools.ietf.org/html/rfc1951](http://tools.ietf.org/html/rfc1951)
Folkner, W. M., Williams, J. G., & Boggs, D. H. 2009, Interplanetary Network
[Progress Report 42-178, available online: http://tmo.jpl.nasa.gov/](http://tmo.jpl.nasa.gov/progress_report/42-178/178C.pdf)
[progress_report/42-178/178C.pdf](http://tmo.jpl.nasa.gov/progress_report/42-178/178C.pdf)
Folkner, W. M. et al. 2014, Interplanetary Network Progress Report 42[196, available online: http://ipnpr.jpl.nasa.gov/progress_report/](http://ipnpr.jpl.nasa.gov/progress_report/42-196/196C.pdf)
[42-196/196C.pdf](http://ipnpr.jpl.nasa.gov/progress_report/42-196/196C.pdf)
Greisen, E. W. & Calabretta, M. R. 2002, A&A, 395, 1061
Greisen, E. W., Calabretta, M. R., Valdes, F. G., & Allen, S. L. 2006, A&A, 446,
747

Greisen, E. W. & Harten, R. H. 1981, A&AS, 44, 371
Grosbøl, P., Harten, R. H., Greisen, E. W., & Wells, D. C. 1988, A&AS, 73, 359
Grosbøl, P. & Wells, D. C. 1994, _Blocking of Fixed-block Sequential Media and_
_Bitstream Devices_ [, http://fits.gsfc.nasa.gov/blocking94.html](http://fits.gsfc.nasa.gov/blocking94.html)
Hanisch, R., et al. 2001, A&A, 376, 359
Harten, R. H., Grosbøl, P., Greisen, E. W., & Wells, D. C. 1988, A&AS, 73, 365
IAU 1983, _Transactions of the IAU_, XVIIIB, 45
IAU 1988, _Transactions of the IAU_, XXB, 51
IAU 1997, Resolution B1 of the XXIIIrd General Assembly – Transactions of the
IAU Vol. XXIII B, Ed. J. Andersen, (Dordrecht: Kluwer). Available online:
http://www.iau.org/static/resolutions/IAU1997 French.pdf
IEEE 1985, _American National Standard – IEEE Standard for Binary Floating_
_Point Arithmetic_, ANSI/IEEE 754–1985, New York: American National
Standards Institute, Inc.
Irwin, A. W. & Fukushima, T. A. 1999, A&A348, 642
ISO 2004, _Information technology – Programming languages – Fortran_, ISO/
IEC 1539-1:2004, Geneva: International Organization for Standardization
ISO 2004b, International Standard ISO 8601:2004(E), _Data elements and in-_
_terchange formats – Information interchange – Representation of dates and_
_times_

[NASA/JPL Planetary Ephemerides 2014a, available online: http://ssd.jpl.](http://ssd.jpl.nasa.gov/?ephemerides)
[nasa.gov/?ephemerides](http://ssd.jpl.nasa.gov/?ephemerides)
[NASA/JPL Solar and Planetary Ephemerides 2014b, available online: http://](http://ssd.jpl.nasa.gov/?planet_eph_export)
[ssd.jpl.nasa.gov/?planet_eph_export](http://ssd.jpl.nasa.gov/?planet_eph_export)
Mallory, T. & Kullberg, A. 1990, IETF RFC 1141,
[https://tools.ietf.org/html/rfc1141](https://tools.ietf.org/html/rfc1141)
McNally, D., ed. 1988, _Transactions of the IAU, Proceedings of the Twentieth_
_General Assembly_ (Dordrecht: Kluwer)
Park, X. & Miller, X. 1988, Comm. ACM, 31, Issue 10, 1192; available online:
[http://dl.acm.org/citation.cfm?id=63042](http://dl.acm.org/citation.cfm?id=63042)
Pence, W. D., Seaman, R., & White, R. L. 2009, PASP, 121, 414
Pence, W. D., Chiappetti, L., Page, C. G., Shaw, R. A., & Stobie, E. 2010, A&A,
524, A42
Pence, W. D., Seaman, R., & White, R. L. 2013, _Tiled Table Convention for_
_Compressing FITS Binary Tables_, _FITS_ Support Office; available online:
[http://fits.gsfc.nasa.gov/registry/tiletablecompression.](http://fits.gsfc.nasa.gov/registry/tiletablecompression.html)
[html](http://fits.gsfc.nasa.gov/registry/tiletablecompression.html)
Ponz, J. D., Thompson, R. W., & Mu˜noz, J. R. 1994, A&AS, 105, 53
Rice, R. F., Yeh, P.-S., & Miller, W. H. 1993, in Proc. 9th AIAA Computing in
Aerospace Conf., AIAA-93-4541-CP, American Institute of Aeronautics and
Astronautics
Rijsinghani, A. (ed.) 1994, IETF RFC 1624,
[https://tools.ietf.org/html/rfc1624](https://tools.ietf.org/html/rfc1624)
Rots, A. H., Bunclark, P. S., Calabretta, M. R., Allen, S. L., Manchester, R. N.
& Thompson, W. T. 2015, A&A, 574, A36
Schmitz, M., et al. 1995, _Information_ & _On-line data in Astronomy_, eds. D. Egret
& M. A. Albrecht (Kluwer Academic Pub.), 259
Standish, E. M. 1990, A&A, 233, 252
Standish, E. M. 1998, JPL Memo IOM 312.F-98-048
Wells, D. C., Greisen, E. W., & Harten, R. H. 1981, A&AS, 44, 363
Wells, D. C. & Grosbøl, P. 1990, _Floating Point Agreement for FITS_ [, http://](http://fits.gsfc.nasa.gov/fp89.txt)
[fits.gsfc.nasa.gov/fp89.txt](http://fits.gsfc.nasa.gov/fp89.txt)
White, R. L. 1992, in Proceedings of the NASA Space and Earth Science Data
Compression Workshop, ed. J. C. Tilton, Snowbird, UT; available online:
[https://archive.org/details/nasa_techdoc_19930016742](https://archive.org/details/nasa_techdoc_19930016742)
White, R. L., & Greenfield, P. 1999, in ADASS VIII, ASP Conf. Ser. 172, eds.
D. M. Mehringer, R. L. Plante, & D. A. Roberts (San Francisco: ASP), 125
White, R. L., Greenfield, P., Pence, W., Tody, D. & Seaman, R. 2013, _Tiled_
_Image Convention for Storing Compressed Images in FITS Binary Tables_,
_FITS_ [Support Office; available online: http://fits.gsfc.nasa.gov/](http://fits.gsfc.nasa.gov/registry/tilecompression.html)
[registry/tilecompression.html](http://fits.gsfc.nasa.gov/registry/tilecompression.html)
Ziv, J., & Lempel, A. 1977, IEEE Transactions on Information Theory, 23 (3),
337

67

<a id="index"></a>
## Index

_N_ bits, 11, 12, 17

angular units, 10, 31
ANSI, 3, 56
ANSI, IEEE, 16, 26
array descriptor, 22–24, 26–29
array size, 11, 12, 17
array value, 3, 4, 14, 20
array, multi-dimensional, 5, 24, 30, 59
array, variable-length, 1, 24, 27–29, 59
ASCII character, 3, 16, 19, 21, 56
ASCII table, 19
ASCII text, 3–5, 7, 13, 21, 25, 56
ASCII, ANSI, 66
AUTHOR, 13

binary table, 4, 16, 22, 54
BITPIX, 11, 12, 14, 16–19, 22
BLANK, 14, 16
blocking, 6
BSCALE, 14, 16
BUNIT, 14
byte order, 5, 16, 26
BZERO, 14, 16

case sensitivity, 6, 7, 10, 20, 23
character string, 3, 7, 21, 24, 25
checksum, 15, 45, 48, 49, 62–65
COMMENT, 6, 13
complex data, 9, 23, 26, 27
compressed binary tables, 48, 54, 62
compressed images, 44, 54, 62
compression algorithms, 44, 49, 50
compression, lossy, 48
conforming extension, 3–5
CONTINUE, 6, 7, 62
coordinate systems, 29

DATAMAX, 14
DATAMIN, 14
DATE, 12, 42
DATE-OBS, 13, 42
DATExxxx, 13, 42
deprecate, 3, 4, 6, 9, 13, 16, 22, 30, 31, 34
dithering, 47
durations, 43

END, 5, 11, 17, 18, 20, 22, 28
EPOCH, 34
EQUINOX, 34
EXTEND, 12
extension, 3–5, 15, 16, 59, 61
extension registration, 5, 11
extension type name, 3, 5, 11, 15
extension, conforming, 3–5
extension, standard, 4, 5
EXTLEVEL, 15
EXTNAME, 15
EXTVER, 15

field, empty, 22, 25, 26
file size, 4
fill, 5, 6, 11, 18, 20, 21, 24, 25, 28
FITS structure, 3, 4, 6, 12
floating-point, 8, 26, 56
floating-point FITS agreement, 67
floating-point, complex, 9, 26
format, data, 16
format, fixed, 7
format, free, 7, 8, 52
format, keywords, 7
Fortran, 5, 19–21, 24, 66

GCOUNT, 12, 17–19, 22
Green Bank convention, 31, 62, 66
group parameter value, 3, 17, 18
GROUPS, 17
GTI tables, 43
Gzip compression, 50

H-compress algorithm, 51
HDU, 4, 11
HDU, extension, 3, 4
HDU, primary, 3–5
header space, preallocation, 14, 62
heap, 4, 12, 22–24, 26, 28
HISTORY, 6, 14
hyphen, 6, 20, 23, 31, 34

IAU, 1, 4, 66
IAU Style Manual, 10, 67
IAUFWG, 1, 4–6, 11, 29, 41, 59
IEEE floating-point, 16
IEEE special values, 4, 14, 16, 56
image extension, 18
INHERIT, 15, 62, 65
INSTRUME, 13
integer, 16-bit, 16, 26
integer, 32-bit, 16, 26
integer, 64-bit, 16, 26
integer, complex, 9
integer, eight-bit, 16, 26
integer, unsigned, 14, 16, 23, 26
ISO-8601 date, 37

JD, 37

keyword record, 5, 6
keyword, commentary, 6, 13
keyword, indexed, 4, 6, 11
keyword, mandatory, 7, 10, 17–19, 22, 54
keyword, new, 16
keyword, order, 11, 17, 19
keyword, required, 4, 10, 11, 17, 18, 22, 44, 48
keyword, reserved, 4, 12, 17, 18, 20, 22, 30, 31, 42, 45, 49,
54
keyword, valid characters, 6

logical value, 8, 25, 26

68

magnetic tape, 6
min and max in columns, keywords, 20, 24, 62
MJD, 37

NaN, IEEE, 16, 26, 56, 57
NAXIS, 4, 5, 11, 12, 17–19, 22
NAXIS1, 17, 19, 21, 22, 24, 25, 28
NAXIS2, 19, 21, 22, 24, 25, 28
NAXISn, 5, 11, 12, 17, 18
NULL, ASCII, 3, 25

OBJECT, 13
OBSERVER, 13
order, byte, 5, 16, 26
order, extensions, 5
order, FITS structures, 4
order, keyword, 6, 11, 17, 19
ORIGIN, 12

PCOUNT, 12, 17–19, 22, 28
phase, 43
physical value, 3, 4, 14, 17, 18, 20, 23
PLIO compression, 50
primary data array, 3–5, 17, 18
primary header, 3, 4, 11, 17
PSCALn, 17, 18
PTYPEn, 17, 18
PZEROn, 17, 18

quantization of data, 46, 63

random groups, 3, 16, 17
random-groups, 14
random-groups array, 17
REFERENC, 13
repeat count, 4, 22, 25
Rice compression, 50

scaling, data, 17, 18, 20, 23
sign bit, 16
sign character, 8, 21
SIMPLE, 5, 11, 17
slash, 7, 10
solar system ephemeris, 41
special records, 3–5
special values, IEEE, 26
standard extension, 4, 5

TABLE, 19
TBCOLn, 19
TDIMn, 24
TDISPn, 20, 24
TELESCOP, 13
TFIELDS, 19, 22
TFORMn, 19, 22, 25, 26, 28
THEAP, 24, 28
time, 12, 13, 34, 36, 37
time keywords, 42
time reference, 38
time reference direction, 40
time reference position, 39
time resolution, 42
time scale, 37, 38

time units, 41
time, universal, 12, 38
timelag, 43
TIMESYS, 37
TNULLn, 20, 21, 24, 26
TSCALn, 20, 23, 28
TTYPEn, 20, 23
TUNITn, 20, 23
two’s complement, 16, 26
TZEROn, 20, 23, 28

underscore, 6, 20, 23
units, 4, 10, 14, 20, 23, 31, 36

value, 6, 7, 12
value, undefined, 7, 14, 16, 20, 21, 24, 60
variable-length array, 1, 24, 27–29, 59
variable-length arrays, compression, 49

WCS, 29
WCS, celestial, 34
WCS, spectral, 34
WCS, timing, 36, 62

XTENSION, 3, 5, 11, 15, 18, 19, 22

69
