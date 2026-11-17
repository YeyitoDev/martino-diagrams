//! Icono de Martino (gato negro, ojos amarillos) generado por código: SVG + PNG propio (zlib).
use std::io::Write;
use std::path::Path;

const ACCENT: [u8; 3] = [0xF5, 0xC5, 0x18];
const CAT: [u8; 3] = [0x0B, 0x0B, 0x0C];
const NOSE: [u8; 3] = [0x3A, 0x3A, 0x3E];

enum Shape {
    /// x, y, w, h, radio
    Rect(f64, f64, f64, f64, f64),
    Ellipse(f64, f64, f64, f64),
    Poly(&'static [(f64, f64)]),
}

/// Geometría en un lienzo de 512×512, de atrás hacia adelante.
const SCENE: &[(Shape, [u8; 3])] = &[
    (Shape::Ellipse(256.0, 520.0, 190.0, 120.0), CAT),
    (Shape::Poly(&[(118.0, 236.0), (138.0, 70.0), (244.0, 170.0)]), CAT),
    (Shape::Poly(&[(394.0, 236.0), (374.0, 70.0), (268.0, 170.0)]), CAT),
    (Shape::Ellipse(256.0, 290.0, 158.0, 140.0), CAT),
    (Shape::Ellipse(196.0, 280.0, 40.0, 32.0), ACCENT),
    (Shape::Ellipse(316.0, 280.0, 40.0, 32.0), ACCENT),
    (Shape::Ellipse(196.0, 280.0, 8.0, 27.0), CAT),
    (Shape::Ellipse(316.0, 280.0, 8.0, 27.0), CAT),
    (Shape::Poly(&[(242.0, 334.0), (270.0, 334.0), (256.0, 350.0)]), NOSE),
];

fn inside(s: &Shape, x: f64, y: f64) -> bool {
    match *s {
        Shape::Rect(rx, ry, w, h, r) => {
            if x < rx || y < ry || x > rx + w || y > ry + h {
                return false;
            }
            let cx = x.clamp(rx + r, rx + w - r);
            let cy = y.clamp(ry + r, ry + h - r);
            (x - cx).powi(2) + (y - cy).powi(2) <= r * r
        }
        Shape::Ellipse(cx, cy, a, b) => ((x - cx) / a).powi(2) + ((y - cy) / b).powi(2) <= 1.0,
        Shape::Poly(p) => {
            let mut c = false;
            let mut j = p.len() - 1;
            for i in 0..p.len() {
                let (xi, yi) = p[i];
                let (xj, yj) = p[j];
                if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                    c = !c;
                }
                j = i;
            }
            c
        }
    }
}

/// `scale` < 1 deja margen (icono maskable); `radius` redondea el fondo.
fn raster(size: u32, scale: f64, radius: f64) -> Vec<u8> {
    const SS: u32 = 4;
    let bg = Shape::Rect(0.0, 0.0, 512.0, 512.0, radius);
    let mut px = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for pxx in 0..size {
            let mut acc = [0f64; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (pxx as f64 + (sx as f64 + 0.5) / SS as f64) * 512.0 / size as f64;
                    let y = (py as f64 + (sy as f64 + 0.5) / SS as f64) * 512.0 / size as f64;
                    if !inside(&bg, x, y) {
                        continue;
                    }
                    let (lx, ly) = ((x - 256.0) / scale + 256.0, (y - 256.0) / scale + 256.0);
                    let col = SCENE.iter().rev().find(|(s, _)| inside(s, lx, ly)).map(|x| x.1).unwrap_or(ACCENT);
                    for k in 0..3 {
                        acc[k] += col[k] as f64;
                    }
                    acc[3] += 255.0;
                }
            }
            let n = (SS * SS) as f64;
            let a = acc[3] / n;
            let i = ((py * size + pxx) * 4) as usize;
            for k in 0..3 {
                px[i + k] = if a > 0.0 { (acc[k] / (acc[3] / 255.0)).round() as u8 } else { 0 };
            }
            px[i + 3] = a.round() as u8;
        }
    }
    px
}

pub fn png(size: u32, rgba: &[u8]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        out.extend(kind);
        out.extend(data);
        out.extend(crc.sum().to_be_bytes());
    }
    let mut raw = Vec::with_capacity(rgba.len() + size as usize);
    for row in rgba.chunks(size as usize * 4) {
        raw.push(0);
        raw.extend(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::best());
    z.write_all(&raw).unwrap();
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = vec![];
    ihdr.extend(size.to_be_bytes());
    ihdr.extend(size.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z.finish().unwrap());
    chunk(&mut out, b"IEND", &[]);
    out
}

pub fn svg() -> String {
    let hex = |c: [u8; 3]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let mut s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 512 512\"><rect width=\"512\" height=\"512\" rx=\"96\" fill=\"{}\"/><g clip-path=\"inset(0 round 96px)\">", hex(ACCENT));
    for (shape, c) in SCENE {
        s += &match shape {
            Shape::Ellipse(cx, cy, a, b) => format!("<ellipse cx=\"{cx}\" cy=\"{cy}\" rx=\"{a}\" ry=\"{b}\" fill=\"{}\"/>", hex(*c)),
            Shape::Poly(p) => format!("<polygon points=\"{}\" fill=\"{}\"/>", p.iter().map(|(x, y)| format!("{x},{y}")).collect::<Vec<_>>().join(" "), hex(*c)),
            Shape::Rect(x, y, w, h, r) => format!("<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"{r}\" fill=\"{}\"/>", hex(*c)),
        };
    }
    s + "</g></svg>\n"
}

pub fn generate(dir: &Path) -> std::io::Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("icon.svg"), svg())?;
    let mut done = vec!["icon.svg".to_string()];
    for (name, size, scale, radius) in [("icon-192.png", 192, 1.0, 96.0), ("icon-512.png", 512, 1.0, 96.0), ("maskable-512.png", 512, 0.78, 0.0), ("apple-touch-icon.png", 180, 0.9, 0.0)] {
        std::fs::write(dir.join(name), png(size, &raster(size, scale, radius)))?;
        done.push(name.into());
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_is_valid_and_has_yellow_eyes() {
        let px = raster(64, 1.0, 96.0);
        let at = |x: u32, y: u32| &px[((y * 64 + x) * 4) as usize..((y * 64 + x) * 4 + 4) as usize];
        assert_eq!(at(0, 0)[3], 0, "esquina transparente");
        assert_eq!(&at(32, 32)[..3], &CAT, "cara negra");
        assert_eq!(&at((190.0 / 8.0) as u32 - 1, 35)[..3], &ACCENT, "ojo amarillo");
        let p = png(64, &px);
        assert!(p.starts_with(b"\x89PNG") && p.ends_with(&[0xAE, 0x42, 0x60, 0x82]));
        assert!(svg().contains("<ellipse"));
    }
}
