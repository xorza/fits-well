use crate::error::FitsError;
use crate::error::Indexed;
use crate::header_model::Header;
use crate::internals::cards::header_of;

use crate::time_coordinates::phase_axis::PhaseAxis;

#[test]
fn reads_phase_axis_metadata() {
    let mut h = header_of(&[
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

    let image = PhaseAxis::from_header(&h, 2, None).unwrap().unwrap();
    assert_eq!(image.zero_phase, 5.0);
    assert_eq!(image.period, Some(2.0));

    let alternate = PhaseAxis::from_header(&h, 2, 'A').unwrap().unwrap();
    assert_eq!(
        alternate,
        PhaseAxis {
            zero_phase: 7.0,
            period: Some(4.0),
        }
    );
    assert_eq!(
        PhaseAxis::from_pixel_list(&h, 3, None).unwrap().unwrap(),
        PhaseAxis {
            zero_phase: 11.0,
            period: Some(5.0),
        }
    );
    assert_eq!(
        PhaseAxis::from_pixel_list(&h, 3, 'A').unwrap().unwrap(),
        PhaseAxis {
            zero_phase: 13.0,
            period: Some(6.0),
        }
    );
    assert_eq!(
        PhaseAxis::from_array_column(&h, 1, 5, None)
            .unwrap()
            .unwrap(),
        PhaseAxis {
            zero_phase: 17.0,
            period: Some(8.0),
        }
    );
    assert_eq!(
        PhaseAxis::from_array_column(&h, 1, 5, 'A')
            .unwrap()
            .unwrap(),
        PhaseAxis {
            zero_phase: 19.0,
            period: Some(10.0),
        }
    );

    h.set_internal("CTYPE1", "RA---TAN");
    assert_eq!(PhaseAxis::from_header(&h, 1, None).unwrap(), None);

    h.set_internal("CTYPE4", "PHASE")
        .set_internal("CZPHS4", 23.0);
    let varying = PhaseAxis::from_header(&h, 4, None).unwrap().unwrap();
    assert_eq!(varying.period, None);
    h.set_internal("CPERI4", 0.0);
    assert_eq!(
        PhaseAxis::from_header(&h, 4, None).unwrap().unwrap().period,
        None
    );

    h.set_internal("CTYPE6", "PHASE");
    assert!(matches!(
        PhaseAxis::from_header(&h, 6, None),
        Err(FitsError::MissingKeyword { keyword }) if keyword.as_str() == "CZPHS6"
    ));

    let empty = Header::new();
    assert!(matches!(
        PhaseAxis::from_header(&empty, 0, None),
        Err(FitsError::OneBasedIndexRequired {
            indexed: Indexed::WcsAxis
        })
    ));
    assert!(matches!(
        PhaseAxis::from_pixel_list(&empty, 0, None),
        Err(FitsError::OneBasedIndexRequired {
            indexed: Indexed::Column
        })
    ));
    assert!(matches!(
        PhaseAxis::from_array_column(&empty, 0, 1, None),
        Err(FitsError::OneBasedIndexRequired {
            indexed: Indexed::WcsAxis
        })
    ));
    assert!(matches!(
        PhaseAxis::from_array_column(&empty, 1, 0, None),
        Err(FitsError::OneBasedIndexRequired {
            indexed: Indexed::Column
        })
    ));
}
