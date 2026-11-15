//! Reading a student's page against a blank copy of the worksheet.
//!
//! The blank is read once by Datalab: every printed line, where it is, and where the Name line
//! is. Each student page is lined up with the blank by its print, the name blanked, and sent to
//! Datalab. A line that reads the same as a printed line in the same place is print; the rest is
//! the student's, boxed by the ink the blank doesn't have. Ink that nothing read accounts for is
//! flagged, so nothing the student wrote goes unseen.
use crate::{
    model::*,
    worksheet::{
        Run,
        student::{self, Item},
    },
};
use image::{GrayImage, RgbImage, imageops::FilterType};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Pages are compared and sent at this many pixels per inch.
pub const DPI: f32 = 150.0;

/// One line of print on the blank.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct PrintLine {
    /// As read, math as LaTeX in `$…$`.
    pub text: String,
    /// What it is compared by (see [`student::skeleton`]).
    pub skel: String,
    /// x0, y0, x1, y1 in pixels of the blank at [`DPI`].
    pub bbox: [f32; 4],
    /// A graph or picture the sheet prints (or a line of Datalab's description of one).
    #[serde(default)]
    pub figure: bool,
}

/// One page of the blank worksheet, read.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct BlankPage {
    pub page: Page,
    /// Its size in pixels at [`DPI`].
    pub size: [u32; 2],
    pub lines: Vec<PrintLine>,
    /// Each printed Name line, x0, y0, x1, y1 in pixels at [`DPI`].
    pub names: Vec<[f32; 4]>,
}

/// The blank worksheet the document's pages are read against: one page, or one for each side.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Blank {
    pub pages: Vec<BlankPage>,
}

/// How the blank lies on a scan: a point of the blank at (x, y) is at (s·x + dx, s·y + dy) on
/// the scan, pixels at [`DPI`]. `score` is how well the print agrees, 0–1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub s: f32,
    pub dx: f32,
    pub dy: f32,
    pub score: f32,
}
impl Fit {
    fn map(&self, [x0, y0, x1, y1]: [f32; 4]) -> [f32; 4] {
        [x0 * self.s + self.dx, y0 * self.s + self.dy, x1 * self.s + self.dx, y1 * self.s + self.dy]
    }
}

/// Below this the scan isn't taken to be this blank (another page, or upside down).
const LINED_UP: f32 = 0.2;

/// The page in colour at [`DPI`].
pub fn colour(p: &Page) -> Result<RgbImage, String> {
    let img = p.render()?;
    let k = DPI / p.dpi.max(50) as f32;
    if (k - 1.0).abs() < 0.01 {
        return Ok(img.to_rgb8());
    }
    let (w, h) = (((img.width() as f32) * k).round().max(1.0) as u32, ((img.height() as f32) * k).round().max(1.0) as u32);
    Ok(img.resize_exact(w, h, FilterType::Triangle).to_rgb8())
}

fn grey(img: &RgbImage) -> GrayImage {
    image::DynamicImage::ImageRgb8(img.clone()).to_luma8()
}

fn png(img: &GrayImage) -> Result<Vec<u8>, String> {
    let mut out = vec![];
    image::DynamicImage::ImageLuma8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Datalab's reading of `img`. With `DOCUMENTASHION_READ_CACHE` set to a folder, replies are
/// kept there as `<tag>.json` and reused, so a page is sent (and paid for) once while working on
/// this.
fn convert(img: &GrayImage, key: &str, tag: &str) -> Result<Value, String> {
    let cache = std::env::var_os("DOCUMENTASHION_READ_CACHE").map(|d| std::path::PathBuf::from(d).join(format!("{tag}.json")));
    if let Some(c) = cache.as_ref().filter(|c| c.is_file()) {
        return serde_json::from_slice(&std::fs::read(c).map_err(|e| e.to_string())?).map_err(|e| e.to_string());
    }
    let v = student::convert(&png(img)?, key)?;
    if let Some(c) = cache {
        let _ = std::fs::write(c, v.to_string());
    }
    Ok(v)
}

fn is_figure(kind: &str) -> bool {
    ["Figure", "Picture", "Image", "Chart", "Diagram"].iter().any(|k| kind.contains(k))
}

/// Row `i` of `n` in a block: Datalab boxes blocks, not lines.
fn row([x0, y0, x1, y1]: [f32; 4], i: usize, n: usize) -> [f32; 4] {
    let h = (y1 - y0) / n.max(1) as f32;
    [x0, y0 + h * i as f32, x1, y0 + h * (i + 1) as f32]
}

/// A line or table of a reading as text, math in `$…$`, a table's cells split by `|`.
fn item_text(it: &Item) -> String {
    fn runs(rs: &[Run]) -> String {
        let mut s = String::new();
        for r in rs {
            match r {
                Run::Text(t) if t.strike => s.push_str(&format!("~~{}~~", t.text)),
                Run::Text(t) if t.sup => s.push_str(&format!("^{}", t.text)),
                Run::Text(t) => s.push_str(&t.text),
                Run::Math { math, .. } => s.push_str(&format!("${math}$")),
                Run::Br { .. } => s.push('\n'),
                Run::Tab { .. } => s.push(' '),
            }
        }
        s.trim().to_string()
    }
    match it {
        Item::Line(rs) => runs(rs),
        Item::Table(rows) => rows.iter().map(|r| r.iter().map(|c| runs(c)).collect::<Vec<_>>().join(" | ")).collect::<Vec<_>>().join("\n"),
    }
}

fn item_skel(it: &Item) -> String {
    match it {
        Item::Line(rs) => student::skeleton_runs(rs),
        Item::Table(rows) => rows.iter().flatten().map(|c| student::skeleton_runs(c)).collect(),
    }
}

/// Reads one page of the blank worksheet.
pub fn read_blank(page: Page, key: &str) -> Result<BlankPage, String> {
    let g = grey(&colour(&page)?);
    let tag = format!("blank-{}", page.image.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    let reply = convert(&g, key, &tag)?;
    let lines = print_lines(&reply, g.width() as f32)?;
    let names = name_lines(&lines, g.height() as f32);
    Ok(BlankPage { page, size: [g.width(), g.height()], lines, names })
}

fn print_lines(reply: &Value, width: f32) -> Result<Vec<PrintLine>, String> {
    let mut lines = vec![];
    for (kind, b, html) in student::page_blocks(reply, width)? {
        let figure = is_figure(&kind);
        let items = student::parse(&html);
        if figure {
            lines.push(PrintLine { text: String::new(), skel: String::new(), bbox: b, figure });
        }
        let n = items.len();
        for (i, it) in items.iter().enumerate() {
            let skel = item_skel(it);
            if skel.is_empty() {
                continue;
            }
            lines.push(PrintLine { text: item_text(it), skel, bbox: row(b, i, n), figure });
        }
    }
    if lines.is_empty() {
        return Err("Datalab found no print on the blank copy.".into());
    }
    Ok(lines)
}

/// The printed lines a name is written on.
fn name_lines(lines: &[PrintLine], height: f32) -> Vec<[f32; 4]> {
    lines
        .iter()
        .filter(|l| !l.figure)
        .filter(|l| {
            let lower = l.text.to_lowercase();
            let named = lower.split(|c: char| !c.is_alphanumeric()).any(|w| w == "name");
            // "Name the property" in an instruction is not where a name goes.
            named && (l.bbox[1] < height * 0.3 || lower.contains("name:") || lower.contains("name_") || lower.contains("name _"))
        })
        .map(|l| l.bbox)
        .collect()
}

/// What is blanked for a Name line found at `line` on the scan: from its start to the paper's
/// right edge, with room above and below for writing that leaves the line. The rest of the line
/// goes too (a date, a class): a name is often written again beside them.
fn name_band(line: [f32; 4], width: f32) -> [f32; 4] {
    let h = (line[3] - line[1]).clamp(14.0, 60.0);
    [line[0] - 20.0, line[1] - 0.9 * h - 12.0, width, line[3] + 0.7 * h + 12.0]
}

/// How alike the blank's Name line must be to the place found for it on the scan.
const NAME_FOUND: f32 = 0.35;

/// Where the blank's Name line `line` is on the scan, looked for near where `fit` puts it, and
/// how alike the two are.
fn find_line(blank: &GrayImage, scan: &GrayImage, fit: Fit, line: [f32; 4]) -> ([f32; 4], f32) {
    const F: u32 = 2;
    const REACH: f32 = 40.0;
    let crop = |img: &GrayImage, [x0, y0, x1, y1]: [f32; 4]| {
        let (x0, y0) = (x0.max(0.0) as u32, y0.max(0.0) as u32);
        let (x1, y1) = ((x1.max(0.0) as u32).min(img.width()), (y1.max(0.0) as u32).min(img.height()));
        (image::imageops::crop_imm(img, x0, y0, x1.saturating_sub(x0).max(F), y1.saturating_sub(y0).max(F)).to_image(), x0 as f32, y0 as f32)
    };
    let (patch, px, py) = crop(blank, [line[0] - 4.0, line[1] - 4.0, line[2] + 4.0, line[3] + 4.0]);
    let at = fit.map([px, py, px, py]);
    let (region, rx, ry) = crop(scan, [at[0] - REACH, at[1] - REACH, at[0] + patch.width() as f32 * fit.s + REACH, at[1] + patch.height() as f32 * fit.s + REACH]);
    let r = (REACH / F as f32) as i32;
    let (cx, cy) = (((at[0] - rx) / F as f32).round() as i32, ((at[1] - ry) / F as f32).round() as i32);
    let (dx, dy, score) = present(&cells(&patch, F), &cells(&region, F), fit.s, cx, cy, r);
    let (x0, y0) = (rx + (dx * F as i32) as f32 + 4.0 * fit.s, ry + (dy * F as i32) as f32 + 4.0 * fit.s);
    ([x0, y0, x0 + (line[2] - line[0]) * fit.s, y0 + (line[3] - line[1]) * fit.s], score)
}

// ---- Lining a scan up with the blank ----

/// Darkness of `img` in cells `f` pixels square, 0–1, paper left out.
fn cells(img: &GrayImage, f: u32) -> (Vec<f32>, usize, usize) {
    let (w, h) = ((img.width() / f) as usize, (img.height() / f) as usize);
    let mut out = vec![0.0f32; w * h];
    for y in 0..(h as u32 * f) {
        for x in 0..(w as u32 * f) {
            let v = 255 - img.get_pixel(x, y)[0];
            if v > 60 {
                out[(y / f) as usize * w + (x / f) as usize] += v as f32;
            }
        }
    }
    let k = 1.0 / (f * f * 255) as f32;
    out.iter_mut().for_each(|v| *v *= k);
    (out, w, h)
}

/// The best scale and shift (in cells) of `b` over `p` among `scales` and shifts within `r` of
/// (`cx`, `cy`): cosine similarity of the blank's ink with the scan's.
fn search(b: &(Vec<f32>, usize, usize), p: &(Vec<f32>, usize, usize), scales: &[f32], cx: i32, cy: i32, r: i32) -> (f32, i32, i32, f32) {
    candidates(b, p, scales, cx, cy, r, 1)[0]
}

/// The shift (in cells) within `r` of (`cx`, `cy`) at which most of `b`'s ink is found in `p`,
/// and how much is: ink `p` has and `b` doesn't (the student's writing) doesn't count.
fn present(b: &(Vec<f32>, usize, usize), p: &(Vec<f32>, usize, usize), s: f32, cx: i32, cy: i32, r: i32) -> (i32, i32, f32) {
    let (bv, bw, _) = b;
    let (pv, pw, ph) = p;
    let pts: Vec<(i32, i32, f32)> = bv
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > 0.02)
        .map(|(i, v)| (((i % bw) as f32 * s).round() as i32, ((i / bw) as f32 * s).round() as i32, *v))
        .collect();
    let total = pts.iter().map(|p| p.2).sum::<f32>().max(1e-6);
    let mut best = (cx, cy, 0.0);
    for dy in cy - r..=cy + r {
        for dx in cx - r..=cx + r {
            let mut found = 0.0;
            for &(x, y, v) in &pts {
                let (x, y) = (x + dx, y + dy);
                if x >= 0 && y >= 0 && (x as usize) < *pw && (y as usize) < *ph {
                    found += v.min(pv[y as usize * pw + x as usize]);
                }
            }
            if found / total > best.2 {
                best = (dx, dy, found / total);
            }
        }
    }
    best
}

/// The `k` best scales and shifts of `b` over `p`, best first, each a few cells or a step of
/// scale from the others: the best at a coarse scale is not always the best at a finer one.
fn candidates(b: &(Vec<f32>, usize, usize), p: &(Vec<f32>, usize, usize), scales: &[f32], cx: i32, cy: i32, r: i32, k: usize) -> Vec<(f32, i32, i32, f32)> {
    let (bv, bw, _) = b;
    let (pv, pw, ph) = p;
    let np = pv.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
    let mut all: Vec<(f32, i32, i32, f32)> = vec![];
    for &s in scales {
        let pts: Vec<(i32, i32, f32)> = bv
            .iter()
            .enumerate()
            .filter(|(_, v)| **v > 0.02)
            .map(|(i, v)| (((i % bw) as f32 * s).round() as i32, ((i / bw) as f32 * s).round() as i32, *v))
            .collect();
        let nb = pts.iter().map(|p| p.2 * p.2).sum::<f32>().sqrt().max(1e-6);
        for dy in cy - r..=cy + r {
            for dx in cx - r..=cx + r {
                let mut sum = 0.0;
                for &(x, y, v) in &pts {
                    let (x, y) = (x + dx, y + dy);
                    if x >= 0 && y >= 0 && (x as usize) < *pw && (y as usize) < *ph {
                        sum += v * pv[y as usize * pw + x as usize];
                    }
                }
                all.push((s, dx, dy, sum / (nb * np)));
            }
        }
    }
    all.sort_by(|a, b| b.3.total_cmp(&a.3));
    let mut out: Vec<(f32, i32, i32, f32)> = vec![];
    for c in all {
        if out.len() >= k {
            break;
        }
        if out.iter().all(|o| (o.1 - c.1).abs() > 2 || (o.2 - c.2).abs() > 2 || (o.0 - c.0).abs() > 0.015) {
            out.push(c);
        }
    }
    if out.is_empty() {
        out.push((1.0, cx, cy, 0.0));
    }
    out
}

/// How the blank lies on the scan, both at [`DPI`]: coarse to fine, allowing for a copier's
/// shrinking or enlarging by up to a tenth, and a shift of up to an inch.
pub fn align(blank: &GrayImage, scan: &GrayImage) -> Fit {
    let steps = |lo: f32, hi: f32, step: f32| -> Vec<f32> {
        let n = ((hi - lo) / step).round() as i32;
        (0..=n).map(|i| lo + step * i as f32).collect()
    };
    let (b10, p10) = (cells(blank, 10), cells(scan, 10));
    let (b3, p3) = (cells(blank, 3), cells(scan, 3));
    let (s, dx, dy, _) = candidates(&b10, &p10, &steps(0.92, 1.08, 0.01), 0, 0, (DPI / 10.0) as i32, 6)
        .into_iter()
        .map(|(s, dx, dy, _)| search(&b3, &p3, &steps(s - 0.012, s + 0.012, 0.003), (dx as f32 * 10.0 / 3.0).round() as i32, (dy as f32 * 10.0 / 3.0).round() as i32, 5))
        .max_by(|a, b| a.3.total_cmp(&b.3))
        .unwrap();
    let (b1, p1) = (cells(blank, 1), cells(scan, 1));
    let (s, dx, dy, _) = search(&b1, &p1, &[s - 0.001, s, s + 0.001], dx * 3, dy * 3, 2);
    // How alike they are is judged in cells a twentieth of an inch square, so a pixel's slip or
    // the scanner's blur doesn't count against it.
    let (b8, p8) = (cells(blank, 8), cells(scan, 8));
    let (_, _, _, score) = search(&b8, &p8, &[s], (dx as f32 / 8.0).round() as i32, (dy as f32 / 8.0).round() as i32, 0);
    Fit { s, dx: dx as f32, dy: dy as f32, score }
}

// ---- The student's ink ----

/// Pixels the student (or anyone) inked on the scan that the blank doesn't print, as a mask the
/// size of `scan`. Red ink is left out: it is the teacher's.
fn student_ink(scan: &RgbImage, blank: &GrayImage, fit: Fit) -> Vec<bool> {
    let (bw, bh) = (blank.width() as usize, blank.height() as usize);
    // The blank's print, widened a little for what lining up and scanning blur.
    let mut print: Vec<bool> = blank.pixels().map(|p| p[0] < 170).collect();
    dilate(&mut print, bw, bh, 3);
    let (w, h) = scan.dimensions();
    let mut out = vec![false; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = scan.get_pixel(x, y).0;
            let lum = (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000;
            if lum >= 185 || r as i32 - (g.max(b) as i32) > 60 {
                continue;
            }
            let bx = ((x as f32 - fit.dx) / fit.s).round();
            let by = ((y as f32 - fit.dy) / fit.s).round();
            let printed = bx >= 0.0 && by >= 0.0 && (bx as usize) < bw && (by as usize) < bh && print[by as usize * bw + bx as usize];
            out[(y * w + x) as usize] = !printed;
        }
    }
    out
}

/// Grows every set cell of `m` by `r` in each direction.
fn dilate(m: &mut [bool], w: usize, h: usize, r: usize) {
    let mut tmp = vec![false; m.len()];
    for y in 0..h {
        let row = &m[y * w..(y + 1) * w];
        let mut last: Option<usize> = None;
        // Distance to the nearest set cell on the left and on the right, within r.
        for x in 0..w {
            if row[x] {
                last = Some(x);
            }
            if last.is_some_and(|l| x - l <= r) {
                tmp[y * w + x] = true;
            }
        }
        last = None;
        for x in (0..w).rev() {
            if row[x] {
                last = Some(x);
            }
            if last.is_some_and(|l| l - x <= r) {
                tmp[y * w + x] = true;
            }
        }
    }
    m.fill(false);
    for x in 0..w {
        let mut last: Option<usize> = None;
        for y in 0..h {
            if tmp[y * w + x] {
                last = Some(y);
            }
            if last.is_some_and(|l| y - l <= r) {
                m[y * w + x] = true;
            }
        }
        last = None;
        for y in (0..h).rev() {
            if tmp[y * w + x] {
                last = Some(y);
            }
            if last.is_some_and(|l| l - y <= r) {
                m[y * w + x] = true;
            }
        }
    }
}

/// The student's ink inside `area`: how many pixels, and the box around them.
fn ink_in(ink: &[bool], w: u32, h: u32, [x0, y0, x1, y1]: [f32; 4]) -> (usize, [f32; 4]) {
    let (xa, ya) = (x0.max(0.0) as u32, y0.max(0.0) as u32);
    let (xb, yb) = ((x1.max(0.0) as u32).min(w), (y1.max(0.0) as u32).min(h));
    let mut n = 0;
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for y in ya..yb {
        for x in xa..xb {
            if ink[(y * w + x) as usize] {
                n += 1;
                b = [b[0].min(x as f32), b[1].min(y as f32), b[2].max(x as f32 + 1.0), b[3].max(y as f32 + 1.0)];
            }
        }
    }
    (n, b)
}

/// Patches of ink that no box covers: writing nothing read accounts for.
fn unread(ink: &[bool], w: u32, h: u32, covered: &[[f32; 4]]) -> Vec<[f32; 4]> {
    const C: u32 = 10;
    let (cw, ch) = ((w / C) as usize, (h / C) as usize);
    let mut count = vec![0u32; cw * ch];
    for y in 0..(ch as u32 * C) {
        for x in 0..(cw as u32 * C) {
            if ink[(y * w + x) as usize] && !covered.iter().any(|b| (x as f32) >= b[0] && (x as f32) < b[2] && (y as f32) >= b[1] && (y as f32) < b[3]) {
                count[(y / C) as usize * cw + (x / C) as usize] += 1;
            }
        }
    }
    // Cells with real ink, joined across a one-cell gap (the strokes of one word).
    let set: Vec<bool> = count.iter().map(|&n| n >= 6).collect();
    let mut seen = vec![false; set.len()];
    let mut out = vec![];
    for start in 0..set.len() {
        if !set[start] || seen[start] {
            continue;
        }
        let (mut stack, mut cells, mut total) = (vec![start], 0, 0);
        let mut b = [usize::MAX, usize::MAX, 0, 0];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % cw, i / cw);
            cells += 1;
            total += count[i];
            b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            for ny in y.saturating_sub(2)..(y + 3).min(ch) {
                for nx in x.saturating_sub(2)..(x + 3).min(cw) {
                    let j = ny * cw + nx;
                    if set[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        // A scanner's streak or a fold's shadow is long and thin, and runs to the paper's edge.
        let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
        let streak = (bw <= 6 && bh >= 8 * bw) || (bh <= 6 && bw >= 8 * bh);
        let edge = b[0] <= 1 || b[1] <= 1 || b[2] + 1 >= cw || b[3] + 1 >= ch;
        if cells >= 5 && total >= 150 && !streak && !edge {
            out.push([(b[0] as u32 * C) as f32, (b[1] as u32 * C) as f32, (b[2] as u32 * C) as f32, (b[3] as u32 * C) as f32]);
        }
    }
    out
}

// ---- Reading a student's page ----

/// A student's page read against the blank.
pub struct Reading {
    pub rebuild: Rebuild,
    /// The page as sent, name blanked, at [`DPI`].
    pub sent: GrayImage,
    /// Which page of the blank it is, and how it lies on the scan.
    pub blank_page: usize,
    pub fit: Fit,
}

/// The page as it will be sent: lined up with the page of the blank it is, at [`DPI`], with
/// the Name line blanked. Nothing is sent when that can't be done safely.
pub fn outgoing(p: &Page, blank: &Blank) -> Result<(RgbImage, GrayImage, usize, Fit), String> {
    let scan = colour(p)?;
    let g = grey(&scan);
    let mut best: Option<(usize, Fit)> = None;
    for (i, bp) in blank.pages.iter().enumerate() {
        let fit = align(&grey(&colour(&bp.page)?), &g);
        if best.is_none_or(|(_, b)| fit.score > b.score) {
            best = Some((i, fit));
        }
    }
    let (i, fit) = best.ok_or("There's no blank copy to read the page against.")?;
    if std::env::var_os("DOCUMENTASHION_FIT_DEBUG").is_some() {
        eprintln!("blank page {} · scale {:.3} · shift {:.0}, {:.0} · {:.0}% alike", i + 1, fit.s, fit.dx, fit.dy, fit.score * 100.0);
    }
    if fit.score < LINED_UP {
        return Err(format!("This page doesn't line up with the blank copy ({:.0}% alike), so it wasn't sent. Is it a page of this worksheet, the right way up?", fit.score * 100.0));
    }
    if blank.pages.iter().all(|bp| bp.names.is_empty()) {
        return Err("The blank copy has no Name line, so there's no telling where the student's name is; the page wasn't sent.".into());
    }
    let bp = &blank.pages[i];
    let bg = grey(&colour(&bp.page)?);
    let (w, h) = (g.width(), g.height());
    let mut bands = vec![];
    for line in &bp.names {
        let (found, score) = find_line(&bg, &g, fit, *line);
        if std::env::var_os("DOCUMENTASHION_FIT_DEBUG").is_some() {
            eprintln!("name line at {:?} · {:.0}% alike", found.map(|v| v.round()), score * 100.0);
        }
        if score < NAME_FOUND {
            return Err("Couldn't find the Name line on this page, so the page wasn't sent.".into());
        }
        // Both where the blank puts it and where it was found, so a slip either way still covers it.
        let put = fit.map(*line);
        bands.push(name_band([found[0].min(put[0]), found[1].min(put[1]), found[2].max(put[2]), found[3].max(put[3])], w as f32));
    }
    // Anything the student wrote above the Name line goes too (a name written again, a doodle),
    // or on a page with no Name line (the back), above the first line of print.
    let top = bands.iter().map(|b| b[3]).fold(f32::MIN, f32::max);
    let top = if top > f32::MIN { top } else { bp.lines.iter().map(|l| fit.map(l.bbox)[1]).fold(f32::MAX, f32::min) - 6.0 };
    let mut ink = student_ink(&scan, &bg, fit);
    let rows = (top.clamp(0.0, h as f32) as usize).min(h as usize);
    ink[rows * w as usize..].fill(false);
    dilate(&mut ink, w as usize, h as usize, 3);
    let mut sent = g;
    let mut scan = scan;
    let white = |x: u32, y: u32, sent: &mut GrayImage, scan: &mut RgbImage| {
        sent.put_pixel(x, y, image::Luma([255]));
        scan.put_pixel(x, y, image::Rgb([255, 255, 255]));
    };
    for y in 0..h {
        for x in 0..w {
            if ink[(y * w + x) as usize] || bands.iter().any(|b| x as f32 >= b[0] && x as f32 <= b[2] && y as f32 >= b[1] && y as f32 <= b[3]) {
                white(x, y, &mut sent, &mut scan);
            }
        }
    }
    Ok((scan, sent, i, fit))
}

/// Reads the student's writing on `p` against the blank.
pub fn read_page(p: &Page, blank: &Blank, key: &str) -> Result<Reading, String> {
    let (scan, sent, bi, fit) = outgoing(p, blank)?;
    let tag = p.image.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let reply = convert(&sent, key, &tag)?;
    let found = student::page_blocks(&reply, sent.width() as f32)?;
    let bp = &blank.pages[bi];
    let ink = student_ink(&scan, &grey(&colour(&bp.page)?), fit);
    let rebuild = assemble(p, bp, fit, &found, &ink, sent.width(), sent.height());
    Ok(Reading { rebuild, sent, blank_page: bi, fit })
}

/// Whether reading `sk` (a skeleton) is one of the printed lines `near` it, or two of them side
/// by side read as one.
fn printed(sk: &str, near: &[&PrintLine]) -> bool {
    if sk.is_empty() {
        return true;
    }
    let bare = student::unlabelled(sk);
    let like = |p: &str| student::same(sk, p) || bare.is_some_and(|b| student::unlabelled(p).is_none() && student::same(b, p));
    if near.iter().any(|l| like(&l.skel)) {
        return true;
    }
    near.iter().any(|a| {
        near.iter().any(|b| {
            let (ca, cb) = ((a.bbox[1] + a.bbox[3]) / 2.0, (b.bbox[1] + b.bbox[3]) / 2.0);
            a.bbox[0] < b.bbox[0] && (ca - cb).abs() < 12.0 && like(&format!("{}{}", a.skel, b.skel))
        })
    })
}

/// The copy of the page: the blank's print where it lies on the scan, and the student's
/// writing as Datalab read it, each piece boxed by its ink.
fn assemble(p: &Page, bp: &BlankPage, fit: Fit, found: &[(String, [f32; 4], String)], ink: &[bool], w: u32, h: u32) -> Rebuild {
    let (wf, hf) = (w as f32, h as f32);
    let thou = |[x0, y0, x1, y1]: [f32; 4]| -> [f32; 4] {
        let (x0, y0) = ((x0 / wf * 1000.0).clamp(0.0, 999.0), (y0 / hf * 1000.0).clamp(0.0, 999.0));
        [x0, y0, ((x1 / wf * 1000.0) - x0).clamp(1.0, 1000.0 - x0), ((y1 / hf * 1000.0) - y0).clamp(1.0, 1000.0 - y0)]
    };
    let mapped: Vec<PrintLine> = bp.lines.iter().map(|l| PrintLine { bbox: fit.map(l.bbox), ..l.clone() }).collect();
    let mut blocks: Vec<Block> = mapped
        .iter()
        .filter(|l| !l.figure && !l.text.is_empty())
        .map(|l| Block { bbox: thou(l.bbox), handwritten: false, text: l.text.clone(), line: (l.bbox[3] - l.bbox[1]) / hf * 1000.0, conf: 1.0 })
        .collect();
    let mut covered: Vec<[f32; 4]> = vec![];
    let work = |lines: &mut Vec<(String, [f32; 4])>, figure: bool, blocks: &mut Vec<Block>, covered: &mut Vec<[f32; 4]>| {
        if lines.is_empty() {
            return;
        }
        let area = lines.iter().map(|l| l.1).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]).unwrap();
        let rows = lines.len() as f32;
        let text = lines.drain(..).map(|l| l.0).collect::<Vec<_>>().join("\n");
        let (n, tight) = ink_in(ink, w, h, [area[0] - 8.0, area[1] - 8.0, area[2] + 8.0, area[3] + 8.0]);
        // Writing read where the scan has none of the student's ink is most likely print the
        // blank doesn't have, or a misreading: shown, but flagged.
        let (bbox, conf) = if n >= 40 { (tight, 1.0) } else { (area, 0.4) };
        covered.push(area);
        covered.push(bbox);
        let line = if figure { 0.0 } else { (area[3] - area[1]) / rows / hf * 1000.0 };
        blocks.push(Block { bbox: thou(bbox), handwritten: true, text, line, conf });
    };
    for (kind, b, html) in found {
        let near: Vec<&PrintLine> = mapped
            .iter()
            .filter(|l| {
                let (cx, cy) = ((l.bbox[0] + l.bbox[2]) / 2.0, (l.bbox[1] + l.bbox[3]) / 2.0);
                cx > b[0] - 25.0 && cx < b[2] + 25.0 && cy > b[1] - 25.0 && cy < b[3] + 25.0
            })
            .collect();
        let items = student::parse(html);
        if is_figure(kind) {
            let area = ((b[2] - b[0]) * (b[3] - b[1])).max(1.0);
            let graphs: Vec<[f32; 4]> = mapped
                .iter()
                .filter(|l| l.figure && l.text.is_empty())
                .filter(|l| {
                    let x = (b[2].min(l.bbox[2]) - b[0].max(l.bbox[0])).max(0.0);
                    let y = (b[3].min(l.bbox[3]) - b[1].max(l.bbox[1])).max(0.0);
                    x * y / area >= 0.3
                })
                .map(|l| l.bbox)
                .collect();
            let over = !graphs.is_empty();
            // What the student drew on a printed graph is in the graph's reading, all of it.
            covered.extend(graphs);
            if !over {
                // Drawn by the student, axes and all: the reading is too often made up.
                if ink_in(ink, w, h, *b).0 >= 200 {
                    blocks.push(Block { bbox: thou(*b), handwritten: true, text: "[unclear] The student's drawing: compare with the scan.".into(), line: 0.0, conf: 0.3 });
                    covered.push(*b);
                }
                continue;
            }
        }
        let n = items.len();
        let mut lines: Vec<(String, [f32; 4])> = vec![];
        for (i, it) in items.iter().enumerate() {
            let sk = item_skel(it);
            let text = item_text(it);
            let caption = is_figure(kind) && ["plot:", "chart:", "graph:"].iter().any(|c| text.contains(c));
            if printed(&sk, &near) || caption {
                work(&mut lines, is_figure(kind), &mut blocks, &mut covered);
                continue;
            }
            lines.push((text, row(*b, i, n)));
        }
        work(&mut lines, is_figure(kind), &mut blocks, &mut covered);
    }
    // Ink on a printed line is the print as scanned, or working written over it (the number each
    // term is multiplied by), read with the line.
    covered.extend(mapped.iter().filter(|l| !l.figure).map(|l| [l.bbox[0] - 4.0, l.bbox[1] - 6.0, l.bbox[2] + 4.0, l.bbox[3] + 6.0]));
    for b in unread(ink, w, h, &covered.iter().map(|b| [b[0] - 10.0, b[1] - 10.0, b[2] + 10.0, b[3] + 10.0]).collect::<Vec<_>>()) {
        blocks.push(Block { bbox: thou(b), handwritten: true, text: "[unclear] Writing here wasn't read: compare with the scan.".into(), line: 0.0, conf: 0.0 });
    }
    blocks.sort_by(|a, b| (a.bbox[1] + a.bbox[3] / 2.0).total_cmp(&(b.bbox[1] + b.bbox[3] / 2.0)).then(a.bbox[0].total_cmp(&b.bbox[0])));
    Rebuild { blocks, geometry: p.geometry(), model: "Datalab".into(), anchored: true, grade: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    /// A page of dark bars, like lines of print, at (x, y) offsets.
    fn page(w: u32, h: u32, bars: &[(u32, u32, u32)]) -> GrayImage {
        let mut g = GrayImage::from_pixel(w, h, Luma([245]));
        for &(x, y, len) in bars {
            for yy in y..y + 6 {
                for xx in x..x + len {
                    g.put_pixel(xx, yy, Luma([20]));
                }
            }
        }
        g
    }

    #[test]
    fn a_shifted_scan_lines_up_with_its_blank() {
        let bars: Vec<(u32, u32, u32)> = (0..14).map(|i| (60 + (i % 3) * 37, 50 + i * 41, 120 + (i * 53) % 170)).collect();
        let blank = page(500, 650, &bars);
        let shifted: Vec<(u32, u32, u32)> = bars.iter().map(|&(x, y, l)| (x + 23, y + 11, l)).collect();
        let mut scan = page(500, 650, &shifted);
        // The student's writing doesn't throw it.
        for y in 300..330 {
            for x in 300..420 {
                scan.put_pixel(x, y, Luma([90]));
            }
        }
        let fit = align(&blank, &scan);
        assert!((fit.dx - 23.0).abs() <= 1.0 && (fit.dy - 11.0).abs() <= 1.0, "{fit:?}");
        assert!((fit.s - 1.0).abs() < 0.01 && fit.score > 0.5, "{fit:?}");
    }

    #[test]
    fn the_name_line_is_found_and_runs_to_the_edge() {
        let l = |t: &str, y: f32| PrintLine { text: t.into(), skel: student::skeleton(t), bbox: [100.0, y, 600.0, y + 20.0], figure: false };
        let lines = name_lines(&[l("Building Up", 60.0), l("Name: ______ Date: ____", 150.0), l("12. Name the property shown.", 900.0)], 1650.0);
        assert_eq!(lines, vec![[100.0, 150.0, 600.0, 170.0]]);
        let band = name_band(lines[0], 1275.0);
        assert_eq!(band[2], 1275.0);
        assert!(band[0] < 100.0 && band[1] < 150.0 && band[3] > 170.0);
    }

    #[test]
    fn print_in_its_place_goes_and_the_students_writing_stays() {
        let l = |t: &str, bbox: [f32; 4]| PrintLine { text: t.into(), skel: student::skeleton(t), bbox, figure: false };
        let bp = BlankPage {
            page: Page::default(),
            size: [400, 400],
            lines: vec![l("(1) $-9 + 15$", [20.0, 20.0, 150.0, 40.0]), l("(2) $-6-(-11)$", [220.0, 20.0, 360.0, 40.0]), l("Answers: 6", [20.0, 360.0, 200.0, 380.0])],
            names: vec![],
        };
        let fit = Fit { s: 1.0, dx: 0.0, dy: 0.0, score: 1.0 };
        let mut ink = vec![false; 400 * 400];
        for y in 50..70 {
            for x in 30..60 {
                ink[y * 400 + x] = true;
            }
        }
        let found = vec![
            ("Text".to_string(), [20.0, 20.0, 150.0, 80.0], "<p>(1) <math>-9+15</math><br/>6</p>".to_string()),
            ("Text".to_string(), [20.0, 18.0, 360.0, 42.0], "<p>(1) <math>-9 + 15</math> (2) <math>-6 - (-11)</math></p>".to_string()),
        ];
        let r = assemble(&Page::default(), &bp, fit, &found, &ink, 400, 400);
        let hand: Vec<&Block> = r.blocks.iter().filter(|b| b.handwritten).collect();
        // "6" is the student's even though "6" is printed lower down; both questions read as one
        // line are print.
        assert_eq!(hand.len(), 1, "{hand:?}");
        assert_eq!(hand[0].text, "6");
        assert_eq!(hand[0].conf, 1.0);
        // Boxed by its ink, not by the block.
        assert!((hand[0].bbox[0] - 75.0).abs() < 1.0 && (hand[0].bbox[1] - 125.0).abs() < 1.0, "{:?}", hand[0].bbox);
        assert_eq!(r.blocks.iter().filter(|b| !b.handwritten).count(), 3);
    }

    #[test]
    fn ink_nothing_read_is_flagged() {
        let (w, h) = (300u32, 300u32);
        let mut ink = vec![false; (w * h) as usize];
        for y in 200..240 {
            for x in 40..140 {
                if (x + y) % 3 != 0 {
                    ink[(y * w + x) as usize] = true;
                }
            }
        }
        // A few stray specks are not writing, nor is a scanner's streak down the page.
        ink[(20 * w + 20) as usize] = true;
        for y in 30..290 {
            ink[(y * w + 250) as usize] = true;
            ink[(y * w + 251) as usize] = true;
        }
        assert_eq!(unread(&ink, w, h, &[]).len(), 1);
        assert!(unread(&ink, w, h, &[[30.0, 190.0, 150.0, 250.0]]).is_empty());
    }
}
