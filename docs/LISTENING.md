# Human Listening Protocol

Metrics get you most of the way; ears decide the rest. This document says
**when** humans listen, **how**, and **how results feed back** into
development.

## When

| Phase | Milestone | Who | Goal |
|---|---|---|---|
| Smoke | M3, then whenever something "looks weird" | Anyone, 5 min | Catch gross bugs: clicks, dropouts, wrong pitch, garbage |
| 🎧 Gate 1: artifact hunt | M6 | Primary listener | Find artifact types the metrics miss |
| 🎧 Gate 2: comparative | M9 | Primary listener (+1 if possible) | Ours vs LAME at matched bitrate |
| 🎧 Gate 3: release | M9, release candidate | At least two listeners | Transparency on the holdout corpus |
| Ad hoc | Any time | Anyone | A metric jump of more than 0.2 MOS in either direction gets a quick listen |

The agent stops and asks for a session at each gate (see `CLAUDE.md`).

## Setup

- **Headphones,** ideally closed or open-back studio models (wired, not
  Bluetooth, which re-encodes audio). Speakers in a quiet room work for a
  second opinion.
- **Fixed level.** Set a comfortable, fairly loud level once per session and
  don't touch it. Louder sounds "better", which skews results.
- **Quiet room,** at most 45 minutes per session, with breaks. Ears fatigue
  fast and start guessing.
- **Train first.** Before formal trials, listen non-blind to a low-bitrate
  encode (e.g. 64 kbps) of the same clip to learn what the artifacts sound like.

## ABX procedure

Use `eval/abx.py` (cross-platform, plays through `ffplay`) or foobar2000's ABX
Comparator. For quick non-blind comparisons in a browser, the Listening Bench
artifact (made in M6) loads an original plus encoded versions, lines them up
automatically, loops a passage and switches between them with keys 1–9; its
"Blind labels" mode hides which encoded version is which. Generate the files with `run_eval.py --keep out`.

```sh
# Ours vs original (transparency test)
python3 eval/abx.py run --ref corpus/dev/castanets.wav --a out/castanets_ours_192.mp3 --trials 16

# Ours vs LAME (comparative test), both against the same reference
python3 eval/abx.py run --ref corpus/dev/castanets.wav --a out/castanets_ours_128.mp3 --b out/castanets_lame_128.mp3 --trials 16
```

In each trial you hear A, B and X as often as you like, then say whether X is
A or B. You can loop a short segment (`--start` / `--duration`). Focusing on
the hardest two seconds is fair and encouraged.

**Interpreting results (16 trials):**

| Correct | p-value (one-sided) | Meaning |
|---|---|---|
| ≥ 12 | ≤ 0.038 | You can hear a difference |
| 10–11 | 0.11–0.23 | Inconclusive. Rerun with 16 more trials if it matters |
| ≤ 9 | ≥ 0.40 | No evidence of an audible difference |

Decide the number of trials **before** starting, and don't stop early when a
streak looks good. That inflates false positives. `abx.py` enforces this.

For a transparency claim, "not distinguishable" needs at least two listeners
on several clips. One failed ABX proves little; a pattern does.

## Describing what you hear

Use consistent words so the agent can map them to causes:

| Artifact | Sounds like | Likely cause |
|---|---|---|
| **Pre-echo** | A smear or "ghost" just before sharp attacks | Missed block switch, short-block noise allocation |
| **Warbling / birdies** | Chirpy, metallic tones that come and go in the highs | Bands toggling between zero and non-zero across granules |
| **Swirling** | Watery, phasey movement in noise-like content | Too few bits in noisy bands, unstable allocation |
| **Dullness** | Missing air or sparkle | Lowpass too aggressive, high bands starved |
| **Graininess / roughness** | Fizzy noise on tonal sounds | Masking threshold too high for tonal components |
| **Stereo collapse** | Image narrows or wanders | M/S decision errors, intensity stereo |
| **Ringing / tonal smearing** | Tones bleed over time | Long blocks on transient content |
| **Clicks / dropouts** | Discrete glitches | Bitstream bugs, reservoir errors. Report these immediately |

## Logging

Every session appends rows to `listening/log.csv`. `abx.py` does this
automatically, and you can add rows by hand for smoke tests:

```
date,listener,commit,clip,bitrate,test,a,b,trials,correct,p_value,artifacts,notes
```

- `test`: `smoke`, `abx-original`, `abx-lame`, or `preference`
- `artifacts`: semicolon-separated words from the table above
- `notes`: a timestamp (e.g. `0:03.2`) and a free-text description

Commit the log. It's the project's record of what humans actually heard.

## Feeding results back to the agent

When a session finds something, open an issue (or tell the agent directly)
with:

1. The clip, bitrate and commit
2. The artifact type from the vocabulary above
3. A timestamp
4. Whether it's present in LAME's encode at the same bitrate

The agent's job is to turn this into a reproducible metric or test where
possible (e.g. a per-band noise-to-mask check on that clip), then fix it.
Some artifacts will never show up in metrics. For those, the human's report is
the test, and the fix is re-verified by ear.
