//! Mid/side (and optionally intensity) stereo decisions.
//!
//! **Milestone:** M7. See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 §2.4.3.4.9 (MS and intensity stereo processing).
//!
//! **Must be tested by:** Mono-compatible material should pick M/S; wide uncorrelated material should stay L/R.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
