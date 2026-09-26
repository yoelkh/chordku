// Time sources for the viewer's transport. The app plays audio; the viewer has none, so the
// chart follows either the embedded YouTube player or a silent clock at the song's tempo.

export interface Loop {
  a: number;
  b: number;
}

export interface Clock {
  readonly kind: "audio" | "youtube" | "virtual";
  readonly isPlaying: boolean;
  /** Seconds. */
  position(): number;
  toggle(): void;
  seek(sec: number): void;
  setRate(rate: number): void;
  getRate(): number;
  setLoop(loop: Loop | null): void;
  getLoop(): Loop | null;
  /** Called on play / pause / seek / rate / loop changes. */
  on(fn: () => void): () => void;
  /** Called from the page's animation frame while playing (loop handling, resync). */
  tick(): void;
  dispose(): void;
}

abstract class Base implements Clock {
  abstract readonly kind: "audio" | "youtube" | "virtual";
  abstract get isPlaying(): boolean;
  abstract position(): number;
  abstract toggle(): void;
  abstract seek(sec: number): void;
  abstract setRate(rate: number): void;
  abstract getRate(): number;
  abstract dispose(): void;
  protected loop: Loop | null = null;
  protected listeners = new Set<() => void>();
  protected emit() {
    for (const fn of this.listeners) fn();
  }
  on(fn: () => void) {
    this.listeners.add(fn);
    return () => void this.listeners.delete(fn);
  }
  setLoop(loop: Loop | null) {
    this.loop = loop;
    this.emit();
  }
  getLoop() {
    return this.loop;
  }
  tick() {
    const l = this.loop;
    if (l && this.isPlaying && this.position() >= l.b) this.seek(l.a);
  }
}

/** A stopwatch: advances at `rate` times real time, stops at the end of the song. */
export class VirtualClock extends Base {
  readonly kind = "virtual" as const;
  private playing = false;
  private rate = 1;
  private base = 0; // position when `stamp` was taken
  private stamp = 0; // performance.now() ms
  constructor(private durationSec: number) {
    super();
  }
  get isPlaying() {
    return this.playing;
  }
  position() {
    const p = this.playing ? this.base + ((performance.now() - this.stamp) / 1000) * this.rate : this.base;
    return Math.min(Math.max(0, p), this.durationSec);
  }
  toggle() {
    if (this.playing) {
      this.base = this.position();
      this.playing = false;
    } else {
      if (this.base >= this.durationSec) this.base = 0;
      this.stamp = performance.now();
      this.playing = true;
    }
    this.emit();
  }
  seek(sec: number) {
    this.base = Math.min(Math.max(0, sec), this.durationSec);
    this.stamp = performance.now();
    this.emit();
  }
  setRate(rate: number) {
    this.base = this.position();
    this.stamp = performance.now();
    this.rate = rate;
    this.emit();
  }
  getRate() {
    return this.rate;
  }
  tick() {
    super.tick();
    if (this.playing && this.position() >= this.durationSec) {
      this.playing = false;
      this.base = this.durationSec;
      this.emit();
    }
  }
  dispose() {
    this.listeners.clear();
  }
}

// ---- the app's own audio engine (pages exported with audio) ----------------------------------

import type { Engine } from "../lib/audio/engine";

/** The Web Audio engine the app uses: sample-locked playback, tempo and pitch shift. */
export class EngineClock implements Clock {
  readonly kind = "audio" as const;
  constructor(readonly engine: Engine) {}
  get isPlaying() {
    return this.engine.isPlaying;
  }
  position() {
    return this.engine.position();
  }
  toggle() {
    this.engine.toggle();
  }
  seek(sec: number) {
    void this.engine.seek(sec);
  }
  setRate(rate: number) {
    void this.engine.setRate(rate);
  }
  getRate() {
    return this.engine.getRate();
  }
  setLoop(loop: Loop | null) {
    void this.engine.setLoop(loop);
  }
  getLoop() {
    return this.engine.getLoop();
  }
  on(fn: () => void) {
    return this.engine.on(() => fn());
  }
  tick() {}
  dispose() {
    void this.engine.dispose();
  }
}

// ---- YouTube IFrame API ---------------------------------------------------------------------

interface YTPlayer {
  playVideo(): void;
  pauseVideo(): void;
  seekTo(sec: number, allowSeekAhead: boolean): void;
  getCurrentTime(): number;
  getPlayerState(): number;
  setPlaybackRate(rate: number): void;
  getPlaybackRate(): number;
  destroy(): void;
}
interface YTNamespace {
  Player: new (
    el: HTMLElement,
    opts: {
      videoId: string;
      playerVars?: Record<string, string | number>;
      events?: { onReady?: () => void; onStateChange?: (e: { data: number }) => void; onError?: (e: { data: number }) => void };
    },
  ) => YTPlayer;
  PlayerState: { PLAYING: number; PAUSED: number; ENDED: number; BUFFERING: number };
}
declare global {
  interface Window {
    YT?: YTNamespace;
    onYouTubeIframeAPIReady?: () => void;
  }
}

let apiPromise: Promise<YTNamespace> | null = null;
/** Loads the IFrame API once; rejects when offline or blocked (about 8 s). */
export function youtubeApi(): Promise<YTNamespace> {
  if (window.YT?.Player) return Promise.resolve(window.YT);
  if (!apiPromise) {
    apiPromise = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("YouTube tidak dapat dimuat")), 8000);
      const prev = window.onYouTubeIframeAPIReady;
      window.onYouTubeIframeAPIReady = () => {
        clearTimeout(timer);
        prev?.();
        resolve(window.YT!);
      };
      const s = document.createElement("script");
      s.src = "https://www.youtube.com/iframe_api";
      s.onerror = () => (clearTimeout(timer), reject(new Error("YouTube tidak dapat dimuat")));
      document.head.appendChild(s);
    });
    apiPromise.catch(() => (apiPromise = null));
  }
  return apiPromise;
}

/** Follows the embedded video. `getCurrentTime` updates about four times a second, so the
 *  position is interpolated between readings for a smooth beat highlight. */
export class YouTubeClock extends Base {
  readonly kind = "youtube" as const;
  private player: YTPlayer | null = null;
  private playing = false;
  private rate = 1;
  private raw = 0;
  private rawStamp = 0;
  private pending: number | null = null; // seek requested before the player was ready
  ready = false;

  constructor(
    private el: HTMLElement,
    private videoId: string,
    private onfail: (why: string) => void,
  ) {
    super();
    youtubeApi().then(
      (YT) => this.create(YT),
      (e: Error) => this.onfail(e.message),
    );
  }

  private create(YT: YTNamespace) {
    this.player = new YT.Player(this.el, {
      videoId: this.videoId,
      playerVars: { playsinline: 1, rel: 0, modestbranding: 1 },
      events: {
        onReady: () => {
          this.ready = true;
          if (this.pending !== null) this.player?.seekTo(this.pending, true);
          this.emit();
        },
        onStateChange: (e) => {
          const S = YT.PlayerState;
          if (e.data === S.PLAYING) this.playing = true;
          else if (e.data === S.PAUSED || e.data === S.ENDED) this.playing = false;
          this.rate = this.player?.getPlaybackRate() ?? 1;
          this.sample();
          this.emit();
        },
        onError: (e) => this.onfail(`video tidak dapat diputar (kode ${e.data})`),
      },
    });
  }

  private sample() {
    const t = this.player?.getCurrentTime() ?? 0;
    if (t !== this.raw) {
      this.raw = t;
      this.rawStamp = performance.now();
    }
  }

  get isPlaying() {
    return this.playing;
  }
  position() {
    this.sample();
    if (!this.playing) return this.raw;
    return this.raw + ((performance.now() - this.rawStamp) / 1000) * this.rate;
  }
  toggle() {
    if (!this.player || !this.ready) return;
    if (this.playing) this.player.pauseVideo();
    else this.player.playVideo();
  }
  seek(sec: number) {
    if (!this.player || !this.ready) {
      this.pending = sec;
      return;
    }
    this.player.seekTo(Math.max(0, sec), true);
    this.raw = sec;
    this.rawStamp = performance.now();
    this.emit();
  }
  setRate(rate: number) {
    this.player?.setPlaybackRate(rate);
    this.rate = rate;
    this.emit();
  }
  getRate() {
    return this.rate;
  }
  dispose() {
    this.listeners.clear();
    this.player?.destroy();
    this.player = null;
  }
}
