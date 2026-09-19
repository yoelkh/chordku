//! Job execution.
//!
//! All neural inference runs on ONE dedicated OS thread, so two models never fight for
//! the same cores, with two lanes:
//!
//!   fast  analyze   decode -> tuning -> beats || chords -> chart   (seconds)
//!   slow  separate  htdemucs stems, 7.8 s segments                 (minutes)
//!
//! A separation hands control back between segments: if a fast job is waiting it is run
//! right there, so adding a song never waits for stems of another one. Network work
//! (yt-dlp, model downloads) lives on the async runtime and only enqueues here when the
//! audio file is on disk.

use std::collections::{HashMap, VecDeque};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use chordku_core::accel::{self, Plan, Trial};
use chordku_core::chart::PIPELINE_VERSION;
use chordku_core::ortutil::Device;
use chordku_core::opus_enc::OggOpusWriter;
use chordku_core::pipeline::{self, Analyzer};
use chordku_core::separate::htdemucs::HtDemucs;
use chordku_core::separate::{Separator, Stem, STEM_RATE};
use chordku_core::{decode, resample, Audio};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::db::{Db, Job, StemFile};
use crate::{models, platform};

pub const MIX_FILE: &str = "mix.opus";
const MIX_BITRATE: i32 = 128_000;
const STEM_BITRATE: i32 = 96_000;
const MONO_BITRATE: i32 = 64_000;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Job { job: Job },
    /// A song's chart, audio or stems changed: refetch it.
    Song { id: String },
    SongDeleted { id: String },
    Model { id: String, progress: f64, error: Option<String>, done: bool },
    System,
}

#[derive(Clone, Debug)]
enum Work {
    Analyze { job_id: String, song_id: String },
    Separate { job_id: String, song_id: String },
}

#[derive(Default)]
struct Queues {
    fast: VecDeque<Work>,
    slow: VecDeque<Work>,
    shutdown: bool,
}

struct Shared {
    queues: Mutex<Queues>,
    wake: Condvar,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    accel: Mutex<AccelState>,
    recalibrate: AtomicBool,
}

/// What the settings page shows about hardware acceleration.
#[derive(Clone, Debug, Default, Serialize)]
pub struct AccelState {
    pub calibrating: bool,
    pub calibration: Option<StoredCalibration>,
}

/// The measured plan plus the conditions it was measured under.
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct StoredCalibration {
    /// Build features and installed runtimes; a change triggers a new calibration.
    pub signature: String,
    pub plan: Plan,
    pub stems_measured: bool,
    pub trials: Vec<Trial>,
}

const SETTING_ACCEL: &str = "accel_calibration";

#[derive(Clone)]
pub struct Jobs {
    shared: Arc<Shared>,
    db: Db,
    events: broadcast::Sender<Event>,
}

impl Jobs {
    /// Start the inference thread and re-enqueue whatever a previous run left unfinished.
    pub fn start(cfg: Config, db: Db, events: broadcast::Sender<Event>) -> Result<Self> {
        let stored: Option<StoredCalibration> =
            db.setting(SETTING_ACCEL).ok().flatten().and_then(|j| serde_json::from_str(&j).ok());
        let shared = Arc::new(Shared {
            queues: Mutex::default(),
            wake: Condvar::new(),
            cancels: Mutex::default(),
            accel: Mutex::new(AccelState { calibrating: false, calibration: stored }),
            recalibrate: AtomicBool::new(false),
        });
        let jobs = Self { shared: shared.clone(), db: db.clone(), events: events.clone() };
        for job in db.requeue_interrupted_jobs()? {
            // A job whose download never finished cannot resume without its URL task.
            let has_source = db.song(&job.song_id)?.and_then(|s| s.source_file).is_some();
            if has_source {
                jobs.push(&job.kind, job.id.clone(), job.song_id.clone());
            } else {
                db.update_job(&job.id, "failed", "", 0.0, Some("Terputus saat mengunduh. Tambahkan lagi lagunya."))?;
            }
        }
        let worker = Worker { cfg, db, events, shared, analyzer: None };
        std::thread::Builder::new().name("inference".into()).spawn(move || worker.run()).context("spawn inference thread")?;
        Ok(jobs)
    }

    fn push(&self, kind: &str, job_id: String, song_id: String) {
        let mut q = self.shared.queues.lock().expect("queue mutex");
        match kind {
            "separate" => q.slow.push_back(Work::Separate { job_id, song_id }),
            _ => q.fast.push_back(Work::Analyze { job_id, song_id }),
        }
        self.shared.wake.notify_all();
    }

    /// Create a job row. The caller either enqueues it right away or after a download.
    pub fn create(&self, song_id: &str, kind: &str) -> Result<Job> {
        let id = ulid::Ulid::new().to_string();
        self.db.insert_job(&id, song_id, kind)?;
        self.shared.cancels.lock().expect("cancel mutex").insert(id.clone(), Arc::new(AtomicBool::new(false)));
        let job = self.db.job(&id)?.ok_or_else(|| anyhow!("job vanished"))?;
        let _ = self.events.send(Event::Job { job: job.clone() });
        Ok(job)
    }

    pub fn enqueue(&self, job: &Job) {
        self.push(&job.kind, job.id.clone(), job.song_id.clone());
    }

    pub fn report(&self, job_id: &str, state: &str, stage: &str, progress: f64, error: Option<&str>) {
        report(&self.db, &self.events, job_id, state, stage, progress, error);
    }

    pub fn is_canceled(&self, job_id: &str) -> bool {
        self.shared.cancels.lock().expect("cancel mutex").get(job_id).is_some_and(|f| f.load(Ordering::Relaxed))
    }

    /// Request cancellation. Queued jobs are dropped immediately; a running job stops at
    /// its next checkpoint (between segments for a separation).
    pub fn cancel(&self, job_id: &str) -> Result<bool> {
        let Some(job) = self.db.job(job_id)? else { return Ok(false) };
        if !matches!(job.state.as_str(), "queued" | "running") {
            return Ok(false);
        }
        if let Some(flag) = self.shared.cancels.lock().expect("cancel mutex").get(job_id) {
            flag.store(true, Ordering::Relaxed);
        }
        let mut q = self.shared.queues.lock().expect("queue mutex");
        let before = q.fast.len() + q.slow.len();
        q.fast.retain(|w| w.job_id() != job_id);
        q.slow.retain(|w| w.job_id() != job_id);
        let dequeued = q.fast.len() + q.slow.len() != before;
        drop(q);
        if dequeued || job.state == "queued" {
            self.report(job_id, "canceled", "", 0.0, None);
        }
        Ok(true)
    }

    pub fn accel_state(&self) -> AccelState {
        self.shared.accel.lock().expect("accel mutex").clone()
    }

    /// Ask the inference thread to measure the devices again as soon as it is idle.
    pub fn request_calibration(&self) {
        self.shared.recalibrate.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }

    pub fn shutdown(&self) {
        self.shared.queues.lock().expect("queue mutex").shutdown = true;
        self.shared.wake.notify_all();
    }
}

impl Work {
    fn job_id(&self) -> &str {
        match self {
            Work::Analyze { job_id, .. } | Work::Separate { job_id, .. } => job_id,
        }
    }
}

fn report(db: &Db, events: &broadcast::Sender<Event>, job_id: &str, state: &str, stage: &str, progress: f64, error: Option<&str>) {
    if let Err(e) = db.update_job(job_id, state, stage, progress, error) {
        tracing::error!("job {job_id}: cannot persist state: {e:#}");
    }
    if let Ok(Some(job)) = db.job(job_id) {
        let _ = events.send(Event::Job { job });
    }
}

struct Worker {
    cfg: Config,
    db: Db,
    events: broadcast::Sender<Event>,
    shared: Arc<Shared>,
    analyzer: Option<Analyzer>,
}

impl Worker {
    fn run(mut self) {
        self.ensure_plan();
        loop {
            let work = {
                let mut q = self.shared.queues.lock().expect("queue mutex");
                loop {
                    if q.shutdown {
                        return;
                    }
                    if let Some(w) = q.fast.pop_front().or_else(|| q.slow.pop_front()) {
                        break Some(w);
                    }
                    if self.shared.recalibrate.load(Ordering::Relaxed) {
                        break None;
                    }
                    q = self.shared.wake.wait(q).expect("queue mutex");
                }
            };
            let Some(work) = work else {
                self.shared.recalibrate.store(false, Ordering::Relaxed);
                self.analyzer = None;
                self.store_calibration(None);
                self.ensure_plan();
                continue;
            };
            let _awake = platform::KeepAwake::new();
            self.execute(work);
        }
    }

    fn execute(&mut self, work: Work) {
        let job_id = work.job_id().to_owned();
        let result = match &work {
            Work::Analyze { song_id, .. } => self.analyze(&job_id, song_id),
            Work::Separate { song_id, .. } => self.separate(&job_id, song_id),
        };
        match result {
            Ok(true) => report(&self.db, &self.events, &job_id, "succeeded", "", 1.0, None),
            Ok(false) => report(&self.db, &self.events, &job_id, "canceled", "", 0.0, None),
            Err(e) => {
                tracing::warn!("job {job_id} failed: {e:#}");
                report(&self.db, &self.events, &job_id, "failed", "", 0.0, Some(&format!("{e:#}")));
            }
        }
        self.shared.cancels.lock().expect("cancel mutex").remove(&job_id);
    }

    fn canceled(&self, job_id: &str) -> bool {
        self.shared.cancels.lock().expect("cancel mutex").get(job_id).is_some_and(|f| f.load(Ordering::Relaxed))
    }

    fn stage(&self, job_id: &str, stage: &str, progress: f64) {
        report(&self.db, &self.events, job_id, "running", stage, progress, None);
    }

    fn source_path(&self, song_id: &str) -> Result<PathBuf> {
        let song = self.db.song(song_id)?.ok_or_else(|| anyhow!("lagu sudah dihapus"))?;
        let file = song.source_file.ok_or_else(|| anyhow!("file sumber belum ada"))?;
        let path = self.cfg.song_dir(song_id).join(file);
        if !path.is_file() {
            bail!("file sumber hilang dari disk");
        }
        Ok(path)
    }

    fn decode(&self, song_id: &str) -> Result<Audio> {
        let audio = decode::decode_file(&self.source_path(song_id)?).context("Format audio tidak bisa dibaca")?;
        let secs = audio.duration_secs();
        if secs < 3.0 {
            bail!("Audio terlalu pendek ({secs:.1} detik)");
        }
        if secs > self.cfg.max_duration_secs as f64 {
            bail!("Audio terlalu panjang ({:.0} menit; batas {} menit)", secs / 60.0, self.cfg.max_duration_secs / 60);
        }
        Ok(audio)
    }

    /// Builds and installed runtimes the stored plan is valid for.
    fn signature(&self) -> String {
        // v3: the separator has a half-precision candidate on CUDA.
        format!(
            "v3|cuda_build={}|cuda_libs={}|stems_model={}",
            cfg!(feature = "cuda"),
            crate::cuda::status(&self.cfg).installed,
            models::group_dir(&self.cfg, "stems").is_some()
        )
    }

    fn candidates(&self) -> Vec<Device> {
        let cuda = crate::cuda::status(&self.cfg);
        accel::CANDIDATES.into_iter().filter(|d| *d != Device::Cuda || (cuda.supported && cuda.installed)).collect()
    }

    fn stored(&self) -> Option<StoredCalibration> {
        self.shared.accel.lock().expect("accel mutex").calibration.clone()
    }

    fn store_calibration(&self, calibration: Option<StoredCalibration>) {
        let json = calibration.as_ref().and_then(|c| serde_json::to_string(c).ok()).unwrap_or_default();
        if let Err(e) = self.db.set_setting(SETTING_ACCEL, &json) {
            tracing::warn!("cannot store the acceleration plan: {e:#}");
        }
        self.shared.accel.lock().expect("accel mutex").calibration = calibration;
        let _ = self.events.send(Event::System);
    }

    fn analysis_models(&self) -> Result<(PathBuf, PathBuf, PathBuf)> {
        let find = |f: &str| self.cfg.find_model(f).ok_or_else(|| anyhow!("Model {f} belum terpasang. Buka Pengaturan lalu Model."));
        let (btc, mel, beat) = (find(pipeline::BTC_ONNX)?, find(pipeline::MEL_ONNX)?, find(pipeline::BEAT_ONNX)?);
        if !btc.with_extension("json").is_file() {
            bail!("Model btc.json belum terpasang di samping btc.onnx");
        }
        Ok((btc, mel, beat))
    }

    /// Measure chords and beats on every device once per build / runtime combination.
    fn ensure_plan(&mut self) {
        let signature = self.signature();
        if self.stored().is_some_and(|c| c.signature == signature) {
            return;
        }
        let Ok((btc, mel, beat)) = self.analysis_models() else { return };
        if std::env::var_os("CHORDKU_NO_ACCEL").is_some() {
            return;
        }
        self.shared.accel.lock().expect("accel mutex").calibrating = true;
        let _ = self.events.send(Event::System);
        tracing::info!("mengkalibrasi akselerasi perangkat keras...");
        let result = accel::calibrate_analysis(&btc, &mel, &beat, self.cfg.threads, &self.candidates());
        let (mut plan, mut trials) = (result.plan, result.trials);
        // The separator too, when its model is installed (about 12 s, once).
        let stems_measured = match models::group_dir(&self.cfg, "stems") {
            Some(dir) => {
                let (device, stem_trials) = accel::calibrate_stems(&dir.join("htdemucs_fwd.onnx"), self.cfg.threads, &self.candidates());
                plan.stems = device;
                trials.extend(stem_trials);
                true
            }
            None => false,
        };
        tracing::info!(
            "rencana perangkat: chord di {}, ketukan di {}, pisah stem di {}",
            plan.chords.name(),
            plan.beats.name(),
            if stems_measured { plan.stems.name() } else { "(model belum diunduh)" }
        );
        self.shared.accel.lock().expect("accel mutex").calibrating = false;
        self.store_calibration(Some(StoredCalibration { signature, plan, stems_measured, trials }));
        self.analyzer = None;
    }

    /// The separator's device is measured the first time stems are requested.
    fn stems_device(&mut self, onnx: &Path) -> Device {
        let Some(mut stored) = self.stored() else { return Device::Cpu };
        if !stored.stems_measured {
            let (device, trials) = accel::calibrate_stems(onnx, self.cfg.threads, &self.candidates());
            tracing::info!("rencana perangkat: pemisah stem di {}", device.name());
            stored.plan.stems = device;
            stored.stems_measured = true;
            stored.trials.retain(|t| t.model != "stems");
            stored.trials.extend(trials);
            self.store_calibration(Some(stored.clone()));
        }
        stored.plan.stems
    }

    fn analyzer(&mut self) -> Result<&mut Analyzer> {
        if self.analyzer.is_none() {
            let (btc, mel, beat) = self.analysis_models()?;
            let plan = self.stored().map(|c| c.plan).unwrap_or_default();
            let loaded = Analyzer::load_on(&btc, &mel, &beat, plan, self.cfg.threads).or_else(|e| {
                // A driver update or a busy GPU must never stop the app from working.
                tracing::warn!("perangkat terakselerasi gagal dimuat ({e:#}); memakai CPU");
                Analyzer::load_from(&btc, &mel, &beat, self.cfg.threads)
            })?;
            self.analyzer = Some(loaded);
        }
        Ok(self.analyzer.as_mut().expect("just loaded"))
    }

    fn analyze(&mut self, job_id: &str, song_id: &str) -> Result<bool> {
        self.stage(job_id, "decode", 0.05);
        let audio = self.decode(song_id)?;
        if self.canceled(job_id) {
            return Ok(false);
        }

        self.stage(job_id, "analyze", 0.2);
        let dir = self.cfg.song_dir(song_id);
        let part = dir.join(format!("{MIX_FILE}.part"));
        self.analyzer()?; // load (or fail) before starting the encoder thread
        // The playback file does not depend on the analysis: encode it on another core
        // while the models run. One canonical file: same timeline, pre-skip and sample
        // rate as the stems.
        let (analysed, encoded) = std::thread::scope(|scope| {
            let encoder = scope.spawn(|| -> Result<()> {
                let [l, r] = canonical_stereo(&audio)?;
                let mut writer = OggOpusWriter::new(std::io::BufWriter::new(std::fs::File::create(&part)?), 2, MIX_BITRATE, 1)?;
                for (a, b) in l.chunks(1 << 16).zip(r.chunks(1 << 16)) {
                    writer.write(&[a, b])?;
                }
                std::io::Write::flush(&mut writer.finish()?)?;
                Ok(())
            });
            // Save and announce the chart as soon as it exists; the encoder may still be running.
            let analysed = (|| -> Result<()> {
                let (analysis, timings) = self.analyzer.as_mut().expect("loaded above").analyze(&audio, |_| {})?;
                tracing::info!(song_id, ?timings, chords = analysis.segments.len(), "analysed");
                self.db.save_analysis(song_id, PIPELINE_VERSION, "mix", &serde_json::to_string(&analysis)?)?;
                let source = self.db.song(song_id)?.and_then(|s| s.source_file).unwrap_or_default();
                let hash = hash_file(&self.cfg.song_dir(song_id).join(&source))?;
                self.db.set_song_source(song_id, &source, &hash, analysis.duration_ms)?;
                let _ = self.events.send(Event::Song { id: song_id.to_owned() });
                Ok(())
            })();
            (analysed, encoder.join().unwrap_or_else(|_| Err(anyhow!("encoder thread panicked"))))
        });
        analysed?;
        if self.canceled(job_id) {
            return Ok(false);
        }

        self.stage(job_id, "encode", 0.9);
        drop(audio);
        encoded?;
        std::fs::rename(&part, dir.join(MIX_FILE))?;
        let mut stems = self.db.stems(song_id)?;
        stems.retain(|s| s.name != "mix");
        stems.push(StemFile { name: "mix".into(), file: MIX_FILE.into(), bytes: file_len(&dir.join(MIX_FILE)), channels: 2 });
        self.db.replace_stems(song_id, &stems)?;
        let _ = self.events.send(Event::Song { id: song_id.to_owned() });
        Ok(true)
    }

    fn separate(&mut self, job_id: &str, song_id: &str) -> Result<bool> {
        self.stage(job_id, "model_check", 0.0);
        let model_dir = models::group_dir(&self.cfg, "stems")
            .ok_or_else(|| anyhow!("Model pemisah stem belum diunduh. Buka Pengaturan lalu Model."))?;
        // The separator peaks around 2.5 GB. With less than the floor available Windows would
        // page the rest of the system out for minutes, so refuse instead of grinding.
        // (On the GPU the activations live in video memory, so the guard only applies to the CPU.)
        let on_cpu = self.stored().is_none_or(|c| !c.stems_measured || c.plan.stems == Device::Cpu);
        let free = platform::free_memory();
        if on_cpu && free != 0 && free < self.cfg.min_free_ram {
            bail!(
                "RAM bebas hanya {:.1} GB; pemisahan stem butuh sekitar 2,5 GB. Tutup aplikasi lain lalu coba lagi.",
                free as f64 / 1e9
            );
        }

        self.stage(job_id, "decode", 0.02);
        let audio = self.decode(song_id)?;
        let mix = canonical_stereo(&audio)?;
        drop(audio);

        let dir = self.cfg.song_dir(song_id);
        let mut outputs = StemOutputs::create(&dir)?;
        let onnx = model_dir.join("htdemucs_fwd.onnx");
        let device = self.stems_device(&onnx);
        let mut actual = device;
        // A driver update or a busy GPU must never stop the job: half precision falls back to
        // float32 on the GPU, and that to the CPU.
        let mut model = HtDemucs::load_on(&onnx, device, self.cfg.threads)
            .or_else(|e| {
                if device != Device::CudaHalf {
                    return Err(e);
                }
                tracing::warn!("pemisah stem presisi setengah gagal dimuat ({e:#}); memakai float32");
                actual = Device::Cuda;
                HtDemucs::load_on(&onnx, Device::Cuda, self.cfg.threads)
            })
            .or_else(|e| {
                tracing::warn!("pemisah stem gagal dimuat di {} ({e:#}); memakai CPU", actual.name());
                actual = Device::Cpu;
                HtDemucs::load(&onnx, self.cfg.threads)
            })?;
        // The stage carries the device so the progress banner can say where it runs.
        let stage_name = format!("segments:{}", actual.name());

        let _priority = platform::BelowNormalPriority::new();
        let started = std::time::Instant::now();
        // The six Opus encoders run on their own thread: on a GPU the network needs half a
        // second per segment and must not wait for the previous block to be encoded. The
        // channel holds two blocks at most, so memory stays bounded.
        let (finished, outputs) = std::thread::scope(|scope| -> Result<(bool, StemOutputs)> {
            let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<[Vec<f32>; 2]>>(2);
            let encoder = scope.spawn(move || -> Result<StemOutputs> {
                for block in rx {
                    outputs.write(&block)?;
                }
                Ok(outputs)
            });
            let finished = model.separate(&mix, &mut |block| {
                if tx.send(block.stems.to_vec()).is_err() {
                    return Ok(ControlFlow::Break(())); // the encoder failed; its error is reported below
                }
                self.stage(job_id, &stage_name, 0.05 + 0.93 * block.segment as f64 / block.segments as f64);
                if self.canceled(job_id) {
                    return Ok(ControlFlow::Break(()));
                }
                // Yield: run any waiting fast-lane job before the next segment.
                while let Some(work) = {
                    let mut q = self.shared.queues.lock().expect("queue mutex");
                    q.fast.pop_front()
                } {
                    let _normal = platform::NormalPriority::new();
                    self.execute(work);
                }
                Ok(ControlFlow::Continue(()))
            });
            drop(tx);
            let outputs = encoder.join().unwrap_or_else(|_| Err(anyhow!("encoder thread panicked")))?;
            Ok((finished?, outputs))
        })?;
        drop(model); // release the weights and activations right away
        tracing::info!(song_id, device = actual.name(), secs = started.elapsed().as_secs_f64(), finished, "separated");
        if !finished {
            outputs.discard();
            return Ok(false);
        }

        self.stage(job_id, "encode", 0.99);
        let mut stems = self.db.stems(song_id)?;
        stems.retain(|s| s.name == "mix");
        stems.extend(outputs.finish()?);
        self.db.replace_stems(song_id, &stems)?;
        let _ = self.events.send(Event::Song { id: song_id.to_owned() });
        Ok(true)
    }
}

/// The playback / separation timeline: 44.1 kHz planar stereo.
fn canonical_stereo(audio: &Audio) -> Result<[Vec<f32>; 2]> {
    let [l, r] = audio.to_stereo_planar();
    if audio.sample_rate == STEM_RATE {
        return Ok([l, r]);
    }
    let interleaved: Vec<f32> = l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]).collect();
    drop((l, r));
    let out = resample::resample_interleaved(&interleaved, 2, STEM_RATE as f64 / audio.sample_rate as f64)?;
    let mut planar = [Vec::with_capacity(out.len() / 2), Vec::with_capacity(out.len() / 2)];
    for f in out.chunks_exact(2) {
        planar[0].push(f[0]);
        planar[1].push(f[1]);
    }
    Ok(planar)
}

type FileWriter = OggOpusWriter<std::io::BufWriter<std::fs::File>>;

/// Six incremental encoders: four stereo stems for the desktop mixer, plus mono vocals and
/// mono backing for phones, where four decoded stereo stems would not fit in memory.
struct StemOutputs {
    dir: PathBuf,
    stems: Vec<(Stem, FileWriter)>,
    vocals_mono: FileWriter,
    backing_mono: FileWriter,
    scratch: [Vec<f32>; 2],
}

const MONO_VOCALS: &str = "vocals_mono";
const MONO_BACKING: &str = "backing_mono";

impl StemOutputs {
    fn part(dir: &Path, name: &str) -> PathBuf {
        dir.join(format!("{name}.opus.part"))
    }

    fn open(dir: &Path, name: &str, channels: usize, bitrate: i32, serial: u32) -> Result<FileWriter> {
        let file = std::fs::File::create(Self::part(dir, name)).with_context(|| format!("buat file stem {name}"))?;
        OggOpusWriter::new(std::io::BufWriter::new(file), channels, bitrate, serial)
    }

    fn create(dir: &Path) -> Result<Self> {
        let mut stems = Vec::new();
        for (i, stem) in Stem::ALL.into_iter().enumerate() {
            stems.push((stem, Self::open(dir, stem.name(), 2, STEM_BITRATE, 10 + i as u32)?));
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            stems,
            vocals_mono: Self::open(dir, MONO_VOCALS, 1, MONO_BITRATE, 20)?,
            backing_mono: Self::open(dir, MONO_BACKING, 1, MONO_BITRATE, 21)?,
            scratch: [Vec::new(), Vec::new()],
        })
    }

    /// Encodes one block into all six files, one thread per file. In sequence the six streams
    /// cost about 0.16 s per block, more than half of what a GPU needs for the block itself.
    fn write(&mut self, block: &[[Vec<f32>; 2]]) -> Result<()> {
        let n = block[0][0].len();
        let [vocals, backing] = &mut self.scratch;
        vocals.clear();
        backing.clear();
        for i in 0..n {
            let mono = |s: Stem| 0.5 * (block[s as usize][0][i] + block[s as usize][1][i]);
            vocals.push(mono(Stem::Vocals));
            backing.push(mono(Stem::Drums) + mono(Stem::Bass) + mono(Stem::Other));
        }
        let (vocals, backing) = (vocals.as_slice(), backing.as_slice());
        let (vocals_mono, backing_mono) = (&mut self.vocals_mono, &mut self.backing_mono);
        std::thread::scope(|scope| {
            let mut jobs = Vec::with_capacity(6);
            for (stem, writer) in &mut self.stems {
                let pair = &block[*stem as usize];
                jobs.push(scope.spawn(move || writer.write(&[&pair[0], &pair[1]])));
            }
            jobs.push(scope.spawn(move || vocals_mono.write(&[vocals])));
            jobs.push(scope.spawn(move || backing_mono.write(&[backing])));
            jobs.into_iter().try_for_each(|j| j.join().unwrap_or_else(|_| Err(anyhow!("encoder thread panicked"))))
        })
    }

    fn names() -> impl Iterator<Item = &'static str> {
        Stem::ALL.into_iter().map(Stem::name).chain([MONO_VOCALS, MONO_BACKING])
    }

    fn discard(self) {
        let dir = self.dir.clone();
        drop(self);
        for name in Self::names() {
            let _ = std::fs::remove_file(Self::part(&dir, name));
        }
    }

    fn finish(self) -> Result<Vec<StemFile>> {
        let Self { dir, stems, vocals_mono, backing_mono, .. } = self;
        let mut all: Vec<(&'static str, u8, FileWriter)> = stems.into_iter().map(|(s, w)| (s.name(), 2, w)).collect();
        all.push((MONO_VOCALS, 1, vocals_mono));
        all.push((MONO_BACKING, 1, backing_mono));
        let mut out = Vec::new();
        for (name, channels, writer) in all {
            std::io::Write::flush(&mut writer.finish()?)?;
            let file = format!("{name}.opus");
            std::fs::rename(Self::part(&dir, name), dir.join(&file))?;
            out.push(StemFile { name: name.into(), bytes: file_len(&dir.join(&file)), file, channels });
        }
        Ok(out)
    }
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut file = std::fs::File::open(path).with_context(|| format!("buka {}", path.display()))?;
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}
