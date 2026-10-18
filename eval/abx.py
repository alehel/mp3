#!/usr/bin/env python3
"""Blind ABX listening test. Results are logged to listening/log.csv.

Transparency test (encode vs. original):
    python3 eval/abx.py run --ref corpus/dev/castanets.wav --a out/castanets_ours_192.mp3

Comparative test (ours vs. LAME):
    python3 eval/abx.py run --ref corpus/dev/castanets.wav \\
        --a out/castanets_ours_128.mp3 --b out/castanets_lame_128.mp3

p-value for results from another tool (e.g. foobar2000 ABX):
    python3 eval/abx.py pvalue --trials 16 --correct 12

Playback uses ffplay. Set the volume once before starting and leave it alone.
The trial count is fixed up front, and the score is hidden until the end, so you
can't stop early on a lucky streak. See docs/LISTENING.md.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import math
import secrets
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import REPO, align, decode, git_commit, have, probe, write_wav_f32  # noqa: E402

LOG = REPO / "listening" / "log.csv"
FIELDS = ["date", "listener", "commit", "clip", "bitrate", "test", "a", "b",
          "trials", "correct", "p_value", "artifacts", "notes"]
ARTIFACTS = ["pre-echo", "warbling", "swirling", "dullness", "graininess",
             "stereo-collapse", "ringing", "clicks"]


def p_value(trials: int, correct: int) -> float:
    """One-sided binomial P(X >= correct) under guessing (p = 0.5)."""
    return sum(math.comb(trials, k) for k in range(correct, trials + 1)) / 2 ** trials


def play(path: Path) -> None:
    subprocess.run(["ffplay", "-nodisp", "-autoexit", "-loglevel", "quiet", str(path)],
                   check=False)


def prepare(ref_path: Path, a_path: Path, b_path: Path | None, start: float,
            duration: float | None, tmp: Path) -> tuple[Path, Path]:
    """Decode, align to the reference, cut the segment, and write A/B as float WAVs."""
    sr, ch = probe(ref_path)
    ref, _ = decode(ref_path, sr, ch)
    a, _ = decode(a_path, sr, ch)
    _, a, _ = align(ref, a)
    if b_path is None:
        b = ref  # transparency test: B is the original
    else:
        b, _ = decode(b_path, sr, ch)
        _, b, _ = align(ref, b)
    n = min(len(a), len(b))
    s = int(start * sr)
    e = n if duration is None else min(n, s + int(duration * sr))
    if e - s < sr // 4:
        sys.exit("segment is too short (check --start/--duration)")
    pa, pb = tmp / "A.wav", tmp / "B.wav"
    # 10 ms fades avoid clicks at the cut points, which would be an obvious tell.
    fade = min(int(0.01 * sr), (e - s) // 4)
    ramp = np.linspace(0.0, 1.0, fade, dtype="float32")[:, None]
    for arr, path in ((a, pa), (b, pb)):
        seg = arr[s:e].copy()
        seg[:fade] *= ramp
        seg[-fade:] *= ramp[::-1]
        write_wav_f32(path, seg, sr)
    return pa, pb


def ask_artifacts() -> str:
    print("\nAny artifacts heard? Comma-separated, from:", ", ".join(ARTIFACTS))
    raw = input("artifacts (blank for none): ").strip()
    words = [w.strip().lower() for w in raw.split(",") if w.strip()]
    unknown = [w for w in words if w not in ARTIFACTS]
    if unknown:
        print(f"  (keeping non-standard terms as-is: {', '.join(unknown)})")
    return ";".join(words)


def log_row(row: dict) -> None:
    LOG.parent.mkdir(parents=True, exist_ok=True)
    new = not LOG.exists() or LOG.stat().st_size == 0
    with LOG.open("a", newline="") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS)
        if new:
            w.writeheader()
        w.writerow(row)


def cmd_run(args: argparse.Namespace) -> int:
    if not have("ffplay"):
        print("ffplay is required for playback (it ships with ffmpeg)", file=sys.stderr)
        return 2
    listener = args.listener or input("listener name: ").strip() or "anonymous"
    with tempfile.TemporaryDirectory() as td:
        pa, pb = prepare(args.ref, args.a, args.b, args.start, args.duration, Path(td))
        b_label = args.b.name if args.b else f"{args.ref.name} (original)"
        print(f"\nA = {args.a.name}\nB = {b_label}\n")
        print("Commands: a / b / x = play,  1 = X is A,  2 = X is B,  q = abort (nothing logged)")
        print(f"{args.trials} trials. Your score is shown at the end.\n")

        correct = 0
        for t in range(1, args.trials + 1):
            x_is_a = secrets.randbelow(2) == 0
            px = pa if x_is_a else pb
            while True:
                c = input(f"trial {t}/{args.trials} > ").strip().lower()
                if c == "a":
                    play(pa)
                elif c == "b":
                    play(pb)
                elif c == "x":
                    play(px)
                elif c in ("1", "2"):
                    correct += int((c == "1") == x_is_a)
                    break
                elif c == "q":
                    print("aborted; nothing logged")
                    return 1
                else:
                    print("  a / b / x to play, 1 or 2 to answer, q to abort")

    p = p_value(args.trials, correct)
    verdict = ("difference heard" if p <= 0.05 else
               "inconclusive" if p <= 0.25 else "no evidence of a difference")
    print(f"\nresult: {correct}/{args.trials} correct, p = {p:.4f} ({verdict})")

    artifacts = ask_artifacts()
    notes = input("notes (timestamps, description): ").strip()
    log_row({
        "date": dt.date.today().isoformat(),
        "listener": listener,
        "commit": git_commit(),
        "clip": args.clip or args.ref.name,
        "bitrate": args.bitrate or "",
        "test": "abx-original" if args.b is None else args.test,
        "a": args.a.name,
        "b": args.b.name if args.b else "original",
        "trials": args.trials,
        "correct": correct,
        "p_value": f"{p:.4f}",
        "artifacts": artifacts,
        "notes": notes,
    })
    print(f"logged to {LOG.relative_to(REPO)}")
    return 0


def cmd_pvalue(args: argparse.Namespace) -> int:
    print(f"{args.correct}/{args.trials}: p = {p_value(args.trials, args.correct):.4f}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    r = sub.add_parser("run", help="run an interactive ABX session")
    r.add_argument("--ref", type=Path, required=True, help="original WAV")
    r.add_argument("--a", type=Path, required=True, help="first candidate (e.g. our MP3)")
    r.add_argument("--b", type=Path, help="second candidate; omit to compare against the original")
    r.add_argument("--trials", type=int, default=16)
    r.add_argument("--start", type=float, default=0.0, help="segment start (seconds)")
    r.add_argument("--duration", type=float, help="segment length (seconds)")
    r.add_argument("--listener")
    r.add_argument("--clip", help="clip name for the log (default: ref filename)")
    r.add_argument("--bitrate", help="bitrate for the log, e.g. 128")
    r.add_argument("--test", default="abx-lame", help="test label when --b is given")
    r.set_defaults(func=cmd_run)

    p = sub.add_parser("pvalue", help="binomial p-value for an ABX result")
    p.add_argument("--trials", type=int, required=True)
    p.add_argument("--correct", type=int, required=True)
    p.set_defaults(func=cmd_pvalue)

    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
