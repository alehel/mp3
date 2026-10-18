//! Region partitioning, Huffman table selection, big_values/count1 coding.
//!
//! **Milestone:** M3. See docs/PLAN.md.
//!
//! **Allowed sources:** ISO/IEC 11172-3 §2.4.2.7, Annex B Table B.7 (Huffman code tables). Transcribe tables from the standard and log them in PROVENANCE.md.
//!
//! **Must be tested by:** Encode -> decode with a test-only table decoder for random spectra; check bit counts match the table lengths.
//!
//! Clean-room reminder: derive from the sources above only. Cite clauses in
//! comments and log every source in PROVENANCE.md (CLEANROOM.md).
