# Build Plan

Build in this order. Each milestone produces files that decode cleanly before
the next one starts. 🎧 marks a **human listening gate**: the milestone is not
done until a result is logged in `listening/log.csv` (see
[`LISTENING.md`](LISTENING.md)).

Primary references: ISO/IEC 11172-3 (MPEG-1 Layer III, 32/44.1/48 kHz) and
ISO/IEC 13818-3 (MPEG-2 LSF, 16/22.05/24 kHz). Section numbers below refer to
11172-3 unless noted. Check them against your copy of the standard.

---

## M0: Scaffolding ✅

- Bit writer (MSB-first), WAV reader (PCM 16/24-bit, float32), frame header
  writer with bitrate/sample-rate tables and frame-length calculation.
- CI: fmt, clippy, test, cargo-deny.

## M1: A valid, silent bitstream ✅

Write correct frames whose granules carry zero spectral data.

- Side information layout for mono and stereo (§2.4.1.7).
- `main_data_begin = 0` (no reservoir yet), padding-bit logic for 44.1 kHz.
- **Exit:** a 10 s input produces an MP3 that `ffmpeg` and minimp3 decode
  without warnings to near-silence, with the correct duration (± encoder
  delay). Add an integration test that runs `ffmpeg -v error` on the output if
  ffmpeg is available.

## M2: Analysis filterbank and MDCT (long blocks only) ✅

- 32-band polyphase analysis filterbank (§2.4.3.2, Annex C.1.3). Analysis
  window coefficients come from the standard's table.
- 18-point MDCT with the long-block window (§2.4.3.4), producing 576 lines per
  granule.
- Alias-reduction butterflies (encoder side, matching the decoder's §2.4.3.4.10).
- **Tests:** compare the filterbank against a direct (slow) evaluation of the
  matrixing formula. Implement a test-only IMDCT and verify TDAC perfect
  reconstruction to within floating-point tolerance. Pure sines should land in
  the expected subband and line.

## M3: Quantization and Huffman coding (no psychoacoustics) 🎧 smoke ✅

- Non-uniform quantizer `ix = nint((|xr| / 2^((global_gain - 210)/4))^0.75 - 0.0946)`
  per Annex C.1.5.4 (verify against your copy). Derive and document the rounding offset.
- Region partitioning: big_values, count1, rzero; region0/1/2 splits.
- Huffman table selection per region (Annex B tables), count1 tables A/B,
  linbits/escape handling.
- Inner loop: binary search on `global_gain` until the granule fits its bit
  budget.
- **Exit:** music decodes recognisably at 320 and 128 kbps; round-trip
  coefficient check (quantize → Huffman → decode tables → dequantize) passes.
- 🎧 **Smoke listen:** one person, five minutes. It should sound like the input
  (noisy is fine). Log `smoke` sessions to `listening/log.csv`.
- **Status:** implemented; objective exit met (ffmpeg and minimp3 decode all
  test configurations at unit gain, 26–44 dB SNR on a synthetic test signal,
  Huffman round trip exact). Smoke listen passed 2026-10-19 (see
  `listening/log.csv`).

## M4: Scalefactors and the outer loop ✅ (accepted without its exit criterion)

- Scalefactor bands (long), `scalefac_compress`, `preflag`,
  `scalefac_scale`, slen tables.
- Outer distortion-control loop: amplify bands whose noise exceeds the allowed
  distortion, then re-run the inner loop (Annex C.1.5.4.4).
- Allowed distortion is temporarily the absolute threshold of hearing only.
- **Exit:** the eval harness shows a clear gain over M3 at 128 kbps.
- **Status (2026-10-19): implemented, exit criterion not met.** Scalefactors,
  `scalefac_compress`, `preflag` (used as scalefactor compression),
  `scalefac_scale` and the outer loop work and decode correctly in ffmpeg and
  minimp3. With the absolute threshold as the only allowed distortion, the
  outer loop scores below M3 on the dev corpus (mean ViSQOL 4.38/4.60/4.64 vs
  4.39/4.63/4.72 at 128/192/320 kbps, runs `m4-annex` vs `m3-baseline`): when
  noise is above the threshold everywhere it has nothing to shape, and
  otherwise it moves noise into the top octave. Two variants were worse
  still (see `quantize::outer_loop`). The outer loop is therefore opt-in
  (`NoiseShaping::AbsoluteThreshold`); the default encoder is bit-identical
  to M3. It needs M5's masking threshold to pay off.
- **Accepted 2026-10-19** by the maintainer as "machinery complete, gain
  deferred to M5". M5's exit (consistent improvement over M4, whose default
  output equals M3) carries the quality requirement forward.

## M5: Psychoacoustic model ✅

- FFT-based model: spectrum, tonality estimate, spreading function,
  masking threshold per scalefactor band, perceptual entropy (PE).
  Annex D Model 2 is the spec's starting point. Johnston (1988) and Painter &
  Spanias (2000) give the theory.
- From now on `eval/run_eval.py` before and after every change is mandatory.
- **Exit:** a consistent improvement over M4 across the dev corpus.
- **Status (2026-10-19): done.** Model 2 with the Layer III adaptations
  (Annex C.1.5.3), long blocks, MPEG-1 rates; its ratios drive the M4 outer
  loop, now the default. Dev corpus, mean ViSQOL vs M4 (= M3) and LAME:
  128 kbps 4.48 vs 4.39 (LAME 4.50), worst clip 4.02 vs 3.36, 16 of 21 clips
  better; 192 kbps 4.63 vs 4.63; 320 kbps 4.72 vs 4.72. Regressions to check
  by ear: organ at 128 kbps (−0.21) and violin at 192 kbps (−0.15). The
  threshold-in-quiet scale (`psy::QTHR_SCALE`) is a tuning constant chosen
  on dev; check it against the holdout at the next listening gate. Encoding
  is now about 2× real time (outer loop on every granule); speed is M9.

## M6: Block switching 🎧 gate 1 ✅

- Short blocks (3 × 192), start and stop windows, mixed blocks optional.
- Transient detection driven by PE jumps and/or energy attacks.
- Short-block scalefactor bands and the outer-loop variant.
- **Exit:** pre-echo on castanets, glockenspiel and applause audibly reduced.
- **Status (2026-10-19): done.** Short listen logged in `listening/log.csv`:
  at 64 kbps M6 is audibly better than M5 on castanets, little difference
  on glockenspiel; at 192 kbps no clear difference and no artifacts.
  Lesson: `pre_echo_db` shows whether pre-echo changed, not how audible it
  is (glockenspiel −18 dB was barely heard; castanets −3 dB was heard).
  Earlier status: Start,
  short and stop blocks (no mixed blocks), PE-driven switching with one
  granule of lookahead, short-block psychoacoustic thresholds, short
  scalefactors and implicit Huffman regions; ffmpeg and minimp3 decode it.
  Pre-echo (noise in the 10 ms before attacks, `pre_echo_db`) vs M5:
  glockenspiel −18 dB, claves −11 dB, castanets −4 dB. ViSQOL mean 4.491 /
  4.633 / 4.720 at 128 / 192 / 320 kbps (M5 4.483 / 4.629 / 4.718; LAME 4.502
  / 4.627 / 4.718), no clip worse by 0.02 or more. LAME still has less
  pre-echo (castanets 192 kbps −34 vs −28 dB): an M9 target. The dev corpus
  has no applause clip; listen to castanets and glockenspiel.
- 🎧 **Gate 1, short listen (maintainer decision 2026-10-19):** about 10
  minutes, non-blind, on castanets and glockenspiel at 192 kbps against the
  original: is the pre-echo before the attacks gone? Log a `smoke` row. The
  blind ABX artifact hunt moves to M9 with the other formal tests.

## M7: Bit reservoir and joint stereo

- Bit reservoir: `main_data_begin`, a per-frame budget borrowing from easy
  frames for hard ones (§2.4.3.4 and Annex C notes on the buffer).
- Mid/side stereo with a per-frame decision. Intensity stereo is optional and
  only useful below ~112 kbps.
- **Exit:** gains on stereo-heavy and transient-heavy clips. No decoder
  reports reservoir errors.
- 🎧 **Short stereo listen (maintainer decision 2026-10-19):** a few minutes on
  headphones with wide-stereo clips: does the image stay stable? Log a
  `smoke` row.
- **Status (2026-10-19): implemented, awaiting the stereo listen.**
  - **Bit reservoir:** `main_data_begin` per the DIS semantics. Capacity is
    min(7680 − mean frame bits, 511 bytes), or 255 for LSF, so it is 0 at
    320 kbps.
  - **Allocation:** Annex C.1.5.4 §2.2 rules, with "max" read as min. Demand
    is judged relative to a running average, and bits are shared within a
    frame in proportion to a rate–distortion demand estimate.
  - **Mid/side:** decided per frame when it lowers estimated bits. Both
    channels' thresholds become min(L, R), so neither channel's noise can
    exceed its own threshold.
  - **Block types:** synced across channels in joint stereo. No intensity
    stereo.
  - **Results** (run `m7-final` vs `m6-blockswitch`, dev ViSQOL):

    | kbps | M6 | M7 | LAME |
    |---|---|---|---|
    | 128 | 4.491 | 4.543 | 4.502 |
    | 192 | 4.633 | 4.664 | 4.627 |
    | 320 | 4.720 | 4.722 | 4.718 |

  - **Gains by kind of clip:**
    - Stereo-heavy at 128 kbps: choir +0.08, vocal quartet +0.07, guitar
      +0.10, ABBA +0.01.
    - Near-mono speech: +0.31/+0.32 (M/S).
    - Transient-heavy clips: flat (castanets −0.002, claves +0.008). The
      castanets pre-echo proxy is unchanged at 192 kbps (−28 dB; LAME −34).
  - **Ablations** (`m7x-*`, `m7y-*`, `m7z-*`):
    - M/S gives most of the gain.
    - The reservoir only helps once demand is judged relatively.
    - PE-driven borrowing cut pre-echo 2–3 dB on transient clips at
      −0.007 mean ViSQOL. Not adopted; it goes to the listen.
  - **Wide recordings rarely pick M/S:** 0–4% of frames for choir, quartet
    and ABBA. The guitar switches in 23% of frames.
  - **Decoders:** ffmpeg and minimp3 decode every case without warnings. A
    correlated-stereo decode test checks that both M/S and reservoir
    spill-over are exercised.

## M8: VBR, LSF, and metadata

- VBR mode driven by a quality target (allowed noise-to-mask ratio) instead
  of a bitrate. Xing/Info header with frame count and TOC for seeking.
- Gapless info (encoder delay and padding) in the Info tag.
- MPEG-2 LSF sample rates (13818-3). MPEG-2.5 is optional.
- **Exit:** VBR files seek correctly in ffmpeg. LSF files decode cleanly.

## M9: Tuning and performance 🎧 gate 2 / gate 3

- Agentic tuning loop: change → `run_eval.py` → keep or revert. Log each
  accepted constant with its run ID.
- Performance: profile, then SIMD and multithreading (per-granule
  parallelism, psy model ahead of quantization).
- 🎧 **Formal listening (maintainer decision 2026-10-19):** metrics drive
  M6–M9; the heavy listening happens here. Start with the blind ABX artifact
  hunt on the killer samples at 192 kbps (formerly gate 1), then review the
  log of tuning decisions made on metrics alone (`PROVENANCE.md`) against
  what listeners hear.
- 🎧 **Gate 2 (comparative):** blind ABX of ours vs LAME at the same bitrate on
  every sample where metrics show a gap or where the artifact hunt found issues.
- 🎧 **Gate 3 (release candidate):** run the holdout corpus. Transparency ABX
  vs original at 192 and 256 kbps by at least two listeners.

---

## Release targets

These are deliberately realistic. LAME represents about 20 years of tuning
against listening tests.

| Bitrate | Target |
|---|---|
| 320 / 256 kbps CBR | Not ABX-distinguishable from the original on the holdout corpus (p ≥ 0.05) for at least two listeners |
| 192 kbps CBR | Transparent on most samples; any failures documented with sample name and artifact type |
| 128 kbps CBR | Within 0.3 ViSQOL MOS of LAME on the dev corpus; no artifact listeners rate as "annoying" |
| VBR | Quality presets that land near 130, 190 and 245 kbps average on the dev corpus |
