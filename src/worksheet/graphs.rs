//! Reads each printed graph again on its own. A whole page is too small for a model to count a
//! grid's lines or read its axis numbers; cropped from the scan at full size, one graph is
//! not. The model decides the graph's bounds, steps and markings, and verSHion draws it anew.
use super::{fit, *};
use base64::Engine;
use image::DynamicImage;
use std::{path::Path, time::Duration};

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
fn read(crop: &DynamicImage, f: &Figure, model: &str) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let id = f.vershion.as_deref().unwrap_or("coordinate-grid");
    let params = super::extract::describe(&[id]);
    let prompt = PROMPT
        .replace("{ID}", id)
        .replace("{CURRENT}", &serde_json::Value::Object(f.params.clone()).to_string())
        .replace("{PARAMS}", &params);
    let mut png = std::io::Cursor::new(Vec::new());
    crop.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let reply = crate::claude::chat(model, &prompt, &b64, Duration::from_secs(600))?;
    let (a, b) = (reply.find('{').ok_or("The model's reply held no settings.")?, reply.rfind('}').ok_or("The model's reply held no settings.")?);
    let v: serde_json::Value = serde_json::from_str(&reply[a..=b]).map_err(|e| format!("The model's graph settings didn't parse: {e}"))?;
    v.as_object().cloned().ok_or_else(|| "The model's graph settings weren't an object.".into())
}

/// Reads every verSHion figure of `sheet` again from its crop of `scan` (the upright scan the
/// sheet is fitted to, sheared by `shear`), several at once. Returns how many were read; a
/// figure that fails keeps its settings, and the failures are reported.
pub fn reread(scan: &Path, sheet: &mut Sheet, shear: f32, model: &str) -> Result<usize, String> {
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
        let handles: Vec<_> = jobs.iter().map(|(p, f, c)| (p.clone(), s.spawn(move || read(c, f, model)))).collect();
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
