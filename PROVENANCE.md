# Provenance Log

Add one row each time an external source informs code, tables or tuning
constants. Log it when it happens, not later. See [`CLEANROOM.md`](CLEANROOM.md) §6.

| Date | Who | Source (title, clause, URL) | Used for | Module / file |
|---|---|---|---|---|
| 2026-10-18 | scaffold | ISO/IEC 11172-3 §2.4.2.3 (header), Table 2.4.2.3 (bitrate index); ISO/IEC 13818-3 §2.4.2.3 (LSF bitrates) | Frame header layout, bitrate and sample-rate tables, frame length formula | `src/header.rs` |
| 2026-10-18 | scaffold | Microsoft/IBM RIFF WAVE format description (public) | WAV parsing | `src/wav.rs` |
| 2026-10-18 | agent (Claude Code) | ISO/IEC 11172-3 §2.4.1.7, §2.4.2.7; ISO/IEC 13818-3 §2.4.1.7 (side information syntax and semantics). No copy of the standard was available in the session: the field layout was written from the agent's knowledge of the spec text, then checked against the side-info sizes already in `header.rs` (17/32/9/17 bytes) and by decoding with minimp3 and ffmpeg | Side information layout (MPEG-1 and LSF), silent-granule frames | `src/sideinfo.rs`, `src/lib.rs` |
| 2026-10-18 | agent (Claude Code) | `rmp3` 0.3.1 crate (CC0-1.0, bundles minimp3), public API docs only | Reference decoder in tests (dev-dependency, not linked into the encoder) | `tests/decode.rs` |
| 2026-10-18 | agent (Claude Code) | CD 11172-3 (Committee Draft text of ISO/IEC 11172-3, main body clauses 1–2.4.3.4 only, no annexes; PDF supplied by the maintainer, not committed) §2.4.1.7, §2.4.2.7 | Verified the side-info field order and widths and the block_type values in `sideinfo.rs` (unchanged). Note: the draft names fields differently (`main_data_end`, `blocksplit_flag`, `switch_point`, `region_address1/2`) and defines `main_data_end` relative to the *next* header; deployed decoders follow the published `main_data_begin` semantics, so the draft is not authoritative for the reservoir (M7) | `src/sideinfo.rs` |
| 2026-10-18 | agent (Claude Code) | CD 11172-3 §2.4.3.4 (IMDCT formula, block_type 0 window, frequency-inversion rule in "Synthesis filterbank") | Long-block forward MDCT (transpose of the IMDCT kernel, 4/n scale derived by us), test-only IMDCT, frequency inversion | `src/mdct.rs` |
| 2026-10-18 | agent (Claude Code) | CD 11172-3 rev 1, full text with annexes (University of Thessaly course copy, https://courses.e-ce.uth.gr/CE401/tree_menu/tutorials/MPEG1/MPEG1_3.PDF; not committed): Annex C.1.3 and Figure 3-C.1 (analysis filterbank), Table 3-C.1 (analysis window C_i), Table 3-B.3 (synthesis window D_i), Figure 3-A.2 (synthesis flow chart), Table 3-B.9 (alias c_i, cs/ca formulas), Figures 3-A.5/3-A.6 and Annex C "Aliasing-Butterfly, Encoder" | Polyphase analysis filterbank; window stored as integer numerators over 2^21 (all 512 entries exact multiples; D_i = 32·C_i verified for all 512); encoder alias reduction; test-only reference decoder (decoder butterflies, IMDCT, synthesis) | `src/filterbank.rs`, `src/mdct.rs`, `src/refdec.rs` |
| 2026-10-18 | agent (Claude Code) | ETSI TS 103 466 V1.2.1 (2019-09), Table C.1 (DAB, reproduces the MPEG-1 analysis window), https://www.etsi.org/deliver/etsi_ts/103400_103499/103466/01.02.01_60/ts_103466v010201p.pdf | Independent cross-check of the transcribed Table 3-C.1: all 512 coefficients identical | `src/filterbank.rs` |

## Contamination incidents

None so far. Use the procedure in CLEANROOM.md §7 if one occurs.
