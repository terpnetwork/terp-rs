#!/usr/bin/env python3
"""Morning board clip: teal shield disc, glow follows speech amplitude.

Matches scripts/terp-orb.svg (#0c1210 field, #98e8c1 disc). The glow uses a
slow release, a faster attack, and a small sine so it does not twitch.
Writes an H.264 MP4. Exits non-zero if ffmpeg or a speech tool is missing.
"""

import argparse
import math
import os
import shutil
import subprocess
import sys
import time
import wave
from array import array
from pathlib import Path
from typing import Optional

TOOL_DIRS = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]


def tool(name: str) -> Optional[str]:
    found = shutil.which(name)
    if found:
        return found
    for directory in TOOL_DIRS:
        candidate = Path(directory) / name
        if candidate.is_file():
            return str(candidate)
    return None

# 720p, four times the old 320x180 board. Glow levels are drawn once and reused.
W, H, FPS = 1280, 720, 24
BG = (12, 18, 16)
DISC = (152, 232, 193)
HALO = (152, 232, 193)
MAX_SECONDS = 90
GLOW_LEVELS = 40


def kokoro_wav(text: str, wav: Path) -> bool:
    """Warm Kokoro queue used by the herdr sayit plugin (~/.sayit-queue)."""
    queue = Path.home() / ".sayit-queue"
    inbox = queue / "in"
    outdir = queue / "out"
    if not inbox.is_dir():
        return False
    jid = f"job-edgar{os.getpid()}"
    src = inbox / f"{jid}.txt"
    dest = outdir / f"{jid}.wav"
    try:
        src.write_text(text, encoding="utf-8")
    except OSError:
        return False
    deadline = time.time() + 90
    while time.time() < deadline:
        if dest.is_file() and dest.stat().st_size > 44:
            shutil.move(str(dest), wav)
            return wav.is_file() and wav.stat().st_size > 44
        time.sleep(0.5)
    src.unlink(missing_ok=True)
    return False


def speech_wav(text: str, wav: Path) -> None:
    text = " ".join(text.split())[:1900] or "Terp morning board. No chain changes since yesterday."
    if kokoro_wav(text, wav):
        return
    attempts = []
    speak = tool("espeak-ng")
    if speak:
        attempts.append([speak, "-w", str(wav), "-s", "150", text])
    say = tool("say")
    if say:
        attempts.append(
            [
                say,
                "-o",
                str(wav),
                "--file-format=WAVE",
                "--data-format=LEI16@22050",
                text,
            ]
        )
    errors = []
    for cmd in attempts:
        try:
            proc = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        except (FileNotFoundError, subprocess.TimeoutExpired) as exc:
            errors.append(str(exc))
            continue
        if proc.returncode == 0 and wav.is_file() and wav.stat().st_size > 44:
            return
        errors.append(proc.stderr.strip() or f"exit {proc.returncode}")
    raise SystemExit("speech tool failed: " + "; ".join(errors))


def load_pcm(wav: Path):
    with wave.open(str(wav), "rb") as handle:
        rate = handle.getframerate()
        channels = handle.getnchannels()
        width = handle.getsampwidth()
        raw = handle.readframes(handle.getnframes())
    if width != 2:
        raise SystemExit(f"expected 16-bit wav, got width {width}")
    samples = array("h")
    samples.frombytes(raw[: len(raw) - (len(raw) % 2)])
    if sys.byteorder != "little":
        samples.byteswap()
    if channels > 1:
        samples = array("h", samples[::channels])
    cap = rate * MAX_SECONDS
    return rate, samples[:cap]


def envelope(samples, rate: int):
    hop = max(1, rate // FPS)
    amps = []
    for start in range(0, len(samples), hop):
        chunk = samples[start : start + hop]
        if not chunk:
            break
        energy = sum(sample * sample for sample in chunk) / len(chunk)
        amps.append(min(1.0, math.sqrt(energy) / 6000.0))
    level = 0.0
    smooth = []
    for index, amp in enumerate(amps):
        rate_k = 0.35 if amp > level else 0.08
        level += (amp - level) * rate_k
        wobble = 0.04 * math.sin(index * 0.37) + 0.02 * math.sin(index * 0.11 + 1.2)
        smooth.append(max(0.06, min(1.0, level * (1.0 + wobble))))
    return smooth or [0.2]


def paint(amp: float, dist) -> bytes:
    """One antialiased frame. The disc edge blends over about two pixels."""
    disc_r = 112.0
    glow = 72.0 + amp * 248.0
    buf = bytearray(W * H * 3)
    offset = 0
    edge = 2.0
    for d in dist:
        if d <= disc_r - edge:
            shade = 0.86 + 0.14 * (1.0 - d / disc_r)
            rgb = tuple(min(255, int(channel * shade)) for channel in DISC)
        elif d <= disc_r + edge:
            t = (d - (disc_r - edge)) / (2.0 * edge)
            t = t * t * (3.0 - 2.0 * t)
            rgb = tuple(int(bg + (fg - bg) * (1.0 - t)) for bg, fg in zip(BG, DISC))
        else:
            fade = max(0.0, 1.0 - (d - disc_r) / glow)
            fade = fade * fade * (0.35 + 0.65 * amp)
            rgb = tuple(int(bg + (fg - bg) * fade) for bg, fg in zip(BG, HALO))
        buf[offset : offset + 3] = bytes(rgb)
        offset += 3
    return bytes(buf)


def glow_bank(dist):
    return [paint(i / (GLOW_LEVELS - 1), dist) for i in range(GLOW_LEVELS)]


def distances():
    cx, cy = (W - 1) / 2, (H - 1) / 2
    return [math.hypot(x - cx, y - cy) for y in range(H) for x in range(W)]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    text = sys.stdin.read()
    out = Path(args.out)
    wav = out.with_suffix(".wav")
    raw = out.with_suffix(".rgb")
    speech_wav(text, wav)
    rate, samples = load_pcm(wav)
    amps = envelope(samples, rate)
    bank = glow_bank(distances())
    ffmpeg = tool("ffmpeg")
    if ffmpeg is None:
        raise SystemExit("ffmpeg not found")
    cmd = [
        ffmpeg,
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "-s",
        f"{W}x{H}",
        "-r",
        str(FPS),
        "-i",
        "pipe:0",
        "-i",
        str(wav),
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-pix_fmt",
        "yuv420p",
        "-crf",
        "18",
        "-profile:v",
        "high",
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        "-shortest",
        "-movflags",
        "+faststart",
        str(out),
    ]
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    assert proc.stdin is not None
    try:
        for amp in amps:
            slot = min(GLOW_LEVELS - 1, int(amp * (GLOW_LEVELS - 1) + 0.5))
            proc.stdin.write(bank[slot])
    finally:
        proc.stdin.close()
    stderr = proc.stderr.read().decode("utf-8", "replace") if proc.stderr else ""
    code = proc.wait()
    wav.unlink(missing_ok=True)
    raw.unlink(missing_ok=True)
    if code != 0 or not out.is_file():
        sys.stderr.write(stderr[-800:])
        raise SystemExit(code or 1)
    if out.stat().st_size > 8 * 1024 * 1024:
        out.unlink()
        raise SystemExit("mp4 over 8MB")


if __name__ == "__main__":
    main()
