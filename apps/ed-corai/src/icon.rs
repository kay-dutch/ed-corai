//! The program icon: the station from the window, drawn at any size. The window uses it directly;
//! `assets/ed-corai.ico` (embedded in the Windows programs) is generated from it by a test, which
//! also fails when the file is stale, so the two never drift apart.

/// Station colours, as in the window.
const HULL: [u8; 3] = [28, 22, 18];
const ORANGE: [u8; 3] = [255, 113, 0];
const SLOT: [u8; 3] = [180, 220, 255];

/// Whether `(x, y)` (relative to the centre) lies in a hexagon with corners at `r`, flat side on
/// top: within the apothem along each of the three edge normals.
fn in_hexagon(x: f32, y: f32, r: f32) -> bool {
    let apothem = r * 3f32.sqrt() / 2.0;
    [30f32, 90.0, 150.0].iter().all(|deg| {
        let (s, c) = deg.to_radians().sin_cos();
        (x * c + y * s).abs() <= apothem
    })
}

/// Colour of one point of the icon, or `None` outside it.
fn colour(x: f32, y: f32, size: f32) -> Option<[u8; 3]> {
    let r = size * 0.48;
    let border = (size * 0.07).max(1.0);
    if x.abs() <= r * 0.28 && y.abs() <= r * 0.1 + size * 0.02 {
        Some(SLOT)
    } else if in_hexagon(x, y, r - border) {
        Some(HULL)
    } else if in_hexagon(x, y, r) {
        Some(ORANGE)
    } else {
        None
    }
}

/// Square RGBA image, 4x4 supersampled for smooth edges.
pub fn rgba(size: u32) -> Vec<u8> {
    const N: u32 = 4;
    let half = size as f32 / 2.0;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut sum, mut hits) = ([0u32; 3], 0u32);
            for sy in 0..N {
                for sx in 0..N {
                    let x = px as f32 + (sx as f32 + 0.5) / N as f32 - half;
                    let y = py as f32 + (sy as f32 + 0.5) / N as f32 - half;
                    if let Some(c) = colour(x, y, size as f32) {
                        for (s, v) in sum.iter_mut().zip(c) {
                            *s += u32::from(v);
                        }
                        hits += 1;
                    }
                }
            }
            if hits == 0 {
                out.extend([0, 0, 0, 0]);
            } else {
                let avg = |i: usize| (sum[i] / hits) as u8;
                out.extend([avg(0), avg(1), avg(2), (hits * 255 / (N * N)) as u8]);
            }
        }
    }
    out
}

#[cfg(test)]
/// A Windows `.ico` with 32-bit images of the given sizes (at most 256).
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| dib(s)).collect();
    let mut out = Vec::new();
    out.extend(0u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend((sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, img) in sizes.iter().zip(&images) {
        let dim = if s >= 256 { 0 } else { s as u8 };
        out.extend([dim, dim, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((img.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend(img);
    }
    out
}

#[cfg(test)]
/// One icon image: BITMAPINFOHEADER, BGRA rows bottom-up, then an all-clear AND mask.
fn dib(size: u32) -> Vec<u8> {
    let rgba = rgba(size);
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::new();
    for v in [40u32, size, size * 2] {
        out.extend(v.to_le_bytes());
    }
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    for v in [0u32, size * size * 4 + mask_row * size, 0, 0, 0, 0] {
        out.extend(v.to_le_bytes());
    }
    for row in (0..size).rev() {
        for px in rgba[(row * size * 4) as usize..((row + 1) * size * 4) as usize].chunks(4) {
            out.extend([px[2], px[1], px[0], px[3]]);
        }
    }
    out.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
    out
}

/// The sizes in `assets/ed-corai.ico`.
#[cfg(test)]
pub const ICO_SIZES: [u32; 5] = [16, 24, 32, 48, 256];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_a_station_on_a_clear_background() {
        let img = rgba(64);
        let at =
            |x: u32, y: u32| &img[((y * 64 + x) * 4) as usize..((y * 64 + x) * 4 + 4) as usize];
        assert_eq!(at(0, 0)[3], 0, "corner is transparent");
        assert_eq!(at(32, 32)[..3], SLOT, "docking slot in the middle");
        assert_eq!(at(32, 7)[..3], ORANGE, "hull edge at the top (flat side)");
        assert_eq!(at(32, 4)[3], 0, "nothing above the flat top");
        assert_eq!(at(32, 16)[..3], HULL);
    }

    /// Regenerate with `CORAI_WRITE_ICON=1 cargo test -p ed-corai icon`.
    #[test]
    fn the_windows_icon_file_matches_the_drawing() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/ed-corai.ico");
        let want = ico(&ICO_SIZES);
        if std::env::var("CORAI_WRITE_ICON").as_deref() == Ok("1") {
            std::fs::write(path, &want).expect("write icon");
        }
        let got = std::fs::read(path).expect("assets/ed-corai.ico exists");
        assert!(
            got == want,
            "assets/ed-corai.ico is stale: run with CORAI_WRITE_ICON=1"
        );
    }
}
