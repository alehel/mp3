#!/usr/bin/env python3
"""Objective evaluation harness: our encoder vs. LAME on a corpus.

For each clip and bitrate it encodes with ours (and LAME if installed), decodes
with ffmpeg, aligns to the reference, and scores. Results go to
eval/results/<run-id>.json, and a scoreboard is printed.

    python3 eval/run_eval.py --corpus corpus/dev --bitrates 128 192 320
    python3 eval/run_eval.py --compare <previous-run-id>   # keep-or-revert view

See docs/EVALUATION.md. Requires numpy and ffmpeg. lame and visqol are optional.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import functools
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import (REPO, align, decode, git_commit, have, pre_echo_db, probe,  # noqa: E402
                    run, segmental_snr, side_snr_db, write_wav_f32)

RESULTS = REPO / "eval" / "results"
VISQOL_RATE = 48_000  # ViSQOL audio mode expects 48 kHz


def build_ours() -> Path:
    print("building encoder (cargo build --release)...", file=sys.stderr)
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=REPO, check=True)
    exe = REPO / "target" / "release" / ("mp3enc.exe" if sys.platform == "win32" else "mp3enc")
    return exe


def encode_ours(exe: Path, wav: Path, out: Path, kbps: int, extra: list[str]) -> str | None:
    proc = run([str(exe), str(wav), str(out), "--bitrate", str(kbps), *extra], check=False)
    return None if proc.returncode == 0 else (proc.stderr.strip() or "encoder failed")


def encode_lame(wav: Path, out: Path, kbps: int) -> str | None:
    # Black-box baseline. LAME is run as a binary only, never read (CLEANROOM.md).
    proc = run(["lame", "--quiet", "--cbr", "-b", str(kbps), str(wav), str(out)], check=False)
    return None if proc.returncode == 0 else (proc.stderr.strip() or "lame failed")


@functools.cache
def visqol_flags() -> tuple[str, str]:
    """Reference/degraded flag names for the installed `visqol`.

    Google's C++ binary takes --reference_file/--degraded_file; the pure
    Python port (visqol-python, Apache-2.0) takes --reference/--degraded.
    Both print "MOS-LQO:" and default to audio mode.
    """
    help_text = run(["visqol", "--help"], check=False)
    if "--reference_file" in help_text.stdout + help_text.stderr:
        return "--reference_file", "--degraded_file"
    return "--reference", "--degraded"


def visqol_score(ref: np.ndarray, deg: np.ndarray, sr: int, tmp: Path) -> float | None:
    if not have("visqol"):
        return None
    r_path, d_path = tmp / "vq_ref.wav", tmp / "vq_deg.wav"
    r48, d48 = tmp / "vq_ref48.wav", tmp / "vq_deg48.wav"
    write_wav_f32(r_path, ref, sr)
    write_wav_f32(d_path, deg, sr)
    for src, dst in ((r_path, r48), (d_path, d48)):
        run(["ffmpeg", "-v", "error", "-y", "-i", str(src), "-ar", str(VISQOL_RATE),
             "-c:a", "pcm_s16le", str(dst)])
    ref_flag, deg_flag = visqol_flags()
    proc = run(["visqol", ref_flag, str(r48), deg_flag, str(d48)], check=False)
    m = re.search(r"MOS-LQO:\s*([0-9.]+)", proc.stdout + proc.stderr)
    return float(m.group(1)) if m else None


def score(mp3: Path, ref: np.ndarray, sr: int, ch: int, tmp: Path) -> dict:
    deg, stderr = decode(mp3, sr, ch)
    r, d, lag = align(ref, deg)
    duration = len(ref) / sr
    return {
        "decode_ok": stderr == "",
        "decode_stderr": stderr,
        "lag_samples": lag,
        "actual_kbps": round(mp3.stat().st_size * 8 / duration / 1000, 2),
        "seg_snr_db": round(segmental_snr(r, d), 2),
        "pre_echo_db": (lambda v: None if v is None else round(v, 2))(pre_echo_db(r, d, sr)),
        "side_snr_db": (lambda v: None if v is None else round(v, 2))(side_snr_db(r, d)),
        "visqol": visqol_score(r, d, sr, tmp),
    }


def primary(entry: dict | None) -> float | None:
    """The scoreboard metric: ViSQOL if available, else segmental SNR."""
    if not entry or "error" in entry:
        return None
    return entry["visqol"] if entry.get("visqol") is not None else entry.get("seg_snr_db")


def fmt(v: float | None) -> str:
    return "  -  " if v is None else f"{v:6.2f}"


def print_scoreboard(results: list[dict], previous: dict | None) -> None:
    metric = "ViSQOL" if have("visqol") else "segSNR"
    prev_map = {}
    if previous:
        prev_map = {(r["clip"], r["kbps"]): primary(r.get("ours")) for r in previous["results"]}
    head = f"{'clip':<28}{'kbps':>5} {'ours':>7}{'lame':>7}{'gap':>7}"
    if previous:
        head += f"{'prev':>7}{'Δ':>7}"
    print(f"\nmetric: {metric} (higher is better)\n{head}\n" + "-" * len(head))
    by_rate: dict[int, list] = {}
    for r in results:
        o, lm = primary(r.get("ours")), primary(r.get("lame"))
        gap = None if o is None or lm is None else o - lm
        line = f"{r['clip'][:27]:<28}{r['kbps']:>5} {fmt(o)} {fmt(lm)} {fmt(gap)}"
        if previous:
            p = prev_map.get((r["clip"], r["kbps"]))
            line += f" {fmt(p)} {fmt(None if p is None or o is None else o - p)}"
        flags = []
        if "error" in r.get("ours", {}):
            flags.append("ENCODE FAILED")
        elif not r.get("ours", {}).get("decode_ok", True):
            flags.append("DECODE ERRORS")
        print(line + ("  ! " + ", ".join(flags) if flags else ""))
        by_rate.setdefault(r["kbps"], []).append((o, lm))
    print("-" * len(head))
    for kbps, rows in sorted(by_rate.items()):
        ours = [o for o, _ in rows if o is not None]
        lame = [lm for _, lm in rows if lm is not None]
        mo = float(np.mean(ours)) if ours else None
        ml = float(np.mean(lame)) if lame else None
        worst = min(ours) if ours else None
        gap = None if mo is None or ml is None else mo - ml
        print(f"{'MEAN':<28}{kbps:>5} {fmt(mo)} {fmt(ml)} {fmt(gap)}   worst(ours): {fmt(worst)}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--corpus", type=Path, default=REPO / "corpus" / "dev")
    ap.add_argument("--bitrates", type=int, nargs="+", default=[128, 192, 320])
    ap.add_argument("--encoder", type=Path, help="path to mp3enc (default: cargo build --release)")
    ap.add_argument("--no-lame", action="store_true", help="skip the LAME baseline")
    ap.add_argument("--ours-args", default="",
                    help='extra arguments for our encoder, e.g. "--shaping psy"')
    ap.add_argument("--keep", type=Path, help="keep encoded MP3s in this directory (e.g. for ABX)")
    ap.add_argument("--compare", help="run ID (or JSON path) to diff against")
    ap.add_argument("--run-id", help="name for this run (default: timestamp + commit)")
    ap.add_argument("--jobs", type=int, default=1,
                    help="clips to encode and score in parallel (ViSQOL is slow)")
    ap.add_argument("--allow-holdout", action="store_true",
                    help="required to evaluate a holdout corpus (human-approved gates only)")
    args = ap.parse_args()

    if not have("ffmpeg") or not have("ffprobe"):
        print("ffmpeg and ffprobe are required", file=sys.stderr)
        return 2
    if "holdout" in args.corpus.parts and not args.allow_holdout:
        print("refusing to evaluate the holdout corpus without --allow-holdout.\n"
              "Holdout runs happen only at human listening gates (docs/EVALUATION.md).",
              file=sys.stderr)
        return 2
    clips = sorted(args.corpus.glob("*.wav"))
    if not clips:
        print(f"no .wav files in {args.corpus} (see corpus/README.md)", file=sys.stderr)
        return 2

    use_lame = not args.no_lame and have("lame")
    if not args.no_lame and not use_lame:
        print("note: lame not found, so there is no baseline column", file=sys.stderr)
    if not have("visqol"):
        print("note: visqol not found, so falling back to segmental SNR (not perceptual!)",
              file=sys.stderr)

    exe = args.encoder.resolve() if args.encoder else build_ours()
    if not exe.exists():
        print(f"encoder not found: {exe}", file=sys.stderr)
        return 2
    commit = git_commit()
    run_id = args.run_id or dt.datetime.now().strftime("%Y%m%d-%H%M%S") + f"-{commit}"
    if args.keep:
        args.keep.mkdir(parents=True, exist_ok=True)

    def evaluate(clip: Path, kbps: int) -> dict:
        # Each task gets its own temp directory: ViSQOL scratch files and
        # encoder outputs must not collide between parallel jobs.
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            sr, ch = probe(clip)
            ref, _ = decode(clip, sr, ch)
            row = {"clip": clip.name, "kbps": kbps, "sample_rate": sr, "channels": ch}
            encoders = [("ours", lambda w, o, k: encode_ours(exe, w, o, k, args.ours_args.split()))]
            if use_lame:
                encoders.append(("lame", encode_lame))
            for name, enc in encoders:
                out = (args.keep or tmp) / f"{clip.stem}_{name}_{kbps}.mp3"
                err = enc(clip, out, kbps)
                if err:
                    row[name] = {"error": err.splitlines()[-1]}
                    continue
                try:
                    row[name] = score(out, ref, sr, ch, tmp)
                except (RuntimeError, ValueError) as e:
                    row[name] = {"error": str(e)}
            print(f"  {clip.name} @ {kbps} kbps done", file=sys.stderr)
            return row

    tasks = [(clip, kbps) for clip in clips for kbps in args.bitrates]
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        results = list(pool.map(lambda t: evaluate(*t), tasks))

    RESULTS.mkdir(parents=True, exist_ok=True)
    report = {
        "run_id": run_id,
        "commit": commit,
        "date": dt.datetime.now().isoformat(timespec="seconds"),
        "corpus": str(args.corpus),
        "visqol": have("visqol"),
        "results": results,
    }
    out_path = RESULTS / f"{run_id}.json"
    out_path.write_text(json.dumps(report, indent=2))

    previous = None
    if args.compare:
        p = Path(args.compare)
        p = p if p.suffix == ".json" else RESULTS / f"{args.compare}.json"
        previous = json.loads(p.read_text())

    print_scoreboard(results, previous)
    print(f"\nsaved {out_path.relative_to(REPO)}")
    failures = [r for r in results if "error" in r.get("ours", {})
                or not r.get("ours", {}).get("decode_ok", True)]
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
