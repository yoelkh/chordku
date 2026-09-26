"""Renders drum patterns with a GM SoundFont into WAV + ground-truth hit lists, for `chordku-cli
drums` evaluation (tools/eval/eval_drums.py).

  python make_drum_set.py <FluidR3_GM.sf2> <out_dir>

Patterns cover pop / worship grooves at several tempos with velocity variation, open hats,
crashes and toms (which the transcriber may report as something else: they are scored
separately). Output: <name>.wav (44.1 kHz mono) and <name>.hits.json with
[{t_ms, kick, snare, hat, other}] booleans.
"""
import json
import random
import sys
from pathlib import Path

import numpy as np
import soundfile as sf
import tinysoundfont as tsf

RATE = 44100
KICK, SNARE, HAT_CLOSED, HAT_OPEN, HAT_PEDAL, CRASH, RIDE, TOM_LO, TOM_HI = 36, 38, 42, 46, 44, 49, 51, 45, 48

# (name, bpm, bars, steps per beat, pattern per beat-step) -- 'k' kick, 's' snare, 'h' closed hat,
# 'o' open hat, 'c' crash, 't' tom, 'r' ride; uppercase = accented.
PATTERNS = [
    ("pop_8ths_100", 100, 12, 2, ["K h", "h", "S h", "h", "K h", "K h", "S h", "h"]),
    ("pop_16ths_120", 120, 12, 4, ["K h", "h", "h", "h", "S h", "h", "h", "k h", "K h", "h", "h", "h", "S h", "h", "h", "h"]),
    ("ballad_72", 72, 10, 2, ["K h", "h", "S h", "h", "h", "K h", "S h", "h"]),
    ("driving_140", 140, 12, 2, ["K h", "K h", "S h", "h", "K h", "K h", "S o", "h"]),
    ("halftime_90", 90, 10, 4, ["K h", "h", "h", "h", "h", "h", "k h", "h", "S h", "h", "h", "h", "h", "h", "k h", "h"]),
    ("ride_crash_110", 110, 10, 2, ["K c", "r", "S r", "r", "K r", "K r", "S r", "r"]),
    ("fills_100", 100, 8, 4, ["K h", "h", "h", "h", "S h", "h", "h", "h", "K h", "h", "h", "h", "S t", "t", "t", "t"]),
]

NOTE = {"k": KICK, "s": SNARE, "h": HAT_CLOSED, "o": HAT_OPEN, "c": CRASH, "r": RIDE, "t": TOM_LO}
CLASS = {"k": "kick", "s": "snare", "h": "hat", "o": "hat", "c": "other", "r": "other", "t": "other"}


def render(sf2: str, name: str, bpm: float, bars: int, per_beat: int, pattern: list[str], seed: int, out: Path):
    rng = random.Random(seed)
    synth = tsf.Synth(gain=0.0, samplerate=RATE)
    sfid = synth.sfload(sf2)
    synth.program_select(9, sfid, 128, 0)  # channel 10 = GM drums
    synth.start()
    step_s = 60.0 / bpm / per_beat
    steps = pattern
    hits = []
    events = []  # (time_s, note, velocity)
    t = 0.5
    for bar in range(bars):
        for i in range(len(steps)):
            beat = i // per_beat
            drift = rng.uniform(-0.012, 0.012)
            at = t + drift
            hit = {"t_ms": round(at * 1000), "kick": False, "snare": False, "hat": False, "other": False}
            any_hit = False
            for token in steps[i].split():
                accent = token.isupper()
                key = token.lower()
                if key == "h" and rng.random() < 0.06:
                    continue  # the odd dropped hat
                vel = rng.randint(96, 118) if accent else rng.randint(60, 92)
                if key == "h":
                    vel = rng.randint(45, 80)
                events.append((at, NOTE[key], vel))
                hit[CLASS[key]] = True
                any_hit = True
            if any_hit:
                hits.append(hit)
            t += step_s
    total = t + 1.5
    # Render by feeding note events in time order.
    n = int(total * RATE)
    audio = np.zeros(n, dtype=np.float32)
    events.sort()
    pos = 0
    ei = 0
    block = 512
    while pos < n:
        now = pos / RATE
        while ei < len(events) and events[ei][0] <= now:
            _, note, vel = events[ei]
            synth.noteon(9, note, vel)
            ei += 1
        chunk = synth.generate(block)
        arr = np.frombuffer(chunk, dtype=np.float32).reshape(-1, 2).mean(axis=1)
        end = min(pos + len(arr), n)
        audio[pos:end] = arr[: end - pos]
        pos += block
    peak = np.abs(audio).max() or 1.0
    audio = audio / peak * 0.8
    sf.write(out / f"{name}.wav", audio, RATE, subtype="PCM_16")
    (out / f"{name}.hits.json").write_text(json.dumps(hits))
    print(f"{name}: {len(hits)} hit slots, {total:.0f} s")


def main():
    sf2, out = sys.argv[1], Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    for i, (name, bpm, bars, per_beat, pattern) in enumerate(PATTERNS):
        render(sf2, name, bpm, bars, per_beat, pattern, seed=i, out=out)


if __name__ == "__main__":
    main()
