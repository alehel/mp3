# Instructions for AI coding agents

Read this whole file before doing anything in this repository.

## Non-negotiable: clean room

This encoder must be written from specifications and published research only.
Full policy: [`CLEANROOM.md`](CLEANROOM.md). In short:

- **Never** read, fetch, quote, reproduce or port source code from LAME, Shine,
  shine-rs, lamejs, FFmpeg, mpg123, the ISO dist10 reference software, or any
  GPL/LGPL encoder. Do not search for it. Do not open it if a search surfaces it.
- **Never** write code "from memory of how LAME does it". Your training data
  contains LAME. If you notice you are recalling specific implementation
  details (function names, comments, magic constants) instead of deriving from
  the spec, stop and say so.
- Every non-trivial algorithm gets a comment citing its source: an ISO clause
  (e.g. `// ISO/IEC 11172-3 §2.4.3.4, Annex C.1.5.4.4`), a paper, or
  `// original: <reasoning>`.
- Every tuning constant gets a citation or a note naming the eval run that
  justified it.
- Log every external source in [`PROVENANCE.md`](PROVENANCE.md) in the same
  commit that uses it.
- Only add permissively licensed dependencies. `cargo deny check licenses` must
  pass.

## How work is organised

- Follow the milestones in [`docs/PLAN.md`](docs/PLAN.md) **in order**. Don't
  start a milestone until the previous one meets its exit criteria.
- Keep the build green: `cargo fmt --check`, `cargo clippy -- -D warnings` and
  `cargo test` must all pass before you report a step as done.
- Write unit tests against known values for every DSP stage (filterbank, MDCT,
  Huffman). Silent numerical bugs are the main failure mode in codecs.

## Quality work (M5 onward)

- Run `python3 eval/run_eval.py` before and after every quality-affecting
  change and include both scoreboards in your summary. A change that does not
  improve the scoreboard is reverted.
- **Never** tune against `corpus/holdout/`. It exists to catch overfitting.
  Only run the holdout at milestone boundaries, when a human asks for it.
- Metrics are proxies. Do not game them (e.g. spending bits on bands the metric
  weights heavily but listeners don't hear).
- **Listening gates:** at the points marked 🎧 in `docs/PLAN.md`, stop and ask
  the human to run a listening session per [`docs/LISTENING.md`](docs/LISTENING.md).
  You cannot hear. Do not mark a gated milestone done until a passing result is
  logged in `listening/log.csv`. When a human reports an artifact by ear, treat
  that as a bug even if every metric looks fine.

## Style

- Rust 2024 edition, no `unsafe` without a comment justifying it, prefer `f32`
  for DSP with `f64` where accumulation precision matters (document which).
- Keep modules aligned with encoder stages: `filterbank`, `mdct`, `psy`,
  `quantize`, `huffman`, `reservoir`, `stereo`, `bitstream`, `header`.
