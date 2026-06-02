//! The complex value type for `C`/`M` binary-table columns (§7.3.2).

/// A complex number `re + im·i`. The element type of single- (`C`) and
/// double-precision (`M`) complex binary-table columns: [`ColumnData::ComplexF32`]
/// holds `Complex<f32>`, [`ColumnData::ComplexF64`] holds `Complex<f64>`, and
/// [`ColumnData::complex`] returns the scaled `Complex<f64>` plane.
///
/// This is a plain data carrier with public `re`/`im` fields and no arithmetic — a
/// dependency-free stand-in for `num_complex::Complex`, so the core stays free of
/// external crates. Convert to your numerics library of choice at the boundary.
///
/// [`ColumnData::ComplexF32`]: crate::ColumnData::ComplexF32
/// [`ColumnData::ComplexF64`]: crate::ColumnData::ComplexF64
/// [`ColumnData::complex`]: crate::ColumnData::complex
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Complex<T> {
    pub re: T,
    pub im: T,
}
