"""Shared helpers for the eval and listening tools.

External tools are invoked as separate processes only. They are never linked
or vendored (CLEANROOM.md §5).
"""

from __future__ import annotations

import json
import shutil
import struct
import subprocess
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parent.parent


def have(tool: str) -> bool:
    return shutil.which(tool) is not None


def run(cmd: list[str], check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, check=check)


def probe(path: Path) -> tuple[int, int]:
    """Return (sample_rate, channels) of the first audio stream."""
    out = run([
        "ffprobe", "-v", "error", "-select_streams", "a:0",
        "-show_entries", "stream=sample_rate,channels", "-of", "json", str(path),
    ]).stdout
    s = json.loads(out)["streams"][0]
    return int(s["sample_rate"]), int(s["channels"])


def decode(path: Path, sample_rate: int, channels: int) -> tuple[np.ndarray, str]:
    """Decode any audio file to float32 of shape (frames, channels).

    Returns (audio, ffmpeg_stderr). Non-empty stderr at `-v error` means the
    decoder complained, which counts as a validity failure for encoder output.
    """
    proc = subprocess.run(
        ["ffmpeg", "-v", "error", "-nostdin", "-i", str(path),
         "-f", "f32le", "-acodec", "pcm_f32le",
         "-ac", str(channels), "-ar", str(sample_rate), "-"],
        capture_output=True, check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"ffmpeg failed on {path}: {proc.stderr.decode(errors='replace')}")
    audio = np.frombuffer(proc.stdout, dtype="<f4").reshape(-1, channels)
    return audio, proc.stderr.decode(errors="replace").strip()


def align(ref: np.ndarray, deg: np.ndarray, max_lag: int = 4096) -> tuple[np.ndarray, np.ndarray, int]:
    """Align `deg` to `ref` by cross-correlating mono downmixes.

    MP3 encode/decode adds delay (encoder delay + decoder delay), so outputs
    must be aligned before any comparison. Returns (ref_trim, deg_trim, lag),
    where a positive lag means `deg` is delayed relative to `ref`.
    """
    n = min(len(ref), len(deg), 48_000 * 10)
    if n < 2048:
        m = min(len(ref), len(deg))
        return ref[:m], deg[:m], 0
    a = ref[:n].mean(axis=1).astype(np.float64)
    b = deg[:n].mean(axis=1).astype(np.float64)
    size = 1 << int(np.ceil(np.log2(2 * n)))
    corr = np.fft.irfft(np.fft.rfft(b, size) * np.conj(np.fft.rfft(a, size)), size)
    lags = np.concatenate([np.arange(0, max_lag + 1), np.arange(-max_lag, 0)])
    vals = np.concatenate([corr[: max_lag + 1], corr[-max_lag:]])
    lag = int(lags[np.argmax(vals)])
    if lag >= 0:
        d, r = deg[lag:], ref
    else:
        d, r = deg, ref[-lag:]
    m = min(len(r), len(d))
    return r[:m], d[:m], lag


def write_wav_f32(path: Path, audio: np.ndarray, sample_rate: int) -> None:
    """Write float32 WAV (format tag 3)."""
    audio = np.ascontiguousarray(audio, dtype="<f4")
    frames, channels = audio.shape
    data = audio.tobytes()
    block = channels * 4
    header = b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVE"
    header += b"fmt " + struct.pack("<IHHIIHH", 16, 3, channels, sample_rate,
                                    sample_rate * block, block, 32)
    header += b"data" + struct.pack("<I", len(data))
    path.write_bytes(header + data)


def segmental_snr(ref: np.ndarray, deg: np.ndarray, frame: int = 1024) -> float:
    """Segmental SNR in dB, per-frame clamped to [-10, 35], silent frames skipped.

    A sanity check for bitstream and DSP bugs (M1–M4). It is NOT a perceptual metric.
    """
    vals = []
    for start in range(0, len(ref) - frame, frame):
        r = ref[start:start + frame].astype(np.float64)
        e = r - deg[start:start + frame]
        sig = float(np.sum(r * r))
        if sig < 1e-6 * frame:
            continue
        noise = float(np.sum(e * e)) + 1e-12
        vals.append(min(35.0, max(-10.0, 10 * np.log10(sig / noise))))
    return float(np.mean(vals)) if vals else float("nan")


def git_commit() -> str:
    try:
        return run(["git", "-C", str(REPO), "rev-parse", "--short", "HEAD"]).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "unknown"


def pre_echo_db(ref: np.ndarray, deg: np.ndarray, sample_rate: int) -> float | None:
    """Mean coding noise in the 10 ms before each attack, relative to the
    attack's own energy over the next 10 ms, in dB (lower is better).

    Attacks are 128-sample frames of the mono reference whose energy jumps by
    more than 15 dB over the preceding ~20 ms; one per 60 ms at most. A
    targeted proxy for pre-echo, which global metrics like ViSQOL barely
    register. Returns None for clips without attacks.
    """
    x = ref.mean(axis=1).astype(np.float64)
    y = deg.mean(axis=1).astype(np.float64)
    frame, pre = 128, int(0.010 * sample_rate)
    e = np.array([np.sum(x[i:i + frame] ** 2) for i in range(0, len(x) - frame, frame)]) + 1e-12
    onsets: list[int] = []
    for k in range(8, len(e)):
        if 10 * np.log10(e[k] / np.mean(e[k - 7:k])) > 15 and (not onsets or k - onsets[-1] > 20):
            onsets.append(k)
    vals = []
    for k in onsets:
        s = k * frame
        if s < pre or s + pre > len(x):
            continue
        noise = np.sum((y[s - pre:s] - x[s - pre:s]) ** 2)
        sig = np.sum(x[s:s + pre] ** 2)
        vals.append(10 * np.log10(noise / sig + 1e-20))
    return float(np.mean(vals)) if vals else None
