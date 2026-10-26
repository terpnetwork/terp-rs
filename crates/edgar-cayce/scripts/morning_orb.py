#!/usr/bin/env python3
"""Morning board clip: teal shield disc, glow follows speech amplitude.

Matches scripts/terp-orb.svg (#0c1210 field, #98e8c1 disc). The glow uses a
slow release, a faster attack, and a small sine so it does not twitch.
Writes an H.264 MP4. Exits non-zero if ffmpeg or a speech tool is missing.
"""

import argparse
import math
import shutil
import subprocess
import sys
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

W, H, FPS = 320, 180, 8
BG = (12, 18, 16)
DISC = (152, 232, 193)
MAX_SECONDS = 45


def speech_wav(text: str, wav: Path) -> None:
    text = " ".join(text.split())[:900] or "Terp morning board. No chain changes since yesterday."
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


def frames(amps):
    cx, cy = W / 2, H / 2
    dist = []
    for y in range(H):
        for x in range(W):
            dist.append(math.hypot(x - cx, y - cy))
    disc_r = 28.0
    for amp in amps:
        glow = 18.0 + amp * 62.0
        buf = bytearray(W * H * 3)
        offset = 0
        for d in dist:
            if d <= disc_r:
                shade = 0.82 + 0.18 * (1.0 - d / disc_r)
                rgb = tuple(min(255, int(channel * shade)) for channel in DISC)
            else:
                fade = max(0.0, 1.0 - (d - disc_r) / glow)
                fade = fade * fade * (0.25 + 0.75 * amp)
                rgb = tuple(
                    int(bg + (fg - bg) * fade) for bg, fg in zip(BG, DISC)
                )
            buf[offset : offset + 3] = bytes(rgb)
            offset += 3
        yield buf


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
    with raw.open("wb") as handle:
        for frame in frames(amps):
            handle.write(frame)
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
        str(raw),
        "-i",
        str(wav),
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-crf",
        "30",
        "-c:a",
        "aac",
        "-b:a",
        "64k",
        "-shortest",
        "-movflags",
        "+faststart",
        str(out),
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True)
    wav.unlink(missing_ok=True)
    raw.unlink(missing_ok=True)
    if proc.returncode != 0 or not out.is_file():
        sys.stderr.write(proc.stderr[-800:])
        raise SystemExit(proc.returncode or 1)
    if out.stat().st_size > 8 * 1024 * 1024:
        out.unlink()
        raise SystemExit("mp4 over 8MB")


if __name__ == "__main__":
    main()
