//! Lines a rebuilt worksheet up with its scan.
//!
//! The model reads sizes and spacing off the scan only roughly. Here the sheet is drawn by
//! WordCraft with a marker at the start of every block, the markers are found in the PDF, and
//! each block's first line is looked for in the scan around where it was drawn. The difference
//! moves the block: its space before, the previous block's space after, or the top margin.
use super::*;
use image::{GrayImage, Luma, Rgb, RgbImage};
use std::{path::Path, process::Command};

/// Pixels per point of the images compared (150 dpi).
const K: f32 = 150.0 / 72.0;

/// Scan and rebuild drawn over each other at the rebuild's size: the scan's ink red, the
/// rebuild's blue, both dark where they agree. The scan's paper is evened out to white first.
pub fn overlay(scan: &Path, rebuilt: &Path, out: &Path, shear: f32) -> Result<(), String> {
    let r = image::open(rebuilt).map_err(|e| format!("{}: {e}", rebuilt.display()))?.to_luma8();
    let s = prepare_scan(scan, r.width(), r.height(), shear)?;
    let mut img = RgbImage::new(r.width(), r.height());
    for (x, y, px) in img.enumerate_pixels_mut() {
        let a = s.get_pixel(x, y).0[0];
        let b = r.get_pixel(x, y).0[0];
        *px = Rgb([b, a.min(b), a]);
    }
    img.save(out).map_err(|e| e.to_string())
}

/// A scan as a page at 150 dpi, `w`×`h` pixels: scaled from the scanner's resolution (not
/// stretched to the paper, since a scan often takes in a margin beyond the sheet), straightened,
/// sheared by `shear` (points across per point down, from the middle: a sheet fed unevenly
/// leans this way while its lines stay level), centred and with its paper levelled to white.
pub fn prepare_scan(scan: &Path, w: u32, h: u32, shear: f32) -> Result<GrayImage, String> {
    let img = image::open(scan).map_err(|e| format!("{}: {e}", scan.display()))?;
    // The scan's resolution: the common scanner setting nearest its width over 8.5 inches.
    let guess = img.width() as f32 / (w as f32 / K / 72.0);
    let dpi = [100.0, 150.0, 200.0, 240.0, 300.0, 400.0, 600.0]
        .into_iter()
        .min_by(|a: &f32, b: &f32| (a - guess).abs().total_cmp(&(b - guess).abs()))
        .filter(|d| (d - guess).abs() / guess < 0.08)
        .unwrap_or(guess);
    let k = 150.0 / dpi;
    let img = img.resize_exact(
        (img.width() as f32 * k).round() as u32,
        (img.height() as f32 * k).round() as u32,
        image::imageops::FilterType::Triangle,
    );
    let rgb = img.to_rgb8();
    let skew = crate::imaging::detect_skew(&img.to_luma8());
    let rgb = crate::imaging::rotate(&rgb, -skew);
    let g = image::DynamicImage::ImageRgb8(rgb).to_luma8();
    let g = if shear == 0.0 {
        g
    } else {
        let cy = g.height() as f32 / 2.0;
        GrayImage::from_fn(g.width(), g.height(), |x, y| {
            let sx = x as f32 + shear * (y as f32 - cy);
            let (x0, f) = (sx.floor(), sx - sx.floor());
            let at = |x: f32| if x < 0.0 || x >= g.width() as f32 { 255.0 } else { g.get_pixel(x as u32, y).0[0] as f32 };
            Luma([(at(x0) * (1.0 - f) + at(x0 + 1.0) * f) as u8])
        })
    };
    let mut page = GrayImage::from_pixel(w, h, Luma([255]));
    let (ox, oy) = ((w as i64 - g.width() as i64) / 2, (h as i64 - g.height() as i64) / 2);
    image::imageops::replace(&mut page, &g, ox, oy);
    Ok(level(page))
}

/// Stretches the paper (the bright end of the histogram) to white.
fn level(g: GrayImage) -> GrayImage {
    let mut hist = [0u32; 256];
    for p in g.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let total = g.width() * g.height();
    let (mut acc, mut paper) = (0u32, 255usize);
    for v in (0..256).rev() {
        acc += hist[v];
        if acc > total / 3 {
            paper = v;
            break;
        }
    }
    let paper = paper.max(64) as f32;
    let mut out = g;
    for p in out.pixels_mut() {
        p.0[0] = ((p.0[0] as f32 / paper) * 255.0).min(255.0) as u8;
    }
    out
}

/// Where each top-level block's first line was drawn: part (`B`, `H`, `F`), index, x and the
/// baseline y, points.
#[derive(Debug)]
struct Mark {
    part: char,
    index: usize,
    x: f32,
    y: f32,
}

/// Draws the sheet with markers; returns the page image at 150 dpi and the markers.
fn probe(sheet: &Sheet) -> Result<(GrayImage, Vec<Mark>), String> {
    let docx = super::cli::tmp("probe.docx");
    docx::write_as(sheet, &docx, &super::cli::figure_png, true)?;
    let pdf = docx.with_extension("pdf");
    let out = Command::new("wordcraft-cli").arg("convert").arg(&docx).arg(&pdf).output().map_err(|e| format!("wordcraft-cli: {e}"))?;
    if !out.status.success() {
        return Err(format!("wordcraft-cli convert: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let bbox = Command::new("pdftotext").args(["-bbox", "-f", "1", "-l", "1"]).arg(&pdf).arg("-").output().map_err(|e| format!("pdftotext: {e}"))?;
    let marks = parse_marks(&String::from_utf8_lossy(&bbox.stdout));
    let stem = docx.with_extension("");
    let out = Command::new("pdftoppm")
        .args(["-r", "150", "-gray", "-png", "-singlefile", "-f", "1", "-l", "1"])
        .arg(&pdf)
        .arg(&stem)
        .output()
        .map_err(|e| format!("pdftoppm: {e}"))?;
    if !out.status.success() {
        return Err(format!("pdftoppm: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let png = stem.with_extension("png");
    let img = image::open(&png).map_err(|e| format!("{}: {e}", png.display()))?.to_luma8();
    Ok((img, marks))
}

/// Markers ([`docx::marker`]) in `pdftotext -bbox` output. A marker is set with no advance,
/// so the PDF holds it as overlapping one-letter words in any order: a run of tiny words is
/// gathered and decoded letter by letter.
fn parse_marks(xml: &str) -> Vec<Mark> {
    let attr = |line: &str, name: &str| -> Option<f32> {
        let i = line.find(&format!("{name}=\""))? + name.len() + 2;
        line[i..].split('"').next()?.parse().ok()
    };
    let mut out = vec![];
    // The tiny letters gathered so far, and the leftmost x and the baseline of the run.
    let mut letters = String::new();
    let mut at = (f32::MAX, 0.0f32);
    let mut flush = |letters: &mut String, at: &mut (f32, f32)| {
        if let Some(m) = decode(letters, *at) {
            out.push(m);
        }
        letters.clear();
        *at = (f32::MAX, 0.0);
    };
    for line in xml.lines() {
        let (Some(x), Some(y0), Some(y1)) = (attr(line, "xMin"), attr(line, "yMin"), attr(line, "yMax")) else { continue };
        let Some(text) = line.split('>').nth(1).and_then(|t| t.split('<').next()) else { continue };
        let tiny = y1 - y0 < 2.5;
        // A run ends at ordinary text or at a tiny word on another line.
        if !tiny || (!letters.is_empty() && (y1 - at.1).abs() > 0.5) {
            flush(&mut letters, &mut at);
        }
        if tiny {
            letters.push_str(text);
            at = (at.0.min(x), y1);
        }
    }
    flush(&mut letters, &mut at);
    out
}

fn decode(letters: &str, (x, y): (f32, f32)) -> Option<Mark> {
    let has = |c: char| letters.contains(c);
    if !has('Q') || !has('Z') {
        return None;
    }
    let part = letters.chars().find(|c| matches!(c, 'B' | 'H' | 'F'))?;
    let digit = |lo: char, hi: char| letters.chars().find(|c| (lo..=hi).contains(c)).map(|c| c as usize - lo as usize);
    let index = digit('0', '9').unwrap_or(0) * 100 + digit('a', 'j')? * 10 + digit('k', 't')?;
    Some(Mark { part, index, x, y })
}

/// The tallest text size on a block's first line, points.
fn first_size(b: &Block, sheet: &Sheet) -> f32 {
    match b {
        Block::P(p) => p
            .runs
            .iter()
            .take_while(|r| !matches!(r, Run::Br { .. }))
            .map(|r| match r {
                Run::Text(t) => t.size.unwrap_or(sheet.size),
                Run::Math { size, .. } => size.unwrap_or(sheet.size) * 1.3,
                _ => 0.0,
            })
            .fold(0.0, f32::max),
        Block::Table(t) => t
            .rows
            .first()
            .and_then(|r| r.cells.first())
            .and_then(|c| c.blocks.first())
            .map(|b| first_size(b, sheet))
            .unwrap_or(sheet.size),
        _ => 0.0,
    }
}

/// Ink of a greyscale pixel, 0 (paper) to 1.
fn ink(v: u8) -> f32 {
    (1.0 - v as f32 / 255.0).max(0.0)
}

/// Normalised cross-correlation of the strip `t` (from the rendering, at `tx`,`ty`) against the
/// scan at offset `dx`,`dy`, sampling every `step` pixels.
fn ncc(t: &GrayImage, tx: u32, ty: u32, tw: u32, th: u32, s: &GrayImage, dx: i32, dy: i32, step: u32) -> f32 {
    let (mut sa, mut sb, mut saa, mut sbb, mut sab, mut n) = (0f32, 0f32, 0f32, 0f32, 0f32, 0f32);
    let mut y = 0;
    while y < th {
        let mut x = 0;
        while x < tw {
            let (ax, ay) = (tx + x, ty + y);
            let (bx, by) = (ax as i32 + dx, ay as i32 + dy);
            if bx >= 0 && by >= 0 && (bx as u32) < s.width() && (by as u32) < s.height() && ax < t.width() && ay < t.height() {
                let a = ink(t.get_pixel(ax, ay).0[0]);
                let b = ink(s.get_pixel(bx as u32, by as u32).0[0]);
                sa += a;
                sb += b;
                saa += a * a;
                sbb += b * b;
                sab += a * b;
                n += 1.0;
            }
            x += step;
        }
        y += step;
    }
    if n < 1.0 {
        return 0.0;
    }
    let cov = sab - sa * sb / n;
    let va = saa - sa * sa / n;
    let vb = sbb - sb * sb / n;
    if va <= 1e-6 || vb <= 1e-6 { 0.0 } else { cov / (va * vb).sqrt() }
}

/// A block's measured offset from where it should be: points to move it down and right, and how
/// sure the match is (correlation, 0 to 1).
#[derive(Clone, Copy, Debug)]
pub struct Offset {
    /// Where the block's first line was drawn, points from the top.
    pub y: f32,
    pub dy: f32,
    pub dx: f32,
    pub score: f32,
    /// The first line as drawn: left, top, width, height in pixels of the rendering.
    strip: [u32; 4],
}

/// Finds the first line of the block whose marker is `m` in the scan, starting the search
/// `guess` points below where it was drawn.
fn locate(render: &GrayImage, scan: &GrayImage, m: &Mark, size: f32, guess: f32, sheet: &Sheet) -> Option<Offset> {
    let size = size.max(6.0);
    let top = ((m.y - size * 0.95) * K).max(0.0) as u32;
    let bottom = ((m.y + size * 0.3) * K) as u32;
    let left = (sheet.page.margins[3].min(m.x) - 6.0).max(0.0) * K;
    let right = (sheet.page.width - sheet.page.margins[1] + 6.0) * K;
    let (tx, ty, tw, th) = (left as u32, top, (right - left).max(1.0) as u32, bottom.saturating_sub(top).max(1));
    // A strip with almost no ink (an empty paragraph, a figure) can't be matched.
    let mut inked = 0;
    for y in ty..(ty + th).min(render.height()) {
        for x in tx..(tx + tw).min(render.width()) {
            if render.get_pixel(x, y).0[0] < 128 {
                inked += 1;
            }
        }
    }
    if inked < 40 {
        return None;
    }
    // Coarse search every 2 px over ±30 pt down and ±8 pt across, then refine by 1 px.
    let g = (guess * K).round() as i32;
    let (wy, wx) = ((30.0 * K) as i32, (8.0 * K) as i32);
    let mut best = (f32::MIN, 0, 0);
    for dy in ((g - wy)..=(g + wy)).step_by(2) {
        for dx in (-wx..=wx).step_by(2) {
            let c = ncc(render, tx, ty, tw, th, scan, dx, dy, 2);
            if c > best.0 {
                best = (c, dx, dy);
            }
        }
    }
    let (_, bx, by) = best;
    for dy in (by - 2)..=(by + 2) {
        for dx in (bx - 2)..=(bx + 2) {
            let c = ncc(render, tx, ty, tw, th, scan, dx, dy, 1);
            if c > best.0 || (dx == bx && dy == by) {
                if c >= best.0 {
                    best = (c, dx, dy);
                }
            }
        }
    }
    Some(Offset { y: m.y, dy: best.2 as f32 / K, dx: best.1 as f32 / K, score: best.0, strip: [tx, ty, tw, th] })
}

/// A line whose glyphs don't match the scan (another font) can still be placed by its shape
/// down the page: the ink per row across its width, correlated against the scan's within
/// ±30 points of `guess`. Returns the offset down, points, and the correlation.
fn band(render: &GrayImage, scan: &GrayImage, o: &Offset, guess: f32) -> Option<(f32, f32)> {
    let [tx, ty, tw, th] = o.strip;
    let pad = (30.0 * K) as i32;
    let row = |img: &GrayImage, y: i32| -> f32 {
        if y < 0 || y as u32 >= img.height() {
            return 0.0;
        }
        (tx..(tx + tw).min(img.width())).map(|x| ink(img.get_pixel(x, y as u32).0[0])).sum::<f32>()
    };
    // The rendering's profile with some paper above and below, so the line's edges count.
    let margin = th as i32 / 2;
    let a: Vec<f32> = ((ty as i32 - margin)..(ty as i32 + th as i32 + margin)).map(|y| row(render, y)).collect();
    let g = (guess * K).round() as i32;
    let mut best = (f32::MIN, 0);
    for dy in (g - pad)..=(g + pad) {
        let b: Vec<f32> = ((ty as i32 - margin + dy)..(ty as i32 + th as i32 + margin + dy)).map(|y| row(scan, y)).collect();
        let n = a.len() as f32;
        let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
        let cov: f32 = a.iter().zip(&b).map(|(x, y)| (x - ma) * (y - mb)).sum();
        let va: f32 = a.iter().map(|x| (x - ma).powi(2)).sum();
        let vb: f32 = b.iter().map(|y| (y - mb).powi(2)).sum();
        let c = if va > 0.0 && vb > 0.0 { cov / (va * vb).sqrt() } else { 0.0 };
        if c > best.0 {
            best = (c, dy);
        }
    }
    Some((best.1 as f32 / K, best.0))
}

/// How much wider a line's ink spans on the scan than as drawn, as a [`Column`] (its offset is
/// how far the middle sits to the right). Handwriting in the same rows would spoil it, so the
/// scan is only looked at a little beyond the drawn line's ends.
fn span_ratio(render: &GrayImage, scan: &GrayImage, o: &Offset) -> Option<Column> {
    let [tx, ty, tw, th] = o.strip;
    let dy = (o.dy * K).round() as i32;
    let extent = |img: &GrayImage, x0: u32, x1: u32, shift: i32| -> Option<(u32, u32)> {
        let inked = |x: u32| {
            (ty..ty + th).any(|y| {
                let y = y as i32 + shift;
                y >= 0 && (y as u32) < img.height() && x < img.width() && img.get_pixel(x, y as u32).0[0] < 110
            })
        };
        let a = (x0..x1).find(|&x| inked(x))?;
        let b = (x0..x1).rev().find(|&x| inked(x))?;
        Some((a, b))
    };
    let (ra, rb) = extent(render, tx, tx + tw, 0)?;
    let reach = ((rb - ra) as f32 * 0.35) as u32;
    let (sa, sb) = extent(scan, ra.saturating_sub(reach), (rb + reach).min(scan.width()), dy)?;
    let ratio = (sb - sa) as f32 / (rb - ra).max(1) as f32;
    let shift = ((sa + sb) as f32 - (ra + rb) as f32) / 2.0 / K;
    Some(Column { dx: shift, scale: ratio, score: 1.0 })
}

/// One column of a line (the text between tab stops) as measured on the scan: how many points
/// it sits to the right of where it was drawn, and how much wider it is there.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub dx: f32,
    pub scale: f32,
    pub score: f32,
}

/// The inked stretches of a strip of the rendering, split at gaps wider than 14 points.
fn clusters(render: &GrayImage, [tx, ty, tw, th]: [u32; 4]) -> Vec<(u32, u32)> {
    let gap = (14.0 * K) as u32;
    let mut out: Vec<(u32, u32)> = vec![];
    for x in tx..(tx + tw).min(render.width()) {
        let inked = (ty..(ty + th).min(render.height())).any(|y| render.get_pixel(x, y).0[0] < 128);
        if !inked {
            continue;
        }
        match out.last_mut() {
            Some(last) if x - last.1 <= gap => last.1 = x,
            _ => out.push((x, x)),
        }
    }
    out
}

/// Correlation of render columns `a..=b` of the strip, stretched by `s` about `a` and moved
/// `u` pixels across and `dy` down, against the scan.
fn ncc_stretched(render: &GrayImage, scan: &GrayImage, (a, b): (u32, u32), [_, ty, _, th]: [u32; 4], u: i32, s: f32, dy: i32, step: u32) -> f32 {
    let (mut sa, mut sb, mut saa, mut sbb, mut sab, mut n) = (0f32, 0f32, 0f32, 0f32, 0f32, 0f32);
    let w = ((b - a) as f32 * s.max(1.0)) as u32 + 1;
    let mut y = ty;
    while y < (ty + th).min(render.height()) {
        let sy = y as i32 + dy;
        if sy >= 0 && (sy as u32) < scan.height() {
            let mut x = 0;
            while x <= w {
                // Walk the scan side and look the rendering up, so stretching never skips pixels.
                let sx = a as i32 + u + x as i32;
                let rx = a as f32 + x as f32 / s;
                if sx >= 0 && (sx as u32) < scan.width() && rx <= (b + 1) as f32 && (rx as u32) < render.width() {
                    let av = ink(render.get_pixel(rx as u32, y).0[0]);
                    let bv = ink(scan.get_pixel(sx as u32, sy as u32).0[0]);
                    sa += av;
                    sb += bv;
                    saa += av * av;
                    sbb += bv * bv;
                    sab += av * bv;
                    n += 1.0;
                }
                x += step;
            }
        }
        y += step;
    }
    if n < 1.0 {
        return 0.0;
    }
    let cov = sab - sa * sb / n;
    let (va, vb) = (saa - sa * sa / n, sbb - sb * sb / n);
    if va <= 1e-6 || vb <= 1e-6 { 0.0 } else { cov / (va * vb).sqrt() }
}

/// Measures each column of a matched block's first line.
fn columns_of(render: &GrayImage, scan: &GrayImage, o: &Offset) -> Vec<Column> {
    let dy = (o.dy * K).round() as i32;
    let base = (o.dx * K).round() as i32;
    let wx = (8.0 * K) as i32;
    clusters(render, o.strip)
        .into_iter()
        .map(|c| {
            let mut best = (f32::MIN, 0, 1.0);
            for si in -7..=8 {
                let s = 1.0 + si as f32 * 0.02;
                for u in ((base - wx)..=(base + wx)).step_by(2) {
                    let v = ncc_stretched(render, scan, c, o.strip, u, s, dy, 2);
                    if v > best.0 {
                        best = (v, u, s);
                    }
                }
            }
            let (_, bu, bs) = best;
            for si in -2..=2 {
                let s = bs + si as f32 * 0.005;
                for u in (bu - 2)..=(bu + 2) {
                    let v = ncc_stretched(render, scan, c, o.strip, u, s, dy, 1);
                    if v > best.0 {
                        best = (v, u, s);
                    }
                }
            }
            Column { dx: best.1 as f32 / K, scale: best.2, score: best.0 }
        })
        .collect()
}

/// Below this correlation a block's match is not trusted.
const SURE: f32 = 0.45;

/// What one measuring pass found.
pub struct Measure {
    /// Each body block's offset from the scan (`None` where it couldn't be matched).
    pub body: Vec<Option<Offset>>,
    /// The columns of each matched body block's first line.
    pub columns: Vec<Vec<Column>>,
    /// The first header and footer block's offsets.
    pub head: Option<Offset>,
    pub foot: Option<Offset>,
}

/// One measuring pass over the sheet as it stands.
pub fn measure(scan: &GrayImage, sheet: &Sheet) -> Result<Measure, String> {
    let (render, marks) = probe(sheet)?;
    let scan_page = if scan.dimensions() == render.dimensions() {
        scan.clone()
    } else {
        image::imageops::resize(scan, render.width(), render.height(), image::imageops::FilterType::Triangle)
    };
    let debug = std::env::var_os("DOCUMENTASHION_FIT_DEBUG").is_some();
    let mut body = vec![None; sheet.blocks.len()];
    let mut columns = vec![vec![]; sheet.blocks.len()];
    let mut guess = 0.0;
    for m in marks.iter().filter(|m| m.part == 'B') {
        let Some(b) = sheet.blocks.get(m.index) else { continue };
        let Some(o) = locate(&render, &scan_page, m, first_size(b, sheet), guess, sheet) else { continue };
        // The columns are matched allowing for a different type size, so they can vouch for a
        // line whose whole-width match is weak because its text runs wider or narrower.
        let c = columns_of(&render, &scan_page, &o);
        let col_score = if c.is_empty() { 0.0 } else { c.iter().map(|c| c.score).sum::<f32>() / c.len() as f32 };
        let mut sure = o.score >= SURE || (o.score >= 0.25 && col_score >= SURE_COLUMN + 0.1);
        let mut o = o;
        let mut c = c;
        if !sure {
            // Placed by shape instead; its glyphs can't be matched, but a centred line's width
            // still can: it scales the type so the line spans what it spans on the scan.
            if let Some((dy, score)) = band(&render, &scan_page, &o, guess).filter(|b| b.1 >= 0.8) {
                o = Offset { dy, dx: 0.0, score, ..o };
                let centred = matches!(b, Block::P(p) if p.align == Align::Center);
                c = if centred { span_ratio(&render, &scan_page, &o).map(|r| vec![r]).unwrap_or_default() } else { vec![] };
                sure = true;
            }
        }
        if debug {
            eprintln!(
                "  block {:>2} at y {:6.1}: down {:+.1} across {:+.1} line {:.2} {} columns: {}",
                m.index,
                m.y,
                o.dy,
                o.dx,
                o.score,
                if sure { "sure" } else { "unsure" },
                c.iter().map(|c| format!("{:+.1}pt x{:.3} ({:.2})", c.dx, c.scale, c.score)).collect::<Vec<_>>().join(", ")
            );
        }
        if sure {
            guess = o.dy;
            body[m.index] = Some(o);
            columns[m.index] = c;
        }
    }
    let edge = |part: char, blocks: &[Block]| {
        marks
            .iter()
            .find(|m| m.part == part && m.index == 0)
            .and_then(|m| locate(&render, &scan_page, m, blocks.first().map(|b| first_size(b, sheet)).unwrap_or(0.0), 0.0, sheet))
            .filter(|o| o.score >= SURE)
    };
    let (head, foot) = (edge('H', &sheet.header), edge('F', &sheet.footer));
    Ok(Measure { body, columns, head, foot })
}

/// Below this correlation a column's measurement is not used.
const SURE_COLUMN: f32 = 0.45;

/// The runs of a paragraph's first line, split at its tabs: one group per column.
fn tab_groups(p: &mut Para) -> Vec<Vec<&mut Run>> {
    let mut groups: Vec<Vec<&mut Run>> = vec![vec![]];
    for r in p.runs.iter_mut() {
        match r {
            Run::Br { .. } => break,
            Run::Tab { .. } => groups.push(vec![]),
            Run::Text(t) if t.text.contains('\t') || t.text.contains('\n') => {
                // Text with tabs typed in it can't be resized column by column.
                return vec![];
            }
            _ => groups.last_mut().unwrap().push(r),
        }
    }
    groups
}

/// Applies a line's measured columns: a column too narrow or wide on the page gets its font
/// sizes scaled, and one in the wrong place moves its indent (the first) or its tab stop.
/// Returns whether anything changed.
fn fit_columns(b: &mut Block, cols: &[Column], base: f32) -> bool {
    let half = |z: f32| (z * 2.0).round() / 2.0;
    let Block::P(p) = b else { return false };
    let mut changed = false;
    let left = p.align == Align::Left || p.align == Align::Justify;
    // A line of answer blanks (tabs that draw a rule) is mostly rule, which says nothing about
    // the type size.
    let ruled = p.tabs.iter().any(|t| t.leader != Leader::None);
    let mut tabs: Vec<usize> = (0..p.tabs.len()).collect();
    tabs.sort_by(|&a, &b| p.tabs[a].pos.total_cmp(&p.tabs[b].pos));
    let mut moves = vec![];
    {
        let groups: Vec<Vec<&mut Run>> = tab_groups(p).into_iter().collect();
        // Columns are told apart by the gaps between them; only a line whose visible columns
        // pair up one to one with its tab groups is adjusted.
        let visible: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, g)| g.iter().any(|r| matches!(r, Run::Math { .. }) || matches!(r, Run::Text(t) if !t.text.trim().is_empty())))
            .map(|(i, _)| i)
            .collect();
        if visible.len() != cols.len() || visible.is_empty() {
            return false;
        }
        let mut groups = groups;
        for (&gi, c) in visible.iter().zip(cols) {
            if c.score < SURE_COLUMN {
                continue;
            }
            // An answer blank's width says nothing about the type size.
            let blank = ruled || groups[gi].iter().any(|r| matches!(r, Run::Text(t) if t.text.contains("__")));
            if (c.scale - 1.0).abs() >= 0.025 && !blank {
                // Damped: a size change moves the lines below it, measured again next round.
                let s = 1.0 + (c.scale.clamp(0.8, 1.25) - 1.0) * 0.7;
                for r in groups[gi].iter_mut() {
                    let size = match r {
                        Run::Text(t) => &mut t.size,
                        Run::Math { size, .. } => size,
                        _ => continue,
                    };
                    let old = size.unwrap_or(base);
                    let new = half(old * s);
                    if new != old {
                        *size = Some(new);
                        changed = true;
                    }
                }
            }
            if c.dx.abs() >= 0.75 {
                moves.push((gi, c.dx));
            }
        }
    }
    for (gi, dx) in moves {
        if gi == 0 {
            if left {
                p.indent += dx;
                changed = true;
            }
        } else if let Some(&t) = tabs.get(gi - 1) {
            p.tabs[t].pos = (p.tabs[t].pos + dx).max(0.0);
            changed = true;
        }
    }
    changed
}

/// Moves body block `i` down by `d` points (up when negative), keeping everything before it.
fn shift(blocks: &mut Vec<Block>, page: &mut PageSetup, i: usize, d: f32) {
    if i == 0 {
        page.margins[0] = (page.margins[0] + d).max(0.0);
        return;
    }
    if let Block::P(p) = &mut blocks[i] {
        // Space is kept one-sided (`one_sided`): before a paragraph, unless what precedes it
        // isn't one.
        let room = p.before + d;
        p.before = room.max(0.0);
        if room >= 0.0 {
            return;
        }
        // Moving up past zero space before: take it from the block above.
        return take_after(blocks, i - 1, room);
    }
    match &mut blocks[i - 1] {
        Block::P(p) => p.after = (p.after + d).max(0.0),
        Block::Space { height } => *height = (*height + d).max(1.0),
        _ if d > 0.0 => blocks.insert(i, Block::Space { height: d }),
        _ => {}
    }
}
fn take_after(blocks: &mut [Block], i: usize, d: f32) {
    match &mut blocks[i] {
        Block::P(p) => p.after = (p.after + d).max(0.0),
        Block::Space { height } => *height = (*height + d).max(1.0),
        _ => {}
    }
}

/// How far the scan leans, points across per point down, from how line starts drift across
/// further down the page (`(y, dx)` pairs; a least-squares line); `None` with too few lines
/// spread over too little of the page.
fn lean(pts: &[(f32, f32)]) -> Option<f32> {
    let n = pts.len() as f32;
    if pts.len() < 4 {
        return None;
    }
    let (my, mx) = (pts.iter().map(|p| p.0).sum::<f32>() / n, pts.iter().map(|p| p.1).sum::<f32>() / n);
    let var: f32 = pts.iter().map(|p| (p.0 - my).powi(2)).sum();
    if (var / n).sqrt() < 100.0 {
        return None;
    }
    Some(pts.iter().map(|p| (p.0 - my) * (p.1 - mx)).sum::<f32>() / var)
}

/// How far the rebuild is from the scan: mean and worst vertical miss of the matched blocks,
/// points, and how many blocks matched.
#[derive(Clone, Copy, Debug, Default)]
pub struct Report {
    pub mean: f32,
    pub worst: f32,
    /// The worst miss across, of left-aligned line starts.
    pub across: f32,
    pub matched: usize,
    pub blocks: usize,
    /// The shear the scan needed (see [`prepare_scan`]).
    pub shear: f32,
}
impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "{}/{} blocks matched, off by {:.1} pt on average and {:.1} pt at worst down, {:.1} pt at worst across (scan shear {:+.4})",
            self.matched, self.blocks, self.mean, self.worst, self.across, self.shear
        )
    }
}
fn report(body: &[Option<Offset>], starts: &[(f32, f32)]) -> Report {
    let m: Vec<f32> = body.iter().flatten().map(|o| o.dy.abs()).collect();
    Report {
        mean: if m.is_empty() { 0.0 } else { m.iter().sum::<f32>() / m.len() as f32 },
        worst: m.iter().copied().fold(0.0, f32::max),
        matched: m.len(),
        blocks: body.len(),
        across: starts.iter().map(|s| s.1.abs()).fold(0.0, f32::max),
        shear: 0.0,
    }
}

/// Adjusts the sheet over up to `rounds` passes so its rendering lines up with the scan.
pub fn fit(scan: &Path, sheet: &mut Sheet, rounds: usize) -> Result<Report, String> {
    sheet.normalize();
    let (w, h) = ((sheet.page.width * K).round() as u32, (sheet.page.height * K).round() as u32);
    let mut shear = 0.0;
    let mut page = prepare_scan(scan, w, h, shear)?;
    let mut last = Report::default();
    // The closest round so far: a measurement can flip between two readings, so the result is
    // the best state seen, not the last.
    let mut best: Option<(f32, Sheet, Report)> = None;
    for round in 0..rounds {
        let Measure { body, columns, head, foot } = measure(&page, sheet)?;
        // Where left-aligned lines start on the scan against the rebuild: the steadiest
        // horizontal reading, since a line's width depends on the font as well.
        let starts: Vec<(f32, f32)> = body
            .iter()
            .zip(&columns)
            .zip(&sheet.blocks)
            .filter_map(|((o, c), b)| {
                let (o, c) = (o.as_ref()?, c.first()?);
                let left = matches!(b, Block::P(p) if p.align == Align::Left);
                (left && c.score >= SURE_COLUMN).then_some((o.y, c.dx))
            })
            .collect();
        last = Report { shear, ..report(&body, &starts) };
        eprintln!("fit round {}: {last}", round + 1);
        let score = last.mean + last.worst * 0.5 + last.across * 0.5 - last.matched as f32 * 2.0;
        if best.as_ref().is_none_or(|b| score < b.0) {
            best = Some((score, sheet.clone(), last));
        }
        // A start that drifts across further down the page is the scan leaning: shear it.
        if let Some(lean) = lean(&starts) {
            if lean.abs() > 0.003 {
                shear += lean;
                page = prepare_scan(scan, w, h, shear)?;
                eprintln!("  sheared the scan by {lean:+.4}");
                continue;
            }
        }
        // Horizontal: first move the whole text column by the median start; once that is in
        // place, each line's columns get their own indent, tab stop and size.
        let mut dxs: Vec<f32> = starts.iter().map(|s| s.1).collect();
        dxs.sort_by(f32::total_cmp);
        let median = dxs.get(dxs.len() / 2).copied().unwrap_or(0.0);
        let mut changed = false;
        // Left-aligned lines fix the left margin; centred lines show where the middle of the
        // text column is, so they fix the right one.
        let mut mids: Vec<f32> = columns
            .iter()
            .zip(&sheet.blocks)
            .filter(|(c, b)| c.len() == 1 && matches!(b, Block::P(p) if p.align == Align::Center))
            .map(|(c, _)| c[0].dx)
            .collect();
        mids.sort_by(f32::total_cmp);
        if let Some(&mid) = mids.get(mids.len() / 2).filter(|m| m.abs() >= 0.75) {
            sheet.page.margins[1] = (sheet.page.margins[1] - 2.0 * mid).max(0.0);
            changed = true;
        }
        if median.abs() >= 0.75 {
            sheet.page.margins[3] = (sheet.page.margins[3] + median).max(0.0);
            sheet.page.margins[1] = (sheet.page.margins[1] - median).max(0.0);
            changed = true;
        } else {
            let base = sheet.size;
            for (b, c) in sheet.blocks.iter_mut().zip(&columns) {
                changed |= fit_columns(b, c, base);
            }
        }
        if let Some(h) = head.filter(|h| h.dy.abs() >= 0.5) {
            sheet.page.header_at = (sheet.page.header_at + h.dy).max(0.0);
            changed = true;
        }
        if let Some(f) = foot.filter(|f| f.dy.abs() >= 0.5) {
            sheet.page.footer_at = (sheet.page.footer_at - f.dy).max(0.0);
            changed = true;
        }
        // Vertical: each matched block moves by its own offset less the one it already moves
        // with (the last matched block above it). Work bottom-up so inserted spaces don't shift
        // the indices still to do.
        let mut moves = vec![];
        let mut above = 0.0;
        for (i, o) in body.iter().enumerate() {
            if let Some(o) = o {
                moves.push((i, o.dy - above));
                above = o.dy;
            }
        }
        // Any whole pixel (0.48 pt) counts: small steps left alone add up down the page.
        for (i, d) in moves.into_iter().rev() {
            if d.abs() >= 0.4 {
                shift(&mut sheet.blocks, &mut sheet.page, i, d);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    if let Some((_, s, r)) = best {
        *sheet = s;
        last = r;
    }
    Ok(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_read_from_pdftotext() {
        // docx::marker('B', 12) is "QBbmZ"; its letters arrive in any order.
        assert_eq!(docx::marker('B', 12), "QBbmZ");
        let xml = r#"<word xMin="72.1" yMin="60.6" xMax="72.4" yMax="61.5">m</word>
<word xMin="72.0" yMin="60.6" xMax="72.4" yMax="61.5">Q</word>
<word xMin="72.2" yMin="60.6" xMax="72.4" yMax="61.5">Z</word>
<word xMin="72.2" yMin="60.6" xMax="72.4" yMax="61.5">b</word>
<word xMin="72.2" yMin="60.6" xMax="72.4" yMax="61.5">B</word>
<word xMin="80.0" yMin="40.1" xMax="90.4" yMax="61.5">Building</word>"#;
        let m = parse_marks(xml);
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].part, m[0].index, m[0].x, m[0].y), ('B', 12, 72.0, 61.5));
    }

    #[test]
    fn a_block_moved_up_past_its_space_takes_from_the_one_above() {
        let p = |before: f32, after: f32| Block::P(Para { before, after, runs: vec![], ..Default::default() });
        let mut blocks = vec![p(0.0, 20.0), p(4.0, 0.0)];
        let mut page = PageSetup::default();
        shift(&mut blocks, &mut page, 1, -10.0);
        let (Block::P(a), Block::P(b)) = (&blocks[0], &blocks[1]) else { panic!() };
        assert_eq!((a.after, b.before), (14.0, 0.0));
        shift(&mut blocks, &mut page, 0, 5.0);
        assert_eq!(page.margins[0], 77.0);
    }

    #[test]
    fn the_strongest_match_is_where_the_ink_is() {
        let mut t = GrayImage::from_pixel(60, 30, Luma([255]));
        let mut s = GrayImage::from_pixel(60, 30, Luma([255]));
        for x in 10..20 {
            t.put_pixel(x, 10, Luma([0]));
            s.put_pixel(x + 3, 14, Luma([0]));
        }
        let at = ncc(&t, 0, 5, 60, 10, &s, 3, 4, 1);
        let off = ncc(&t, 0, 5, 60, 10, &s, 0, 0, 1);
        assert!(at > 0.9 && off < 0.2, "{at} {off}");
    }
}
