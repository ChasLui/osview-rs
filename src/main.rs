mod npy;
mod ply;
mod render;

use render::{Camera, View};
use std::io::Write;
use std::process::ExitCode;

struct Opts {
    points: String,
    features: String,
    query: String,
    out: String,
    report: Option<String>,
    view: View,
    width: usize,
    height: usize,
    radius: i32,
    range_min: f32,
    range_max: f32,
    topk: usize,
}

fn usage() -> ! {
    eprintln!(
        "osview-rs 0.1.0 - headless point-cloud feature viewer\n\
\n\
USAGE: osview-rs --points cloud.ply --features feats.npy --query query.npy --out render.png [options]\n\
\n\
  --points <ply>      PLY point cloud (ascii or binary_little_endian)\n\
  --features <npy>    per-point features [N, D] float16/32/64, C order\n\
  --query <npy>       query vector [1, D] or [D]; rows are L2-normalized on load\n\
  --out <png>         output image\n\
  --view xy|xz|yz     projection plane (default xy)\n\
  --width <px>        default 1600\n\
  --height <px>       default 1200\n\
  --radius <px>       splat radius, default 1\n\
  --range-min <f>     heat ramp lower bound, default 0.05\n\
  --range-max <f>     heat ramp upper bound, default 0.10\n\
  --topk <n>          matches written to --report, default 500\n\
  --report <json>     write top-k matches (index, score, xyz)\n\
  --selftest          run the built-in synthetic test and exit\n\
  -h, --help          show this text"
    );
    std::process::exit(2);
}

fn parse_view(s: &str) -> View {
    match s {
        "xz" => View::XZ,
        "yz" => View::YZ,
        _ => View::XY,
    }
}

fn parse_args() -> Opts {
    let mut o = Opts {
        points: String::new(),
        features: String::new(),
        query: String::new(),
        out: String::new(),
        report: None,
        view: View::XY,
        width: 1600,
        height: 1200,
        radius: 1,
        range_min: 0.05,
        range_max: 0.10,
        topk: 500,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--points" => o.points = val(),
            "--features" => o.features = val(),
            "--query" => o.query = val(),
            "--out" => o.out = val(),
            "--report" => o.report = Some(val()),
            "--view" => o.view = parse_view(&val()),
            "--width" => o.width = val().parse().unwrap_or(1600),
            "--height" => o.height = val().parse().unwrap_or(1200),
            "--radius" => o.radius = val().parse().unwrap_or(1),
            "--range-min" => o.range_min = val().parse().unwrap_or(0.05),
            "--range-max" => o.range_max = val().parse().unwrap_or(0.10),
            "--topk" => o.topk = val().parse().unwrap_or(500),
            "--selftest" => {
                selftest::run();
                std::process::exit(0);
            }
            "-h" | "--help" => usage(),
            _ => {
                eprintln!("unknown argument: {a}");
                usage();
            }
        }
    }
    if o.points.is_empty() || o.features.is_empty() || o.query.is_empty() || o.out.is_empty() {
        usage();
    }
    o
}

fn main() -> ExitCode {
    let o = parse_args();
    match run(&o) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("osview-rs: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(o: &Opts) -> Result<(), String> {
    let cloud = ply::read(&o.points).map_err(|e| format!("{}: {e}", o.points))?;
    let feats = npy::read(&o.features).map_err(|e| format!("{}: {e}", o.features))?;
    let q = npy::read(&o.query).map_err(|e| format!("{}: {e}", o.query))?;

    if feats.rows != cloud.xyz.len() {
        return Err(format!(
            "feature rows ({}) != points ({}) — osview pairs the i-th feature row with the i-th vertex",
            feats.rows,
            cloud.xyz.len()
        ));
    }
    if q.rows != 1 {
        return Err(format!(
            "query must be a single vector, got {} rows — split class features and score per class",
            q.rows
        ));
    }
    if q.dim != feats.dim {
        return Err(format!(
            "query dim ({}) != feature dim ({}) — same CLIP text tower must have produced both",
            q.dim, feats.dim
        ));
    }

    let dim = feats.dim;
    let query = &q.data[..dim];
    if query.iter().all(|v| v.abs() < 1e-9) {
        return Err(
            "query vector is all zeros — refusing to emit an all-zero/one-color image; \
             pass a real text embedding"
                .to_string(),
        );
    }
    let mut scores = Vec::with_capacity(feats.rows);
    for row in feats.data.chunks_exact(dim) {
        let mut s = 0.0f32;
        for (a, b) in row.iter().zip(query.iter()) {
            s += a * b;
        }
        scores.push(s);
    }

    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    let topk = o.topk.min(order.len());
    let above = scores.iter().filter(|s| **s >= o.range_min).count();

    let cam = Camera {
        view: o.view,
        width: o.width,
        height: o.height,
        radius: o.radius,
        range_min: o.range_min,
        range_max: o.range_max,
    };
    let rgb = render::render(&cloud, &scores, &cam);
    render::write_png(&o.out, o.width, o.height, &rgb)
        .map_err(|e| format!("{}: {e}", o.out))?;

    if let Some(path) = &o.report {
        let mut f = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
        write!(
            f,
            "{{\"points\":{},\"dim\":{},\"view\":\"{}\",\"range\":[{},{}],\"above_min\":{},\"matches\":[",
            cloud.xyz.len(),
            dim,
            o.view.name(),
            o.range_min,
            o.range_max,
            above
        )
        .map_err(|e| e.to_string())?;
        for (k, &i) in order.iter().take(topk).enumerate() {
            let p = cloud.xyz[i];
            if k > 0 {
                write!(f, ",").map_err(|e| e.to_string())?;
            }
            write!(
                f,
                "{{\"i\":{},\"score\":{:.6},\"p\":[{:.6},{:.6},{:.6}]}}",
                i, scores[i], p[0], p[1], p[2]
            )
            .map_err(|e| e.to_string())?;
        }
        writeln!(f, "]}}").map_err(|e| e.to_string())?;
    }

    let mut out = std::io::stdout().lock();
    write!(
        out,
        "{{\"points\":{},\"dim\":{},\"view\":\"{}\",\"image\":\"{}\",\"above_min\":{},\"max\":{:.6},\"min\":{:.6},\"top\":[",
        cloud.xyz.len(),
        dim,
        o.view.name(),
        o.out,
        above,
        scores[order[0]],
        scores[order[order.len() - 1]]
    )
    .map_err(|e| e.to_string())?;
    for (k, &i) in order.iter().take(10).enumerate() {
        if k > 0 {
            write!(out, ",").map_err(|e| e.to_string())?;
        }
        let p = cloud.xyz[i];
        write!(
            out,
            "{{\"i\":{},\"score\":{:.4},\"p\":[{:.4},{:.4},{:.4}]}}",
            i, scores[i], p[0], p[1], p[2]
        )
        .map_err(|e| e.to_string())?;
    }
    writeln!(out, "]}}").map_err(|e| e.to_string())?;
    Ok(())
}

mod selftest {
    use crate::npy;
    use crate::ply::{self, Cloud};
    use crate::render::{self, Camera, View};

    pub fn run() {
        // Synthetic sphere grid with a deliberate left/right feature split.
        let n = 40usize;
        let mut xyz = Vec::with_capacity(n * n);
        for i in 0..n {
            for j in 0..n {
                let theta = std::f32::consts::PI * (i as f32 + 0.5) / n as f32;
                let phi = std::f32::consts::TAU * (j as f32 + 0.5) / n as f32;
                xyz.push([
                    theta.sin() * phi.cos(),
                    theta.sin() * phi.sin(),
                    theta.cos(),
                ]);
            }
        }
        let cloud = Cloud {
            xyz: xyz.clone(),
            rgb: None,
        };

        let dim = 16usize;
        let mut data = vec![0.0f32; xyz.len() * dim];
        let mut seed = 0x1234_5678u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1 << 24) as f32
        };
        for (i, p) in xyz.iter().enumerate() {
            let base = i * dim;
            data[base] = if p[0] > 0.0 { 1.0 } else { 0.05 };
            for k in 1..dim {
                data[base + k] = 0.02 * rnd();
            }
        }
        // Normalize rows exactly like the NPY loader would.
        for row in data.chunks_exact_mut(dim) {
            let nrm = row.iter().map(|v| v * v).sum::<f32>().sqrt();
            for v in row.iter_mut() {
                *v /= nrm;
            }
        }
        let feats = npy::Matrix {
            rows: xyz.len(),
            dim,
            data,
        };
        let mut q = vec![0.0f32; dim];
        q[0] = 1.0;

        let mut scores = Vec::with_capacity(feats.rows);
        for row in feats.data.chunks_exact(dim) {
            scores.push(row.iter().zip(q.iter()).map(|(a, b)| a * b).sum::<f32>());
        }
        let pos: f32 = xyz
            .iter()
            .zip(scores.iter())
            .filter(|(p, _)| p[0] > 0.0)
            .map(|(_, s)| *s)
            .sum::<f32>()
            / xyz.iter().filter(|p| p[0] > 0.0).count() as f32;
        let neg: f32 = xyz
            .iter()
            .zip(scores.iter())
            .filter(|(p, _)| p[0] <= 0.0)
            .map(|(_, s)| *s)
            .sum::<f32>()
            / xyz.iter().filter(|p| p[0] <= 0.0).count() as f32;
        assert!(pos > neg + 0.2, "ranking failed: pos={pos} neg={neg}");

        let cam = Camera {
            view: View::XY,
            width: 320,
            height: 240,
            radius: 1,
            range_min: 0.05,
            range_max: 0.10,
        };
        let rgb = render::render(&cloud, &scores, &cam);
        let colored = rgb
            .chunks_exact(3)
            .filter(|px| **px != [24, 24, 28])
            .count();
        assert!(colored > (cam.width * cam.height) / 20, "image nearly empty");

        let tmp = std::env::temp_dir().join("osview-rs-selftest.ply");
        let path = tmp.to_str().unwrap().to_string();
        let text = "ply\nformat ascii 1.0\nelement vertex 2\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n1 1 1\n";
        std::fs::write(&path, text.as_bytes()).unwrap();
        let c2 = ply::read(&path).unwrap();
        assert_eq!(c2.xyz.len(), 2);
        assert!((c2.xyz[1][2] - 1.0).abs() < 1e-6);
        let _ = std::fs::remove_file(&path);

        println!("selftest passed: ranking, render coverage, PLY ascii");
    }
}