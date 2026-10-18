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

## Contamination incidents

None so far. Use the procedure in CLEANROOM.md §7 if one occurs.
