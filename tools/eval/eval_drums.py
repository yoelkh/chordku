"""Scores `chordku-cli drums` against the ground truth of make_drum_set.py.

  python eval_drums.py <chordku-cli.exe> <set_dir> [--tol 35] [--min 25]

Per class: precision / recall / F1 with a +-tol ms window. A ground-truth hit counts as found
when a detected onset within the window reports that class at strength >= --min. Detected
onsets with no ground-truth slot nearby are false positives (also when only "other" drums,
toms / crash / ride, were played there: those are reported separately, not penalised).
"""
import json
import subprocess
import sys
from pathlib import Path


def score(cli: str, wav: Path, tol: int, min_strength: int):
    truth = json.loads(wav.with_suffix("").with_suffix(".hits.json").read_text())
    out = wav.with_suffix(".drums.json")
    subprocess.run([cli, "drums", str(wav), "--json", str(out)], check=True, capture_output=True)
    got = json.loads(out.read_text())["onsets"]
    stats = {c: [0, 0, 0] for c in ("kick", "snare", "hat")}  # tp, fp, fn
    spurious = 0
    matched_det = set()
    for h in truth:
        near = [o for o in got if abs(o["t_ms"] - h["t_ms"]) <= tol]
        for c in stats:
            det = any(o[c] >= min_strength for o in near)
            if h[c] and det:
                stats[c][0] += 1
            elif h[c]:
                stats[c][2] += 1
            elif det and not (h["other"] and c != "kick"):
                stats[c][1] += 1
        for o in near:
            matched_det.add(o["t_ms"])
    for o in got:
        if o["t_ms"] not in matched_det:
            spurious += 1
    return stats, spurious, len(truth), len(got)


def main():
    cli, d = sys.argv[1], Path(sys.argv[2])
    tol = int(sys.argv[sys.argv.index("--tol") + 1]) if "--tol" in sys.argv else 35
    mn = int(sys.argv[sys.argv.index("--min") + 1]) if "--min" in sys.argv else 25
    total = {c: [0, 0, 0] for c in ("kick", "snare", "hat")}
    sp = 0
    for wav in sorted(d.glob("*.wav")):
        stats, spurious, nt, ng = score(cli, wav, tol, mn)
        sp += spurious
        row = []
        for c, (tp, fp, fn) in stats.items():
            total[c][0] += tp
            total[c][1] += fp
            total[c][2] += fn
            p = tp / (tp + fp) if tp + fp else 0
            r = tp / (tp + fn) if tp + fn else 0
            row.append(f"{c} P{p:.2f} R{r:.2f}")
        print(f"{wav.stem:16} slots {nt:4} det {ng:4} spurious {spurious:3} | " + "  ".join(row))
    print("-- overall")
    for c, (tp, fp, fn) in total.items():
        p = tp / (tp + fp) if tp + fp else 0
        r = tp / (tp + fn) if tp + fn else 0
        f = 2 * p * r / (p + r) if p + r else 0
        print(f"{c:6} precision {p:.3f} recall {r:.3f} F1 {f:.3f}")
    print(f"spurious onsets total: {sp}")


if __name__ == "__main__":
    main()
