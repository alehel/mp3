# Provenance Log

Add one row each time an external source informs code, tables or tuning
constants. Log it when it happens, not later. See [`CLEANROOM.md`](CLEANROOM.md) §6.

| Date | Who | Source (title, clause, URL) | Used for | Module / file |
|---|---|---|---|---|
| 2026-10-18 | scaffold | ISO/IEC 11172-3 §2.4.2.3 (header), Table 2.4.2.3 (bitrate index); ISO/IEC 13818-3 §2.4.2.3 (LSF bitrates) | Frame header layout, bitrate and sample-rate tables, frame length formula | `src/header.rs` |
| 2026-10-18 | scaffold | Microsoft/IBM RIFF WAVE format description (public) | WAV parsing | `src/wav.rs` |

## Contamination incidents

None so far. Use the procedure in CLEANROOM.md §7 if one occurs.
