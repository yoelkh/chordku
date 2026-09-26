// Playback engine.
//
//   stem buffers -> one AudioBufferSourceNode each (started at the same context time, so
//   they are sample-locked by construction) -> per-stem gain -> bus -> [pitch shifter] -> out
//
// Tempo is the native `playbackRate`, identical on every source, which costs nothing and
// cannot drift. That alone would change the pitch, so ONE signalsmith-stretch node on the
// mixed bus shifts it back, plus the transposition the user asked for:
//     semitones = transpose - 12 * log2(rate)
// At rate 1 and no transposition the shifter is bypassed entirely: zero CPU, zero latency.
// One shifter on the bus instead of one per stem is a 4x saving a mid-range phone needs.

export interface TrackSpec {
  name: string;
  url: string;
}

type StretchNode = AudioNode & {
  schedule(change: { active?: boolean; semitones?: number; output?: number }): Promise<unknown>;
  start(): Promise<unknown>;
  stop(): Promise<unknown>;
  latency(): Promise<number>;
};

export type EngineEvent = "state" | "ended";

// A page opened from disk (a shared setlist file) has an opaque origin, and Chrome then refuses
// worklet modules served from blob: URLs, which is how signalsmith-stretch ships its processor.
// The same source inlined as a data: URL loads fine, so on file: pages the blob is read back
// and re-issued that way. Installed once; http(s) pages are left alone.
let workletShim = false;
function shimWorkletForFilePages() {
  if (workletShim || location.protocol !== "file:" || typeof AudioWorklet === "undefined") return;
  workletShim = true;
  const original = AudioWorklet.prototype.addModule;
  AudioWorklet.prototype.addModule = async function (url: string | URL, options?: WorkletOptions) {
    let target = String(url);
    if (target.startsWith("blob:")) {
      const source = await (await fetch(target)).text();
      target = "data:text/javascript;base64," + btoa(unescape(encodeURIComponent(source)));
    }
    return original.call(this, target, options);
  };
}

export class Engine {
  readonly ctx: AudioContext;
  private buffers = new Map<string, AudioBuffer>();
  private gains = new Map<string, GainNode>();
  private sources: AudioBufferSourceNode[] = [];
  private bus: GainNode;
  private master: GainNode;
  private stretch: StretchNode | null = null;
  private stretchLoading: Promise<void> | null = null;
  private shifterActive = false;
  private stretchLatency = 0;

  private playing = false;
  private startCtxTime = 0;
  private startPos = 0;
  private rate = 1;
  private transpose = 0;
  private loop: { a: number; b: number } | null = null;
  private listeners = new Set<(e: EngineEvent) => void>();
  private endTimer: number | null = null;
  duration = 0;

  constructor() {
    // 44.1 kHz matches the files, so decodeAudioData does not resample (and phones keep
    // buffers 8% smaller than at 48 kHz). "playback" lets mobile browsers use big buffers.
    const Ctx = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    try {
      this.ctx = new Ctx({ latencyHint: "playback", sampleRate: 44100 });
    } catch {
      this.ctx = new Ctx({ latencyHint: "playback" });
    }
    this.bus = this.ctx.createGain();
    this.master = this.ctx.createGain();
    this.bus.connect(this.master);
    this.master.connect(this.ctx.destination);
  }

  on(fn: (e: EngineEvent) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private emit(e: EngineEvent) {
    this.listeners.forEach((fn) => fn(e));
  }

  get isPlaying() {
    return this.playing;
  }
  get trackNames() {
    return [...this.buffers.keys()];
  }

  /** Replace the loaded tracks. Position and settings are kept; playback resumes if it was running. */
  async load(tracks: TrackSpec[], onProgress?: (fraction: number) => void, signal?: AbortSignal): Promise<void> {
    const wasPlaying = this.playing;
    const at = this.position();
    let done = 0;
    const decoded = await Promise.all(
      tracks.map(async (t) => {
        const res = await fetch(t.url, { signal });
        if (!res.ok) throw new Error(`${t.name}: HTTP ${res.status}`);
        const data = await res.arrayBuffer();
        const buffer = await this.ctx.decodeAudioData(data);
        onProgress?.(++done / tracks.length);
        return [t.name, buffer] as const;
      }),
    );
    if (signal?.aborted) return;
    this.stopSources();
    this.buffers = new Map(decoded);
    this.duration = Math.max(0, ...decoded.map(([, b]) => b.duration));
    for (const [name] of decoded) {
      if (!this.gains.has(name)) {
        const g = this.ctx.createGain();
        g.connect(this.bus);
        this.gains.set(name, g);
      }
    }
    for (const [name, g] of [...this.gains]) {
      if (!this.buffers.has(name)) {
        g.disconnect();
        this.gains.delete(name);
      }
    }
    this.startPos = Math.min(at, this.duration);
    if (wasPlaying) await this.play();
    else this.emit("state");
  }

  /** Current position in seconds of song time, compensated for output and shifter latency. */
  position(): number {
    if (!this.playing) return this.startPos;
    const latency = (this.ctx.outputLatency || this.ctx.baseLatency || 0) + (this.shifterActive ? this.stretchLatency : 0);
    const elapsed = Math.max(0, this.ctx.currentTime - this.startCtxTime - latency);
    let pos = this.startPos + elapsed * this.rate;
    if (this.loop && this.loop.b > this.loop.a && pos >= this.loop.b) {
      const span = this.loop.b - this.loop.a;
      pos = this.loop.a + ((pos - this.loop.a) % span);
    }
    return Math.min(pos, this.duration);
  }

  async play(): Promise<void> {
    if (!this.buffers.size) return;
    if (this.ctx.state !== "running") await this.ctx.resume();
    this.stopSources();
    if (this.startPos >= this.duration - 0.05) this.startPos = this.loop ? this.loop.a : 0;
    await this.updateShifter();

    const when = this.ctx.currentTime + 0.06;
    for (const [name, buffer] of this.buffers) {
      const src = this.ctx.createBufferSource();
      src.buffer = buffer;
      src.playbackRate.value = this.rate;
      if (this.loop && this.loop.b > this.loop.a) {
        src.loop = true;
        src.loopStart = this.loop.a;
        src.loopEnd = Math.min(this.loop.b, buffer.duration);
      }
      src.connect(this.gains.get(name)!);
      src.start(when, Math.min(this.startPos, Math.max(0, buffer.duration - 0.01)));
      this.sources.push(src);
    }
    this.startCtxTime = when;
    this.playing = true;
    this.scheduleEnd();
    this.emit("state");
  }

  pause() {
    if (!this.playing) return;
    this.startPos = this.position();
    this.stopSources();
    this.emit("state");
  }

  toggle() {
    return this.playing ? (this.pause(), Promise.resolve()) : this.play();
  }

  async seek(seconds: number) {
    this.startPos = Math.max(0, Math.min(seconds, this.duration));
    if (this.playing) await this.play();
    else this.emit("state");
  }

  async setRate(rate: number) {
    const r = Math.max(0.4, Math.min(1.5, rate));
    if (r === this.rate) return;
    if (this.playing) {
      // Re-anchor so position() stays continuous across the change.
      this.startPos = this.position();
      this.startCtxTime = this.ctx.currentTime;
      this.sources.forEach((s) => s.playbackRate.setValueAtTime(r, this.ctx.currentTime));
    }
    this.rate = r;
    await this.updateShifter();
    this.scheduleEnd();
    this.emit("state");
  }

  async setTranspose(semitones: number) {
    this.transpose = semitones;
    await this.updateShifter();
  }

  async setLoop(loop: { a: number; b: number } | null) {
    this.loop = loop && loop.b - loop.a > 0.2 ? loop : null;
    if (this.playing) {
      const pos = this.position();
      this.startPos = this.loop && (pos < this.loop.a || pos >= this.loop.b) ? this.loop.a : pos;
      await this.play();
    } else if (this.loop) {
      this.startPos = this.loop.a;
      this.emit("state");
    }
  }

  getLoop() {
    return this.loop;
  }
  getRate() {
    return this.rate;
  }

  setGain(name: string, value: number) {
    const g = this.gains.get(name);
    if (g) g.gain.setTargetAtTime(value, this.ctx.currentTime, 0.015);
  }

  setMasterGain(value: number) {
    this.master.gain.setTargetAtTime(value, this.ctx.currentTime, 0.015);
  }

  private neededSemitones() {
    return this.transpose - 12 * Math.log2(this.rate);
  }

  private async updateShifter() {
    const semitones = this.neededSemitones();
    const need = Math.abs(semitones) > 0.01;
    if (need && !this.stretch) {
      // Loaded on first use only: most sessions never change tempo or key, and the WASM is
      // half of the application's size.
      shimWorkletForFilePages();
      this.stretchLoading ??= import("signalsmith-stretch")
        .then((m) => m.default(this.ctx))
        .then(async (node: StretchNode) => {
          // Methods of the node are proxied into the worklet, so they answer with promises.
          const latency = Number(await node.latency());
          this.stretchLatency = Number.isFinite(latency) ? latency : 0;
          this.stretch = node;
        });
      try {
        await this.stretchLoading;
      } catch (e) {
        console.warn("pitch shifter unavailable", e);
        this.stretchLoading = null;
        return;
      }
    }
    if (!this.stretch) return;
    if (need) {
      this.stretch.schedule({ active: true, semitones });
      if (!this.shifterActive) {
        this.bus.disconnect();
        this.bus.connect(this.stretch);
        this.stretch.connect(this.master);
        this.stretch.start();
        this.shifterActive = true;
      }
    } else if (this.shifterActive) {
      this.bus.disconnect();
      this.stretch.disconnect();
      this.stretch.stop();
      this.bus.connect(this.master);
      this.shifterActive = false;
    }
  }

  private scheduleEnd() {
    if (this.endTimer !== null) clearTimeout(this.endTimer);
    this.endTimer = null;
    if (!this.playing || this.loop) return;
    const remaining = (this.duration - this.position()) / this.rate;
    this.endTimer = window.setTimeout(() => {
      if (this.playing && !this.loop && this.position() >= this.duration - 0.1) {
        this.stopSources();
        this.startPos = 0;
        this.emit("ended");
        this.emit("state");
      } else {
        this.scheduleEnd();
      }
    }, Math.max(50, remaining * 1000 + 80));
  }

  private stopSources() {
    for (const s of this.sources) {
      try {
        s.stop();
      } catch {
        /* not started yet */
      }
      s.disconnect();
    }
    this.sources = [];
    this.playing = false;
    if (this.endTimer !== null) clearTimeout(this.endTimer);
    this.endTimer = null;
  }

  /** Short synthesized chord, to audition a suggestion. `pcs` are pitch classes, root first. */
  audition(pcs: number[], seconds = 1.2) {
    if (this.ctx.state !== "running") void this.ctx.resume();
    const now = this.ctx.currentTime;
    const out = this.ctx.createGain();
    out.gain.setValueAtTime(0, now);
    out.gain.linearRampToValueAtTime(0.18, now + 0.02);
    out.gain.exponentialRampToValueAtTime(0.001, now + seconds);
    out.connect(this.master);
    const midi = [36 + pcs[0], ...pcs.map((pc, i) => 60 + ((pc - pcs[0] + 12) % 12) + pcs[0] - (pcs[0] > 6 ? 12 : 0) + (i === 0 ? 0 : 0))];
    for (const m of midi) {
      const osc = this.ctx.createOscillator();
      osc.type = "triangle";
      osc.frequency.value = 440 * 2 ** ((m - 69) / 12);
      osc.connect(out);
      osc.start(now);
      osc.stop(now + seconds + 0.05);
    }
  }

  async dispose() {
    this.stopSources();
    this.listeners.clear();
    try {
      await this.ctx.close();
    } catch {
      /* already closed */
    }
  }
}
