# Evaluation

An AI can't hear, so the eval harness is the agent's ears for day-to-day work.
It isn't perfect, which is why [`LISTENING.md`](LISTENING.md) exists. But
without it, "vibing" an encoder is just guessing.

## Corpus

Audio files are **not committed** (licensing and size). Put them in:

```
corpus/dev/       20–30 clips the agent tunes against
corpus/holdout/   10–15 different clips, only run at milestone gates
```

Clips should be 5–15 s, 44.1 kHz or 48 kHz WAV, stereo unless noted. Cover
these categories, which are where codecs fail first:

| Category | Why it's hard | Examples |
|---|---|---|
| Sharp transients | Pre-echo, block switching | Castanets, claves, snare, tap dance |
| Tonal, sparse | Quantization noise unmasked between partials | Glockenspiel, harpsichord, pitch pipe, solo flute |
| Dense noise-like | Bit starvation, "swirling" | Applause, rain, cymbal wash |
| Wide stereo | M/S decisions, stereo image collapse | Orchestral hall recording, ping-pong mixes |
| Speech | Familiar sound, so artifacts are obvious | Male and female voice, dry and reverberant |
| Loud modern masters | Clipping, sustained high bitrate demand | Dense rock or EDM |
| Low level / fades | Low-level noise, ATH handling | Piano decay, fade-outs |

Sources with usable licences:

- **EBU SQAM** (Sound Quality Assessment Material). This is the classic codec
  test set (castanets, glockenspiel, harpsichord, speech). Check the EBU's
  current terms before redistributing, and keep it out of the repo.
- **Freesound** clips under CC0 or CC-BY, logged with attribution in
  `corpus/SOURCES.md`.
- **Your own recordings.** These are often the best holdout material, because
  nobody has tuned against them.

Keep `corpus/SOURCES.md` listing every file, its origin and its licence.

## Metrics

| Metric | Tool | Role |
|---|---|---|
| **ViSQOL v3, audio mode** (MOS-LQO, 1–5) | `visqol` binary (Apache-2.0, Google) | Primary objective score |
| **PEAQ** (ODG, −4 to 0) | GstPEAQ or another implementation | Optional second opinion. Plug it into `run_eval.py` |
| **Decode validity** | `ffmpeg -v error` | Hard pass/fail: any decoder error fails the run |
| **Bitrate accuracy** | File size / duration | CBR must hit the target exactly; VBR is reported |
| **Segmental SNR** | Built in | Sanity check only, not perceptual. Useful for catching bugs in M2–M4 |

MP3 encoders and decoders add delay (encoder delay plus decoder delay, often
around 1105 samples total, but it depends). Decoders like ffmpeg trim it
automatically only when the file carries gapless info, which ours won't until M8. `run_eval.py` aligns decoded output
to the reference by cross-correlation before scoring. Never compare unaligned
audio.

## The scoreboard

`run_eval.py` writes `eval/results/<run-id>.json` and prints a table:

```
clip              kbps   ours   lame   gap
castanets.wav      128   3.61   4.05  -0.44
glockenspiel.wav   128   3.88   4.12  -0.24
...
MEAN               128   3.74   4.08  -0.34
```

Track the **mean gap to LAME** per bitrate and the **worst clip**. A change
that raises the mean but makes the worst clip notably worse needs a human look.

## Overfitting guards

1. **Holdout corpus.** The agent never tunes against it. Run it only at 🎧
   gates and compare the dev-vs-holdout gap. A growing gap means overfitting.
2. **Two metrics.** When ViSQOL and PEAQ disagree on a change, a human listens.
3. **Human ears outrank metrics.** A reported audible artifact is a bug
   regardless of scores.
4. **Rotate in new material.** After each 🎧 gate, add two or three fresh clips
   to dev and to holdout.
