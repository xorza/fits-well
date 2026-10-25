use crate::fits_error::FitsError;
use crate::fits_error::Indexed;
use crate::header_model::Header;
use crate::header_model::record_name::RecordName;
use crate::internals::cards;
use crate::time_coordinates::phase_axis::PhaseAxis;
use std::assert_matches;

const fn phase(zero_phase: f64, period: f64) -> PhaseAxis {
    PhaseAxis {
        zero_phase,
        period: Some(period),
    }
}

/// The image, pixel-list and array-column forms, primary and alternate, each read their own
/// zero phase and period.
#[test]
fn each_keyword_form_reads_its_own_zero_phase_and_period() {
    let h = cards::header_of(&[
        ("CTYPE2", "PHASE".into()),
        ("CZPHS2", 5.0.into()),
        ("CPERI2", 2.0.into()),
        ("CTYPE2A", "PHASE".into()),
        ("CZPHS2A", 7.0.into()),
        ("CPERI2A", 4.0.into()),
        ("TCTYP3", "PHASE".into()),
        ("TCZPH3", 11.0.into()),
        ("TCPER3", 5.0.into()),
        ("TCTY3A", "PHASE".into()),
        ("TCZP3A", 13.0.into()),
        ("TCPR3A", 6.0.into()),
        ("1CTYP5", "PHASE".into()),
        ("1CZPH5", 17.0.into()),
        ("1CPER5", 8.0.into()),
        ("1CTY5A", "PHASE".into()),
        ("1CZP5A", 19.0.into()),
        ("1CPR5A", 10.0.into()),
    ]);
    for (read, expected) in [
        (PhaseAxis::from_header(&h, 1, None), phase(5.0, 2.0)),
        (PhaseAxis::from_header(&h, 1, 'A'), phase(7.0, 4.0)),
        (PhaseAxis::from_pixel_list(&h, 3, None), phase(11.0, 5.0)),
        (PhaseAxis::from_pixel_list(&h, 3, 'A'), phase(13.0, 6.0)),
        (
            PhaseAxis::from_array_column(&h, 0, 5, None),
            phase(17.0, 8.0),
        ),
        (
            PhaseAxis::from_array_column(&h, 0, 5, 'A'),
            phase(19.0, 10.0),
        ),
    ] {
        assert_eq!(read.unwrap(), Some(expected));
    }
}

#[test]
fn axis_of_another_type_is_no_phase_axis() {
    let h = cards::header_of(&[("CTYPE1", "RA---TAN".into())]);
    assert_eq!(PhaseAxis::from_header(&h, 0, None).unwrap(), None);
}

/// An absent period, or a zero one, is a phase that does not repeat, in each family.
#[test]
fn absent_or_zero_period_is_none() {
    type Read = fn(&Header) -> Option<PhaseAxis>;
    let unrepeated = PhaseAxis {
        zero_phase: 23.0,
        period: None,
    };
    let families: [([&str; 3], Read); 3] = [
        (["CTYPE4", "CZPHS4", "CPERI4"], |h| {
            PhaseAxis::from_header(h, 3, None).unwrap()
        }),
        (["TCTYP4", "TCZPH4", "TCPER4"], |h| {
            PhaseAxis::from_pixel_list(h, 4, None).unwrap()
        }),
        (["1CTYP5", "1CZPH5", "1CPER5"], |h| {
            PhaseAxis::from_array_column(h, 0, 5, None).unwrap()
        }),
    ];
    for ([ctype, zero_phase, period], read) in families {
        let mut h = cards::header_of(&[(ctype, "PHASE".into()), (zero_phase, 23.0.into())]);
        assert_eq!(read(&h), Some(unrepeated), "{ctype}");
        h.set_internal(period, 0.0);
        assert_eq!(read(&h), Some(unrepeated), "{period}");
    }
}

#[test]
fn phase_axis_without_its_zero_phase_is_refused() {
    let h = cards::header_of(&[("CTYPE6", "PHASE".into())]);
    assert_matches!(
        PhaseAxis::from_header(&h, 5, None),
        Err(FitsError::MissingKeyword { name: RecordName::Keyword(keyword) }) if keyword.as_str() == "CZPHS6"
    );
}

/// Axes count from zero, as `Wcs::axis_world` counts them: axis 0 names `CTYPE1`. The
/// columns count from one, as their keywords do.
#[test]
fn axes_count_from_zero_and_columns_from_one() {
    let empty = Header::new();
    assert_eq!(PhaseAxis::from_header(&empty, 0, None).unwrap(), None);
    assert_matches!(
        PhaseAxis::from_pixel_list(&empty, 0, None),
        Err(FitsError::OneBasedIndexRequired {
            indexed: Indexed::Column
        })
    );
    assert_eq!(
        PhaseAxis::from_array_column(&empty, 0, 1, None).unwrap(),
        None
    );
    assert_matches!(
        PhaseAxis::from_array_column(&empty, 0, 0, None),
        Err(FitsError::OneBasedIndexRequired {
            indexed: Indexed::Column
        })
    );
}
