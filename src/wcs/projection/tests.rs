use crate::wcs::projection::evaluate_zpn;

#[test]
fn zpn_extended_horner_evaluates_value_and_derivative() {
    let mut pv = [0.0; 21];
    pv[..4].copy_from_slice(&[2.0, -3.0, 4.0, 5.0]);
    let evaluation = evaluate_zpn(2.0, &pv);
    // P(2) = 2 - 3·2 + 4·2² + 5·2³ = 52; P'(2) = -3 + 8·2 + 15·2² = 73.
    assert_eq!(evaluation.value, 52.0);
    assert_eq!(evaluation.derivative, 73.0);
}
