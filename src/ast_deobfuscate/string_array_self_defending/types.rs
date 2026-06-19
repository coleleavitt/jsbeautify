use crate::ast_deobfuscate::state::OffsetOperation;

#[derive(Debug, Clone)]
pub(super) struct ChecksumTerm {
    pub(super) indices: Vec<i32>,
    pub(super) divisors: Vec<f64>,
    pub(super) sign: i8,
}

/// Candidate decoder: references a wrapper by name but we don't validate the wrapper exists yet.
#[derive(Debug, Clone)]
pub(super) struct DecoderCandidate {
    pub(super) wrapper_ref: String,
    pub(super) offset: i32,
    pub(super) offset_op: OffsetOperation,
}

/// Candidate rotation IIFE: references wrapper + decoder by name.
#[derive(Debug, Clone)]
pub(super) struct RotationCandidate {
    pub(super) wrapper_ref: String,
    pub(super) decoder_ref: String,
    pub(super) terms: Vec<ChecksumTerm>,
    pub(super) target: f64,
}
