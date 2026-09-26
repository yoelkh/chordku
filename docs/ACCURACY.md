# Accuracy

What is measured, on what, and what is not known. All numbers are weighted chord symbol recall
(WCSR, the MIREX measure, implemented after `mir_eval.chord` in `chordku-core/src/eval.rs`):
the share of time the estimate matches the reference, at three levels of detail.

## GuitarSet (real guitars, 91 minutes)

GuitarSet (Xi et al., ISMIR 2018, CC BY 4.0): 180 comping takes by six players in five styles,
microphone recordings, scored against the lead-sheet chords the players were given.

```
python tools/eval/guitarset_to_lab.py <annotation> <audio_mono-mic>
chordku-cli eval <audio_mono-mic> --json result.json
```

| System | root | maj/min | sevenths |
|---|---|---|---|
| frame-wise argmax of the averaged BTC posteriors | 81.1 | 77.4 | 66.1 |
| **ChordKu chart** (sticky Viterbi + beat-synchronous chords) | **82.9** | **79.2** | **67.6** |

| Style (36 takes each, unweighted mean) | root | maj/min | sevenths |
|---|---|---|---|
| Singer-songwriter | 95.2 | 91.8 | 83.9 |
| Rock | 94.3 | 90.1 | 84.4 |
| Bossa nova | 82.9 | 80.8 | 64.3 |
| Jazz | 69.5 | 68.6 | 49.1 |
| Funk | 67.1 | 59.6 | 46.5 |

(Final settings, `target/eval/guitarset_final.json`; the tuning correction fired on 6 of 180 takes.)

Reading: the beat-synchronous chart is worth about two points over raw frames. Pop, rock and
singer-songwriter material, which is what BTC was trained on, lands around 90 %. Dense jazz
voicings and percussive, heavily muted funk strumming are hard; a solo guitar is also out of
distribution for a model trained on full band mixes, so these figures say more about the
microphone use case than about produced songs. For reference, BTC's authors report 82.7 % maj/min
and 80.8 % MIREX on their pop test sets with the same weights.

## Tuning correction: a controlled experiment

The same first 50 takes, artificially detuned by resampling, scored with and without the
correction (`chordku-cli eval --detune <cents> [--no-tuning]`):

| Input | correction | root | maj/min | sevenths |
|---|---|---|---|---|
| as recorded | off | 88.6 | 87.0 | 77.3 |
| +30 cents | off | 82.0 | 79.3 | 73.0 |
| +30 cents | **on** | **88.5** | **87.6** | **76.8** |
| -40 cents | off | 80.4 | 79.6 | 72.8 |
| -40 cents | **on** | **85.4** | **85.6** | **75.2** |

A recording a third of a semitone off costs eight points; measuring the offset and folding it
into the analysis resampler wins them back at no extra cost.

Two further findings set the threshold. In-tune guitars measure **+8 cents** (sd 2.9, independent
of the frequency band analysed): a plucked string is sharp at its attack, and the chord model was
trained on exactly such audio. And correcting offsets of 8-12 cents changes nothing (79.17 vs
79.22 over all 180 takes; 20 takes better, 21 worse). ChordKu therefore corrects only offsets of
15 cents or more, and only when the estimate is confident.

## Overlapped inference: stability

BTC labels depend on where its 10 s windows fall. Shifting a song by 2.5 s and comparing the
labels with the unshifted run (`tools/export/compare_lab.py`):

| | exact label | maj/min | root |
|---|---|---|---|
| reference inference (hard 10 s blocks) | 79.8 % | 89.7 % | 92.1 % |
| ChordKu (instances every 5 s, triangular log-prob averaging) | 88.3 % | 94.2 % | 95.8 % |

## Front-end and model parity

See `MODELS.md`: CQT 99.3 % identical BTC labels against librosa, ONNX logits within 1.5e-5 of
PyTorch, htdemucs 73.5-100 dB SNR against PyTorch Demucs.

## A retrained BTC, measured and not adopted

BTC-PL (ChordMini, MIT) is the same network retrained with pseudo-labels and continual
learning; its authors report +1.2 maj/min and +3.1 sevenths over the original weights on their
test set. It is a drop-in file (`tools/export/export_btc.py --weights btc_model_best.pth`), so
it was run through the same 180 takes:

| chart WCSR | root | maj/min | sevenths |
|---|---|---|---|
| BTC (in use) | 82.9 | **79.2** | **67.6** |
| BTC-PL | **83.7** | 79.1 | 65.5 |

Rock and singer-songwriter are unchanged (90.1 / 91.0 against 90.1 / 91.8 maj/min); the
sevenths drop comes from bossa nova, funk and jazz (-3 to -5). On this material the original
weights stay.

## What is not measured

- **Full band mixes with ground truth.** The standard sets (Isophonics, Billboard, RWC, USPop) do
  not come with audio that may be redistributed. Put your own `<name>.lab` files next to the
  audio and run `chordku-cli eval`; the format is `start end label` in Harte syntax.
- **Indonesian repertoire.** No chord model has published results on it. Harmonically it is close
  to the Western pop the model knows, which is an expectation, not a measurement.
- **"Refine chords with stems".** Feeding bare stems to a recogniser is known to hurt; re-weighting
  the mix (vocals and drums down, accompaniment up) helped BTC in one 2025 study. The remix code
  exists (`separate/remix.rs`) but the feature is not exposed, because without annotated full mixes
  its benefit cannot be verified here. The rule stays: ship it only if it gains at least one point.
- **Beat tracking accuracy** beyond the synthetic test; Beat This! small reports beat F1 of about
  88 % on GTZAN in its paper.

## Drum tab

`chordku-core/src/drums.rs` reads kick, snare and hi-hat hits from the separated drum stem with
band-wise spectral flux (four bands, per-band peak picking, rules for which drum a rise
belongs to). Scored with `tools/eval/eval_drums.py` on `tools/eval/make_drum_set.py`: seven
pop / worship grooves rendered from the FluidR3 GM SoundFont at 72-140 BPM with velocity
variation, 801 hit slots, +-35 ms tolerance:

| drum | precision | recall | F1 |
|---|---|---|---|
| kick | 0.91 | 0.97 | 0.94 |
| snare | 0.95 | 1.00 | 0.97 |
| hat | 0.97 | 0.83 | 0.90 |

Toms read as kicks and crashes / rides as hi-hats, which is why the app calls the tab an
estimate. Missed hi-hats are mostly the ones under a snare; the client completes a bar's hat
pattern when most of its eighths (or sixteenths) were heard, and draws those dimmer. On real
stems (two library songs) the hit rates are 0.9-1.3 kicks and 1.0-1.2 snares per beat and the
inter-onset intervals sit on 16th multiples, which is consistent with the grooves; there is
no ground truth for them.
