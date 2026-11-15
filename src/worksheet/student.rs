//! The student's work on a scanned worksheet, typed out beside the scan so a teacher can compare.
//!
//! The page goes to Datalab's hosted Chandra (handwriting, math as LaTeX, tables, graphs), with
//! the student's name blanked first. Its reading comes back in blocks, each with a box on the
//! page. What the worksheet printed is already known exactly from the rebuild, so any line that
//! reads the same as a printed one is dropped and only what the student added is kept. Each
//! block is then set under the question it sits beside, in a two-column box: the student's work
//! typed out, and the same part of the scan.
use super::*;
use image::{GrayImage, Luma};
use serde_json::Value;
use std::path::Path as FsPath;

/// Datalab's document conversion endpoint.
const CONVERT: &str = "https://www.datalab.to/api/v1/convert";

/// The colour the student's typed work is set in.
const INK: &str = "1F4E9A";

/// The keychain account the Datalab key is kept under.
pub const ACCOUNT: &str = "Datalab";

/// The Datalab key: `DATALAB_API_KEY`, else the one saved in the keychain from the app.
pub fn key() -> Option<String> {
    std::env::var("DATALAB_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .or_else(|| crate::secrets::load(ACCOUNT).ok().flatten())
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

/// One piece of the student's work.
#[derive(Clone, Debug)]
pub struct Piece {
    /// Its box on the page, pixels at 150 dpi.
    pub bbox: [f32; 4],
    /// What it reads as, ready to set in the document.
    pub blocks: Vec<Block>,
    /// The anchor it is set after (see [`super::anchors`]).
    pub at: Option<super::Path>,
}

/// The page as it will be sent, 150 dpi, lined up with the rebuild, with the student's name
/// blanked; and the rebuild measured against it.
pub fn outgoing(scan: &FsPath, sheet: &Sheet, shear: f32) -> Result<(GrayImage, fit::Measure), String> {
    let (w, h) = ((sheet.page.width * fit::K).round() as u32, (sheet.page.height * fit::K).round() as u32);
    let page = fit::prepare_scan(scan, w, h, shear)?;
    let m = fit::measure(&page, sheet)?;
    let mut page = m.scan.clone();
    let names = name_boxes(&m.words, sheet.page.width);
    // A name box that came out narrow means the name's place wasn't found: nothing is sent.
    if names.iter().any(|b| b[2] - b[0] < 60.0 * fit::K) {
        return Err("Couldn't tell where the student's name is written, so the page wasn't sent.".into());
    }
    for b in names {
        blank(&mut page, b);
    }
    Ok((page, m))
}

/// Reads the student's work on `scan`, the page `sheet` was rebuilt and fitted from. Returns
/// the pieces and the page as sent (see [`outgoing`]).
pub fn read(scan: &FsPath, sheet: &Sheet, shear: f32, key: &str) -> Result<(Vec<Piece>, GrayImage), String> {
    let (page, m) = outgoing(scan, sheet, shear)?;
    let reply = match std::env::var_os("DOCUMENTASHION_WORK_CACHE").map(std::path::PathBuf::from) {
        // While working on this, the reply is kept so the page is sent (and paid for) once.
        Some(cache) if cache.is_file() => serde_json::from_slice(&std::fs::read(&cache).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?,
        cache => {
            let mut png = vec![];
            image::DynamicImage::ImageLuma8(page.clone())
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            let v = convert(&png, key)?;
            if let Some(c) = cache {
                let _ = std::fs::write(c, v.to_string());
            }
            v
        }
    };
    let found = page_blocks(&reply, page.width() as f32)?;
    let prints = Prints::of(sheet);
    let mut places = places(sheet, &m);
    let reads: Vec<([f32; 4], Vec<Read>)> = found
        .into_iter()
        .map(|(kind, bbox, html)| {
            let near = |hits: &[usize]| {
                hits.iter().any(|&i| places.iter().any(|p| p.at == prints.anchors[i].0 && p.top < bbox[3] + 10.0 && p.top > bbox[1] - 40.0 && p.left < bbox[2] && p.right > bbox[0]))
            };
            let reads = if ["Figure", "Picture", "Image", "Chart", "Diagram"].iter().any(|k| kind.contains(k)) {
                // A graph printed on the sheet: what the student plotted on it. Anywhere else the
                // student drew the graph, axes and all, and the reading is too often made up.
                if printed_figure(sheet, &m, bbox) {
                    classify(&parse(&html), &prints, true, &near)
                } else {
                    vec![Read::Work(note("The student's drawing: compare with the scan."))]
                }
            } else {
                classify(&parse(&html), &prints, false, &near)
            };
            (bbox, reads)
        })
        .collect();
    // Where Datalab found a printed line at the top of a block, that is how high the line is on
    // the scan, measured or not. (Only how high: one printed line can hold two questions side by
    // side, each its own block.)
    let mut located: Vec<super::Path> = vec![];
    for (bbox, rs) in &reads {
        if let Some(Read::Print(hits)) = rs.first() {
            if let Some(at) = nearest(&prints, &places, hits, bbox[0], bbox[1]) {
                if let Some(p) = places.iter_mut().find(|p| p.at == at && !located.contains(&p.at) && bbox[0] < p.right) {
                    p.top = bbox[1];
                    located.push(at);
                }
            }
        }
    }
    let mut pieces = vec![];
    for (bbox, rs) in reads {
        // The student's lines after a printed line belong to it; before any, to where the block is.
        let n = rs.len().max(1) as f32;
        let mut at: Option<super::Path> = None;
        let mut blocks: Vec<Block> = vec![];
        let flush = |at: &Option<super::Path>, blocks: &mut Vec<Block>, pieces: &mut Vec<Piece>| {
            if !blocks.is_empty() {
                let at = at.clone().or_else(|| place(&places, bbox));
                pieces.push(Piece { at, bbox, blocks: std::mem::take(blocks) });
            }
        };
        for (i, r) in rs.into_iter().enumerate() {
            match r {
                Read::Print(hits) if !hits.is_empty() => {
                    flush(&at, &mut blocks, &mut pieces);
                    let y = bbox[1] + (bbox[3] - bbox[1]) * i as f32 / n;
                    at = nearest(&prints, &places, &hits, bbox[0], y).or(at);
                }
                Read::Print(_) => {}
                Read::Work(b) => blocks.push(b),
            }
        }
        flush(&at, &mut blocks, &mut pieces);
    }
    if std::env::var_os("DOCUMENTASHION_FIT_DEBUG").is_some() {
        for p in &places {
            eprintln!("place {:?} top {:.0} x {:.0}..{:.0}", p.at, p.top, p.left, p.right);
        }
        for p in &pieces {
            let first = p.blocks.first().map(|b| serde_json::to_string(b).unwrap_or_default().chars().take(90).collect::<String>()).unwrap_or_default();
            eprintln!("piece {:?} at {:?}: {first}", p.bbox.map(|v| v.round()), p.at);
        }
    }
    Ok((pieces, page))
}

/// Of the measured lines a printed reading could be, the one nearest (`x`, `y`) on the page (a
/// line printed twice, like "d. Complete the table.", is told apart by where it is).
fn nearest(prints: &Prints, places: &[Place], hits: &[usize], x: f32, y: f32) -> Option<super::Path> {
    hits.iter()
        .map(|&i| &prints.anchors[i].0)
        .min_by(|a, b| {
            let d = |at: &super::Path| places.iter().find(|p| &p.at == at).map(|p| (p.top - y).abs() + (p.left - x).abs()).unwrap_or(f32::MAX);
            d(a).total_cmp(&d(b))
        })
        .cloned()
}

/// Writes the comparison document: the printed worksheet read top to bottom in one column (see
/// [`flatten`]), each question followed by the student's work on it, typed out beside the same
/// part of the scan.
pub fn write(sheet: &Sheet, pieces: &[Piece], page: &GrayImage, out: &FsPath) -> Result<(), String> {
    let crops = super::cli::tmp(&format!("work-crops-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&crops).map_err(|e| format!("{}: {e}", crops.display()))?;
    let width = sheet.page.text_width();
    let mut flat = vec![];
    flatten(&sheet.blocks, &mut vec![], &mut flat);
    let key = |p: &Piece| p.at.as_ref().map(|a| insertion(&sheet.blocks, a));
    let mut blocks = vec![];
    let mut placed = vec![false; pieces.len()];
    let work = |list: Vec<(usize, &Piece)>, blocks: &mut Vec<Block>| -> Result<(), String> {
        let (left, right) = (width * 0.56, width * 0.44);
        let mut rows = vec![Row { height: None, cells: vec![header("Typed from the scan"), header("On the scan")] }];
        for (i, p) in list {
            let png = crops.join(format!("{i:03}.png"));
            crop(page, p.bbox).save(&png).map_err(|e| format!("{}: {e}", png.display()))?;
            let (cw, ch) = ((p.bbox[2] - p.bbox[0]).max(1.0) / fit::K, (p.bbox[3] - p.bbox[1]).max(1.0) / fit::K);
            let k = ((right - 10.0) / cw).min(300.0 / ch).min(1.0);
            let fig = Figure {
                vershion: None,
                params: Default::default(),
                path: Some(png.to_string_lossy().into_owned()),
                width: cw * k,
                height: ch * k,
                align: Align::Left,
                before: 2.0,
                indent: 0.0,
                alt: "The student's work as scanned".into(),
            };
            rows.push(Row { height: None, cells: vec![Cell { blocks: p.blocks.clone(), ..Default::default() }, Cell { blocks: vec![Block::Figure(fig)], ..Default::default() }] });
        }
        blocks.push(Block::Table(Table { widths: vec![left, right], borders: Borders::All, align: Align::Left, rows }));
        blocks.push(Block::P(Para { before: 6.0, runs: vec![], ..Default::default() }));
        Ok(())
    };
    for (path, b) in flat {
        blocks.push(b);
        let list: Vec<(usize, &Piece)> = pieces.iter().enumerate().filter(|(i, p)| !placed[*i] && key(p).as_deref() == Some(&path[..])).collect();
        if list.is_empty() {
            continue;
        }
        for (i, _) in &list {
            placed[*i] = true;
        }
        work(list, &mut blocks)?;
    }
    let rest: Vec<(usize, &Piece)> = pieces.iter().enumerate().filter(|(i, _)| !placed[*i]).collect();
    if !rest.is_empty() {
        blocks.push(note("Elsewhere on the page"));
        work(rest, &mut blocks)?;
    }
    let page_setup = PageSetup { background: None, ..sheet.page.clone() };
    let doc = Sheet { page: page_setup, font: sheet.font.clone(), size: sheet.size, blocks, header: sheet.header.clone(), footer: sheet.footer.clone() };
    let r = docx::write(&doc, out, &super::cli::figure_png);
    // The crops are inside the document now.
    let _ = std::fs::remove_dir_all(&crops);
    r
}

/// The printed worksheet as one column, top to bottom, with the path each piece came from:
/// paragraphs and tables of values (every cell boxed) whole, layout grids read row by row and
/// cell by cell, a printed graph as a note. Room for working out and page breaks go.
fn flatten(blocks: &[Block], prefix: &mut Vec<usize>, out: &mut Vec<(super::Path, Block)>) {
    for (i, b) in blocks.iter().enumerate() {
        prefix.push(i);
        match b {
            Block::P(p) if !p.runs.is_empty() => {
                let p = Para { before: p.before.clamp(2.0, 10.0), after: 0.0, ..p.clone() };
                out.push((prefix.clone(), Block::P(p)));
            }
            Block::Figure(f) => out.push((prefix.clone(), note(&format!("Graph{}", if f.alt.is_empty() { String::new() } else { format!(": {}", f.alt) })))),
            Block::Table(t) if t.borders != Borders::All => {
                for (r, row) in t.rows.iter().enumerate() {
                    for (c, cell) in row.cells.iter().enumerate() {
                        prefix.extend([r, c]);
                        flatten(&cell.blocks, prefix, out);
                        prefix.truncate(prefix.len() - 2);
                    }
                }
            }
            Block::Table(_) => out.push((prefix.clone(), b.clone())),
            _ => {}
        }
        prefix.pop();
    }
}

/// The student's work on `scan` written as `out` (see [`read`] and [`write`]); returns how many
/// pieces were found.
pub fn work(scan: &FsPath, sheet: &Sheet, shear: f32, key: &str, out: &FsPath) -> Result<usize, String> {
    let (pieces, page) = read(scan, sheet, shear, key)?;
    write(sheet, &pieces, &page, out)?;
    Ok(pieces.len())
}

// ---- Datalab ----

/// Sends a page (PNG) to Datalab's hosted Chandra in its most accurate mode and returns the
/// whole reply. The page is deleted from Datalab as soon as the reading is back.
pub fn convert(png: &[u8], key: &str) -> Result<Value, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let boundary = format!("documentashion-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    let mut body: Vec<u8> = vec![];
    let field = |body: &mut Vec<u8>, name: &str, value: &str| {
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    };
    field(&mut body, "output_format", "json");
    field(&mut body, "mode", "accurate");
    field(&mut body, "additional_config", r#"{"keep_pageheader_in_output":true,"keep_pagefooter_in_output":true}"#);
    body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"page.png\"\r\nContent-Type: image/png\r\n\r\n").as_bytes());
    body.extend(png);
    body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
    let json = |r: Result<ureq::http::Response<ureq::Body>, ureq::Error>| -> Result<Value, String> {
        let mut r = r.map_err(|e| format!("Datalab: {e}"))?;
        let status = r.status();
        let text = r.body_mut().with_config().limit(64 << 20).read_to_string().map_err(|e| format!("Datalab: {e}"))?;
        let v: Value = serde_json::from_str(&text).map_err(|_| format!("Datalab answered {status}: {}", text.chars().take(200).collect::<String>()))?;
        if status == 401 || status == 403 {
            return Err("Datalab didn't accept the API key.".into());
        }
        if !status.is_success() {
            return Err(format!("Datalab answered {status}: {}", v.get("detail").or(v.get("error")).unwrap_or(&v)));
        }
        Ok(v)
    };
    let sent = json(agent.post(CONVERT).header("X-API-Key", key).header("Content-Type", &format!("multipart/form-data; boundary={boundary}")).send(&body[..]))?;
    let check = sent.get("request_check_url").and_then(Value::as_str).ok_or("Datalab didn't take the page.")?.to_string();
    let start = std::time::Instant::now();
    let mut wait = std::time::Duration::from_millis(500);
    let done = loop {
        std::thread::sleep(wait);
        wait = (wait * 5 / 4).min(std::time::Duration::from_secs(3));
        let v = json(agent.get(&check).header("X-API-Key", key).call())?;
        match v.get("status").and_then(Value::as_str) {
            Some("complete") => break v,
            Some("failed") => return Err(format!("Datalab couldn't read the page: {}", v.get("error").unwrap_or(&Value::Null))),
            _ if start.elapsed().as_secs() > 300 => return Err("Datalab took too long to read the page.".into()),
            _ => {}
        }
    };
    // A result kept in another region is downloaded from its own signed address, without the key.
    let mut result = done.clone();
    if let Some(url) = done.get("result_url").and_then(Value::as_str) {
        let mut full = json(agent.get(url).call())?;
        if let (Some(f), Some(d)) = (full.as_object_mut(), done.as_object()) {
            for (k, v) in d.iter().filter(|(_, v)| !v.is_null()) {
                f.insert(k.clone(), v.clone());
            }
        }
        result = full;
    }
    // Nothing of the page is left with Datalab.
    if let Some(id) = check.rsplit('/').next() {
        let _ = agent.delete(&format!("https://www.datalab.to/api/v1/results/{id}")).header("X-API-Key", key).call();
    }
    if result.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(format!("Datalab couldn't read the page: {}", result.get("error").unwrap_or(&Value::Null)));
    }
    Ok(result)
}

/// The blocks of the first page of a Datalab JSON reply: type, box (pixels of an image `width`
/// wide, as sent) and HTML.
pub(crate) fn page_blocks(reply: &Value, width: f32) -> Result<Vec<(String, [f32; 4], String)>, String> {
    let json = match reply.get("json") {
        Some(Value::String(s)) => serde_json::from_str(s).map_err(|e| e.to_string())?,
        Some(v) => v.clone(),
        None => return Err("Datalab's reply had no reading of the page.".into()),
    };
    let page = json.get("children").and_then(|c| c.get(0)).ok_or("Datalab's reply had no page.")?;
    let size = page.get("bbox").and_then(bbox).map(|b| b[2] - b[0]).filter(|w| *w > 0.0).unwrap_or(width);
    let k = width / size;
    Ok(page
        .get("children")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|b| {
                    let kind = b.get("block_type").and_then(Value::as_str).unwrap_or("").to_string();
                    let html = b.get("html").and_then(Value::as_str).unwrap_or("").to_string();
                    let r = bbox(b.get("bbox")?)?;
                    Some((kind, [r[0] * k, r[1] * k, r[2] * k, r[3] * k], html))
                })
                .collect()
        })
        .unwrap_or_default())
}

fn bbox(v: &Value) -> Option<[f32; 4]> {
    let a = v.as_array()?;
    let n: Vec<f32> = a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
    (n.len() == 4).then(|| [n[0], n[1], n[2], n[3]])
}

// ---- The name ----

/// Where the student's name may be written, pixels at 150 dpi: the whole line right of each
/// printed "Name" label, to `right` (points, the paper's right edge), with room above and below
/// for handwriting that leaves the line. The rest of the line goes too (a date, a period): a
/// name is often written again beside them.
fn name_boxes(words: &[fit::WordBox], right: f32) -> Vec<[f32; 4]> {
    let k = fit::K;
    words
        .iter()
        .filter(|w| w.text.trim().trim_end_matches(':').eq_ignore_ascii_case("name"))
        .map(|w| {
            let [_, y0, x1, y1] = w.rect;
            let h = y1 - y0;
            [(x1 + 1.0) * k, (y0 - 1.2 * h) * k, right * k, (y1 + 0.7 * h) * k]
        })
        .collect()
}

pub(crate) fn blank(page: &mut GrayImage, [x0, y0, x1, y1]: [f32; 4]) {
    let (w, h) = page.dimensions();
    for y in (y0.max(0.0) as u32)..(y1.max(0.0) as u32).min(h) {
        for x in (x0.max(0.0) as u32)..(x1.max(0.0) as u32).min(w) {
            page.put_pixel(x, y, Luma([255]));
        }
    }
}

// ---- Reading the HTML ----

/// A line of Datalab's reading, or a table.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item {
    Line(Vec<Run>),
    Table(Vec<Vec<Vec<Run>>>),
}

/// Datalab's HTML for one block as lines of runs (math as LaTeX) and tables.
pub(crate) fn parse(html: &str) -> Vec<Item> {
    let mut items = vec![];
    let mut line: Vec<Run> = vec![];
    let mut style = Text::default();
    let flush = |line: &mut Vec<Run>, items: &mut Vec<Item>| {
        trim(line);
        if !line.is_empty() {
            items.push(Item::Line(std::mem::take(line)));
        }
    };
    let mut rest = html;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            push_text(&mut line, rest, &style);
            break;
        };
        push_text(&mut line, &rest[..open], &style);
        rest = &rest[open..];
        let Some(close) = rest.find('>') else { break };
        let tag = &rest[1..close];
        rest = &rest[close + 1..];
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        let end = tag.starts_with('/');
        match name.as_str() {
            "math" if !end => {
                let stop = rest.find("</math>").unwrap_or(rest.len());
                let latex = unescape(&rest[..stop]);
                rest = rest.get(stop + 7..).unwrap_or("");
                for (i, part) in math_lines(&latex).into_iter().enumerate() {
                    if i > 0 {
                        flush(&mut line, &mut items);
                    }
                    if !part.is_empty() {
                        line.push(Run::Math { math: part, display: false, size: None });
                    }
                }
            }
            "table" if !end => {
                flush(&mut line, &mut items);
                let stop = rest.find("</table>").unwrap_or(rest.len());
                items.push(Item::Table(table_rows(&rest[..stop])));
                rest = rest.get(stop + 8..).unwrap_or("");
            }
            "br" | "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "tr" | "ul" | "ol" | "caption" | "pre" => flush(&mut line, &mut items),
            "sup" => style.sup = !end,
            "sub" => style.sub = !end,
            "b" | "strong" => style.bold = !end,
            "i" | "em" => style.italic = !end,
            "u" => style.underline = !end,
            "del" | "s" | "strike" => style.strike = !end,
            "input" => {
                let checked = tag.contains("checked");
                push_text(&mut line, if checked { "☑ " } else { "☐ " }, &Text::default());
            }
            _ => {}
        }
    }
    flush(&mut line, &mut items);
    items
}

fn table_rows(html: &str) -> Vec<Vec<Vec<Run>>> {
    let mut rows = vec![];
    for tr in html.split("<tr").skip(1) {
        let tr = tr.split("</tr>").next().unwrap_or("");
        let mut cells = vec![];
        for td in tr.split("<t").skip(1).filter(|c| ["d>", "d ", "h>", "h "].iter().any(|p| c.starts_with(p))) {
            let inner = td.split_once('>').map(|x| x.1).unwrap_or("");
            let inner = inner.split("</td>").next().unwrap_or(inner).split("</th>").next().unwrap_or(inner);
            let mut runs = vec![];
            for (i, item) in parse(inner).into_iter().enumerate() {
                if let Item::Line(l) = item {
                    if i > 0 {
                        runs.push(Run::Br { br: true });
                    }
                    runs.extend(l);
                }
            }
            cells.push(runs);
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
    }
    rows
}

/// Text between tags: entities decoded, runs of white space made one space.
fn push_text(line: &mut Vec<Run>, raw: &str, style: &Text) {
    let t = unescape(raw);
    let mut s = String::with_capacity(t.len());
    for c in t.chars() {
        if c.is_whitespace() {
            if !s.ends_with(' ') {
                s.push(' ');
            }
        } else {
            s.push(c);
        }
    }
    if s.is_empty() || (s == " " && line.is_empty()) {
        return;
    }
    if let Some(Run::Text(last)) = line.last_mut() {
        if (Text { text: String::new(), ..last.clone() }) == (Text { text: String::new(), ..style.clone() }) {
            last.text.push_str(&s);
            return;
        }
    }
    line.push(Run::Text(Text { text: s, ..style.clone() }));
}

fn trim(line: &mut Vec<Run>) {
    if let Some(Run::Text(t)) = line.first_mut() {
        t.text = t.text.trim_start().to_string();
    }
    if let Some(Run::Text(t)) = line.last_mut() {
        t.text = t.text.trim_end().to_string();
    }
    line.retain(|r| !matches!(r, Run::Text(t) if t.text.is_empty()));
}

fn unescape(s: &str) -> String {
    s.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&")
}

/// One LaTeX equation as the lines it was written on: a stacked `array` or `aligned` and `\\`
/// breaks become separate lines, alignment marks dropped.
fn math_lines(latex: &str) -> Vec<String> {
    let mut s = latex.to_string();
    for env in ["array", "aligned", "align", "align*", "gathered", "cases", "matrix"] {
        // `\begin{array}{r}` takes a column spec.
        while let Some(i) = s.find(&format!("\\begin{{{env}}}")) {
            let mut j = i + env.len() + 8;
            if s[j..].starts_with('{') {
                j = s[j..].find('}').map(|e| j + e + 1).unwrap_or(j);
            }
            s.replace_range(i..j, "");
        }
        s = s.replace(&format!("\\end{{{env}}}"), "");
    }
    s.split("\\\\")
        // A row's extra space (`\\[6pt]`) and rules between rows are layout, not writing.
        .map(|l| {
            let l = l.trim_start();
            let l = if l.starts_with('[') { l.find(']').map(|e| &l[e + 1..]).unwrap_or(l) } else { l };
            l.replace("\\hline", " ").replace("\\midrule", " ")
        })
        .map(|l| l.replace('&', " ").replace("\\text{ }", " ").replace("\\quad", " ").replace("\\qquad", " ").split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect()
}

// ---- Keeping only what the student added ----

/// The printed text of a sheet, as skeletons (see [`skeleton`]): for each line the rebuild
/// measures (see [`super::anchors`]) its lines, the pieces between its tabs (two questions
/// often share a printed line) and the whole of it; and, belonging to no line, the header and
/// footer and every ruled table whole.
struct Prints {
    anchors: Vec<(super::Path, Vec<String>)>,
    other: Vec<String>,
}

impl Prints {
    fn of(sheet: &Sheet) -> Prints {
        fn para(p: &Para, out: &mut Vec<String>) {
            let mut whole = String::new();
            for line in p.runs.split(|r| matches!(r, Run::Br { .. })) {
                let s = skeleton_runs(line);
                whole.push_str(&s);
                out.push(s);
                out.extend(line.split(|r| matches!(r, Run::Tab { .. })).map(skeleton_runs));
            }
            out.push(whole);
            out.retain(|s| !s.is_empty());
            out.sort();
            out.dedup();
        }
        fn tables(blocks: &[Block], out: &mut Vec<String>) {
            for b in blocks {
                match b {
                    Block::P(p) => para(p, out),
                    Block::Table(t) => {
                        let mut whole = String::new();
                        for c in t.rows.iter().flat_map(|r| &r.cells) {
                            for b in &c.blocks {
                                if let Block::P(p) = b {
                                    whole.push_str(&p.runs.iter().map(|r| skeleton_runs(std::slice::from_ref(r))).collect::<String>());
                                }
                            }
                            tables(&c.blocks, out);
                        }
                        if t.borders != Borders::None && !whole.is_empty() {
                            out.push(whole);
                        }
                    }
                    _ => {}
                }
            }
        }
        let anchors = super::anchors(&sheet.blocks)
            .into_iter()
            .filter_map(|at| match super::block_at(&sheet.blocks, &at) {
                Some(Block::P(p)) => {
                    let mut lines = vec![];
                    para(p, &mut lines);
                    (!lines.is_empty()).then_some((at, lines))
                }
                _ => None,
            })
            .collect();
        let mut other = vec![];
        tables(&sheet.blocks, &mut other);
        tables(&sheet.header, &mut other);
        tables(&sheet.footer, &mut other);
        other.sort();
        other.dedup();
        Prints { anchors, other }
    }

    /// Whether `s` (a skeleton) reads as printed, and if so which measured lines it could be
    /// (none: printed, but in a header, footer or table). `None`: the student's.
    fn find(&self, s: &str) -> Option<Vec<usize>> {
        if s.is_empty() {
            return Some(vec![]);
        }
        // A question's number is often printed apart from it (a column of its own): then a
        // reading's label is left out to compare. Where the printed line has its own label the
        // two are compared whole, so a circled choice, "(A)" for "A.", stays the student's.
        let bare = unlabelled(s);
        let like = |p: &String| same(s, p) || bare.is_some_and(|b| unlabelled(p).is_none() && same(b, p));
        let hits: Vec<usize> = self.anchors.iter().enumerate().filter(|(_, (_, lines))| lines.iter().any(like)).map(|(i, _)| i).collect();
        if !hits.is_empty() || self.other.iter().any(like) {
            return Some(hits);
        }
        None
    }
}

/// A skeleton with the question label it starts with (`9.`, `(3)`, `b)`) taken off; `None`
/// when it starts with none.
pub(crate) fn unlabelled(s: &str) -> Option<&str> {
    let end = if let Some(rest) = s.strip_prefix('(') {
        let close = rest.find(')').filter(|&i| (1..=3).contains(&i))?;
        close + 2
    } else {
        // `12.3x` is question 12 as often as `3.5` is a number: what follows must match the
        // printed line either way.
        let label = s.find(['.', ')']).filter(|&i| (1..=3).contains(&i))?;
        s[..label].chars().all(char::is_alphanumeric).then_some(label + 1)?
    };
    Some(&s[end..]).filter(|r| !r.is_empty())
}

/// Whether reading `s` is printed line `p` (both skeletons): nearly all of `s` is in `p`, and
/// nearly all of `p` in `s`. A very short reading must match exactly, or any lone number would
/// match some printed one.
pub(crate) fn same(s: &str, p: &str) -> bool {
    let (n, m) = (s.chars().count(), p.chars().count());
    if n < 4 {
        return s == p;
    }
    if (m as f32) < 0.85 * n as f32 || (m as f32) > n as f32 / 0.75 {
        return false;
    }
    let (a, b): (Vec<char>, Vec<char>) = (s.chars().collect(), p.chars().collect());
    let common = lcs(&a, &b) as f32;
    common >= 0.9 * n as f32 && common >= 0.8 * m as f32
}

/// The length of the longest common subsequence.
pub(crate) fn lcs(a: &[char], b: &[char]) -> usize {
    let mut row = vec![0usize; b.len() + 1];
    for &x in a {
        let mut prev = 0;
        for (j, &y) in b.iter().enumerate() {
            let keep = row[j + 1];
            row[j + 1] = if x == y { prev + 1 } else { row[j + 1].max(row[j]) };
            prev = keep;
        }
    }
    row[b.len()]
}

/// What a printed line and a reading of it have in common, whoever wrote the LaTeX: letters
/// and digits (lower case) and the signs that carry meaning, with LaTeX commands, braces, spaces
/// and superscripts left out. Superscripts go because a student's working is often written
/// small above a printed equation (the number each term is multiplied by), and reads as a power.
pub(crate) fn skeleton(s: &str) -> String {
    let mut out = String::new();
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' => {
                i += 1;
                let start = i;
                while i < c.len() && c[i].is_ascii_alphabetic() {
                    i += 1;
                }
                // A blank drawn as space (`\underline{\hspace{2cm}}`) is nothing written: its
                // size is not text.
                let name: String = c[start..i].iter().collect();
                if ["hspace", "vspace", "rule", "phantom", "hphantom", "vphantom"].contains(&name.as_str()) {
                    while i < c.len() && c[i] == '{' {
                        let mut depth = 0;
                        while i < c.len() {
                            if c[i] == '{' {
                                depth += 1;
                            } else if c[i] == '}' {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            i += 1;
                        }
                        i += 1;
                    }
                }
                continue;
            }
            '^' => {
                i += 1;
                if i < c.len() && c[i] == '{' {
                    let mut depth = 0;
                    while i < c.len() {
                        if c[i] == '{' {
                            depth += 1;
                        } else if c[i] == '}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        i += 1;
                    }
                }
                i += 1;
                continue;
            }
            '−' | '–' | '—' => out.push('-'),
            ch if ch.is_alphanumeric() => out.extend(ch.to_lowercase()),
            ch if "+-=()/,.<>".contains(ch) => out.push(ch),
            _ => {}
        }
        i += 1;
    }
    out
}

pub(crate) fn skeleton_runs(runs: &[Run]) -> String {
    let mut s = String::new();
    for r in runs {
        match r {
            Run::Text(t) if !t.sup => s.push_str(&skeleton(&t.text)),
            Run::Math { math, .. } => s.push_str(&skeleton(math)),
            _ => {}
        }
    }
    s
}

/// One line (or table) of a block's reading: printed (and which measured lines it could be), or
/// the student's, ready to set.
enum Read {
    Print(Vec<usize>),
    Work(Block),
}

/// A block's reading sorted into what was printed and what the student added. In a graph's
/// reading (`figure`) the picture's own caption goes. A very short reading (an answer like
/// "-17") is taken for print only where its printed twin is beside it (`near`): the same
/// number printed elsewhere, in a list of answers to check against, doesn't make it print.
fn classify(items: &[Item], prints: &Prints, figure: bool, near: &dyn Fn(&[usize]) -> bool) -> Vec<Read> {
    let mut out = vec![];
    for it in items {
        match it {
            Item::Line(runs) => {
                let sk = skeleton_runs(runs);
                if let Some(hits) = prints.find(&sk).filter(|h| sk.chars().count() >= 4 || near(h)) {
                    out.push(Read::Print(hits));
                    continue;
                }
                if figure && matches!(runs.first(), Some(Run::Text(t)) if t.text.contains("plot:") || t.text.contains("chart:") || t.text.contains("graph:")) {
                    continue;
                }
                out.push(Read::Work(Block::P(Para { after: 2.0, runs: runs.iter().map(inked).collect(), ..Default::default() })));
            }
            Item::Table(rows) => {
                let flat: String = rows.iter().flatten().map(|c| skeleton_runs(c)).collect();
                if let Some(hits) = prints.find(&flat) {
                    out.push(Read::Print(hits));
                    continue;
                }
                let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                out.push(Read::Work(Block::Table(Table {
                    widths: vec![(230.0 / cols as f32).min(80.0); cols],
                    borders: Borders::All,
                    align: Align::Left,
                    rows: rows
                        .iter()
                        .map(|r| Row {
                            height: None,
                            cells: (0..cols)
                                .map(|i| Cell {
                                    blocks: vec![Block::P(Para { runs: r.get(i).map(|c| c.iter().map(inked).collect()).unwrap_or_default(), ..Default::default() })],
                                    ..Default::default()
                                })
                                .collect(),
                        })
                        .collect(),
                })));
            }
        }
    }
    out
}

/// A run set in the student's colour.
fn inked(r: &Run) -> Run {
    match r {
        Run::Text(t) => Run::Text(Text { color: Some(INK.into()), ..t.clone() }),
        r => r.clone(),
    }
}

fn note(text: &str) -> Block {
    Block::P(Para { runs: vec![Run::Text(Text { text: text.into(), italic: true, color: Some("6B7280".into()), ..Default::default() })], ..Default::default() })
}

fn header(text: &str) -> Cell {
    Cell {
        blocks: vec![Block::P(Para { runs: vec![Run::Text(Text { text: text.into(), bold: true, size: Some(8.0), color: Some("4B5563".into()), ..Default::default() })], ..Default::default() })],
        shade: Some("EEF2F7".into()),
        ..Default::default()
    }
}

// ---- Which question a piece belongs to ----

/// An anchor's first line on the page and the column it heads, pixels at 150 dpi.
struct Place {
    at: super::Path,
    top: f32,
    left: f32,
    right: f32,
}

fn places(sheet: &Sheet, m: &fit::Measure) -> Vec<Place> {
    let text_right = (sheet.page.width - sheet.page.margins[1]) * fit::K;
    m.anchors
        .iter()
        .zip(&m.drawn)
        .zip(&m.found)
        .filter_map(|((at, drawn), found)| {
            let [x, y, _, _] = (*drawn)?;
            let (dx, dy) = found.as_ref().map(|o| (o.dx * fit::K, o.dy * fit::K)).unwrap_or((0.0, 0.0));
            let left = x as f32 + dx;
            let right = fit::cell_width(&sheet.blocks, at).map(|w| left + w * fit::K).unwrap_or(text_right);
            Some(Place { at: at.clone(), top: y as f32 + dy, left, right })
        })
        .collect()
}

/// The anchor a piece at `bbox` belongs under: the lowest line that starts above it (or level
/// with it) in the column its left edge is in; failing that, the nearest line above it anywhere.
fn place(places: &[Place], [x0, y0, _, y1]: [f32; 4]) -> Option<super::Path> {
    // Working often starts a little above the question's line (a number written over it).
    let y0 = y0 + ((y1 - y0) * 0.3).min(20.0);
    let slack = 12.0;
    let above = |p: &&Place| p.top <= y0 + slack;
    // The lowest line above; lines level with it (side by side in columns) go to the one whose
    // column starts nearest the work's left edge.
    let lowest = |list: Vec<&Place>| -> Option<super::Path> {
        let top = list.iter().map(|p| p.top).fold(f32::MIN, f32::max);
        list.into_iter().filter(|p| p.top >= top - 8.0).min_by(|a, b| (x0 - a.left).abs().total_cmp(&(x0 - b.left).abs())).map(|p| p.at.clone())
    };
    lowest(places.iter().filter(above).filter(|p| x0 >= p.left - 40.0 && x0 < p.right).collect())
        .or_else(|| lowest(places.iter().filter(above).collect()))
        .or_else(|| places.first().map(|p| p.at.clone()))
}

/// Whether a box on the page is mostly over a graph the sheet prints.
fn printed_figure(sheet: &Sheet, m: &fit::Measure, [x0, y0, x1, y1]: [f32; 4]) -> bool {
    let area = ((x1 - x0) * (y1 - y0)).max(1.0);
    m.anchors.iter().zip(&m.drawn).zip(&m.found).any(|((at, drawn), found)| {
        let (Some(Block::Figure(_)), Some([fx, fy, fw, fh])) = (super::block_at(&sheet.blocks, at), drawn) else { return false };
        let (dx, dy) = found.as_ref().map(|o| (o.dx * fit::K, o.dy * fit::K)).unwrap_or((0.0, 0.0));
        let (fx0, fy0) = (*fx as f32 + dx, *fy as f32 + dy);
        let (fx1, fy1) = (fx0 + *fw as f32, fy0 + *fh as f32);
        let w = (x1.min(fx1) - x0.max(fx0)).max(0.0);
        let h = (y1.min(fy1) - y0.max(fy0)).max(0.0);
        w * h / area >= 0.3
    })
}

/// Where the work under an anchor goes: after it, or, inside a table of values (every cell
/// boxed; a layout grid has no lines or only a rule between its columns), after the table.
fn insertion(blocks: &[Block], at: &[usize]) -> super::Path {
    let mut best = at.to_vec();
    let mut depth = 1;
    while depth + 3 <= at.len() {
        if let Some(Block::Table(t)) = super::block_at(blocks, &at[..depth]) {
            if t.borders == Borders::All {
                best = at[..depth].to_vec();
                break;
            }
        }
        depth += 3;
    }
    best
}

/// The part of the page at `bbox`, with a little margin.
fn crop(page: &GrayImage, [x0, y0, x1, y1]: [f32; 4]) -> GrayImage {
    let pad = 6.0;
    let (w, h) = page.dimensions();
    let x = (x0 - pad).max(0.0) as u32;
    let y = (y0 - pad).max(0.0) as u32;
    let cw = ((x1 + pad).min(w as f32) as u32).saturating_sub(x).max(1);
    let ch = ((y1 + pad).min(h as f32) as u32).saturating_sub(y).max(1);
    image::imageops::crop_imm(page, x, y, cw, ch).to_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                Item::Line(runs) => runs
                    .iter()
                    .map(|r| match r {
                        Run::Text(t) => t.text.clone(),
                        Run::Math { math, .. } => format!("${math}$"),
                        _ => "|".into(),
                    })
                    .collect(),
                Item::Table(rows) => format!("table {}x{}", rows.len(), rows.first().map(Vec::len).unwrap_or(0)),
            })
            .collect()
    }

    #[test]
    fn html_lines_math_and_tables_are_read() {
        let html = r#"<p><p><span data-bbox="1 2 3 4">9.</span> <math>\frac{x^{14}}{2} = 34</math><br/><math>7x + 10x = 476</math></p></p><p>yes,<br/>27<sup>th</sup> one &amp; <del>no</del></p><table><thead><tr><th>n</th><th>t(n)</th></tr></thead><tbody><tr><td>0</td><td><math>-1</math></td></tr></table>"#;
        let items = parse(html);
        assert_eq!(text(&items), ["9. $\\frac{x^{14}}{2} = 34$", "$7x + 10x = 476$", "yes,", "27th one & no", "table 2x2"]);
        let Item::Line(l) = &items[3] else { panic!() };
        assert!(matches!(&l[1], Run::Text(t) if t.sup && t.text == "th"));
        assert!(matches!(l.last(), Some(Run::Text(t)) if t.strike && t.text == "no"));
    }

    #[test]
    fn stacked_working_becomes_lines() {
        assert_eq!(math_lines(r"\begin{array}{r} 5x - 7 = 18 \\ +7 \quad +7 \\ 5x = 25 \end{array}"), ["5x - 7 = 18", "+7 +7", "5x = 25"]);
        assert_eq!(math_lines(r"\begin{aligned} 4 &= -3(2) + b \\ 10 &= b \end{aligned}"), ["4 = -3(2) + b", "10 = b"]);
        assert_eq!(math_lines(r"\begin{array}{l} 2x + 6 = 45 \\[6pt] \hline x = 15 \end{array}"), ["2x + 6 = 45", "x = 15"]);
    }

    #[test]
    fn printed_lines_go_and_the_students_stay() {
        let sheet: Sheet = serde_json::from_str(
            r#"{"blocks":[
                {"type":"p","runs":[{"text":"9.\t"},{"math":"\\frac{x}{2}+\\frac{5x}{7}=34"}]},
                {"type":"p","runs":[{"text":"1. Complete the arithmetic sequence"}]},
                {"type":"p","runs":[{"text":"-1, 3, 7, 11, 15, ____, ____, ____."}]},
                {"type":"p","runs":[{"text":"A. parallel"}]},
                {"type":"p","runs":[{"text":"(1) "},{"math":"-9 + 15"}]}
            ]}"#,
        )
        .unwrap();
        let prints = Prints::of(&sheet);
        let reads = |html: &str| text(&parse(html)).into_iter().zip(parse(html)).filter(|(_, i)| !matches!(i, Item::Line(r) if prints.find(&skeleton_runs(r)).is_some())).map(|(t, _)| t).collect::<Vec<_>>();
        // The multipliers written over the printed equation read as powers; it is still the print.
        assert_eq!(reads(r"<p>9. <math>\frac{x^{14}}{2} + \frac{5x^{14}}{7} = 34^{14}</math><br/><math>7x + 10x = 476</math></p>"), ["$7x + 10x = 476$"]);
        // Blanks filled in keep the line; a circled choice keeps it too.
        assert_eq!(reads("<p>-1, 3, 7, 11, 15, 19, 23, 27.</p><p>1. Complete the arithmetic sequence</p><p>(A) parallel</p>"), ["-1, 3, 7, 11, 15, 19, 23, 27.", "(A) parallel"]);
        // A lone answer is the student's even if the same number is printed somewhere.
        assert_eq!(reads("<p>(1) <math>-9 + 15</math></p><p>6</p><p>15</p>"), ["6", "15"]);
        // Two questions printed on one tabbed line are each found, and found as that line.
        let both = Prints::of(&serde_json::from_str(r#"{"blocks":[{"type":"p","runs":[{"text":"(1) "},{"math":"-9 + 15"},{"tab":true},{"text":"(2) "},{"math":"-6 - (-11)"}]}]}"#).unwrap());
        assert_eq!(both.find(&skeleton("(1) -9 + 15")), Some(vec![0]));
        assert_eq!(both.find(&skeleton("(2) -6 - (-11)")), Some(vec![0]));
        // A number printed in a column of its own.
        let apart = Prints::of(&serde_json::from_str(r#"{"blocks":[{"type":"p","runs":[{"text":"9."}]},{"type":"p","runs":[{"math":"\\frac{x}{2}+\\frac{5x}{7}=34"}]}]}"#).unwrap());
        assert_eq!(apart.find(&skeleton(r"9. \frac{x^{14}}{2} + \frac{5x^{14}}{7} = 34^{14}")), Some(vec![1]));
        assert_eq!(unlabelled("(12)x=4"), Some("x=4"));
        assert_eq!(unlabelled("b)x=4"), Some("x=4"));
        assert_eq!(unlabelled("x=4"), None);
        assert_eq!(unlabelled("12.3x5-5x2=572"), Some("3x5-5x2=572"));
    }

    #[test]
    fn work_goes_under_the_question_in_its_column() {
        let p = |at: usize, top: f32, left: f32, right: f32| Place { at: vec![at], top, left, right };
        let places = [p(0, 100.0, 50.0, 600.0), p(1, 100.0, 650.0, 1200.0), p(2, 400.0, 50.0, 600.0), p(3, 420.0, 650.0, 1200.0)];
        assert_eq!(place(&places, [70.0, 150.0, 300.0, 300.0]), Some(vec![0]));
        assert_eq!(place(&places, [700.0, 300.0, 900.0, 380.0]), Some(vec![1]));
        assert_eq!(place(&places, [690.0, 415.0, 900.0, 500.0]), Some(vec![3]));
        assert_eq!(place(&places, [60.0, 395.0, 300.0, 500.0]), Some(vec![2]));
        // Columns that meet: work starting right at the second one's edge is in the second.
        let side = [p(0, 719.0, 102.0, 664.0), p(1, 719.0, 662.0, 1174.0), p(2, 787.0, 101.0, 664.0), p(3, 786.0, 666.0, 1178.0)];
        assert_eq!(place(&side, [662.0, 752.0, 1164.0, 839.0]), Some(vec![1]));
        assert_eq!(place(&side, [638.0, 842.0, 1122.0, 1498.0]), Some(vec![3]));
    }

    #[test]
    fn the_name_line_is_blanked_to_the_edge() {
        let w = |t: &str, x0: f32, x1: f32| fit::WordBox { text: t.into(), rect: [x0, 100.0, x1, 110.0] };
        let b = name_boxes(&[w("Name:", 72.0, 100.0), w("________", 104.0, 290.0), w("Date:", 300.0, 325.0), w("Problems", 72.0, 120.0)], 612.0);
        assert_eq!(b.len(), 1);
        let k = fit::K;
        assert_eq!((b[0][0], b[0][2]), (101.0 * k, 612.0 * k));
        assert!(b[0][1] < 100.0 * k && b[0][3] > 110.0 * k);
    }

    #[test]
    fn a_value_table_takes_its_work_after_it() {
        let s: Sheet = serde_json::from_str(
            r#"{"blocks":[{"type":"table","widths":[100,100],"rows":[{"cells":[{"blocks":[
                {"type":"table","widths":[20,20],"borders":"all","rows":[{"cells":[{"blocks":[{"type":"p","runs":[]}]},{"blocks":[]}]}]}
            ]},{"blocks":[{"type":"p","runs":[]}]}]}]}]}"#,
        )
        .unwrap();
        assert_eq!(insertion(&s.blocks, &[0, 0, 0, 0, 0, 0, 0]), vec![0, 0, 0, 0]);
        assert_eq!(insertion(&s.blocks, &[0, 0, 1, 0]), vec![0, 0, 1, 0]);
    }
}
