//! MDCT with window switching, plus encoder-side alias reduction.
//!
//! **Milestone:** M2 (long blocks), M6 (short/start/stop). See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 §2.4.3.4 (window shapes, IMDCT definition, alias reduction coefficients); Princen & Bradley 1986 (TDAC).
//!
//! **Must be tested by:** Implement a test-only IMDCT and verify perfect reconstruction (TDAC) for every window sequence.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
