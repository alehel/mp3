//! Bit reservoir: main_data_begin and cross-frame bit borrowing.
//!
//! **Milestone:** M7. See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 §2.4.2.7 (main_data_begin), §2.4.3.4 and Annex C notes on the bit reservoir.
//!
//! **Must be tested by:** No decoder ever reports a reservoir underflow; CBR output size stays exact.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
