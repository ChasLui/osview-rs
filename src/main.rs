use osview_rs::{render, Camera, Dataset, View};
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
    yaw: f32,
    pitch: f32,
    zoom: f32,
    pan_x: i32,
    pan_y: i32,
    topk: usize,
}

fn usage(code: i32) -> ! {
    eprintln!(
        "osview-rs {} - headless point-cloud feature viewer\n\
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
  --yaw <deg>         rotate about the vertical screen axis, default 0\n\
  --pitch <deg>       tilt about the horizontal screen axis, default 0\n\
  --zoom <f>          zoom factor about the image centre, default 1\n\
  --pan-x <px>        shift right (+) or left (-) in pixels, default 0\n\
  --pan-y <px>        shift up (+) or down (-) in pixels, default 0\n\
  --radius <px>       splat half-width in px, 0..=64, default 1\n\
  --range-min <f>     heat ramp lower bound, default 0.05\n\
  --range-max <f>     heat ramp upper bound, default 0.10\n\
  --topk <n>          matches written to --report, default 500\n\
  --report <json>     write top-k matches (i, score, p) for the top --topk points\n\
  --selftest          run the built-in synthetic test and exit\n\
  -h, --help          show this text",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(code);
}

fn die(msg: &str) -> ! {
    eprintln!("osview-rs: {msg}");
    std::process::exit(2);
}

fn num<T: std::str::FromStr>(flag: &str, s: &str) -> T {
    s.parse()
        .unwrap_or_else(|_| die(&format!("bad value for {flag}: {s}")))
}

fn parse_view(s: &str) -> Option<View> {
    match s {
        "xy" => Some(View::XY),
        "xz" => Some(View::XZ),
        "yz" => Some(View::YZ),
        _ => None,
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
        yaw: 0.0,
        pitch: 0.0,
        zoom: 1.0,
        pan_x: 0,
        pan_y: 0,
        topk: 500,
    };
    let mut it = std::env::args_os().skip(1);
    let text = |a: std::ffi::OsString| {
        a.into_string()
            .unwrap_or_else(|_| die("argument is not valid UTF-8"))
    };
    while let Some(a) = it.next() {
        let a = text(a);
        let mut val = || it.next().map(text).unwrap_or_else(|| usage(2));
        match a.as_str() {
            "--points" => o.points = val(),
            "--features" => o.features = val(),
            "--query" => o.query = val(),
            "--out" => o.out = val(),
            "--report" => o.report = Some(val()),
            "--view" => {
                o.view = parse_view(&val())
                    .unwrap_or_else(|| die("bad value for --view (need xy, xz or yz)"))
            }
            "--width" => o.width = num("--width", &val()),
            "--height" => o.height = num("--height", &val()),
            "--radius" => o.radius = num("--radius", &val()),
            "--yaw" => o.yaw = num("--yaw", &val()),
            "--pitch" => o.pitch = num("--pitch", &val()),
            "--zoom" => o.zoom = num("--zoom", &val()),
            "--pan-x" => o.pan_x = num("--pan-x", &val()),
            "--pan-y" => o.pan_y = num("--pan-y", &val()),
            "--range-min" => o.range_min = num("--range-min", &val()),
            "--range-max" => o.range_max = num("--range-max", &val()),
            "--topk" => o.topk = num("--topk", &val()),
            "--selftest" => {
                selftest::run();
                std::process::exit(0);
            }
            "-h" | "--help" => usage(0),
            _ => {
                eprintln!("unknown argument: {a}");
                usage(2);
            }
        }
    }
    if !(1..=8192).contains(&o.width) || !(1..=8192).contains(&o.height) {
        die("width/height must be in 1..=8192");
    }
    if !(0..=64).contains(&o.radius) {
        die("radius must be in 0..=64");
    }
    if !o.yaw.is_finite() || !o.pitch.is_finite() {
        die("yaw/pitch must be finite degrees");
    }
    if !o.zoom.is_finite() || o.zoom <= 0.0 {
        die("zoom must be finite and > 0");
    }
    let finite = o.range_min.is_finite() && o.range_max.is_finite();
    if !finite || o.range_max <= o.range_min {
        die("range-max must be greater than range-min (both finite)");
    }
    if o.points.is_empty() || o.features.is_empty() || o.query.is_empty() || o.out.is_empty() {
        usage(2);
    }
    for p in [&o.points, &o.features, &o.query] {
        if same_file(p, &o.out) {
            die("--out must not overwrite an input file");
        }
    }
    if let Some(r) = &o.report {
        if same_file(r, &o.out) {
            die("--out and --report must differ");
        }
    }
    o
}

fn same_file(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
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
    let ds = Dataset::load(&o.points, &o.features, &o.query)?;

    let cam = Camera {
        view: o.view,
        width: o.width,
        height: o.height,
        radius: o.radius,
        range_min: o.range_min,
        range_max: o.range_max,
        yaw: o.yaw,
        pitch: o.pitch,
        zoom: o.zoom,
        pan_x: o.pan_x,
        pan_y: o.pan_y,
    };
    let rgb = render::render(&ds.cloud, &ds.scores, &cam);
    render::write_png(&o.out, o.width, o.height, &rgb).map_err(|e| format!("{}: {e}", o.out))?;

    if let Some(path) = &o.report {
        osview_rs::write_report(path, &ds, &cam, o.topk).map_err(|e| format!("{path}: {e}"))?;
    }

    let s = osview_rs::summary_json(&ds, &o.out, &cam);
    // Ignore EPIPE so `osview-rs ... | head` exits 0.
    let _ = writeln!(std::io::stdout().lock(), "{s}");
    Ok(())
}

mod selftest {
    use osview_rs::{npy, ply, render, Camera, Cloud, View};

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
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
            pan_x: 0,
            pan_y: 0,
        };
        let rgb = render::render(&cloud, &scores, &cam);
        let colored = rgb
            .chunks_exact(3)
            .filter(|px| **px != [24, 24, 28])
            .count();
        assert!(
            colored > (cam.width * cam.height) / 20,
            "image nearly empty"
        );

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
