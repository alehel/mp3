//! 32-band polyphase analysis filterbank.
//!
//! **Milestone:** M2. See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 §2.4.3.2 and Annex C.1.3 (analysis window coefficients: Table C.1).
//!
//! **Must be tested by:** Test against a slow direct evaluation of the matrixing formula; sines must land in the right subband.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
