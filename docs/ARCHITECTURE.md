# Architecture

ChordKu is one Rust binary that serves a web UI on `127.0.0.1` and does all analysis locally.
No audio leaves the machine; Python is used only once, at development time, to export a model.

```
 URL ──► yt-dlp (managed exe) ─┐
 file upload ──────────────────┼─► source file ─► decode ─┬─► FAST LANE  (seconds)
 microphone (MediaRecorder) ───┘   symphonia + libopus     │    22.05 kHz mono
                                   (ffmpeg fallback)       │    ├─ tuning estimate
                                                           │    ├─ Beat This!  ─┐ (parallel)
                                                           │    ├─ CQT + BTC   ─┤
                                                           │    └─ chart: Viterbi, beat sync, key, sections
                                                           │    └─ mix.opus (playback timeline)
                                                           └─► SLOW LANE  (minutes, on request)
                                                                44.1 kHz stereo ─► htdemucs, 7.8 s segments
                                                                ─► 4 stereo stems + 2 mono (phone) as Ogg Opus
```

## Crates

| Crate | Role |
|---|---|
| `chordku-core` | decode, resample, tuning, CQT, BTC, beats, Viterbi, key, sections, chart, separation DSP, Opus writer, WCSR evaluation |
| `chordku-server` | axum API, SQLite library, job worker, yt-dlp and model management, LAN mode, embedded UI |
| `chordku-cli` | `analyze`, `chords`, `beats`, `separate`, `eval` on the same core, for tests and benchmarks |
| `web/` | Svelte 5 UI; all view transforms (transpose, capo, spelling, Nashville, exports) are client-side |

## Decisions that are not obvious from the code

**CQT in Rust instead of inside the ONNX graph.** A convolutional CQT in the graph costs about
1 GMAC per 10 s window. One real FFT per frame plus a sparse frequency-domain product is ~50x
cheaper, and it is the same mathematics. Parity with `librosa.cqt` is tested: mean log difference
0.0001 on music, 99.3 % identical BTC labels (`tools/export/proto_decomp.py`).

**Overlapped BTC instances.** BTC was trained on 10 s instances cut every 5 s, each with its own
CQT. The reference script runs hard 10 s blocks, which makes its output depend on where the blocks
fall: shifted by 2.5 s it agrees with itself on only 79.8 % of frames. ChordKu feeds instances 5 s
apart and averages log-probabilities with a triangular weight (88.3 % under the same shift).

**One inference thread, two lanes.** Two models at once would fight for the same cores. A
separation yields between segments, so adding a song never waits for another song's stems.
ONNX Runtime's arena allocator is disabled for the separator (4.1 GB peak becomes 2.5 GB).

**Tempo by `playbackRate`, pitch by one shifter on the bus.** Every stem source gets the same
native playback rate (free, sample-locked). A single signalsmith-stretch node on the mixed bus
shifts by `transpose - 12*log2(rate)`. At rate 1 without transposition it is bypassed entirely,
and its WASM is not even downloaded.

**Chart data in three layers.** `analysis` (immutable, pitch classes, no spelling), `edits`
(time-ranged overlay that survives re-analysis), `view_prefs`. Transposition and capo are separate:
transposing moves the sound, a capo only changes the printed shapes.

**Stems as Ogg Opus at 44.1 kHz-derived 48 kHz.** Four stems of a four minute song are ~12 MB
instead of ~160 MB of FLAC. Phones get two mono stems (vocals, backing) because four decoded
stereo buffers would not fit in memory.

## Security model

- Loopback listener: plain HTTP on 127.0.0.1. `Host` allow-list (DNS rebinding), `Origin` check on
  state-changing requests, strict CSP, ids and file names validated against allow-lists; client
  file names never become paths.
- URL ingest: http(s) only, public addresses only (resolved before use), passed to yt-dlp after
  `--`, playlists and live streams refused, output path chosen by the server.
- LAN listener (opt-in): see `LAN.md`.

## Performance on the development laptop (Core Ultra 5 125H, CPU only)

| Step | 4 min 17 s song |
|---|---|
| decode MP3 | 0.3 s |
| beats (Beat This! small) | 5.3 s |
| chords (CQT + BTC, in parallel with beats) | 1.2 s |
| whole fast lane including model load | 6.9 s |
| separation (htdemucs), RTF 0.38 | ~100 s, 2.5 GB peak |

## Setlist viewer (static)

`web/viewer.html` is a second Vite entry built into one self-contained file (`scripts/inline-viewer.mjs`
inlines the script and stylesheet; `vite.viewer.config.ts`). It reuses the chart, theory, diagram,
toolbar (`ViewControls`) and transport components; the time source is either the YouTube IFrame API
(`viewer/clock.ts`, interpolated between `getCurrentTime` readings) or a silent clock. The server
embeds a setlist into the template at `GET /api/setlists/{id}/share.html` (`routes/setlists.rs`)
as a `<script type="application/json">` block and serves it as a download; `share.json` is the
same data for development. Readers' prefs live in `localStorage`, keyed by setlist and song.
`tools/publish-setlist.ps1` pushes such a file to a GitHub Pages repository.
