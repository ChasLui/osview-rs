// Minimal NPY reader: little-endian float16/32/64, C order, 2-D (or 1-D as N x 1).
// Rows are L2-normalized on load, mirroring OpenScene's osview behavior
// (zero rows stay zero so they never win a query).
use std::fs;
use std::io;

pub struct Matrix {
    pub rows: usize,
    pub dim: usize,
    pub data: Vec<f32>, // row-major, rows normalized
}

pub fn read(path: &str) -> io::Result<Matrix> {
    let bytes = fs::read(path)?;
    parse(&bytes)
}

pub fn parse(bytes: &[u8]) -> io::Result<Matrix> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, format!("NPY: {m}"));
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
        return Err(bad("bad magic"));
    }
    let major = bytes[6];
    let (hlen, hstart) = match major {
        1 => {
            if bytes.len() < 10 {
                return Err(bad("truncated header"));
            }
            (u16::from_le_bytes([bytes[8], bytes[9]]) as usize, 10usize)
        }
        2 | 3 => {
            if bytes.len() < 12 {
                return Err(bad("truncated header"));
            }
            (
                u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize,
                12usize,
            )
        }
        v => return Err(bad(&format!("unsupported version {v}"))),
    };
    if bytes.len() < hstart + hlen {
        return Err(bad("truncated header dict"));
    }
    let header = std::str::from_utf8(&bytes[hstart..hstart + hlen])
        .map_err(|_| bad("non-utf8 header"))?
        .trim();

    let descr = field(header, "'descr':")?;
    let descr = descr.trim().trim_matches(|c| c == '\'' || c == '"');
    let fortran = field(header, "'fortran_order':")?;
    if fortran.contains("True") {
        return Err(bad("fortran_order is not supported"));
    }
    let shape = shape_field(header)?;
    let dims: Vec<usize> = shape
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split(',')
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                s.parse::<usize>().ok()
            }
        })
        .collect();
    let (rows, dim) = match dims.as_slice() {
        [n] => (1, *n),
        [n, d] => (*n, *d),
        _ => return Err(bad("only 1-D or 2-D arrays are supported")),
    };

    let data_start = hstart + hlen;
    let avail = &bytes[data_start..];
    let mut data = Vec::with_capacity(rows * dim);
    match descr {
        "<f2" | "f2" => {
            let need = rows * dim * 2;
            if avail.len() < need {
                return Err(bad("truncated f16 payload"));
            }
            for c in avail[..need].chunks_exact(2) {
                let bits = u16::from_le_bytes([c[0], c[1]]);
                data.push(f16_to_f32(bits));
            }
        }
        "<f4" | "f4" => {
            let need = rows * dim * 4;
            if avail.len() < need {
                return Err(bad("truncated f32 payload"));
            }
            for c in avail[..need].chunks_exact(4) {
                data.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
            }
        }
        "<f8" | "f8" => {
            let need = rows * dim * 8;
            if avail.len() < need {
                return Err(bad("truncated f64 payload"));
            }
            for c in avail[..need].chunks_exact(8) {
                data.push(f64::from_le_bytes([
                    c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7],
                ]) as f32);
            }
        }
        other => return Err(bad(&format!("unsupported dtype {other}"))),
    }

    normalize_rows(&mut data, dim);
    Ok(Matrix { rows, dim, data })
}

fn normalize_rows(data: &mut [f32], dim: usize) {
    for row in data.chunks_mut(dim) {
        let norm = row.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 1e-12 {
            for v in row.iter_mut() {
                *v /= norm;
            }
        } else {
            for v in row.iter_mut() {
                *v = 0.0;
            }
        }
    }
}

fn shape_field(header: &str) -> io::Result<&str> {
    let i = header
        .find("'shape':")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "NPY: missing shape"))?;
    let rest = &header[i + "'shape':".len()..];
    let start = rest
        .find('(')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "NPY: bad shape"))?;
    let end = rest[start..]
        .find(')')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "NPY: bad shape"))?;
    Ok(&rest[start..start + end + 1])
}

fn field<'a>(header: &'a str, key: &str) -> io::Result<&'a str> {
    let i = header
        .find(key)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("NPY: missing {key}")))?;
    let rest = &header[i + key.len()..];
    let end = rest.find(',').unwrap_or(rest.len());
    Ok(&rest[..end])
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = (bits >> 15) & 1;
    let exp = (bits >> 10) & 0x1f;
    let frac = bits & 0x3ff;
    let val = if exp == 0 {
        if frac == 0 {
            0.0
        } else {
            (frac as f32 / 1024.0) * 2f32.powi(-14)
        }
    } else if exp == 31 {
        if frac == 0 {
            f32::INFINITY
        } else {
            f32::NAN
        }
    } else {
        (1.0 + frac as f32 / 1024.0) * 2f32.powi(exp as i32 - 15)
    };
    if sign == 1 {
        -val
    } else {
        val
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn npy_f32(rows: usize, dim: usize, vals: &[f32]) -> Vec<u8> {
        let header = format!(
            "{{'descr': '<f4', 'fortran_order': False, 'shape': ({rows}, {dim}), }}"
        );
        let mut h = header.into_bytes();
        while (10 + h.len() + 1) % 16 != 0 {
            h.push(b' ');
        }
        h.push(b'\n');
        let mut out = Vec::new();
        out.extend_from_slice(b"\x93NUMPY");
        out.push(1);
        out.push(0);
        out.extend_from_slice(&(h.len() as u16).to_le_bytes());
        out.extend_from_slice(&h);
        for v in vals {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    fn npy_f16(rows: usize, dim: usize, bits: &[u16]) -> Vec<u8> {
        let header = format!(
            "{{'descr': '<f2', 'fortran_order': False, 'shape': ({rows}, {dim}), }}"
        );
        let mut h = header.into_bytes();
        while (10 + h.len() + 1) % 16 != 0 {
            h.push(b' ');
        }
        h.push(b'\n');
        let mut out = Vec::new();
        out.extend_from_slice(b"\x93NUMPY");
        out.push(1);
        out.push(0);
        out.extend_from_slice(&(h.len() as u16).to_le_bytes());
        out.extend_from_slice(&h);
        for b in bits {
            out.extend_from_slice(&b.to_le_bytes());
        }
        out
    }

    #[test]
    fn reads_float16_rows() {
        // 1.0 = 0x3C00, 0.0 = 0
        let raw = npy_f16(1, 2, &[0x3C00, 0x0000]);
        let m = parse(&raw).unwrap();
        assert_eq!((m.rows, m.dim), (1, 2));
        assert!((m.data[0] - 1.0).abs() < 1e-6);
        assert_eq!(m.data[1], 0.0);
    }

    #[test]
    fn reads_and_normalizes_rows() {
        let raw = npy_f32(2, 3, &[3.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let m = parse(&raw).unwrap();
        assert_eq!((m.rows, m.dim), (2, 3));
        assert!((m.data[0] - 1.0).abs() < 1e-6);
        assert_eq!(m.data[3..6], [0.0, 0.0, 0.0]);
    }
}