//! Non-uniform quantization, rate loop and distortion-control loop.
//!
//! **Milestone:** M3 (inner loop), M4 (outer loop, scalefactors). See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 Annex C.1.5.4 (informative encoder description).
//!
//! **Must be tested by:** Round-trip: quantize -> dequantize per §2.4.3.4.7 must reproduce magnitudes within quantizer error.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
