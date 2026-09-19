"""Prepare GuitarSet (Xi et al., ISMIR 2018, CC BY 4.0, zenodo.org/records/3371780) for
`chordku-cli eval`: writes `<id>_mic.lab` next to each comping recording.

  python guitarset_to_lab.py <annotation_dir> <audio_mono-mic_dir> [instructed|performed]

Only the comping takes are used (the solo takes are single-note lines). GuitarSet carries
two chord annotations per take: the lead-sheet chords the player was given ("instructed",
default, what a chart should say) and chords inferred from the notes actually played.
"""
import json
import sys
from pathlib import Path

ann_dir, audio_dir = Path(sys.argv[1]), Path(sys.argv[2])
which = sys.argv[3] if len(sys.argv) > 3 else "instructed"
index = {"instructed": 0, "performed": 1}[which]
written = 0
for jams_path in sorted(ann_dir.rglob("*_comp.jams")):
    audio = audio_dir / f"{jams_path.stem}_mic.wav"
    if not audio.exists():
        hits = list(audio_dir.rglob(audio.name))
        if not hits:
            continue
        audio = hits[0]
    jams = json.loads(jams_path.read_text())
    chords = [a for a in jams["annotations"] if a["namespace"] == "chord"]
    if len(chords) <= index:
        continue
    rows = [(d["time"], d["time"] + d["duration"], d["value"]) for d in chords[index]["data"]]
    audio.with_suffix(".lab").write_text("".join(f"{a:.6f}\t{b:.6f}\t{v}\n" for a, b, v in rows))
    written += 1
print(f"{written} .lab files written ({which} chords)")
