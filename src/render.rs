// Headless point splat renderer: orthographic projection on one plane,
// depth buffer for occlusion, blue->yellow heat ramp over the affinity range.
use crate::ply::Cloud;

#[derive(Clone, Copy, Debug, PartialEq)]
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
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub pan_x: i32,
    pub pan_y: i32,
}

pub fn render(cloud: &Cloud, scores: &[f32], cam: &Camera) -> Vec<u8> {
    let (w, h) = (cam.width, cam.height);
    let mut rgb = vec![0u8; w * h * 3];
    let mut depth = vec![f64::NEG_INFINITY; w * h];
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

    // Rotation happens in f64: rotating coordinates near f32::MAX overflows
    // in f32 (a*cos + d*sin can exceed f32::MAX). yaw turns about the
    // vertical screen axis, pitch tilts about the horizontal one.
    let yaw = (cam.yaw as f64).to_radians();
    let pitch = (cam.pitch as f64).to_radians();
    let (sin_y, cos_y) = yaw.sin_cos();
    let (sin_p, cos_p) = pitch.sin_cos();
    let project = |p: &[f32; 3]| {
        let a = p[ha] as f64;
        let b = p[va] as f64;
        let d = p[depth_axis] as f64;
        let a1 = a * cos_y + d * sin_y;
        let d1 = -a * sin_y + d * cos_y;
        (a1, b * cos_p - d1 * sin_p, b * sin_p + d1 * cos_p)
    };

    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in &cloud.xyz {
        let (a, b, _) = project(p);
        lo[0] = lo[0].min(a);
        hi[0] = hi[0].max(a);
        lo[1] = lo[1].min(b);
        hi[1] = hi[1].max(b);
    }
    // Work with half-spans and half-offsets: (hi + lo) and (hi - lo) can
    // overflow to infinity for finite coordinates near the type's max.
    let span_a = (hi[0] * 0.5 - lo[0] * 0.5).abs().max(1e-6);
    let span_b = (hi[1] * 0.5 - lo[1] * 0.5).abs().max(1e-6);
    let margin = 0.04;
    let scale = ((w as f64 * (1.0 - 2.0 * margin)) / span_a)
        .min((h as f64 * (1.0 - 2.0 * margin)) / span_b)
        * cam.zoom as f64;
    let ca = lo[0] * 0.5 + hi[0] * 0.5;
    let cb = lo[1] * 0.5 + hi[1] * 0.5;
    let rng = (cam.range_max - cam.range_min).max(1e-6);

    let w_i = w as i64;
    let h_i = h as i64;
    let r_i = (cam.radius as i64).min(w_i.max(h_i));
    let pan_x = cam.pan_x as f64;
    let pan_y = cam.pan_y as f64;
    for (i, p) in cloud.xyz.iter().enumerate() {
        let (a, b, d) = project(p);
        let sx = (w as f64 * 0.5 + (a * 0.5 - ca * 0.5) * scale + pan_x)
            .round()
            .clamp(-1e7, 1e7) as i64;
        let sy = (h as f64 * 0.5 - (b * 0.5 - cb * 0.5) * scale - pan_y)
            .round()
            .clamp(-1e7, 1e7) as i64;
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
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
            pan_x: 0,
            pan_y: 0,
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

    #[test]
    fn yaw_turns_the_projection_plane() {
        let cloud = Cloud {
            xyz: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            rgb: None,
        };
        let mut c = cam();
        c.radius = 0;
        assert_eq!(painted(&render(&cloud, &[1.0, 1.0], &c)), 2);
        c.yaw = 90.0;
        assert_eq!(
            painted(&render(&cloud, &[1.0, 1.0], &c)),
            1,
            "yaw=90 should turn +x into the depth axis"
        );
    }

    #[test]
    fn rotation_works_for_every_view() {
        let cloud = Cloud {
            xyz: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            rgb: None,
        };
        for view in [View::XY, View::XZ, View::YZ] {
            let mut base = cam();
            base.view = view;
            base.zoom = 1.1;
            base.pan_x = 4;
            base.pan_y = -3;
            let mut rot = cam();
            rot.view = view;
            rot.zoom = base.zoom;
            rot.pan_x = base.pan_x;
            rot.pan_y = base.pan_y;
            rot.yaw = 30.0;
            rot.pitch = 15.0;
            let plain = render(&cloud, &[1.0; 4], &base);
            let yawed = render(&cloud, &[1.0; 4], &rot);
            assert_eq!(plain.len(), 64 * 64 * 3);
            assert!(
                painted(&yawed) > 0,
                "{} lost every point under rotation",
                view.name()
            );
            assert_ne!(
                plain,
                yawed,
                "{} ignored yaw/pitch (identical images)",
                view.name()
            );
        }
    }

    #[test]
    fn depth_order_survives_rotation_beyond_f32_range() {
        let mut c = cam();
        c.radius = 0;
        c.yaw = 45.0;
        let cloud = Cloud {
            xyz: vec![
                [-3.181_980_6e38, 0.0, 3.181_980_6e38],
                [-2.474_873_6e38, 0.0, 2.474_873_6e38],
                [1e38, 0.0, 0.0],
            ],
            rgb: None,
        };
        let img = render(&cloud, &[1.0, 0.0, 0.0], &c);
        assert!(
            img.chunks_exact(3).any(|px| px == [255, 232, 40]),
            "the nearer point must win even when both depths exceed f32::MAX"
        );
    }

    #[test]
    fn rotation_near_f32_max_stays_finite() {
        let mut c = cam();
        c.radius = 0;
        c.yaw = 45.0;
        let cloud = Cloud {
            xyz: vec![[3e38, 0.0, 3e38]],
            rgb: None,
        };
        let img = render(&cloud, &[1.0], &c);
        let center = (32 * 64 + 32) * 3;
        assert_ne!(
            &img[center..center + 3],
            [24, 24, 28],
            "rotation at f32::MAX overflowed; the point should land at the centre"
        );
    }

    #[test]
    fn zoom_scales_and_pan_shifts_pixels() {
        let cloud = Cloud {
            xyz: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            rgb: None,
        };
        let cols = |img: &[u8]| {
            let (mut lo, mut hi) = (usize::MAX, 0usize);
            for (x, px) in img.chunks_exact(3).enumerate() {
                if px != [24, 24, 28] {
                    lo = lo.min(x % 64);
                    hi = hi.max(x % 64);
                }
            }
            (lo, hi)
        };
        let mut c = cam();
        c.radius = 0;
        let (lo1, hi1) = cols(&render(&cloud, &[1.0, 1.0], &c));
        c.zoom = 0.5;
        let (lo2, hi2) = cols(&render(&cloud, &[1.0, 1.0], &c));
        assert!(
            hi2 - lo2 < hi1 - lo1,
            "zoom=0.5 should narrow the projected span"
        );
        c.zoom = 1.0;
        c.pan_x = 2;
        let (lo3, hi3) = cols(&render(&cloud, &[1.0, 1.0], &c));
        assert_eq!((lo3, hi3), (lo1 + 2, hi1 + 2), "pan-x should shift pixels");
    }
}
