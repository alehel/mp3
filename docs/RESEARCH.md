# Research Notes

Gathered October 2026. Licences change, so recheck before relying on them.

## The MP3 encoder landscape

| Encoder | Licence | Notes | Clean-room status |
|---|---|---|---|
| LAME | LGPL-2.1+ | The quality reference. Usable in proprietary software via dynamic linking | **Forbidden** to read. Binary OK as baseline |
| Shine (and shine-rs, other ports) | LGPL | Fixed-point, fast, lower quality | **Forbidden** |
| lamejs, Mp3LameEncoder.js and other JS/WASM ports | LGPL (inherit LAME) | Often wrapped by MIT packages, but the encoder code is still LGPL | **Forbidden** |
| FFmpeg, mpg123 | LGPL | mpg123 decodes only | **Forbidden** to read. ffmpeg CLI OK as a tool |
| mp3-enc-bsd (Andreas Johansson, encoder 1.02a) | BSD-2-Clause | Old, but genuinely permissive | Grey zone. Don't read without an issue and a provenance entry |
| glint (C) / glint_audio_pure (Dart) | MIT | Recent and obscure; encodes MP3, AAC-LC and Opus. Quality unknown | Grey zone |
| oxideav-mp3 (Rust) | MIT | Reported as a yanked pre-production stub | Not useful |

Takeaway: when a Rust project surveyed the field in 2026 (nvisycom/elide
PR #119), it concluded there is no production-quality permissively licensed MP3
encoder. That gap is the reason for this project.

Decoders for testing (permissive): minimp3 (CC0) and dr_mp3 (public domain or
MIT-0).

## Patents

The MP3 patents (Fraunhofer, Technicolor) expired worldwide by 2017.
Implementing MPEG-1/2 Layer III no longer requires a licence. AAC is a separate
matter and out of scope.

## Allowed technical references

- ISO/IEC 11172-3:1993, *Coding of moving pictures and associated audio … Part 3:
  Audio*. Layer III bitstream, decoder, and informative encoder annexes (C, D).
- ISO/IEC 13818-3:1998, *MPEG-2 Audio*. Low sampling frequencies.
- J. Princen, A. Bradley, "Analysis/synthesis filter bank design based on time
  domain aliasing cancellation," IEEE Trans. ASSP, 1986.
- J. D. Johnston, "Transform coding of audio signals using perceptual noise
  criteria," IEEE JSAC, 1988.
- K. Brandenburg, G. Stoll, "ISO-MPEG-1 Audio: A generic standard for coding of
  high-quality digital audio," JAES, 1994.
- T. Painter, A. Spanias, "Perceptual coding of digital audio," Proc. IEEE, 2000.
- M. Bosi, R. Goldberg, *Introduction to Digital Audio Coding and Standards*,
  Springer, 2003.
- E. Zwicker, H. Fastl, *Psychoacoustics: Facts and Models*.
- ITU-R BS.1116 (small impairments) and BS.1534 (MUSHRA) for listening-test
  methodology. ITU-R BS.1387 for PEAQ.

Log each one in `PROVENANCE.md` when you use it.

## Sources

- https://github.com/nvisycom/elide/pull/119
- https://github.com/lieff/mp3-enc-bsd
- https://pub.dev/documentation/glint_audio/0.11.0/
- https://pub.dev/packages/glint_audio_pure
- https://www.nuget.org/packages/EggEncoder (example of MIT wrapper + dynamically loaded LGPL LAME)
- https://www.nuget.org/packages/SoundFlow.Codecs.FFMpeg/1.4.0
