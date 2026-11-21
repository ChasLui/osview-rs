// Interactive minifb front end: load -> score -> render -> present.
// Rendering only runs when input or size changed; limit_update_rate paces
// the event poll so the loop never spins.
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use osview_rs::{render, Camera, Dataset, View};
use std::process::ExitCode;
use std::time::Duration;

struct Opts {
    points: String,
    features: String,
    query: String,
    range_min: f32,
    range_max: f32,
    view: View,
    width: usize,
    height: usize,
    radius: i32,
    frames: usize,
}

fn usage(code: i32) -> ! {
    eprintln!(
        "osview-gui {} - interactive point-cloud feature viewer\n\
\n\
USAGE: osview-gui --points cloud.ply --features feats.npy --query query.npy [options]\n\
\n\
  --points <ply>      PLY point cloud\n\
  --features <npy>    per-point features [N, D]\n\
  --query <npy>       query vector [1, D] or [D]\n\
  --range-min <f>     heat ramp lower bound, default 0.05\n\
  --range-max <f>     heat ramp upper bound, default 0.10\n\
  --view xy|xz|yz     projection plane, default xy\n\
  --width <px>        initial width, 320..=4096, default 1280\n\
  --height <px>       initial height, 320..=4096, default 900\n\
  --radius <px>       splat half-width, 0..=64, default 1\n\
  --frames <n>        present n frames and exit (0 = interactive, default)\n\
  -h, --help          show this text",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(code);
}

fn die(msg: &str) -> ! {
    eprintln!("osview-gui: {msg}");
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
        range_min: 0.05,
        range_max: 0.10,
        view: View::XY,
        width: 1280,
        height: 900,
        radius: 1,
        frames: 0,
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
            "--range-min" => o.range_min = num("--range-min", &val()),
            "--range-max" => o.range_max = num("--range-max", &val()),
            "--view" => {
                o.view = parse_view(&val())
                    .unwrap_or_else(|| die("bad value for --view (need xy, xz or yz)"))
            }
            "--width" => o.width = num::<usize>("--width", &val()).clamp(320, 4096),
            "--height" => o.height = num::<usize>("--height", &val()).clamp(320, 4096),
            "--radius" => o.radius = num("--radius", &val()),
            "--frames" => o.frames = num("--frames", &val()),
            "-h" | "--help" => usage(0),
            _ => {
                eprintln!("unknown argument: {a}");
                usage(2);
            }
        }
    }
    if o.points.is_empty() || o.features.is_empty() || o.query.is_empty() {
        usage(2);
    }
    if !(0..=64).contains(&o.radius) {
        die("radius must be in 0..=64");
    }
    let finite = o.range_min.is_finite() && o.range_max.is_finite();
    if !finite || o.range_max <= o.range_min {
        die("range-max must be greater than range-min (both finite)");
    }
    o
}

fn main() -> ExitCode {
    let o = parse_args();
    match run(&o) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("osview-gui: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(o: &Opts) -> Result<(), String> {
    let ds = Dataset::load(&o.points, &o.features, &o.query)?;
    let mut cam = Camera {
        view: o.view,
        width: o.width,
        height: o.height,
        radius: o.radius,
        range_min: o.range_min,
        range_max: o.range_max,
        yaw: 0.0,
        pitch: 0.0,
        zoom: 1.0,
        pan_x: 0,
        pan_y: 0,
    };
    let title = format!(
        "osview-rs {} — {} points, dim {}",
        env!("CARGO_PKG_VERSION"),
        ds.cloud.xyz.len(),
        ds.features.dim
    );
    let mut window = Window::new(
        &title,
        cam.width,
        cam.height,
        WindowOptions {
            resize: true,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| format!("cannot create window: {e}"))?;
    #[allow(deprecated)]
    window.limit_update_rate(Some(Duration::from_millis(16)));

    println!(
        "left drag: orbit\n\
right drag or shift+left drag: pan\n\
wheel: zoom\n\
1/2/3: view xy/xz/yz\n\
[ / ]: range-max -/+ 10%\n\
r: reset camera\n\
q/Esc: quit"
    );

    let mut buf = render_buffer(&ds, &cam);

    if o.frames > 0 {
        for _ in 0..o.frames {
            window
                .update_with_buffer(&buf, cam.width, cam.height)
                .map_err(|e| format!("cannot present frame: {e}"))?;
        }
        let painted = buf.iter().filter(|&&v| v != 0x0018_181C).count();
        println!("rendered {}x{}, painted {painted}", cam.width, cam.height);
        return Ok(());
    }

    let mut last_mouse: Option<(f32, f32)> = None;
    while window.is_open() {
        window
            .update_with_buffer(&buf, cam.width, cam.height)
            .map_err(|e| format!("cannot present frame: {e}"))?;
        if !window.is_open() {
            break;
        }

        let mut dirty = false;
        let (ww, wh) = window.get_size();
        if (ww, wh) != (cam.width, cam.height) && ww > 0 && wh > 0 {
            cam.width = ww;
            cam.height = wh;
            dirty = true;
        }

        for k in window.get_keys_pressed(KeyRepeat::No) {
            match k {
                Key::Escape | Key::Q => return Ok(()),
                Key::Key1 => cam.view = View::XY,
                Key::Key2 => cam.view = View::XZ,
                Key::Key3 => cam.view = View::YZ,
                Key::LeftBracket => {
                    let step = (cam.range_max - cam.range_min) * 0.1;
                    if cam.range_max - step > cam.range_min {
                        cam.range_max -= step;
                    } else {
                        continue;
                    }
                }
                Key::RightBracket => {
                    cam.range_max += (cam.range_max - cam.range_min) * 0.1;
                }
                Key::R => {
                    cam.yaw = 0.0;
                    cam.pitch = 0.0;
                    cam.zoom = 1.0;
                    cam.pan_x = 0;
                    cam.pan_y = 0;
                }
                _ => continue,
            }
            dirty = true;
        }

        let left = window.get_mouse_down(MouseButton::Left);
        let right = window.get_mouse_down(MouseButton::Right);
        let shift = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
        let pos = window.get_mouse_pos(MouseMode::Clamp);
        if left || right {
            if let (Some(p), Some(prev)) = (pos, last_mouse) {
                let (dx, dy) = (p.0 - prev.0, p.1 - prev.1);
                if dx != 0.0 || dy != 0.0 {
                    if right || shift {
                        cam.pan_x += dx as i32;
                        cam.pan_y -= dy as i32;
                    } else {
                        cam.yaw += dx * 0.4;
                        cam.pitch = (cam.pitch + dy * 0.4).clamp(-89.0, 89.0);
                    }
                    dirty = true;
                }
            }
            last_mouse = pos;
        } else {
            last_mouse = None;
        }

        if let Some((_, dy)) = window.get_scroll_wheel() {
            if dy != 0.0 && dy.is_finite() {
                cam.zoom = (cam.zoom * 1.15f32.powf(dy)).clamp(0.01, 100.0);
                dirty = true;
            }
        }

        if dirty {
            buf = render_buffer(&ds, &cam);
            window
                .update_with_buffer(&buf, cam.width, cam.height)
                .map_err(|e| format!("cannot present frame: {e}"))?;
        }
    }
    Ok(())
}

fn render_buffer(ds: &Dataset, cam: &Camera) -> Vec<u32> {
    let rgb = render::render(&ds.cloud, &ds.scores, cam);
    rgb.chunks_exact(3)
        .map(|p| ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32)
        .collect()
}
