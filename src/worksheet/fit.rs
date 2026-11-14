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
pub const K: f32 = 150.0 / 72.0;

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
    let k = 150.0 / scan_dpi(img.width(), w as f32 / K);
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

/// A scan's resolution, from its width in pixels and the paper's in points: the common scanner
/// setting nearest to it (a scan often takes in a little beyond the sheet), else as measured.
pub fn scan_dpi(pixels: u32, paper: f32) -> f32 {
    let guess = pixels as f32 / (paper / 72.0);
    [100.0, 150.0, 200.0, 240.0, 300.0, 400.0, 600.0]
        .into_iter()
        .min_by(|a: &f32, b: &f32| (a - guess).abs().total_cmp(&(b - guess).abs()))
        .filter(|d| (d - guess).abs() / guess < 0.08)
        .unwrap_or(guess)
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
    let mut tried = 0.0;
    let mut y = 0;
    while y < th {
        let mut x = 0;
        while x < tw {
            tried += 1.0;
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
    // A strip mostly off the page matches a few stray pixels by chance: it doesn't count.
    if n < 1.0 || n < tried * 0.9 {
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
    /// The first line (a figure: its box) as drawn: left, top, width, height in pixels of the
    /// rendering.
    pub strip: [u32; 4],
}

/// The first line of the block whose marker is `m`, as drawn: left, top, width, height in
/// pixels of the rendering. `None` when it has almost no ink (an empty paragraph, a figure).
fn strip_of(render: &GrayImage, m: &Mark, size: f32, sheet: &Sheet) -> Option<[u32; 4]> {
    let size = size.max(6.0);
    let top = ((m.y - size * 0.95) * K).max(0.0) as u32;
    let bottom = ((m.y + size * 0.3) * K) as u32;
    let left = (sheet.page.margins[3].min(m.x) - 6.0).max(0.0) * K;
    let right = (sheet.page.width - sheet.page.margins[1] + 6.0) * K;
    let (tx, ty, tw, th) = (left as u32, top, (right - left).max(1.0) as u32, bottom.saturating_sub(top).max(1));
    let mut inked = 0;
    for y in ty..(ty + th).min(render.height()) {
        for x in tx..(tx + tw).min(render.width()) {
            if render.get_pixel(x, y).0[0] < 128 {
                inked += 1;
            }
        }
    }
    (inked >= 40).then_some([tx, ty, tw, th])
}

/// A place a line might be on the scan: pixels down and across from where it was drawn, and
/// how well it matches there.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    dy: i32,
    dx: i32,
    score: f32,
    /// A figure's size on the scan against as drawn (1 for text).
    scale: f32,
}

/// How far down the page a line is looked for, either way, points.
const SEARCH: f32 = 150.0;

/// The best few places for a strip on the scan, within `SEARCH` points down or up and 8 across:
/// the peaks of a coarse correlation, strongest first.
fn candidates(render: &GrayImage, scan: &GrayImage, [tx, ty, tw, th]: [u32; 4]) -> Vec<Candidate> {
    let (wy, wx) = ((SEARCH * K) as i32, (8.0 * K) as i32);
    let curve: Vec<Candidate> = (-wy..=wy)
        .step_by(2)
        .map(|dy| {
            (-wx..=wx)
                .step_by(4)
                .map(|dx| Candidate { dy, dx, score: ncc(render, tx, ty, tw, th, scan, dx, dy, 3), scale: 1.0 })
                .fold(Candidate { dy, dx: 0, score: f32::MIN, scale: 1.0 }, |a, b| if b.score > a.score { b } else { a })
        })
        .collect();
    let mut peaks: Vec<Candidate> = (0..curve.len())
        .filter(|&i| {
            let c = curve[i].score;
            c >= 0.15 && (i == 0 || curve[i - 1].score <= c) && (i + 1 == curve.len() || curve[i + 1].score < c)
        })
        .map(|i| curve[i])
        .collect();
    peaks.sort_by(|a, b| b.score.total_cmp(&a.score));
    peaks.truncate(6);
    peaks
}

/// A candidate polished to the pixel.
fn refine(render: &GrayImage, scan: &GrayImage, m: &Mark, strip: [u32; 4], c: Candidate) -> Offset {
    let [tx, ty, tw, th] = strip;
    let mut best = (f32::MIN, c.dx, c.dy);
    for dy in (c.dy - 3)..=(c.dy + 3) {
        for dx in (c.dx - 3)..=(c.dx + 3) {
            let v = ncc(render, tx, ty, tw, th, scan, dx, dy, 1);
            if v > best.0 {
                best = (v, dx, dy);
            }
        }
    }
    Offset { y: m.y, dy: best.2 as f32 / K, dx: best.1 as f32 / K, score: best.0, strip }
}

/// Picks one candidate per line (or none) so the lines keep their order down the page without
/// running into each other, with the most match in all and least jumping about: a list of
/// look-alike rows can't be matched a row off, and a line far from where it was drawn (answer
/// space misjudged) is still found. Returns the chosen candidate per line.
///
/// Each line comes with the score a match must beat to be worth choosing (lower for figures,
/// whose drawing differs from the print more than text does). `start` is the offset, pixels,
/// the list as a whole is expected near (that of the nearest line placed above it): the first
/// line chosen pays for straying from it.
fn align(lines: &[(f32, Vec<Candidate>, f32)], start: i32) -> Vec<Option<Candidate>> {
    // Per point of change in offset from one chosen line to the next, and from `start`.
    const JUMP: f32 = 0.003;
    const STRAY: f32 = 0.006;
    let mut best: Vec<Vec<(f32, Option<(usize, usize)>)>> = vec![];
    for (i, (y, cands, floor)) in lines.iter().enumerate() {
        let mut row = vec![];
        for c in cands {
            let at = y + c.dy as f32 / K;
            let mut total = (c.score - floor - STRAY * (c.dy - start).abs() as f32 / K, None);
            for (j, (yj, prev, _)) in lines[..i].iter().enumerate() {
                // At least a third of the drawn distance apart, and 4 points.
                let gap = ((y - yj) * 0.3).max(4.0);
                for (q, p) in prev.iter().enumerate() {
                    if at - (yj + p.dy as f32 / K) < gap {
                        continue;
                    }
                    let t = best[j][q].0 + c.score - floor - JUMP * (c.dy - p.dy).abs() as f32 / K;
                    if t > total.0 {
                        total = (t, Some((j, q)));
                    }
                }
            }
            row.push(total);
        }
        best.push(row);
    }
    let mut out = vec![None; lines.len()];
    let mut at = best
        .iter()
        .enumerate()
        .flat_map(|(i, r)| r.iter().enumerate().map(move |(k, t)| (t.0, i, k)))
        .filter(|t| t.0 > 0.0)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|t| (t.1, t.2));
    while let Some((i, k)) = at {
        out[i] = Some(lines[i].1[k]);
        at = best[i][k].1;
    }
    out
}

/// A line whose glyphs don't match the scan (another font) can still be placed by its shape
/// down the page: the ink per row across its width, correlated against the scan's within
/// ±`reach` points of `guess`. Returns the offset down, points, and the correlation.
fn band(render: &GrayImage, scan: &GrayImage, o: &Offset, guess: f32, reach: f32) -> Option<(f32, f32)> {
    let [tx, ty, tw, th] = o.strip;
    let pad = (reach * K).max(2.0) as i32;
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

/// What one measuring pass found, per anchor (paragraph, spacer or figure, in
/// [`super::anchors`] order).
pub struct Measure {
    pub anchors: Vec<super::Path>,
    /// How far each anchor's first line (a figure: its box) is from the scan; `None` where it
    /// couldn't be matched.
    pub found: Vec<Option<Offset>>,
    /// The columns of each matched paragraph's first line.
    pub columns: Vec<Vec<Column>>,
    /// How much bigger each matched figure is on the scan.
    pub scales: Vec<Option<f32>>,
    /// Where each anchor's first line (a figure: its box) was drawn, pixels of the rendering;
    /// `None` for one with nothing to match (a spacer, an empty paragraph).
    pub drawn: Vec<Option<[u32; 4]>>,
    /// The first header and footer anchor's offsets.
    pub head: Option<Offset>,
    pub foot: Option<Offset>,
}

/// The box of a figure drawn with marker `m`: an inline picture stands on the line's baseline,
/// and the marker just before it gives its left edge.
fn figure_strip(render: &GrayImage, m: &Mark, f: &Figure) -> Option<[u32; 4]> {
    let (x, y) = ((m.x * K).max(0.0) as u32, ((m.y - f.height) * K).max(0.0) as u32);
    let (w, h) = ((f.width * K) as u32, (f.height * K) as u32);
    (w > 4 && h > 4 && x + w <= render.width() && y + h <= render.height()).then_some([x, y, w, h])
}

/// The first line of a paragraph inside a table cell `width` points wide: from its marker to
/// the cell's width further on, so the next cell's text stays out of it.
fn cell_strip(render: &GrayImage, m: &Mark, size: f32, width: f32) -> Option<[u32; 4]> {
    let size = size.max(6.0);
    let top = ((m.y - size * 0.95) * K).max(0.0) as u32;
    let bottom = ((m.y + size * 0.3) * K) as u32;
    let left = ((m.x - 4.0) * K).max(0.0) as u32;
    let right = (((m.x + width) * K) as u32).min(render.width());
    let strip = [left, top, right.saturating_sub(left).max(1), bottom.saturating_sub(top).max(1)];
    let [tx, ty, tw, th] = strip;
    let inked = (ty..(ty + th).min(render.height())).flat_map(|y| (tx..tx + tw).map(move |x| (x, y))).filter(|&(x, y)| render.get_pixel(x, y).0[0] < 128).count();
    (inked >= 40).then_some(strip)
}

/// The width, points, of the cell a nested anchor sits in.
fn cell_width(blocks: &[Block], path: &[usize]) -> Option<f32> {
    let (inner, cell) = (path.len() >= 4).then(|| (&path[..path.len() - 1], &path[path.len() - 3..path.len() - 1]))?;
    let table_path = &inner[..inner.len() - 2];
    let Block::Table(t) = super::block_at(blocks, table_path)? else { return None };
    let row = t.rows.get(cell[0])?;
    let start: usize = row.cells.iter().take(cell[1]).map(|c| c.span.max(1) as usize).sum();
    let span = row.cells.get(cell[1])?.span.max(1) as usize;
    Some(t.widths.iter().skip(start).take(span).sum())
}

/// Correlation of the rendering's box `[tx, ty, tw, th]`, scaled by `s` about its top-left
/// corner and moved `dx`, `dy`, against the scan, sampling every `step` pixels of the scan.
fn scaled_ncc(render: &GrayImage, scan: &GrayImage, [tx, ty, tw, th]: [u32; 4], s: f32, dx: i32, dy: i32, step: u32) -> f32 {
    let (mut sa, mut sb, mut saa, mut sbb, mut sab, mut n, mut tried) = (0f32, 0f32, 0f32, 0f32, 0f32, 0f32, 0f32);
    let (sw, sh) = ((tw as f32 * s) as u32, (th as f32 * s) as u32);
    let mut y = 0;
    // Walk the scan's box and look the rendering up, so stretching skips nothing.
    while y < sh {
        let mut x = 0;
        while x < sw {
            tried += 1.0;
            let (rx, ry) = (tx + (x as f32 / s) as u32, ty + (y as f32 / s) as u32);
            let (sx, sy) = (tx as i32 + dx + x as i32, ty as i32 + dy + y as i32);
            if rx < render.width() && ry < render.height() && sx >= 0 && sy >= 0 && (sx as u32) < scan.width() && (sy as u32) < scan.height() {
                let a = ink(render.get_pixel(rx, ry).0[0]);
                let b = ink(scan.get_pixel(sx as u32, sy as u32).0[0]);
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
    if n < 1.0 || n < tried * 0.9 {
        return 0.0;
    }
    let cov = sab - sa * sb / n;
    let (va, vb) = (saa - sa * sa / n, sbb - sb * sb / n);
    if va <= 1e-6 || vb <= 1e-6 { 0.0 } else { cov / (va * vb).sqrt() }
}

/// The best few places and sizes for a figure on the scan. A whole figure is coarse, so the
/// search runs on quarter-size copies over a wide range: `SEARCH` points down or up, 40 across,
/// and 0.8 to 1.25 times the size. Strongest first, in full-size pixels.
fn figure_candidates(render: &GrayImage, scan: &GrayImage, [tx, ty, tw, th]: [u32; 4]) -> Vec<Candidate> {
    const Q: u32 = 4;
    let small = |g: &GrayImage| image::imageops::resize(g, g.width() / Q, g.height() / Q, image::imageops::FilterType::Triangle);
    let (r, sc) = (small(render), small(scan));
    let strip = [tx / Q, ty / Q, tw / Q, th / Q];
    let (wy, wx) = ((SEARCH * K) as i32 / Q as i32, (40.0 * K) as i32 / Q as i32);
    let scales: Vec<f32> = (0..10).map(|i| 0.8 + i as f32 * 0.05).collect();
    let curve: Vec<Candidate> = (-wy..=wy)
        .map(|dy| {
            let mut best = Candidate { dy: dy * Q as i32, dx: 0, score: f32::MIN, scale: 1.0 };
            for &s in &scales {
                // A bigger figure reaches further from the same middle: centre the search there.
                let (cx, cy) = (-((s - 1.0) * strip[2] as f32 / 2.0) as i32, -((s - 1.0) * strip[3] as f32 / 2.0) as i32);
                for dx in (-wx..=wx).step_by(1) {
                    let v = scaled_ncc(&r, &sc, strip, s, cx + dx, cy + dy, 2);
                    if v > best.score {
                        best = Candidate { dy: (cy + dy) * Q as i32, dx: (cx + dx) * Q as i32, score: v, scale: s };
                    }
                }
            }
            best
        })
        .collect();
    let mut peaks: Vec<Candidate> = (0..curve.len())
        .filter(|&i| {
            let c = curve[i].score;
            c >= 0.15 && (i == 0 || curve[i - 1].score <= c) && (i + 1 == curve.len() || curve[i + 1].score < c)
        })
        .map(|i| curve[i])
        .collect();
    peaks.sort_by(|a, b| b.score.total_cmp(&a.score));
    peaks.truncate(6);
    peaks
}

/// A figure candidate polished at full size: scale within ±0.06 and offset within a few
/// pixels. Returns the scale and the offset of the figure's top-left corner.
fn figure_refine(render: &GrayImage, scan: &GrayImage, m: &Mark, strip: [u32; 4], c: Candidate) -> (f32, Offset) {
    let mut best = (f32::MIN, c.scale, c.dx, c.dy);
    for si in -6..=6 {
        let s = c.scale + si as f32 * 0.01;
        for dy in ((c.dy - 6)..=(c.dy + 6)).step_by(2) {
            for dx in ((c.dx - 6)..=(c.dx + 6)).step_by(2) {
                let v = scaled_ncc(render, scan, strip, s, dx, dy, 3);
                if v > best.0 {
                    best = (v, s, dx, dy);
                }
            }
        }
    }
    let (_, s, bx, by) = best;
    for dy in (by - 2)..=(by + 2) {
        for dx in (bx - 2)..=(bx + 2) {
            let v = scaled_ncc(render, scan, strip, s, dx, dy, 2);
            if v > best.0 {
                best = (v, s, dx, dy);
            }
        }
    }
    (best.1, Offset { y: m.y, dy: best.3 as f32 / K, dx: best.2 as f32 / K, score: best.0, strip })
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
    let anchors = super::anchors(&sheet.blocks);
    let n = anchors.len();
    let (mut found, mut columns, mut scales) = (vec![None; n], vec![vec![]; n], vec![None; n]);
    // Each anchor that has something to match: its marker and its strip (a figure: its box).
    let mut lines: Vec<(usize, &Mark, [u32; 4])> = vec![];
    for m in marks.iter().filter(|m| m.part == 'B') {
        let Some(path) = anchors.get(m.index) else { continue };
        let Some(b) = super::block_at(&sheet.blocks, path) else { continue };
        let strip = match b {
            Block::Figure(f) => figure_strip(&render, m, f),
            Block::P(_) if path.len() > 1 => cell_width(&sheet.blocks, path).and_then(|w| cell_strip(&render, m, first_size(b, sheet), w)),
            Block::P(_) => strip_of(&render, m, first_size(b, sheet), sheet),
            _ => None,
        };
        if let Some(strip) = strip {
            lines.push((m.index, m, strip));
        }
    }
    // Candidate places for each, then the best consistent choice within each list of blocks
    // (the body, each table cell): order down the page holds within a list, not across lists.
    // A drawn figure's lines are thinner or bolder than the print's: figures are compared
    // blurred, which lets a thin line and a thick one in the same place agree.
    let soft_render = image::imageops::blur(&render, 2.0);
    let soft_scan = image::imageops::blur(&scan_page, 2.0);
    let is_figure = |k: usize| matches!(super::block_at(&sheet.blocks, &anchors[lines[k].0]), Some(Block::Figure(_)));
    let cands: Vec<Vec<Candidate>> = (0..lines.len())
        .map(|k| if is_figure(k) { figure_candidates(&soft_render, &soft_scan, lines[k].2) } else { candidates(&render, &scan_page, lines[k].2) })
        .collect();
    if debug {
        for k in (0..lines.len()).filter(|&k| is_figure(k)) {
            eprintln!("  figure anchor {} candidates: {:?}", lines[k].0, cands[k].iter().map(|c| (c.dy as f32 / K, c.dx as f32 / K, c.score)).collect::<Vec<_>>());
        }
    }
    let mut chosen: Vec<Option<Candidate>> = vec![None; lines.len()];
    // Each list in turn, outer lists first (paths sort a table's cells after the body), so a
    // cell can start from the nearest line already placed above it.
    let mut lists: Vec<&[usize]> = lines.iter().map(|(i, ..)| &anchors[*i][..anchors[*i].len() - 1]).collect();
    lists.sort_by_key(|l| l.len());
    lists.dedup();
    for list in lists {
        let members: Vec<usize> = (0..lines.len()).filter(|&k| &anchors[lines[k].0][..anchors[lines[k].0].len() - 1] == list).collect();
        let first = lines[members[0]].0;
        let start = (0..lines.len()).filter(|&k| lines[k].0 < first).filter_map(|k| chosen[k]).last().map(|c| c.dy).unwrap_or(0);
        let input: Vec<(f32, Vec<Candidate>, f32)> =
            members.iter().map(|&k| (lines[k].1.y, cands[k].clone(), if is_figure(k) { 0.12 } else { 0.3 })).collect();
        for (k, c) in members.iter().zip(align(&input, start)) {
            chosen[*k] = c;
        }
    }
    let mut guess = 0.0;
    for (k, &(i, m, strip)) in lines.iter().enumerate() {
        let b = super::block_at(&sheet.blocks, &anchors[i]).expect("anchor");
        let figure = matches!(b, Block::Figure(_));
        let (r, sc) = if figure { (&soft_render, &soft_scan) } else { (&render, &scan_page) };
        let mut o = match chosen[k] {
            Some(c) if figure => {
                let (s, o) = figure_refine(r, sc, m, strip, c);
                scales[i] = Some(s);
                o
            }
            Some(c) => refine(r, sc, m, strip, c),
            None => Offset { y: m.y, dy: guess, dx: 0.0, score: 0.0, strip },
        };
        let mut c = vec![];
        let mut sure;
        if figure {
            sure = o.score >= SURE_FIGURE;
        } else {
            // The columns are matched allowing for a different type size, so they can vouch for
            // a line whose whole-width match is weak because its text runs wider or narrower.
            c = if chosen[k].is_some() { columns_of(&render, &scan_page, &o) } else { vec![] };
            let col_score = if c.is_empty() { 0.0 } else { c.iter().map(|c| c.score).sum::<f32>() / c.len() as f32 };
            sure = o.score >= SURE || (o.score >= 0.25 && col_score >= SURE_COLUMN + 0.1);
            if !sure {
                // Placed by shape instead; its glyphs can't be matched, but a centred line's
                // width still can: it scales the type so the line spans what it spans there.
                if let Some((dy, score)) = band(&render, &scan_page, &o, guess, 30.0).filter(|b| b.1 >= 0.8) {
                    o = Offset { dy, dx: 0.0, score, ..o };
                    let centred = matches!(b, Block::P(p) if p.align == Align::Center);
                    c = if centred { span_ratio(&render, &scan_page, &o).map(|r| vec![r]).unwrap_or_default() } else { vec![] };
                    sure = true;
                }
            }
        }
        if debug {
            eprintln!(
                "  anchor {:>2} {:?} at y {:6.1}: down {:+.1} across {:+.1} match {:.2} {}{} columns: {}",
                i,
                anchors[i],
                m.y,
                o.dy,
                o.dx,
                o.score,
                if sure { "sure" } else { "unsure" },
                scales[i].map(|s| format!(" scale x{s:.3}")).unwrap_or_default(),
                c.iter().map(|c| format!("{:+.1}pt x{:.3} ({:.2})", c.dx, c.scale, c.score)).collect::<Vec<_>>().join(", ")
            );
        }
        if sure {
            if anchors[i].len() == 1 {
                guess = o.dy;
            }
            found[i] = Some(o);
            columns[i] = c;
        } else {
            scales[i] = None;
        }
    }
    let edge = |part: char, blocks: &[Block]| {
        marks
            .iter()
            .find(|m| m.part == part && m.index == 0)
            .and_then(|m| {
                let strip = strip_of(&render, m, blocks.first().map(|b| first_size(b, sheet)).unwrap_or(0.0), sheet)?;
                let c = *candidates(&render, &scan_page, strip).first()?;
                Some(refine(&render, &scan_page, m, strip, c))
            })
            .filter(|o| o.score >= SURE)
    };
    let (head, foot) = (edge('H', &sheet.header), edge('F', &sheet.footer));
    let mut drawn = vec![None; n];
    for &(i, _, strip) in &lines {
        drawn[i] = Some(strip);
    }
    Ok(Measure { anchors, found, columns, scales, drawn, head, foot })
}

/// Below this correlation a figure's match is not trusted.
const SURE_FIGURE: f32 = 0.3;

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

/// A change the fit makes to the sheet's vertical layout.
#[derive(Clone, Debug, PartialEq)]
enum Edit {
    /// Move the block at `path` (and everything after it in its list) down by `d` points.
    Move { path: super::Path, d: f32 },
    /// Set a table row's minimum height, points.
    Row { table: super::Path, row: usize, height: f32 },
}

/// Plans the vertical moves for one list of blocks (the body, or a cell), given each anchor's
/// offset and where it was drawn. `start` is the offset the list's first block already moves
/// with: the page top (0) for the body, the row's offset for a cell.
fn plan_list(blocks: &[Block], prefix: &[usize], start: f32, offs: &std::collections::HashMap<super::Path, (f32, f32)>, edits: &mut Vec<Edit>) {
    let mut prev = start;
    for (k, b) in blocks.iter().enumerate() {
        let path: super::Path = prefix.iter().copied().chain([k]).collect();
        let dy = match b {
            Block::Table(t) => plan_table(t, &path, offs, edits),
            _ => offs.get(&path).map(|o| o.0),
        };
        if let Some(dy) = dy {
            // Any whole pixel (0.48 pt) counts: small steps left alone add up down the page.
            if (dy - prev).abs() >= 0.4 {
                edits.push(Edit::Move { path, d: dy - prev });
            }
            prev = dy;
        }
    }
}

/// Plans a table's rows and cells; returns its offset (its first row's).
fn plan_table(t: &Table, path: &[usize], offs: &std::collections::HashMap<super::Path, (f32, f32)>, edits: &mut Vec<Edit>) -> Option<f32> {
    let median = |mut v: Vec<f32>| -> Option<f32> {
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied()
    };
    // Each row's offset and where it was drawn, from the first block of each of its cells.
    let rows: Vec<Option<(f32, f32)>> = (0..t.rows.len())
        .map(|r| {
            let firsts: Vec<(f32, f32)> = (0..t.rows[r].cells.len())
                .filter_map(|c| offs.get(&path.iter().copied().chain([r, c, 0]).collect::<Vec<_>>()).copied())
                .collect();
            Some((median(firsts.iter().map(|f| f.0).collect())?, median(firsts.iter().map(|f| f.1).collect())?))
        })
        .collect();
    let mut last: Option<(usize, (f32, f32))> = None;
    for (r, row) in rows.iter().enumerate() {
        if let Some((dy, y)) = *row {
            // A row that should start lower makes the row above it taller.
            if let Some((q, (pdy, py))) = last {
                let d = dy - pdy;
                if q + 1 == r && d.abs() >= 0.4 {
                    edits.push(Edit::Row { table: path.to_vec(), row: q, height: (y - py + d).max(4.0) });
                }
            }
            last = Some((r, (dy, y)));
        }
        let start = row.map(|r| r.0).or(last.map(|l| l.1.0)).unwrap_or(0.0);
        for (c, cell) in t.rows[r].cells.iter().enumerate() {
            let prefix: Vec<usize> = path.iter().copied().chain([r, c]).collect();
            plan_list(&cell.blocks, &prefix, start, offs, edits);
        }
    }
    rows.first().copied().flatten().map(|r| r.0)
}

/// Applies a planned edit.
fn apply(sheet: &mut Sheet, e: &Edit) {
    match e {
        Edit::Row { table, row, height } => {
            if let Some(r) = super::table_at_mut(&mut sheet.blocks, table).and_then(|t| t.rows.get_mut(*row)) {
                r.height = Some(*height);
            }
        }
        Edit::Move { path, d } => {
            let k = *path.last().expect("path");
            if path.len() == 1 && k == 0 {
                sheet.page.margins[0] = (sheet.page.margins[0] + d).max(0.0);
                return;
            }
            let Some(list) = super::list_at(&mut sheet.blocks, path) else { return };
            shift(list, k, *d);
        }
    }
}

/// Moves block `k` of a list down by `d` points (up when negative), keeping what is above it.
/// Space is kept one-sided (`one_sided`): before a paragraph or figure, after anything else.
fn shift(list: &mut Vec<Block>, k: usize, d: f32) {
    let own = match &mut list[k] {
        Block::P(p) => Some(&mut p.before),
        Block::Figure(f) => Some(&mut f.before),
        Block::Space { height } if k == 0 => Some(height),
        _ => None,
    };
    if let Some(before) = own {
        let room = *before + d;
        *before = room.max(0.0);
        // Moving up past no space before: take it from the block above.
        if room < 0.0 && k > 0 {
            take_after(list, k - 1, room);
        }
        return;
    }
    if k == 0 {
        if d > 0.0 {
            list.insert(0, Block::Space { height: d });
        }
        return;
    }
    match &mut list[k - 1] {
        Block::P(p) => p.after = (p.after + d).max(0.0),
        Block::Space { height } => *height = (*height + d).max(1.0),
        _ if d > 0.0 => list.insert(k, Block::Space { height: d }),
        _ => {}
    }
}
fn take_after(list: &mut [Block], i: usize, d: f32) {
    match &mut list[i] {
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
            "{}/{} lines and figures matched, off by {:.1} pt on average and {:.1} pt at worst down, {:.1} pt at worst across (scan shear {:+.4})",
            self.matched, self.blocks, self.mean, self.worst, self.across, self.shear
        )
    }
}
fn report(body: &[Option<Offset>], lines: usize, starts: &[(f32, f32)]) -> Report {
    let m: Vec<f32> = body.iter().flatten().map(|o| o.dy.abs()).collect();
    Report {
        mean: if m.is_empty() { 0.0 } else { m.iter().sum::<f32>() / m.len() as f32 },
        worst: m.iter().copied().fold(0.0, f32::max),
        matched: m.len(),
        blocks: lines,
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
        let Measure { anchors, found, columns, scales, head, foot, .. } = measure(&page, sheet)?;
        let block = |i: usize| super::block_at(&sheet.blocks, &anchors[i]);
        // Where left-aligned body lines start on the scan against the rebuild: the steadiest
        // horizontal reading, since a line's width depends on the font as well.
        let starts: Vec<(f32, f32)> = (0..anchors.len())
            .filter(|&i| anchors[i].len() == 1 && matches!(block(i), Some(Block::P(p)) if p.align == Align::Left))
            .filter_map(|i| Some((found[i]?.y, columns[i].first().filter(|c| c.score >= SURE_COLUMN)?.dx)))
            .collect();
        let lines = (0..anchors.len()).filter(|&i| matches!(block(i), Some(Block::P(_) | Block::Figure(_)))).count();
        last = Report { shear, ..report(&found, lines, &starts) };
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
        // Left-aligned lines fix the left margin; centred body lines show where the middle of
        // the text column is, so they fix the right one.
        let mut mids: Vec<f32> = (0..anchors.len())
            .filter(|&i| anchors[i].len() == 1 && columns[i].len() == 1 && matches!(block(i), Some(Block::P(p)) if p.align == Align::Center))
            .map(|i| columns[i][0].dx)
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
            for (i, path) in anchors.iter().enumerate() {
                let Some(b) = super::block_at_mut(&mut sheet.blocks, path) else { continue };
                match b {
                    Block::P(_) => changed |= fit_columns(b, &columns[i], base),
                    Block::Figure(f) => {
                        // Damped like type sizes: a bigger figure moves what is below it.
                        if let Some(s) = scales[i].filter(|s| (s - 1.0).abs() >= 0.02) {
                            let s = 1.0 + (s.clamp(0.7, 1.4) - 1.0) * 0.8;
                            f.width = (f.width * s * 2.0).round() / 2.0;
                            f.height = (f.height * s * 2.0).round() / 2.0;
                            changed = true;
                        }
                        if let Some(o) = found[i].filter(|o| o.dx.abs() >= 0.75 && f.align == Align::Left) {
                            f.indent += o.dx;
                            changed = true;
                        }
                    }
                    _ => {}
                }
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
        // with; table rows grow or shrink to put the next row where it belongs. Planned from
        // this measurement, then applied last-first so an inserted space can't shift a path
        // still to do.
        let offs: std::collections::HashMap<super::Path, (f32, f32)> =
            (0..anchors.len()).filter_map(|i| Some((anchors[i].clone(), (found[i]?.dy, found[i]?.y)))).collect();
        let mut edits = vec![];
        plan_list(&sheet.blocks, &[], 0.0, &offs, &mut edits);
        edits.sort_by(|a, b| {
            let key = |e: &Edit| match e {
                Edit::Move { path, .. } => path.clone(),
                Edit::Row { table, row, .. } => table.iter().copied().chain([*row, usize::MAX]).collect(),
            };
            key(b).cmp(&key(a))
        });
        changed |= !edits.is_empty();
        for e in &edits {
            apply(sheet, e);
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
    fn look_alike_rows_keep_their_order() {
        let c = |dy: f32, score: f32| Candidate { dy: (dy * K) as i32, dx: 0, score, scale: 1.0 };
        // Three rows 16 pt apart that all look alike: each matches its own place (0) and its
        // neighbours' (±16) about as well. A row-off choice would break the order.
        let lines = vec![
            (100.0, vec![c(16.0, 0.82), c(0.0, 0.8)], 0.3),
            (116.0, vec![c(-16.0, 0.81), c(0.0, 0.8), c(16.0, 0.8)], 0.3),
            (132.0, vec![c(-16.0, 0.83), c(0.0, 0.8)], 0.3),
        ];
        let got: Vec<i32> = align(&lines, 0).into_iter().map(|c| c.unwrap().dy).collect();
        assert_eq!(got, vec![0, 0, 0]);
    }

    #[test]
    fn a_line_far_below_where_it_was_drawn_is_found() {
        let c = |dy: f32, score: f32| Candidate { dy: (dy * K) as i32, dx: 0, score, scale: 1.0 };
        // The answer space was drawn 45 pt too short: the second line and those after it sit
        // lower on the scan.
        let lines = vec![(100.0, vec![c(0.0, 0.8)], 0.3), (122.0, vec![c(45.0, 0.8), c(0.0, 0.2)], 0.3), (140.0, vec![c(45.0, 0.8)], 0.3)];
        let got: Vec<i32> = align(&lines, 0).into_iter().map(|c| c.unwrap().dy).collect();
        assert_eq!(got, vec![0, (45.0 * K) as i32, (45.0 * K) as i32]);
    }

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
        shift(&mut blocks, 1, -10.0);
        let (Block::P(a), Block::P(b)) = (&blocks[0], &blocks[1]) else { panic!() };
        assert_eq!((a.after, b.before), (14.0, 0.0));
    }

    #[test]
    fn rows_and_cells_are_planned_from_their_first_lines() {
        let src = r#"{"blocks":[{"type":"p","runs":[]},{"type":"table","widths":[100,100],"rows":[
            {"cells":[{"blocks":[{"type":"p","runs":[]},{"type":"p","runs":[]}]},{"blocks":[{"type":"p","runs":[]}]}]},
            {"cells":[{"blocks":[{"type":"p","runs":[]}]},{"blocks":[{"type":"p","runs":[]}]}]}]}]}"#;
        let mut sheet: Sheet = serde_json::from_str(src).unwrap();
        // Drawn: body line at 50, row 0 at 100, row 1 at 200. On the scan: the table is 2 pt
        // lower, the second line of cell (0,0) 3 pt lower again, and row 1 10 pt lower.
        let offs: std::collections::HashMap<Vec<usize>, (f32, f32)> = [
            (vec![0], (0.0, 50.0)),
            (vec![1, 0, 0, 0], (2.0, 100.0)),
            (vec![1, 0, 0, 1], (5.0, 120.0)),
            (vec![1, 0, 1, 0], (2.0, 100.0)),
            (vec![1, 1, 0, 0], (12.0, 200.0)),
            (vec![1, 1, 1, 0], (12.0, 200.0)),
        ]
        .into_iter()
        .collect();
        let mut edits = vec![];
        plan_list(&sheet.blocks, &[], 0.0, &offs, &mut edits);
        assert!(edits.contains(&Edit::Move { path: vec![1], d: 2.0 }), "{edits:?}");
        assert!(edits.contains(&Edit::Move { path: vec![1, 0, 0, 1], d: 3.0 }), "{edits:?}");
        assert!(edits.contains(&Edit::Row { table: vec![1], row: 0, height: 110.0 }), "{edits:?}");
        assert_eq!(edits.len(), 3, "{edits:?}");
        for e in &edits {
            apply(&mut sheet, e);
        }
        // The table's move went into the space after the paragraph above it (one-sided, as a
        // table has no space before); the cell's second line into its own space before.
        assert!(matches!(&sheet.blocks[0], Block::P(p) if p.after == 2.0));
        let Block::Table(t) = &sheet.blocks[1] else { panic!() };
        assert_eq!(t.rows[0].height, Some(110.0));
        assert!(matches!(&t.rows[0].cells[0].blocks[1], Block::P(p) if p.before == 3.0));
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
