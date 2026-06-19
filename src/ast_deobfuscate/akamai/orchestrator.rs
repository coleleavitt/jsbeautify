use oxc_ast::ast::Program;

use super::boolean_arithmetic::BooleanArithmeticFolder;
use super::detector::{AkamaiDetector, DETECTION_THRESHOLD, walk_detector};
use super::equality_proxy::EqualityProxyUnwrapper;
use super::stack_tracker::StackTrackerRemover;
use super::undefined_pattern::UndefinedPatternNormalizer;

/// High-level orchestrator that applies every Akamai-specific pass when the
/// detector says the input is BMP.
pub struct AkamaiDeobfuscator {
    pub bool_folder: BooleanArithmeticFolder,
    pub undef_normalizer: UndefinedPatternNormalizer,
    pub eq_unwrapper: EqualityProxyUnwrapper,
    pub tracker_remover: StackTrackerRemover,
    pub detected: bool,
}

impl AkamaiDeobfuscator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            bool_folder: BooleanArithmeticFolder::new(),
            undef_normalizer: UndefinedPatternNormalizer::new(),
            eq_unwrapper: EqualityProxyUnwrapper::new(),
            tracker_remover: StackTrackerRemover::new(),
            detected: false,
        }
    }

    /// Run detection over `program`. Must be called before [`Self::is_akamai`].
    pub fn detect(&mut self, program: &Program<'_>) -> bool {
        let mut detector = AkamaiDetector::new();
        walk_detector(&program.body, &mut detector);
        self.detected = detector.is_akamai();

        let sigs = detector.signature_count();
        let acc = detector.self_init_accessor_count();
        let mut hit_list: Vec<&String> = detector.hits().iter().collect();
        hit_list.sort();

        eprintln!("[AKAMAI] ─── BMP detection report ───");
        eprintln!("[AKAMAI]   identifier/string hits: {sigs} ({hit_list:?})");
        eprintln!(
            "[AKAMAI]   self-init accessors   : {acc} (first {}: {:?})",
            detector.accessor_names().len().min(8),
            detector.accessor_names().iter().take(8).collect::<Vec<_>>()
        );
        eprintln!("[AKAMAI]   thresholds            : signatures >= {DETECTION_THRESHOLD} OR accessors >= 3");
        if self.detected {
            eprintln!("[AKAMAI]   => BMP DETECTED, Akamai passes will run");
        } else {
            eprintln!("[AKAMAI]   => NOT BMP, Akamai passes will be skipped");
        }
        self.detected
    }

    #[must_use]
    pub const fn is_detected(&self) -> bool {
        self.detected
    }
}

impl Default for AkamaiDeobfuscator {
    fn default() -> Self {
        Self::new()
    }
}
