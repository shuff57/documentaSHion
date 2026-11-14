//! Reads each printed graph again on its own. A whole page is too small for a model to count a
//! grid's lines or read its axis numbers; cropped from the scan at full size, one graph is
//! not. The model decides the graph's bounds, steps and markings, and verSHion draws it anew.
use super::{fit, *};
use base64::Engine;
use image::DynamicImage;
use std::path::Path;

/// Where a figure is in the sheet (see [`super::Path`]).
type At = super::Path;

const PROMPT: &str = r#"This is one graph cropped from a scanned, printed school worksheet. Rebuild the PRINTED graph as a verSHion "{ID}" figure.
Read it carefully:
- Count the grid squares across and down, and read the numbers printed on the axes, to find where each axis starts and ends and what it counts by. An axis whose numbers stop short of the grid's edge still runs to the edge: count the squares.
- "square": true makes one unit on x as long as one unit on y. When the axes count by different steps (say x by 1 and y by 5) but the printed squares are square, set "square": false.
- Note whether the axes are numbered (and on every line or every 2nd, 5th, 10th), whether they have arrowheads, the axis letters, a title, and how heavy the grid lines are.
- Include lines, curves and points only if they were PRINTED. Leave out everything drawn or written by hand (students' lines, points, labels, answers): a blank printed grid has no equations and no points.
Its current settings (some may be wrong): {CURRENT}
The parameters you can set (name (kind; default): meaning):
{PARAMS}
Always set "style":"Print" and "show_answers":false, and give "equations" and "points" (empty when nothing is printed).
Return ONLY a JSON object of the parameters to set, e.g. {"xmin":-6,"xmax":6,"xstep":1,"ymin":-6,"ymax":6,"ystep":1,"equations":"","points":""}."#;

/// Each figure's box on the scan, points, and whether it was found there: from one fitting
/// measurement. A figure that can't be found is taken to be where it was drawn, moved as far
/// as the nearest thing found above it.
fn boxes(page: &image::GrayImage, sheet: &Sheet) -> Result<Vec<(At, [f32; 4], bool)>, String> {
    let m = fit::measure(page, sheet)?;
    let mut out = vec![];
    let mut near = (0.0, 0.0);
    for (i, p) in m.anchors.iter().enumerate() {
        let figure = matches!(block_at(&sheet.blocks, p), Some(Block::Figure(_)));
        match (m.found[i], m.drawn[i]) {
            (Some(o), Some(strip)) => {
                near = (o.dx, o.dy);
                if figure {
                    let [x, y, w, h] = strip.map(|v| v as f32 / fit::K);
                    let s = m.scales[i].unwrap_or(1.0);
                    out.push((p.clone(), [x + o.dx, y + o.dy, w * s, h * s], true));
                }
            }
            (None, Some(strip)) if figure => {
                let [x, y, w, h] = strip.map(|v| v as f32 / fit::K);
                out.push((p.clone(), [x + near.0, y + near.1, w, h], false));
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The part of the scan at `bx` (points on the page), with a margin, as the scanner saw it.
fn crop(scan: &Path, bx: [f32; 4], page: &PageSetup, sure: bool) -> Result<DynamicImage, String> {
    let img = image::open(scan).map_err(|e| format!("{}: {e}", scan.display()))?;
    let k = fit::scan_dpi(img.width(), page.width) / 72.0;
    // The scan is centred on the page (see `fit::prepare_scan`).
    let (ox, oy) = ((page.width * k - img.width() as f32) / 2.0, (page.height * k - img.height() as f32) / 2.0);
    // Room around it, more when it's only where it was drawn.
    let pad = bx[2].max(bx[3]) * if sure { 0.12 } else { 0.3 };
    let x0 = ((bx[0] - pad) * k - ox).max(0.0) as u32;
    let y0 = ((bx[1] - pad) * k - oy).max(0.0) as u32;
    let x1 = (((bx[0] + bx[2] + pad) * k - ox) as u32).min(img.width());
    let y1 = (((bx[1] + bx[3] + pad) * k - oy) as u32).min(img.height());
    if x1 <= x0 + 8 || y1 <= y0 + 8 {
        return Err("The figure is off the scan.".into());
    }
    let c = img.crop_imm(x0, y0, x1 - x0, y1 - y0);
    // Small print needs pixels: at least 1100 across.
    Ok(if c.width() < 1100 { c.resize(1100, 1100 * c.height() / c.width().max(1), image::imageops::FilterType::Lanczos3) } else { c })
}

/// The verSHion parameters the model reads off one graph's crop.
fn read(crop: &DynamicImage, f: &Figure, ask: Ask) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let id = f.vershion.as_deref().unwrap_or("coordinate-grid");
    let params = super::extract::describe(&[id]);
    let prompt = PROMPT
        .replace("{ID}", id)
        .replace("{CURRENT}", &serde_json::Value::Object(f.params.clone()).to_string())
        .replace("{PARAMS}", &params);
    let mut png = std::io::Cursor::new(Vec::new());
    crop.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let reply = ask(&prompt, &b64)?;
    let (a, b) = (reply.find('{').ok_or("The model's reply held no settings.")?, reply.rfind('}').ok_or("The model's reply held no settings.")?);
    let v: serde_json::Value = serde_json::from_str(&reply[a..=b]).map_err(|e| format!("The model's graph settings didn't parse: {e}"))?;
    v.as_object().cloned().ok_or_else(|| "The model's graph settings weren't an object.".into())
}

/// Reads every verSHion figure of `sheet` again from its crop of `scan` (the upright scan the
/// sheet is fitted to, sheared by `shear`), several at once. Returns how many were read; a
/// figure that fails keeps its settings, and the failures are reported.
pub fn reread(scan: &Path, sheet: &mut Sheet, shear: f32, ask: Ask) -> Result<usize, String> {
    let (w, h) = ((sheet.page.width * fit::K).round() as u32, (sheet.page.height * fit::K).round() as u32);
    let page = fit::prepare_scan(scan, w, h, shear)?;
    let found = boxes(&page, sheet)?;
    let jobs: Vec<(At, Figure, DynamicImage)> = found
        .into_iter()
        .filter_map(|(p, bx, sure)| {
            let Some(Block::Figure(f)) = block_at(&sheet.blocks, &p) else { return None };
            if f.vershion.is_none() {
                return None;
            }
            match crop(scan, bx, &sheet.page, sure) {
                Ok(c) => Some((p, f.clone(), c)),
                Err(e) => {
                    eprintln!("graph {p:?}: {e}");
                    None
                }
            }
        })
        .collect();
    if let Some(dir) = std::env::var_os("DOCUMENTASHION_GRAPH_CROPS") {
        for (p, _, c) in &jobs {
            let _ = c.save(Path::new(&dir).join(format!("graph-{}.png", p.iter().map(|n| n.to_string()).collect::<Vec<_>>().join("-"))));
        }
    }
    let results: Vec<(At, Result<serde_json::Map<String, serde_json::Value>, String>)> = std::thread::scope(|s| {
        let handles: Vec<_> = jobs.iter().map(|(p, f, c)| (p.clone(), s.spawn(move || read(c, f, ask)))).collect();
        handles.into_iter().map(|(p, h)| (p, h.join().unwrap_or_else(|_| Err("crashed".into())))).collect()
    });
    let mut n = 0;
    for (p, r) in results {
        match r {
            Ok(params) => {
                if let Some(Block::Figure(f)) = block_at_mut(&mut sheet.blocks, &p) {
                    for (k, v) in params {
                        f.params.insert(k, v);
                    }
                    n += 1;
                }
            }
            Err(e) => eprintln!("graph {p:?}: {e}"),
        }
    }
    Ok(n)
}

const CHECK: &str = r#"Left: one printed graph cropped from a scanned school worksheet. Right: our rebuild of it drawn over the same crop: the scan's ink red, the rebuild's blue, dark where the two agree.
The rebuild is a verSHion "{ID}" figure drawn with these settings: {CURRENT}
Where blue lines don't sit on the printed red ones, a setting is wrong. Count the printed grid squares across and down again, read the axis numbers again, and check each axis's range and step, which lines are numbered, where the axes cross, the arrowheads, and whether one unit is as long on x as on y ("square"). Red marks that aren't part of the printed graph (a student's lines, points and writing) don't count.
The parameters you can set (name (kind; default): meaning):
{PARAMS}
Return ONLY a JSON object of the parameters to change, e.g. {"xmax":8}, or {} if the blue already lies on the printed graph."#;

/// A graph whose drawing lands on its print this well (the first of [`overlap`]) is left as it
/// is. A printed grid is often dashed, so even a right one misses in the gaps (about a fifth);
/// a wrong range or step puts about half its lines between the printed ones.
const GOOD: f32 = 0.7;

/// A figure as drawn and as printed, cropped to the same box of the scan (150 dpi): the drawing
/// moved and scaled the way the fit found it on the scan.
struct View {
    ours: image::GrayImage,
    theirs: image::GrayImage,
}

/// Each verSHion figure the fit finds on `page` (the prepared scan), as drawn and as printed.
fn views(page: &image::GrayImage, sheet: &Sheet) -> Result<Vec<(At, View)>, String> {
    let m = fit::measure(page, sheet)?;
    let mut out = vec![];
    // A figure drawn too wrong to be found is looked at where it was drawn, moved as far as the
    // nearest thing found above it (as in `boxes`).
    let mut near = (0.0, 0.0);
    for (i, p) in m.anchors.iter().enumerate() {
        if let Some(o) = m.found[i] {
            near = (o.dx, o.dy);
        }
        let Some(Block::Figure(f)) = block_at(&sheet.blocks, p) else { continue };
        let Some([x, y, w, h]) = m.drawn[i] else { continue };
        if f.vershion.is_none() {
            continue;
        }
        let (o, s) = match m.found[i] {
            Some(o) => ((o.dx, o.dy), m.scales[i].unwrap_or(1.0)),
            None => (near, 1.0),
        };
        // A margin around it: a graph drawn too big or too small reaches past its box.
        let pad = (w.max(h) as f32 * 0.08) as i64;
        let (rx, ry) = (x as i64 - pad, y as i64 - pad);
        let (rw, rh) = (w as i64 + 2 * pad, h as i64 + 2 * pad);
        let (tx, ty) = (x as i64 + (o.0 * fit::K) as i64 - (pad as f32 * s) as i64, y as i64 + (o.1 * fit::K) as i64 - (pad as f32 * s) as i64);
        let (tw, th) = ((rw as f32 * s) as i64, (rh as f32 * s) as i64);
        let crop = |g: &image::GrayImage, x: i64, y: i64, w: i64, h: i64| {
            image::GrayImage::from_fn(w.max(1) as u32, h.max(1) as u32, |a, b| {
                let (u, v) = (x + a as i64, y + b as i64);
                if u >= 0 && v >= 0 && (u as u32) < g.width() && (v as u32) < g.height() { *g.get_pixel(u as u32, v as u32) } else { image::Luma([255]) }
            })
        };
        // Lined up on its own, a little either way: the fit may not have the figure's place
        // and size exactly yet, and the question here is whether the drawing is right.
        let drawn = crop(&m.render, rx, ry, rw, rh);
        let mut best: Option<(f32, View)> = None;
        for k in [0.96, 1.0, 1.04] {
            let (tw, th) = ((tw as f32 * k) as i64, (th as f32 * k) as i64);
            let ours = image::imageops::resize(&drawn, tw.max(1) as u32, th.max(1) as u32, image::imageops::FilterType::Triangle);
            let (cx, cy) = (tx - (tw as f32 * (k - 1.0) / 2.0) as i64, ty - (th as f32 * (k - 1.0) / 2.0) as i64);
            for dy in (-12..=12).step_by(3) {
                for dx in (-12..=12).step_by(3) {
                    let v = View { ours: ours.clone(), theirs: crop(&m.scan, cx + dx, cy + dy, tw, th) };
                    let sc = score_at(&v, 2);
                    if best.as_ref().is_none_or(|b| sc > b.0) {
                        best = Some((sc, v));
                    }
                }
            }
        }
        if let Some((_, v)) = best {
            out.push((p.clone(), v));
        }
    }
    Ok(out)
}

/// How well a drawing lies on its print, inside its own box (the margin [`views`] adds is
/// only for showing): how much of the drawing's ink lands near printed ink (2 pixels either
/// way), and how much of the printed ink has drawing near it. A wrong range or step puts grid
/// lines between the printed ones and costs the first; a student's lines on the graph cost the
/// second the same in every try. The light grid of a drawing counts as ink.
fn overlap(v: &View) -> (f32, f32) {
    overlap_at(v, 1)
}

/// [`overlap`] from every `step`th pixel each way.
fn overlap_at(v: &View, step: usize) -> (f32, f32) {
    let (w, h) = v.ours.dimensions();
    // The box without the margin (8% of the longer side, of a box 16% bigger).
    let pad = (w.max(h) as f32 * 0.08 / 1.16) as u32;
    let near = |g: &image::GrayImage, x: u32, y: u32, t: u8| {
        (y.saturating_sub(2)..(y + 3).min(h)).any(|b| (x.saturating_sub(2)..(x + 3).min(w)).any(|a| g.get_pixel(a, b).0[0] < t))
    };
    let (mut ours, mut ours_hit, mut theirs, mut theirs_hit) = (0u32, 0u32, 0u32, 0u32);
    for y in (pad..h.saturating_sub(pad)).step_by(step) {
        for x in (pad..w.saturating_sub(pad)).step_by(step) {
            if v.ours.get_pixel(x, y).0[0] < 225 {
                ours += 1;
                ours_hit += near(&v.theirs, x, y, 200) as u32;
            }
            if v.theirs.get_pixel(x, y).0[0] < 200 {
                theirs += 1;
                theirs_hit += near(&v.ours, x, y, 225) as u32;
            }
        }
    }
    (ours_hit as f32 / ours.max(1) as f32, theirs_hit as f32 / theirs.max(1) as f32)
}

/// One number for [`overlap`], for comparing tries of the same graph: mostly how much of the
/// drawing lands on print, since how much of the print is drawn also counts the student's.
fn score(v: &View) -> f32 {
    score_at(v, 1)
}

/// [`score`] from every `step`th pixel each way.
fn score_at(v: &View, step: usize) -> f32 {
    let (p, r) = overlap_at(v, step);
    0.7 * p + 0.3 * r
}

/// The print beside the drawing laid over it (the scan red, the drawing blue), at twice the
/// size, for the model.
fn side_by_side(v: &View) -> DynamicImage {
    let (w, h) = v.ours.dimensions();
    let mut img = image::RgbImage::from_pixel(w * 2 + 12, h, image::Rgb([128, 128, 128]));
    for (x, y, p) in v.theirs.enumerate_pixels() {
        let (a, b) = (p.0[0], v.ours.get_pixel(x, y).0[0]);
        img.put_pixel(x, y, image::Rgb([a, a, a]));
        img.put_pixel(x + w + 12, y, image::Rgb([b, a.min(b), a]));
    }
    let (w, h) = img.dimensions();
    DynamicImage::ImageRgb8(img).resize(w * 2, h * 2, image::imageops::FilterType::Lanczos3)
}

/// [`check`] for the sheet fitted to `scan` (sheared by `shear`).
pub fn check_scan(scan: &Path, sheet: &mut Sheet, shear: f32, ask: Ask) -> Result<usize, String> {
    let (w, h) = ((sheet.page.width * fit::K).round() as u32, (sheet.page.height * fit::K).round() as u32);
    check(&fit::prepare_scan(scan, w, h, shear)?, sheet, ask)
}

/// Shows the model each graph that doesn't line up with its print, laid over it, for the
/// settings to change; keeps a change only where the graph then lines up better. Twice at
/// most. Returns how many graphs were improved.
fn check(page: &image::GrayImage, sheet: &mut Sheet, ask: Ask) -> Result<usize, String> {
    let mut improved = 0;
    for round in 0..2 {
        let views = views(page, sheet)?;
        let before: std::collections::HashMap<At, f32> = views.iter().map(|(p, v)| (p.clone(), score(v))).collect();
        let jobs: Vec<(At, Figure, &View)> = views
            .iter()
            .filter(|(_, v)| overlap(v).0 < GOOD)
            .filter_map(|(p, v)| match block_at(&sheet.blocks, p) {
                Some(Block::Figure(f)) => Some((p.clone(), f.clone(), v)),
                _ => None,
            })
            .collect();
        for (p, v) in &views {
            let (dp, pr) = overlap(v);
            eprintln!("graph {p:?}: {:.0}% of the drawing on print, {:.0}% of the print drawn{}", dp * 100.0, pr * 100.0, if dp < GOOD { "; a second look" } else { "" });
        }
        if jobs.is_empty() {
            break;
        }
        if let Some(dir) = std::env::var_os("DOCUMENTASHION_GRAPH_CROPS") {
            for (p, _, v) in &jobs {
                let name = p.iter().map(|n| n.to_string()).collect::<Vec<_>>().join("-");
                let _ = side_by_side(v).save(Path::new(&dir).join(format!("check-{}-{name}.png", round + 1)));
            }
        }
        let answers: Vec<(At, Figure, Result<serde_json::Map<String, serde_json::Value>, String>)> = std::thread::scope(|s| {
            let handles: Vec<_> = jobs.iter().map(|(p, f, v)| (p.clone(), f.clone(), s.spawn(move || look_again(v, f, ask)))).collect();
            handles.into_iter().map(|(p, f, h)| (p, f, h.join().unwrap_or_else(|_| Err("crashed".into())))).collect()
        });
        let mut tried = vec![];
        for (p, old, r) in answers {
            match r {
                Ok(params) if !params.is_empty() => {
                    if let Some(Block::Figure(f)) = block_at_mut(&mut sheet.blocks, &p) {
                        for (k, v) in params {
                            f.params.insert(k, v);
                        }
                        tried.push((p, old));
                    }
                }
                Ok(_) => {}
                Err(e) => eprintln!("graph {p:?}: {e}"),
            }
        }
        if tried.is_empty() {
            break;
        }
        let after: std::collections::HashMap<At, f32> = self::views(page, sheet)?.into_iter().map(|(p, v)| (p, score(&v))).collect();
        let mut better = 0;
        for (p, old) in tried {
            let (b, a) = (before.get(&p).copied().unwrap_or(0.0), after.get(&p).copied().unwrap_or(0.0));
            if a > b + 0.01 {
                eprintln!("graph {p:?}: scored {:.0}% → {:.0}%, kept", b * 100.0, a * 100.0);
                better += 1;
            } else {
                eprintln!("graph {p:?}: scored {:.0}% → {:.0}%, put back", b * 100.0, a * 100.0);
                if let Some(Block::Figure(f)) = block_at_mut(&mut sheet.blocks, &p) {
                    *f = old;
                }
            }
        }
        improved += better;
        if better == 0 {
            break;
        }
    }
    Ok(improved)
}

/// The settings the model would change, shown graph `v` laid over its print.
fn look_again(v: &View, f: &Figure, ask: Ask) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let id = f.vershion.as_deref().unwrap_or("coordinate-grid");
    let prompt = CHECK
        .replace("{ID}", id)
        .replace("{CURRENT}", &serde_json::Value::Object(f.params.clone()).to_string())
        .replace("{PARAMS}", &super::extract::describe(&[id]));
    let mut png = std::io::Cursor::new(Vec::new());
    side_by_side(v).write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let reply = ask(&prompt, &b64)?;
    let (a, b) = (reply.find('{').ok_or("The model's reply held no settings.")?, reply.rfind('}').ok_or("The model's reply held no settings.")?);
    let v: serde_json::Value = serde_json::from_str(&reply[a..=b]).map_err(|e| format!("The model's graph settings didn't parse: {e}"))?;
    v.as_object().cloned().ok_or_else(|| "The model's graph settings weren't an object.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid of lines every `step` pixels, `w` square, with an 8% margin as [`views`] crops.
    fn grid(w: u32, step: u32) -> image::GrayImage {
        let pad = (w as f32 * 0.08 / 1.16) as u32;
        image::GrayImage::from_fn(w, w, |x, y| {
            let on = |v: u32| v >= pad && v < w - pad && (v - pad) % step == 0;
            image::Luma([if on(x) || on(y) { 0 } else { 255 }])
        })
    }

    #[test]
    fn a_grid_with_the_wrong_step_lands_between_the_printed_lines() {
        let print = grid(240, 20);
        let right = View { ours: grid(240, 20), theirs: print.clone() };
        let wrong = View { ours: grid(240, 15), theirs: print };
        assert!(overlap(&right).0 > 0.95, "{:?}", overlap(&right));
        assert!(overlap(&wrong).0 < GOOD, "{:?}", overlap(&wrong));
        assert!(score(&right) > score(&wrong) + 0.2);
    }
}
