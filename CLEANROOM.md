# Clean-Room Policy

This project must be releasable under MIT OR Apache-2.0 with no copyleft
taint. A single pasted function from an LGPL encoder would undo that. These
rules apply equally to humans and to AI assistants working in this repo.

I am not a lawyer and this is not legal advice. It is a conservative
engineering policy. If the project gets commercial use or wider adoption, have
a lawyer review it.

## 1. What "clean room" means here

Code is written only from **specifications, published research, and
first-principles reasoning**, never from another encoder's source code. The
goal is that every line can be traced to an allowed source or to original work,
and that the provenance log proves it.

## 2. Source categories

### Allowed: read and use freely

- **The standards:** ISO/IEC 11172-3 (MPEG-1 Audio) and ISO/IEC 13818-3
  (MPEG-2 Audio, low sampling frequencies). Normative tables (Huffman code
  tables, scalefactor band boundaries, bitrate and sample-rate tables, window
  shapes) are functional data needed for interoperability. Transcribe them from
  the standard and log the clause or annex in `PROVENANCE.md`.
- **Peer-reviewed papers, textbooks and theses** on perceptual audio coding,
  MDCT, filterbanks, psychoacoustics and rate control. Examples:
  Princen & Bradley (TDAC/MDCT), Johnston (perceptual entropy, 1988),
  Brandenburg & Stoll (ISO MPEG-1 audio overview, 1994),
  Painter & Spanias (perceptual coding review, 2000),
  Zwicker & Fastl (*Psychoacoustics*), Bosi & Goldberg
  (*Introduction to Digital Audio Coding and Standards*).
- **Prose descriptions of techniques** in articles, wikis, forum posts and
  mailing lists, as long as they contain no copied code. Ideas are not
  copyrightable. Expression is.
- **Running other encoders as black boxes.** Executing the `lame` binary to
  produce baseline files and comparing outputs is fine. So is decoding with
  `ffmpeg`. Neither is linked or read.
- **Permissively licensed decoders** (e.g. minimp3, CC0) for testing only. Even
  so, write the encoder from the spec, not by inverting decoder code.

### Forbidden: never read, never paste, never feed to an AI

- **LAME** source (LGPL), including ports such as lamejs and any wrapper that
  vendors its source.
- **Shine** and its ports such as shine-rs (LGPL).
- **FFmpeg's** MP3 code and **mpg123** (LGPL), plus any other GPL/LGPL audio
  codec source.
- **Code of unclear licence**, including the ISO "dist10" reference software.
  Its licence terms are ambiguous, so treat it as forbidden.
- **Disassembly or decompilation** of any proprietary encoder.

### Grey zone: ask before touching

- **Permissive encoders** (e.g. the BSD-2 `mp3-enc-bsd`, MIT `glint`). Their
  licences would allow copying with attribution, but mixing in their code
  weakens the "written from the spec" story and drags in their bugs and
  heuristics. Default: do not read them. If you have a strong reason, open an
  issue first, log it in `PROVENANCE.md`, and preserve any required notices.

## 3. The tainted-contributor rule

Anyone who has **studied the source** of a forbidden encoder (for example, a
past LAME contributor) should not write the corresponding module here. They can
still help by writing tests, building the eval harness, curating the corpus,
listening, and reviewing behaviour from the outside. If this is in doubt,
disclose it in the PR.

## 4. Rules for AI assistants

AI coding tools are the biggest contamination risk, because their training data
almost certainly includes LAME and Shine.

1. **Never** paste forbidden source into a prompt, issue or file, even "just to
   compare".
2. **Never** ask an assistant to "port", "translate", "reproduce" or "do what
   LAME does in function X". Ask for the technique instead: "implement the
   outer distortion-control loop as described in ISO 11172-3 Annex C".
3. **Require derivation.** Every non-trivial algorithm an AI writes should come
   with a comment citing the spec clause or paper it implements. If it cannot
   name an allowed source, treat the output as suspect.
4. **Watch for regurgitation.** Suspect memorized code when output contains
   distinctive variable names, magic constants or comments that the spec does
   not explain, or code that looks unusually "finished" for a first draft.
   Discard it and regenerate from the spec, or rewrite it by hand.
5. **Magic numbers need a home.** Every tuning constant must either cite its
   source or be logged as "found by our own tuning" with the eval run that
   justified it. A constant that matches a known LAME value with no
   independent derivation is a red flag.
6. Agents follow [`CLAUDE.md`](CLAUDE.md), which restates these rules.

## 5. Dependency rules

- Runtime dependencies must be permissive (MIT, Apache-2.0, BSD, ISC, Zlib,
  CC0, Unicode). `deny.toml` enforces this. Run `cargo deny check licenses`.
- External tools (ffmpeg, lame, visqol) are invoked as separate processes by the
  eval scripts only. They are never linked, vendored or redistributed.

## 6. Provenance log

Every external source consulted gets one line in [`PROVENANCE.md`](PROVENANCE.md):
date, who (human or agent), source, what it was used for, and which module it
informed. Log it at the time, not after the fact. This log is the evidence that
the clean room held.

## 7. If contamination happens

It happens. Handle it openly:

1. Stop and note it in `PROVENANCE.md` (what was seen, by whom, which code).
2. Delete the affected code, including from unmerged branches.
3. Have someone who did not see the forbidden source reimplement it from the
   spec.
4. If tainted code reached `main`, rewrite it in a new commit and say so in the
   commit message. Rewriting history is optional; documenting it is not.
