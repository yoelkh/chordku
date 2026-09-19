//! In-memory PCM container shared by every stage.

/// Interleaved 32-bit float PCM.
#[derive(Clone, Debug, Default)]
pub struct Audio {
    pub sample_rate: u32,
    pub channels: usize,
    /// Interleaved samples, `frames * channels` long.
    pub samples: Vec<f32>,
}

impl Audio {
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels
        }
    }

    pub fn duration_secs(&self) -> f64 {
        self.frames() as f64 / self.sample_rate.max(1) as f64
    }

    /// Average all channels into one (what `librosa.load(mono=True)` does).
    pub fn to_mono(&self) -> Vec<f32> {
        match self.channels {
            0 => Vec::new(),
            1 => self.samples.clone(),
            n => {
                let inv = 1.0 / n as f32;
                self.samples.chunks_exact(n).map(|f| f.iter().sum::<f32>() * inv).collect()
            }
        }
    }

    /// Planar stereo copy; mono is duplicated, extra channels are dropped.
    pub fn to_stereo_planar(&self) -> [Vec<f32>; 2] {
        let frames = self.frames();
        let mut l = Vec::with_capacity(frames);
        let mut r = Vec::with_capacity(frames);
        match self.channels {
            0 => {}
            1 => {
                l.extend_from_slice(&self.samples);
                r.extend_from_slice(&self.samples);
            }
            n => {
                for f in self.samples.chunks_exact(n) {
                    l.push(f[0]);
                    r.push(f[1]);
                }
            }
        }
        [l, r]
    }
}
