# Provenance Log

Add one row each time an external source informs code, tables or tuning
constants. Log it when it happens, not later. See [`CLEANROOM.md`](CLEANROOM.md) §6.

| Date | Who | Source (title, clause, URL) | Used for | Module / file |
|---|---|---|---|---|
| 2026-10-18 | scaffold | ISO/IEC 11172-3 §2.4.2.3 (header), Table 2.4.2.3 (bitrate index); ISO/IEC 13818-3 §2.4.2.3 (LSF bitrates) | Frame header layout, bitrate and sample-rate tables, frame length formula | `src/header.rs` |
| 2026-10-18 | scaffold | Microsoft/IBM RIFF WAVE format description (public) | WAV parsing | `src/wav.rs` |
| 2026-10-18 | agent (Claude Code) | ISO/IEC 11172-3 §2.4.1.7, §2.4.2.7; ISO/IEC 13818-3 §2.4.1.7 (side information syntax and semantics). No copy of the standard was available in the session: the field layout was written from the agent's knowledge of the spec text, then checked against the side-info sizes already in `header.rs` (17/32/9/17 bytes) and by decoding with minimp3 and ffmpeg | Side information layout (MPEG-1 and LSF), silent-granule frames | `src/sideinfo.rs`, `src/lib.rs` |
| 2026-10-18 | agent (Claude Code) | `rmp3` 0.3.1 crate (CC0-1.0, bundles minimp3), public API docs only | Reference decoder in tests (dev-dependency, not linked into the encoder) | `tests/decode.rs` |

## Contamination incidents

None so far. Use the procedure in CLEANROOM.md §7 if one occurs.
