//! Psychoacoustic model: spectrum, tonality, spreading, masking threshold, perceptual entropy.
//!
//! **Milestone:** M5. See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 Annex D (Model 2 as the starting point); Johnston 1988; Painter & Spanias 2000; Zwicker & Fastl.
//!
//! **Must be tested by:** Threshold of a pure tone vs. noise masker should match published masking curves in shape; PE should spike on attacks.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
