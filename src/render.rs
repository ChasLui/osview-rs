// Headless point splat renderer: orthographic projection on one plane,
// depth buffer for occlusion, blue->yellow heat ramp over the affinity range.
use crate::ply::Cloud;

#[derive(Clone, Copy, PartialEq)]
pub enum View {
    XY,
    XZ,
    YZ,
}

impl View {
    pub fn name(self) -> &'static str {
        match self {
            View::XY => "xy",
            View::XZ => "xz",
            View::YZ => "yz",
        }
    }
}

pub struct Camera {
    pub view: View,
    pub width: usize,
    pub height: usize,
    pub radius: i32,
    pub range_min: f32,
    pub range_max: f32,
}

pub fn render(cloud: &Cloud, scores: &[f32], cam: &Camera) -> Vec<u8> {
    let (w, h) = (cam.width, cam.height);
    let mut rgb = vec![0u8; w * h * 3];
    let mut depth = vec![f32::NEG_INFINITY; w * h];
    for px in rgb.chunks_exact_mut(3) {
        px.copy_from_slice(&[24, 24, 28]);
    }

    let (ha, va) = match cam.view {
        View::XY => (0usize, 1usize),
        View::XZ => (0, 2),
        View::YZ => (1, 2),
    };
    let depth_axis = match cam.view {
        View::XY => 2usize,
        View::XZ => 1,
        View::YZ => 0,
    };

    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    for p in &cloud.xyz {
        for (k, axis) in [ha, va].iter().enumerate() {
            lo[k] = lo[k].min(p[*axis]);
            hi[k] = hi[k].max(p[*axis]);
        }
    }
    // Work with half-spans and half-offsets: (hi + lo) and (hi - lo) can
    // overflow to infinity for finite coordinates near f32::MAX.
    let span_a = (hi[0] * 0.5 - lo[0] * 0.5).abs().max(1e-6);
    let span_b = (hi[1] * 0.5 - lo[1] * 0.5).abs().max(1e-6);
    let margin = 0.04;
    let scale = ((w as f32 * (1.0 - 2.0 * margin)) / span_a)
        .min((h as f32 * (1.0 - 2.0 * margin)) / span_b);
    let ca = lo[0] * 0.5 + hi[0] * 0.5;
    let cb = lo[1] * 0.5 + hi[1] * 0.5;
    let rng = (cam.range_max - cam.range_min).max(1e-6);

    let w_i = w as i64;
    let h_i = h as i64;
    let r_i = (cam.radius as i64).min(w_i.max(h_i));
    for (i, p) in cloud.xyz.iter().enumerate() {
        let sx = (w as f32 * 0.5 + (p[ha] * 0.5 - ca * 0.5) * scale)
            .round()
            .clamp(-1e7, 1e7) as i64;
        let sy = (h as f32 * 0.5 - (p[va] * 0.5 - cb * 0.5) * scale)
            .round()
            .clamp(-1e7, 1e7) as i64;
        let d = p[depth_axis];
        let base = cloud.rgb.as_ref().map(|c| c[i]).unwrap_or([190, 190, 190]);
        let score = scores.get(i).copied().unwrap_or(f32::NEG_INFINITY);
        let color = if score >= cam.range_min {
            let t = ((score - cam.range_min) / rng).clamp(0.0, 1.0);
            heat(t)
        } else {
            base
        };
        let x0 = (sx - r_i).max(0);
        let x1 = (sx + r_i).min(w_i - 1);
        let y0 = (sy - r_i).max(0);
        let y1 = (sy + r_i).min(h_i - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let idx = y as usize * w + x as usize;
                if d >= depth[idx] {
                    depth[idx] = d;
                    let o = idx * 3;
                    rgb[o..o + 3].copy_from_slice(&color);
                }
            }
        }
    }
    rgb
}

fn heat(t: f32) -> [u8; 3] {
    // blue -> yellow, same direction as osview's default ramp
    let cold = [26.0, 60.0, 255.0];
    let warm = [255.0, 232.0, 40.0];
    let mix = |a: f32, b: f32| (a + (b - a) * t).round().clamp(0.0, 255.0) as u8;
    [
        mix(cold[0], warm[0]),
        mix(cold[1], warm[1]),
        mix(cold[2], warm[2]),
    ]
}

pub fn write_png(path: &str, width: usize, height: usize, rgb: &[u8]) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let w = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(w, width as u32, height as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer
        .write_image_data(rgb)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    writer
        .finish()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera {
            view: View::XY,
            width: 64,
            height: 64,
            radius: 1,
            range_min: 0.05,
            range_max: 0.10,
        }
    }

    fn painted(img: &[u8]) -> usize {
        img.chunks_exact(3).filter(|p| p != &[24, 24, 28]).count()
    }

    #[test]
    fn extreme_finite_coords_still_render() {
        let cloud = Cloud {
            xyz: vec![[3e38, 0.0, 0.0], [3e38, 1e38, 0.0]],
            rgb: None,
        };
        let img = render(&cloud, &[1.0, 0.0], &cam());
        assert!(
            painted(&img) > 0,
            "overflowing autoscale dropped all splats"
        );
    }

    #[test]
    fn huge_radius_does_not_wrap_the_splat_box() {
        let mut c = cam();
        c.radius = i32::MAX;
        let cloud = Cloud {
            xyz: vec![[0.0, 0.0, 0.0]],
            rgb: None,
        };
        let img = render(&cloud, &[1.0], &c);
        assert!(painted(&img) > 0, "radius overflow produced an empty splat");
    }
}
