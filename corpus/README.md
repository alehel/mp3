# Test corpus

Audio files are **not committed**. Put WAV files here locally:

- `dev/`: 20–30 clips the agent and developers tune against.
- `holdout/`: 10–15 different clips. Only used at 🎧 listening gates.
  `run_eval.py` refuses to touch it without `--allow-holdout`.

What to include (categories, clip length, licensed sources) is described in
[`docs/EVALUATION.md`](../docs/EVALUATION.md#corpus).

Record every file in [`SOURCES.md`](SOURCES.md) with its origin and licence,
so the corpus can be rebuilt on another machine.
