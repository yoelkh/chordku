//! Viterbi decoding for a "sticky" chord HMM.
//!
//! Every state has the same self-transition probability and spreads the rest uniformly
//! over the other states. With that structure the best predecessor of state `j` is either
//! `j` itself or the globally best state, so decoding is O(T*K) instead of O(T*K^2).
//! The switch cost may vary per step, which is how chord changes are made cheaper on
//! downbeats than in the middle of a bar.

/// Decode the most likely state path.
///
/// * `emissions`: row-major `[steps, k]` log-likelihoods.
/// * `switch_cost[t]`: non-negative log-domain penalty for changing state between
///   step `t - 1` and `t` (index 0 is unused).
pub fn decode(emissions: &[f32], k: usize, switch_cost: &[f32]) -> Vec<u16> {
    let steps = if k == 0 { 0 } else { emissions.len() / k };
    if steps == 0 {
        return Vec::new();
    }
    assert_eq!(switch_cost.len(), steps, "one switch cost per step");

    let mut delta: Vec<f32> = emissions[..k].to_vec();
    let mut next = vec![0f32; k];
    // back[t * k + j] = predecessor of state j at step t
    let mut back = vec![0u16; steps * k];

    for t in 1..steps {
        let (best_i, best_v) = argmax(&delta);
        let via_switch = best_v - switch_cost[t];
        let e = &emissions[t * k..(t + 1) * k];
        let b = &mut back[t * k..(t + 1) * k];
        for j in 0..k {
            if delta[j] >= via_switch {
                next[j] = delta[j] + e[j];
                b[j] = j as u16;
            } else {
                next[j] = via_switch + e[j];
                b[j] = best_i as u16;
            }
        }
        std::mem::swap(&mut delta, &mut next);
    }

    let mut path = vec![0u16; steps];
    let mut state = argmax(&delta).0;
    for t in (0..steps).rev() {
        path[t] = state as u16;
        state = back[t * k + state] as usize;
    }
    path
}

fn argmax(v: &[f32]) -> (usize, f32) {
    let mut best = 0;
    for i in 1..v.len() {
        if v[i] > v[best] {
            best = i;
        }
    }
    (best, v[best])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_single_step_flicker() {
        // Three states; state 0 dominates except one weak blip of state 1.
        let mut e = Vec::new();
        for t in 0..9 {
            if t == 4 {
                e.extend_from_slice(&[-1.2, -1.0, -5.0]);
            } else {
                e.extend_from_slice(&[-0.1, -3.0, -5.0]);
            }
        }
        let path = decode(&e, 3, &[2.0; 9]);
        assert_eq!(path, vec![0; 9]);
        // Without a switch cost the blip survives.
        let raw = decode(&e, 3, &[0.0; 9]);
        assert_eq!(raw[4], 1);
    }

    #[test]
    fn follows_a_real_change() {
        let mut e = Vec::new();
        for t in 0..10 {
            if t < 5 {
                e.extend_from_slice(&[-0.1, -4.0]);
            } else {
                e.extend_from_slice(&[-4.0, -0.1]);
            }
        }
        let path = decode(&e, 2, &[2.0; 10]);
        assert_eq!(path, vec![0, 0, 0, 0, 0, 1, 1, 1, 1, 1]);
    }

    #[test]
    fn cheap_switch_attracts_the_change() {
        // Ambiguous evidence at steps 3..5; the change should land where switching is free.
        let e: Vec<f32> = vec![
            -0.1, -3.0, -0.1, -3.0, -0.1, -3.0, // 0..2 clearly state 0
            -0.7, -0.7, -0.7, -0.7, // 3..4 ambiguous
            -3.0, -0.1, -3.0, -0.1, -3.0, -0.1, // 5..7 clearly state 1
        ];
        let mut cost = vec![3.0; 8];
        cost[4] = 0.5;
        let path = decode(&e, 2, &cost);
        assert_eq!(path, vec![0, 0, 0, 0, 1, 1, 1, 1]);
    }
}
