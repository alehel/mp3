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

## M3: Quantization and Huffman coding (no psychoacoustics) 🎧 smoke

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

## M4: Scalefactors and the outer loop

- Scalefactor bands (long), `scalefac_compress`, `preflag`,
  `scalefac_scale`, slen tables.
- Outer distortion-control loop: amplify bands whose noise exceeds the allowed
  distortion, then re-run the inner loop (Annex C.1.5.4.4).
- Allowed distortion is temporarily the absolute threshold of hearing only.
- **Exit:** the eval harness shows a clear gain over M3 at 128 kbps.

## M5: Psychoacoustic model

- FFT-based model: spectrum, tonality estimate, spreading function,
  masking threshold per scalefactor band, perceptual entropy (PE).
  Annex D Model 2 is the spec's starting point. Johnston (1988) and Painter &
  Spanias (2000) give the theory.
- From now on `eval/run_eval.py` before and after every change is mandatory.
- **Exit:** a consistent improvement over M4 across the dev corpus.

## M6: Block switching 🎧 gate 1

- Short blocks (3 × 192), start and stop windows, mixed blocks optional.
- Transient detection driven by PE jumps and/or energy attacks.
- Short-block scalefactor bands and the outer-loop variant.
- **Exit:** pre-echo on castanets, glockenspiel and applause audibly reduced.
- 🎧 **Gate 1, artifact hunt:** a blind ABX against the original on the killer
  samples at 192 kbps, plus a written list of audible artifact types. Expect
  failures here. The point is to find out what metrics are missing.

## M7: Bit reservoir and joint stereo

- Bit reservoir: `main_data_begin`, a per-frame budget borrowing from easy
  frames for hard ones (§2.4.3.4 and Annex C notes on the buffer).
- Mid/side stereo with a per-frame decision. Intensity stereo is optional and
  only useful below ~112 kbps.
- **Exit:** gains on stereo-heavy and transient-heavy clips. No decoder
  reports reservoir errors.

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
- 🎧 **Gate 2 (comparative):** blind ABX of ours vs LAME at the same bitrate on
  every sample where metrics show a gap or where gate 1 found issues.
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
