// Minimal PLY reader for point-cloud viewers:
// supports ascii and binary_little_endian, vertex element only.
use std::fs;
use std::io;

pub struct Cloud {
    pub xyz: Vec<[f32; 3]>,
    pub rgb: Option<Vec<[u8; 3]>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Ty {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Ty {
    fn size(self) -> usize {
        match self {
            Ty::I8 | Ty::U8 => 1,
            Ty::I16 | Ty::U16 => 2,
            Ty::I32 | Ty::U32 | Ty::F32 => 4,
            Ty::F64 => 8,
        }
    }
    fn parse(s: &str) -> Option<Ty> {
        Some(match s {
            "char" | "int8" => Ty::I8,
            "uchar" | "uint8" => Ty::U8,
            "short" | "int16" => Ty::I16,
            "ushort" | "uint16" => Ty::U16,
            "int" | "int32" => Ty::I32,
            "uint" | "uint32" => Ty::U32,
            "float" | "float32" => Ty::F32,
            "double" | "float64" => Ty::F64,
            _ => return None,
        })
    }
}

struct Prop {
    name: String,
    ty: Ty,
}

pub fn read(path: &str) -> io::Result<Cloud> {
    let bytes = read_regular(path)?;
    let hdr_end = find_header_end(&bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "PLY: no end_header"))?;
    let header = std::str::from_utf8(&bytes[..hdr_end.0])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "PLY: non-utf8 header"))?;
    let body = &bytes[hdr_end.1..];

    let mut format = String::new();
    let mut n_vertex = 0usize;
    let mut in_vertex = false;
    let mut saw_other = false;
    let mut props: Vec<Prop> = Vec::new();

    for line in header.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("format ") {
            format = rest.split_whitespace().next().unwrap_or("").to_string();
        } else if let Some(rest) = line.strip_prefix("element ") {
            let mut it = rest.split_whitespace();
            let name = it.next().unwrap_or("");
            let count_tok = it.next().unwrap_or("0");
            let count: usize = count_tok.parse().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("PLY: bad element count {count_tok:?}"),
                )
            })?;
            if name == "vertex" {
                if saw_other {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "PLY: vertex element must come first (cannot skip other elements)",
                    ));
                }
                in_vertex = true;
                n_vertex = count;
            } else {
                in_vertex = false;
                saw_other = true;
            }
        } else if let Some(rest) = line.strip_prefix("property ") {
            if !in_vertex {
                continue;
            }
            let mut it = rest.split_whitespace();
            let ty = it.next().unwrap_or("");
            if ty == "list" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "PLY: list property inside vertex element",
                ));
            }
            let name = it.next().unwrap_or("").to_string();
            let ty = Ty::parse(ty).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, format!("PLY: bad type {ty}"))
            })?;
            props.push(Prop { name, ty });
        }
    }

    if n_vertex == 0 || props.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PLY: no vertex element/properties",
        ));
    }
    let idx = |name: &str| props.iter().position(|p| p.name == name);
    let (ix, iy, iz) = match (idx("x"), idx("y"), idx("z")) {
        (Some(a), Some(b), Some(c)) => (a, b, c),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "PLY: vertices lack x/y/z",
            ))
        }
    };
    let (ir, ig, ib) = (idx("red"), idx("green"), idx("blue"));
    let has_rgb = ir.is_some() && ig.is_some() && ib.is_some();

    let stride: usize = props.iter().map(|p| p.ty.size()).sum();
    let need = n_vertex
        .checked_mul(stride)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "PLY: vertex count too large"))?;
    // Cap the allocation hint: a lying huge count must not panic on
    // with_capacity; the format loops below reject truncation instead.
    let mut xyz = Vec::with_capacity(n_vertex.min(4096));
    let mut rgb = if has_rgb {
        Some(Vec::with_capacity(n_vertex.min(4096)))
    } else {
        None
    };

    match format.as_str() {
        "ascii" => {
            let text = std::str::from_utf8(body)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "PLY: non-utf8 ascii"))?;
            let mut lines = text.lines().filter(|l| !l.trim().is_empty());
            for _ in 0..n_vertex {
                let line = lines.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "PLY: truncated ascii body")
                })?;
                let nums: Vec<f32> = line
                    .split_whitespace()
                    .take(props.len())
                    .map(|v| {
                        let f: f32 = v.parse().map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("PLY: bad number {v:?} in ascii row"),
                            )
                        })?;
                        if !f.is_finite() {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("PLY: non-finite value {v:?} in ascii row"),
                            ));
                        }
                        Ok(f)
                    })
                    .collect::<io::Result<Vec<_>>>()?;
                if nums.len() < props.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "PLY: short ascii row",
                    ));
                }
                xyz.push([nums[ix], nums[iy], nums[iz]]);
                if let Some(r) = rgb.as_mut() {
                    let (cr, cg, cb) = (ir.unwrap(), ig.unwrap(), ib.unwrap());
                    r.push([
                        to_u8(nums[cr], props[cr].ty),
                        to_u8(nums[cg], props[cg].ty),
                        to_u8(nums[cb], props[cb].ty),
                    ]);
                }
            }
        }
        "binary_little_endian" => {
            if body.len() < need {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "PLY: truncated body: header claims {n_vertex} vertices ({need} bytes), body is {}",
                        body.len()
                    ),
                ));
            }
            let mut offs = Vec::with_capacity(props.len());
            let mut o = 0usize;
            for p in &props {
                offs.push(o);
                o += p.ty.size();
            }
            for i in 0..n_vertex {
                let row = &body[i * stride..(i + 1) * stride];
                let val = |pi: usize| read_val(row, offs[pi], props[pi].ty);
                let p = [val(ix), val(iy), val(iz)];
                if !p.iter().all(|v| v.is_finite()) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("PLY: non-finite coordinate at vertex {i}"),
                    ));
                }
                xyz.push(p);
                if let Some(r) = rgb.as_mut() {
                    let (cr, cg, cb) = (ir.unwrap(), ig.unwrap(), ib.unwrap());
                    r.push([
                        to_u8(val(cr), props[cr].ty),
                        to_u8(val(cg), props[cg].ty),
                        to_u8(val(cb), props[cb].ty),
                    ]);
                }
            }
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("PLY: unsupported format {other} (need ascii or binary_little_endian)"),
            ))
        }
    }

    Ok(Cloud { xyz, rgb })
}

fn read_regular(path: &str) -> io::Result<Vec<u8>> {
    let md = fs::metadata(path)?;
    if !md.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    if md.len() > 8 << 30 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "input larger than 8 GiB",
        ));
    }
    fs::read(path)
}

fn to_u8(v: f32, ty: Ty) -> u8 {
    // Wide integer colors are scaled by their max (e.g. ushort 0-65535);
    // float colors are assumed 0-1, byte colors 0-255.
    let v = match ty {
        Ty::I16 | Ty::U16 => v / 65535.0 * 255.0,
        Ty::I32 | Ty::U32 => v / 4294967295.0 * 255.0,
        _ => v,
    };
    if ty == Ty::F32 || ty == Ty::F64 {
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        v.clamp(0.0, 255.0).round() as u8
    }
}

fn read_val(row: &[u8], off: usize, ty: Ty) -> f32 {
    match ty {
        Ty::I8 => row[off] as i8 as f32,
        Ty::U8 => row[off] as f32,
        Ty::I16 => i16::from_le_bytes([row[off], row[off + 1]]) as f32,
        Ty::U16 => u16::from_le_bytes([row[off], row[off + 1]]) as f32,
        Ty::I32 => i32::from_le_bytes([row[off], row[off + 1], row[off + 2], row[off + 3]]) as f32,
        Ty::U32 => u32::from_le_bytes([row[off], row[off + 1], row[off + 2], row[off + 3]]) as f32,
        Ty::F32 => f32::from_le_bytes([row[off], row[off + 1], row[off + 2], row[off + 3]]),
        Ty::F64 => f64::from_le_bytes([
            row[off],
            row[off + 1],
            row[off + 2],
            row[off + 3],
            row[off + 4],
            row[off + 5],
            row[off + 6],
            row[off + 7],
        ]) as f32,
    }
}

fn find_header_end(bytes: &[u8]) -> Option<(usize, usize)> {
    let needle = b"end_header";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle
            && (i == 0 || bytes[i - 1] == b'\n')
            && matches!(
                bytes.get(i + needle.len()),
                None | Some(b'\n') | Some(b'\r')
            )
        {
            let mut j = i + needle.len();
            while j < bytes.len() && bytes[j] != b'\n' {
                j += 1;
            }
            return Some((i, (j + 1).min(bytes.len())));
        }
        i += 1;
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_binary_little_endian_with_colors() {
        let header = b"ply\nformat binary_little_endian 1.0\nelement vertex 2\n\
property float x\nproperty float y\nproperty float z\n\
property uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n";
        let mut bytes = header.to_vec();
        for (p, c) in [([0f32, 0., 0.], [255u8, 0, 0]), ([1., 2., 3.], [0, 255, 0])] {
            for v in p {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            bytes.extend_from_slice(&c);
        }
        let tmp = std::env::temp_dir().join("osview-rs-test.ply");
        std::fs::write(&tmp, &bytes).unwrap();
        let c = read(tmp.to_str().unwrap()).unwrap();
        assert_eq!(c.xyz.len(), 2);
        assert!((c.xyz[1][2] - 3.0).abs() < 1e-6);
        assert_eq!(c.rgb.unwrap()[1], [0, 255, 0]);
        let _ = std::fs::remove_file(tmp);
    }

    fn write_read(text: &str, name: &str) -> io::Result<Cloud> {
        let tmp = std::env::temp_dir().join(name);
        std::fs::write(&tmp, text).unwrap();
        let r = read(tmp.to_str().unwrap());
        let _ = std::fs::remove_file(tmp);
        r
    }

    #[test]
    fn rejects_bad_ascii_numbers() {
        let text = "ply\nformat ascii 1.0\nelement vertex 2\nproperty float x\n\
property float y\nproperty float z\nend_header\n0 0 0\n1 oops 1\n";
        assert!(write_read(text, "osview-rs-badascii.ply").is_err());
    }

    #[test]
    fn rejects_huge_vertex_count() {
        let text = "ply\nformat ascii 1.0\nelement vertex 10000000000000000000\n\
property float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n";
        assert!(write_read(text, "osview-rs-hugecount.ply").is_err());
    }

    #[test]
    fn rejects_element_before_vertex() {
        let text = "ply\nformat ascii 1.0\nelement face 1\n\
property list uchar int vertex_indices\nelement vertex 1\n\
property float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n3 0 0 0\n";
        assert!(write_read(text, "osview-rs-facefirst.ply").is_err());
    }

    #[test]
    fn scales_ushort_colors() {
        let text = "ply\nformat ascii 1.0\nelement vertex 1\n\
property float x\nproperty float y\nproperty float z\n\
property ushort red\nproperty ushort green\nproperty ushort blue\n\
end_header\n0 0 0 65535 0 0\n";
        let c = write_read(text, "osview-rs-ushort.ply").unwrap();
        assert_eq!(c.rgb.unwrap()[0], [255, 0, 0]);
    }
}
