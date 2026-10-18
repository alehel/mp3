# Contributing

1. Read [`CLEANROOM.md`](CLEANROOM.md). If you have studied the source of LAME,
   Shine or another LGPL/GPL encoder, read §3 (tainted contributors) before
   writing encoder code.
2. Pick the next open item in [`docs/PLAN.md`](docs/PLAN.md). Milestones go in
   order.
3. Before opening a PR: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test`, `cargo deny check`.
4. From M5 on, include `eval/run_eval.py --compare <previous-run>` output.
5. Fill in the PR template's clean-room attestation honestly. If contamination
   happened, say so; CLEANROOM.md §7 explains how to fix it.

Listening contributions are just as valuable as code: run sessions per
[`docs/LISTENING.md`](docs/LISTENING.md) and commit your rows in
`listening/log.csv`, or file a listening-report issue.
