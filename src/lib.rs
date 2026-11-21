//! osview-rs core: PLY + NPY loading, cosine scoring, splat rendering and the
//! JSON report shared by the CLI, the GUI and the C SDK.
//!
//! The i-th feature row belongs to the i-th PLY vertex. Feature rows are
//! L2-normalized on load; the query is normalized the same way. `Dataset`
//! refuses mismatched shapes and all-zero queries so downstream surfaces only
//! ever see a scorable pair.

use std::fmt::Write as _;
use std::io;

pub mod ffi;
pub mod npy;
pub mod ply;
pub mod render;

pub use npy::Matrix;
pub use ply::Cloud;
pub use render::{Camera, View};

/// A point cloud, its per-point feature rows, the query and the cosine scores.
#[derive(Debug)]
pub struct Dataset {
    pub cloud: Cloud,
    pub features: Matrix,
    pub query: Vec<f32>,
    pub scores: Vec<f32>,
}

/// One scored point as emitted in the top-k report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Match {
    pub index: usize,
    pub score: f32,
    pub p: [f32; 3],
}

impl Dataset {
    /// Read the three inputs and validate/score them.
    pub fn load(points: &str, features: &str, query: &str) -> Result<Self, String> {
        let cloud = ply::read(points).map_err(|e| format!("{points}: {e}"))?;
        let feats = npy::read(features).map_err(|e| format!("{features}: {e}"))?;
        let q = npy::read(query).map_err(|e| format!("{query}: {e}"))?;
        Self::from_parts(cloud, feats, q)
    }

    /// Validate an already-loaded triple and compute its scores.
    pub fn from_parts(cloud: Cloud, features: Matrix, query: Matrix) -> Result<Self, String> {
        if features.rows != cloud.xyz.len() {
            return Err(format!(
                "feature rows ({}) != points ({}) — osview pairs the i-th feature row with the i-th vertex",
                features.rows,
                cloud.xyz.len()
            ));
        }
        if query.rows != 1 {
            return Err(format!(
                "query must be a single vector, got {} rows — split class features and score per class",
                query.rows
            ));
        }
        if query.dim != features.dim {
            return Err(format!(
                "query dim ({}) != feature dim ({}) — same CLIP text tower must have produced both",
                query.dim, features.dim
            ));
        }
        if query.data.iter().all(|v| v.abs() < 1e-9) {
            return Err(
                "query vector is all zeros — refusing to emit an all-zero/one-color image; \
                 pass a real text embedding"
                    .to_string(),
            );
        }
        let dim = features.dim;
        let q = &query.data[..dim];
        let mut scores = Vec::with_capacity(features.rows);
        for row in features.data.chunks_exact(dim) {
            let mut s = 0.0f32;
            for (a, b) in row.iter().zip(q.iter()) {
                s += a * b;
            }
            scores.push(s);
        }
        Ok(Self {
            cloud,
            features,
            query: q.to_vec(),
            scores,
        })
    }

    /// Score-descending order; ties keep ascending vertex index.
    pub fn sorted_indices(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.scores.len()).collect();
        order.sort_by(|&a, &b| self.scores[b].total_cmp(&self.scores[a]));
        order
    }

    /// Number of points at or above the heat ramp's lower bound.
    pub fn above_min(&self, range_min: f32) -> usize {
        self.scores.iter().filter(|s| **s >= range_min).count()
    }

    /// Top `k` matches in score-descending order.
    pub fn top_k(&self, k: usize) -> Vec<Match> {
        self.sorted_indices()
            .into_iter()
            .take(k.min(self.scores.len()))
            .map(|index| Match {
                index,
                score: self.scores[index],
                p: self.cloud.xyz[index],
            })
            .collect()
    }
}

/// Write the machine-readable top-k report (`--report`).
pub fn write_report(path: &str, ds: &Dataset, cam: &Camera, topk: usize) -> io::Result<()> {
    let order = ds.sorted_indices();
    let above = ds.above_min(cam.range_min);
    let topk = topk.min(order.len());
    let mut s = String::new();
    write!(
        s,
        "{{\"points\":{},\"dim\":{},\"view\":\"{}\",\"camera\":{{\"yaw\":{},\"pitch\":{},\"zoom\":{},\"pan\":[{},{}]}},\"range\":[{},{}],\"above_min\":{},\"matches\":[",
        ds.cloud.xyz.len(),
        ds.features.dim,
        cam.view.name(),
        cam.yaw,
        cam.pitch,
        cam.zoom,
        cam.pan_x,
        cam.pan_y,
        cam.range_min,
        cam.range_max,
        above
    )
    .expect("writing to a String cannot fail");
    for (k, &i) in order.iter().take(topk).enumerate() {
        let p = ds.cloud.xyz[i];
        if k > 0 {
            s.push(',');
        }
        write!(
            s,
            "{{\"i\":{},\"score\":{:.6},\"p\":[{:.6},{:.6},{:.6}]}}",
            i, ds.scores[i], p[0], p[1], p[2]
        )
        .expect("writing to a String cannot fail");
    }
    s.push_str("]}\n");
    std::fs::write(path, s)
}

/// One-line stdout summary with the top 10 matches (4 decimals).
pub fn summary_json(ds: &Dataset, image: &str, cam: &Camera) -> String {
    let order = ds.sorted_indices();
    let above = ds.above_min(cam.range_min);
    let mut s = String::new();
    write!(
        s,
        "{{\"points\":{},\"dim\":{},\"view\":\"{}\",\"image\":\"{}\",\"above_min\":{},\"max\":{:.6},\"min\":{:.6},\"top\":[",
        ds.cloud.xyz.len(),
        ds.features.dim,
        cam.view.name(),
        json_str(image),
        above,
        ds.scores[order[0]],
        ds.scores[order[order.len() - 1]]
    )
    .expect("writing to a String cannot fail");
    for (k, &i) in order.iter().take(10).enumerate() {
        if k > 0 {
            s.push(',');
        }
        let p = ds.cloud.xyz[i];
        write!(
            s,
            "{{\"i\":{},\"score\":{:.4},\"p\":[{:.4},{:.4},{:.4}]}}",
            i, ds.scores[i], p[0], p[1], p[2]
        )
        .expect("writing to a String cannot fail");
    }
    s.push_str("]}");
    s
}

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                write!(o, "\\u{:04x}", c as u32).expect("writing to a String cannot fail")
            }
            c => o.push(c),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> Dataset {
        let cloud = Cloud {
            xyz: vec![[0.0, 0.0, 0.0], [1.0, 2.0, 3.0]],
            rgb: None,
        };
        let feats = Matrix {
            rows: 2,
            dim: 2,
            data: vec![1.0, 0.0, 0.0, 1.0],
        };
        let query = Matrix {
            rows: 1,
            dim: 2,
            data: vec![1.0, 0.0],
        };
        Dataset::from_parts(cloud, feats, query).unwrap()
    }

    #[test]
    fn scores_and_orders_matches() {
        let ds = tiny();
        assert_eq!(ds.scores, vec![1.0, 0.0]);
        assert_eq!(ds.top_k(1)[0].index, 0);
        let all = ds.top_k(10);
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].index, 1);
        assert_eq!(ds.above_min(0.5), 1);
    }

    #[test]
    fn rejects_shape_and_query_mismatches() {
        let cloud = Cloud {
            xyz: vec![[0.0; 3]],
            rgb: None,
        };
        let feats = Matrix {
            rows: 2,
            dim: 2,
            data: vec![1.0, 0.0, 0.0, 1.0],
        };
        let query = Matrix {
            rows: 1,
            dim: 2,
            data: vec![1.0, 0.0],
        };
        let err = Dataset::from_parts(cloud, feats, query).unwrap_err();
        assert!(err.contains("feature rows"));
    }
}
