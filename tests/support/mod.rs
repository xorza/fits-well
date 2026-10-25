//! The parts of the read-bytes tests each of their binaries shares.

#[expect(
    dead_code,
    reason = "the read-bytes tests count the bytes held, not calls"
)]
pub(crate) mod counting;
pub(crate) mod peak;
