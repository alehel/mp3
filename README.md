# mp3enc

A clean-room MPEG-1/2 Audio Layer III (MP3) encoder in Rust, licensed
**MIT OR Apache-2.0**.

> `mp3enc` is a working name. Rename the crate freely.

## Why this exists

Every production-quality MP3 encoder available today is LGPL (LAME, Shine and
their ports). The permissive alternatives are either very old (a BSD-2 encoder
from the late 1990s) or new and unproven. The MP3 patents expired in 2017, so
nothing stops a new permissive implementation. The hard part is quality, not
legality. See [`docs/RESEARCH.md`](docs/RESEARCH.md).

## The rules that matter most

1. **Clean room.** No LGPL/GPL encoder source may be read, pasted, or fed to an
   AI assistant, ever. Read [`CLEANROOM.md`](CLEANROOM.md) before writing a line.
2. **Measure everything.** No change to quality-affecting code is accepted
   without an eval run. See [`docs/EVALUATION.md`](docs/EVALUATION.md).
3. **Ears decide.** From milestone M6 on, metrics are advisory and blind human
   listening is the gate. See [`docs/LISTENING.md`](docs/LISTENING.md).

## Layout

```
CLEANROOM.md          clean-room policy (mandatory reading)
PROVENANCE.md         log of every external source consulted
CLAUDE.md / AGENTS.md instructions for AI coding agents
docs/PLAN.md          staged milestones with exit criteria
docs/EVALUATION.md    corpus, metrics, scoreboard
docs/LISTENING.md     human listening protocol
docs/RESEARCH.md      landscape and allowed references
src/                  the encoder (bitstream, header and WAV I/O done; rest staged)
eval/run_eval.py      objective eval harness (ffmpeg + optional ViSQOL, LAME baseline)
eval/abx.py           blind ABX listening tool
corpus/               test audio (not committed, see corpus/README.md)
listening/            human listening logs (committed)
deny.toml             cargo-deny config that rejects copyleft dependencies
```

## Status

| Milestone | State |
|---|---|
| M0 Scaffolding: bit writer, WAV reader, frame header | done |
| M1–M9 | see [`docs/PLAN.md`](docs/PLAN.md) |

## Quick start

```sh
cargo test
cargo run --release -- input.wav output.mp3 --bitrate 192   # errors until M3
python3 eval/run_eval.py --corpus corpus/dev --bitrates 128 192 320 --keep out
python3 eval/abx.py run --ref corpus/dev/castanets.wav --a out/castanets_ours_128.mp3 --trials 16
```

Tools the harness uses, all invoked as external programs and never linked:
`ffmpeg` (required), `lame` (baseline, optional), `visqol` (metric, optional),
`ffplay` (for ABX playback).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state
otherwise, any contribution intentionally submitted for inclusion shall be
dual licensed as above, without additional terms or conditions.
