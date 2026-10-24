use crate::header_model::{Header, keywords};

use crate::time_coordinates::coordinate::{Coordinate, Epoch};
use crate::time_coordinates::mjd::Mjd;
use crate::time_coordinates::scale::{Scale, ScaleKind};

#[test]
fn reads_jepoch_and_bepoch_keywords() {
    // JEPOCH is in its implied TDB, BEPOCH in ET ≈ TT; `numeric_epochs_match_astropy`
    // pins the dates themselves.
    let mut hj = Header::new();
    hj.set_internal(keywords::JEPOCH, 2000.0);
    assert_eq!(
        Coordinate::epoch(&hj).unwrap(),
        Some(Coordinate {
            mjd: Epoch::Julian(2000.0).to_mjd().unwrap(),
            scale: Scale::known(ScaleKind::Tdb),
        })
    );
    let mut hb = Header::new();
    hb.set_internal(keywords::BEPOCH, 1950.0);
    assert_eq!(
        Coordinate::epoch(&hb).unwrap(),
        Some(Coordinate {
            mjd: Epoch::Besselian(1950.0).to_mjd().unwrap(),
            scale: Scale::known(ScaleKind::Tt),
        })
    );
    // Neither keyword ⇒ None.
    let empty = Header::new();
    assert!(Coordinate::epoch(&empty).unwrap().is_none());
}

#[test]
fn observation_falls_back_to_jepoch_in_its_own_scale() {
    // §9.5: absent DATE-OBS/MJD-OBS, JEPOCH stands in for the observation time, in
    // its implied TDB.
    let mut h = Header::new();
    h.set_internal(keywords::JEPOCH, 2000.0);
    assert_eq!(
        Coordinate::observation(&h).unwrap(),
        Some(Coordinate {
            mjd: Epoch::Julian(2000.0).to_mjd().unwrap(),
            scale: Scale::known(ScaleKind::Tdb)
        })
    );
    // An explicit MJD-OBS still wins, in the declared scale (UTC by default).
    h.set_internal(keywords::MJD_OBS, 58000.0);
    assert_eq!(
        Coordinate::observation(&h).unwrap(),
        Some(Coordinate {
            mjd: Mjd::new(58000, 0.0),
            scale: Scale::known(ScaleKind::Utc)
        })
    );
}

#[test]
fn numeric_epochs_match_astropy() {
    // astropy's JDs less 2400000.5; each rounds to the f64 nearest its decimal.
    let cases: &[(Epoch, f64)] = &[
        (Epoch::Julian(2000.0), 51544.5),
        (Epoch::Besselian(1950.0), 33_281.923_459_05),
        (Epoch::Julian(2015.5), 57205.875),
        (Epoch::Besselian(1900.0), 15019.81352),
    ];
    for &(epoch, mjd) in cases {
        assert_eq!(epoch.to_mjd().unwrap().to_f64(), mjd, "{epoch:?}");
    }
    assert_eq!(Epoch::Julian(2015.5).to_mjd(), Some(Mjd::new(57205, 0.875)));
}
