use crate::{
    imaging::{self, Analysis},
    model::*,
};
use base64::Engine;
use serde_json::{Value, json};
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub fn process(cmd: Command, cancel: &AtomicBool) -> Result<String, String> {
    process_for(cmd, cancel, Duration::from_secs(300))
}
pub fn process_for(mut cmd: Command, cancel: &AtomicBool, limit: Duration) -> Result<String, String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Cannot start command: {e}"))?;
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let a = std::thread::spawn(move || {
        let mut b = String::new();
        let _ = out.read_to_string(&mut b);
        b
    });
    let b = std::thread::spawn(move || {
        let mut b = String::new();
        let _ = err.read_to_string(&mut b);
        b
    });
    let start = Instant::now();
    let status = loop {
        if cancel.load(Ordering::Relaxed) || start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            let _ = a.join();
            let _ = b.join();
            return Err(
                "Operation cancelled or timed out. Captured pages remain in the project folder."
                    .into(),
            );
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let out = a.join().unwrap_or_default();
    let err = b.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        Err(format!("Command failed ({status}): {err}"))
    }
}
fn scanner_command() -> Result<Command, String> {
    // Load only Fujitsu: unrelated system SANE backends can crash discovery.
    let dir = root().join("sane");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("dll.conf"), "fujitsu\n").map_err(|e| e.to_string())?;
    if let Ok(config) = std::fs::read("/etc/sane.d/fujitsu.conf") {
        std::fs::write(dir.join("fujitsu.conf"), config).map_err(|e| e.to_string())?;
    }
    let mut cmd = Command::new("scanimage");
    cmd.env("SANE_CONFIG_DIR", dir);
    Ok(cmd)
}
pub fn discover(cancel: &AtomicBool) -> Result<String, String> {
    let mut c = scanner_command()?;
    c.args(["-f", "%d%n"]);
    process(c, cancel)
}
pub fn scan(
    p: &Project,
    s: &Settings,
    cancel: &AtomicBool,
) -> Result<(Vec<std::path::PathBuf>, Option<String>), String> {
    if s.device.trim().is_empty() {
        return Err("Discover and select your scanner first.".into());
    }
    let dir = p.dir().join(format!("capture-{}", id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut c = scanner_command()?;
    c.arg("--device-name")
        .arg(&s.device)
        .args([
            "--source",
            if s.duplex { "ADF Duplex" } else { "ADF Front" },
            "--mode",
            "Color",
            "--resolution",
        ])
        .arg(s.dpi.to_string())
        .arg("--format=png")
        .arg(format!("--batch={}/page-%04d.png", dir.display()));
    let result = process(c, cancel);
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| image::open(p).is_ok())
        .collect();
    paths.sort();
    if paths.is_empty() {
        result?;
        return Err("No pages received. Check the feeder and scanner settings.".into());
    }
    Ok((paths, result.err()))
}
/// Text read from a page. OCR also returns word boxes for the PDF text layer.
pub struct Reading {
    pub text: String,
    pub words: Vec<Word>,
    /// `Page::geometry()` of the image the boxes were measured on.
    pub geometry: String,
}
impl Reading {
    pub fn text(text: String) -> Self {
        Self {
            text,
            words: vec![],
            geometry: String::new(),
        }
    }
}
fn tesseract() -> std::ffi::OsString {
    std::env::var_os("DOCUMENTASHION_TESSERACT").unwrap_or_else(|| "tesseract".into())
}
/// Whether local OCR can run, for Settings.
pub fn ocr_status() -> Result<String, String> {
    let missing = "Tesseract isn't installed. On Fedora: sudo dnf install tesseract tesseract-langpack-eng";
    let out = Command::new(tesseract())
        .arg("--list-langs")
        .output()
        .map_err(|_| missing.to_string())?;
    let version = Command::new(tesseract())
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
            s.lines().next().map(str::to_owned)
        })
        .unwrap_or_else(|| "tesseract".into());
    let langs = String::from_utf8_lossy(&out.stdout);
    if langs.lines().any(|l| l.trim() == "eng") {
        Ok(format!("{version} with English"))
    } else {
        Err("Tesseract is installed but English data is missing. On Fedora: sudo dnf install tesseract-langpack-eng".into())
    }
}
pub fn ocr(p: &Page, cancel: &AtomicBool) -> Result<Reading, String> {
    let path = std::env::temp_dir().join(format!("documentashion-{}.png", id()));
    p.render()?.save(&path).map_err(|e| e.to_string())?;
    let mut c = Command::new(tesseract());
    c.arg(&path)
        .args(["stdout", "-l", "eng", "--psm", "3", "--dpi"])
        .arg(p.dpi.max(70).to_string())
        .arg("tsv");
    let r = process(c, cancel);
    let _ = std::fs::remove_file(path);
    let (text, words) = parse_tsv(&r.map_err(|e| {
        format!("Local OCR needs Tesseract with English data. You can use AI transcription instead. {e}")
    })?);
    if words.is_empty() {
        return Err("Tesseract found no text on this page.".into());
    }
    Ok(Reading {
        text,
        words,
        geometry: p.geometry(),
    })
}
/// Reads Tesseract's TSV: one row per word with its block, paragraph and line numbers.
fn parse_tsv(tsv: &str) -> (String, Vec<Word>) {
    let mut text = String::new();
    let mut words = vec![];
    let mut last: Option<(u32, u32, u32)> = None;
    for row in tsv.lines().skip(1) {
        let f: Vec<&str> = row.splitn(12, '\t').collect();
        if f.len() < 12 || f[0] != "5" || f[11].trim().is_empty() {
            continue;
        }
        let n = |i: usize| f[i].trim().parse::<u32>().unwrap_or(0);
        let line = (n(2), n(3), n(4));
        if let Some(prev) = last {
            text.push_str(if (prev.0, prev.1) != (line.0, line.1) {
                "\n\n"
            } else if prev != line {
                "\n"
            } else {
                " "
            });
        }
        last = Some(line);
        let word = f[11].trim();
        text.push_str(word);
        words.push(Word {
            text: word.into(),
            conf: f[10].trim().parse().unwrap_or(0.0),
            bbox: [n(6), n(7), n(8), n(9)],
        });
    }
    (text, words)
}
/// Pages ready to add: each file with its automatic cleanup, worked out off the UI thread.
pub type Prepared = Vec<(std::path::PathBuf, Option<Analysis>)>;
/// Opens and analyses each file. Files that aren't readable images are skipped and named in
/// the returned note, so one bad file doesn't lose the rest of a batch.
pub fn prepare(
    paths: Vec<std::path::PathBuf>,
    cleanup: bool,
    cancel: &AtomicBool,
) -> Result<(Prepared, Option<String>), String> {
    let mut out = vec![];
    let mut skipped = vec![];
    for p in paths {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled before all pages were added.".into());
        }
        match image::open(&p) {
            Ok(img) => out.push((p, cleanup.then(|| imaging::analyze(&img)))),
            Err(_) => skipped.push(
                p.file_name()
                    .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned()),
            ),
        }
    }
    if out.is_empty() && !skipped.is_empty() {
        return Err(format!("Not readable as images: {}", skipped.join(", ")));
    }
    let note = (!skipped.is_empty()).then(|| format!("skipped unreadable files: {}", skipped.join(", ")));
    Ok((out, note))
}
/// The page as a PNG for a vision model, base64-encoded, with the rendered page's pixel size.
pub(crate) fn page_png(p: &Page) -> Result<(String, u32, u32), String> {
    let full = p.render()?;
    let (w, h) = (full.width(), full.height());
    let mut bytes = std::io::Cursor::new(Vec::new());
    full.resize(2200, 3000, image::imageops::FilterType::Triangle)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok((base64::engine::general_purpose::STANDARD.encode(bytes.into_inner()), w, h))
}
/// Sends one prompt and one image to the configured vision model and returns its reply.
/// `json`: ask a local Ollama for JSON output (other providers are told in the prompt).
pub(crate) fn chat(s: &Settings, key: &str, prompt: &str, img: &str, json: bool, limit: Duration) -> Result<String, String> {
    if s.model.trim().is_empty() {
        return Err("Set a vision-capable model in Settings first.".into());
    }
    if s.provider == "Claude" {
        return crate::claude::chat(s.model.trim(), prompt, img, limit);
    }
    let local = s.provider == "Local Ollama";
    let base = s.endpoint.trim_end_matches('/');
    let (url, body) = if local {
        let mut body = json!({"model":s.model,"stream":false,"options":{"temperature":0.1},"messages":[{"role":"user","content":prompt,"images":[img]}]});
        if json {
            body["format"] = "json".into();
        }
        (format!("{base}/api/chat"), body)
    } else {
        (
            format!("{base}/chat/completions"),
            json!({"model":s.model,"messages":[{"role":"user","content":[{"type":"text","text":prompt},{"type":"image_url","image_url":{"url":format!("data:image/png;base64,{img}")}}]}],"temperature":0.1}),
        )
    };
    let env = if s.provider == "OpenRouter" {
        "OPENROUTER_API_KEY"
    } else if s.provider == "Ollama Cloud" {
        "OLLAMA_API_KEY"
    } else {
        "DOCUMENTASHION_AI_KEY"
    };
    let token = if key.is_empty() {
        std::env::var(env).unwrap_or_default()
    } else {
        key.into()
    };
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(limit))
        .build()
        .new_agent();
    let mut req = agent.post(&url);
    if !token.is_empty() {
        req = req.header("Authorization", &format!("Bearer {token}"));
    }
    let response = req
        .header("Content-Type", "application/json")
        .send(serde_json::to_vec(&body).map_err(|e| e.to_string())?)
        .map_err(|e| format!("AI request failed: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&response).map_err(|e| e.to_string())?;
    v.pointer(if local {
        "/message/content"
    } else {
        "/choices/0/message/content"
    })
    .and_then(Value::as_str)
    .filter(|t| !t.trim().is_empty())
    .map(str::to_owned)
    .ok_or_else(|| "The model returned no text. Check that it accepts images.".into())
}
pub fn ai(p: &Page, s: &Settings, key: &str, task: &str) -> Result<String, String> {
    let prompt = match task {
        "Rewrite" => format!(
            "Rewrite this school document clearly, preserving questions, facts, numbers, mathematical meaning and answer blanks. Do not answer exam questions. Mark uncertain handwriting [unclear]. Return only the rewritten text. Existing transcription:\n{}",
            p.text
        ),
        _ => format!(
            "Transcribe this page faithfully in English. Correct OCR mistakes by checking the image. Preserve question numbering, equations, answer blanks and line structure. Do not answer questions or invent unreadable writing; use [unclear]. Return only the transcription. Existing OCR:\n{}",
            p.text
        ),
    };
    let (img, _, _) = page_png(p)?;
    chat(s, key, &prompt, &img, false, Duration::from_secs(180))
}
const REBUILD_PROMPT: &str = r#"You are rebuilding a scanned school page as an exact digital copy.
The image is {W} pixels wide and {H} pixels tall.
Find every element on the page: titles, instructions, question numbers and text, printed equations, answer lines and boxes, tables, labels, and everything the student wrote by hand (answers, working, crossed-out work, notes).
Return ONLY a JSON object, no commentary, in this form:
{"blocks":[{"bbox":[left,top,right,bottom],"handwritten":false,"text":"...","line":40,"conf":0.95}]}
Rules:
- bbox: the element's left, top, right and bottom edges in PIXELS of this image. Be precise; the copy is drawn at these positions.
- One block per visual unit: one line or paragraph of printed text, one question, one answer line, one equation. Keep the student's writing in separate blocks from printed text, marked "handwritten": true.
- Text separated by a wide gap is separate blocks, so the copy keeps the gap: a question number and an answer set apart from it, each column of a list, each table cell. Never merge two columns into one block.
- text: exactly what is written, with line breaks as "\n". Write ALL mathematics as LaTeX inside $...$ (inline) or $$...$$ (a display equation on its own line), e.g. "2. Solve $x^2-5x+6=0$". Fractions \frac{a}{b}, roots \sqrt{x}, powers x^{2}, words inside math \text{...}.
- Transcribe handwriting exactly as written, including mistakes. Do not correct, solve or answer anything. Use [unclear] for writing you cannot read. Crossed-out work: wrap in ~~...~~.
- Answer blanks: "____". Checkboxes: "☐" or "☑". Bold text: wrap in **...**.
- A printed horizontal line or rule is its own block, with text "---" (a dotted line: "...") and the line's box.
- line: the height of one line of that text, in pixels.
- conf: your confidence in the reading of that block, 0 to 1. Be honest; messy handwriting should be low.
- Order blocks top to bottom, left to right."#;
/// Asks the model for a digital copy of the page. Its positions are then anchored to the lines
/// Tesseract finds on the same page, when Tesseract is installed.
pub fn rebuild(p: &Page, s: &Settings, key: &str, cancel: &AtomicBool) -> Result<Rebuild, String> {
    let (img, w, h) = page_png(p)?;
    let (iw, ih) = sent_size(w, h);
    let prompt = rebuild_prompt(p, iw, ih);
    // Tesseract reads the page while the model works.
    let page = p.clone();
    let cancel_ocr = Arc::new(AtomicBool::new(false));
    let c2 = cancel_ocr.clone();
    let ocr = std::thread::spawn(move || read_scan(&page, &c2));
    let reply = chat(s, key, &prompt, &img, true, Duration::from_secs(300));
    if cancel.load(Ordering::Relaxed) {
        cancel_ocr.store(true, Ordering::Relaxed);
    }
    let scan = ocr.join().unwrap_or_default();
    let mut blocks = parse_blocks(&reply?, iw as f32, ih as f32)?;
    let anchored = anchor(&mut blocks, &scan.lines);
    if let Some(ink) = &scan.ink {
        ink_extents(&mut blocks, ink);
    }
    Ok(Rebuild {
        blocks,
        geometry: p.geometry(),
        model: s.model.clone(),
        anchored,
        grade: None,
    })
}
fn rebuild_prompt(p: &Page, iw: u32, ih: u32) -> String {
    let mut prompt = REBUILD_PROMPT.replace("{W}", &iw.to_string()).replace("{H}", &ih.to_string());
    if !p.text.trim().is_empty() {
        prompt += "\nA transcription of the page, which may help with printed text:\n";
        prompt += &p.text;
    }
    prompt
}
/// The size `page_png` scales a `w`×`h` page to.
fn sent_size(w: u32, h: u32) -> (u32, u32) {
    let k = (2200.0 / w.max(1) as f64).min(3000.0 / h.max(1) as f64);
    ((w as f64 * k).round() as u32, (h as f64 * k).round() as u32)
}
/// Reads the model's JSON, tolerating code fences and text around it. Boxes arrive as pixel
/// edges of the `w`×`h` image sent; they are stored as thousandths of the page.
fn parse_blocks(reply: &str, w: f32, h: f32) -> Result<Vec<Block>, String> {
    let bad = || "The model's reply wasn't a page layout. Try again, or pick a stronger vision model.".to_string();
    let (a, b) = (reply.find('{').ok_or_else(bad)?, reply.rfind('}').ok_or_else(bad)?);
    let v: Value = serde_json::from_str(reply.get(a..=b).ok_or_else(bad)?).map_err(|_| bad())?;
    let list = v.get("blocks").and_then(Value::as_array).ok_or_else(bad)?;
    let num = |v: &Value| v.as_f64().map(|f| f as f32);
    let mut out = vec![];
    for item in list {
        let Some(text) = item.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) else { continue };
        let Some(bb) = item.get("bbox").and_then(Value::as_array).filter(|b| b.len() == 4) else { continue };
        let Some(e) = bb.iter().map(num).collect::<Option<Vec<f32>>>() else { continue };
        // Pixels as asked; some models answer in fractions of 1 instead.
        let (sx, sy) = if e.iter().all(|v| *v <= 1.0) { (1000.0, 1000.0) } else { (1000.0 / w, 1000.0 / h) };
        let (x0, x1) = (e[0].min(e[2]) * sx, e[0].max(e[2]) * sx);
        let (y0, y1) = (e[1].min(e[3]) * sy, e[1].max(e[3]) * sy);
        let (x0, y0) = (x0.clamp(0.0, 999.0), y0.clamp(0.0, 999.0));
        let bbox = [x0, y0, (x1 - x0).clamp(1.0, 1000.0 - x0), (y1 - y0).clamp(1.0, 1000.0 - y0)];
        out.push(Block {
            bbox,
            handwritten: item.get("handwritten").and_then(Value::as_bool).unwrap_or(false),
            text: text.to_owned(),
            line: (item.get("line").and_then(num).unwrap_or(0.0) * sy).clamp(0.0, 200.0),
            conf: item.get("conf").and_then(num).unwrap_or(1.0).clamp(0.0, 1.0),
        });
    }
    if out.is_empty() {
        return Err(bad());
    }
    Ok(out)
}
/// A line of printed text Tesseract found: its words and box in thousandths of the page.
#[derive(Clone, Debug)]
struct Line {
    text: String,
    bbox: [f32; 4],
}
/// Tesseract's text lines on the page, with boxes in thousandths of the rendered page.
/// What AI grade measures on the scan itself: Tesseract's runs of printed text, and the page
/// in grey, small, to find the student's ink.
#[derive(Default)]
struct Scan {
    lines: Vec<Line>,
    ink: Option<image::GrayImage>,
}
fn read_scan(p: &Page, cancel: &AtomicBool) -> Scan {
    let Ok(img) = p.render() else { return Scan::default() };
    let ink = Some(img.resize(600, 2400, image::imageops::FilterType::Triangle).to_luma8());
    let lines = ocr_status().and_then(|_| ocr_lines(p, &img, cancel)).unwrap_or_default();
    Scan { lines, ink }
}
fn ocr_lines(p: &Page, img: &image::DynamicImage, cancel: &AtomicBool) -> Result<Vec<Line>, String> {
    let (w, h) = (img.width().max(1) as f32, img.height().max(1) as f32);
    let path = std::env::temp_dir().join(format!("documentashion-{}.png", id()));
    img.save(&path).map_err(|e| e.to_string())?;
    let mut c = Command::new(tesseract());
    c.arg(&path)
        .args(["stdout", "-l", "eng", "--psm", "3", "--dpi"])
        .arg(p.dpi.max(70).to_string())
        .arg("tsv");
    let r = process_for(c, cancel, Duration::from_secs(120));
    let _ = std::fs::remove_file(path);
    Ok(tsv_lines(&r?, w, h))
}
/// Groups Tesseract's words into runs: one per line, split again where a wide gap separates
/// columns, so "(2) 5      (17) -10x + 30" gives "(2) 5" and "(17) -10x + 30".
fn tsv_lines(tsv: &str, w: f32, h: f32) -> Vec<Line> {
    let mut lines: Vec<((u32, u32, u32), f32, Line)> = vec![];
    for row in tsv.lines().skip(1) {
        let f: Vec<&str> = row.splitn(12, '\t').collect();
        if f.len() < 12 || f[0] != "5" || f[11].trim().is_empty() {
            continue;
        }
        let n = |i: usize| f[i].trim().parse::<f32>().unwrap_or(0.0);
        let id = (n(2) as u32, n(3) as u32, n(4) as u32);
        // Pixels: left, right, height.
        let (left, right, height) = (n(6), n(6) + n(8), n(9));
        let r = [n(6) / w * 1000.0, n(7) / h * 1000.0, n(8) / w * 1000.0, n(9) / h * 1000.0];
        match lines.last_mut() {
            Some((last, end, l)) if *last == id && left - *end < 1.2 * height.max(l.bbox[3] * h / 1000.0) => {
                let x1 = (l.bbox[0] + l.bbox[2]).max(r[0] + r[2]);
                let y1 = (l.bbox[1] + l.bbox[3]).max(r[1] + r[3]);
                l.bbox[0] = l.bbox[0].min(r[0]);
                l.bbox[1] = l.bbox[1].min(r[1]);
                l.bbox[2] = x1 - l.bbox[0];
                l.bbox[3] = y1 - l.bbox[1];
                l.text.push(' ');
                l.text.push_str(f[11].trim());
                *end = right;
            }
            _ => lines.push((id, right, Line { text: f[11].trim().into(), bbox: r })),
        }
    }
    lines.into_iter().map(|(_, _, l)| l).collect()
}
/// Words of a line for matching: lower-case letter and digit runs of two or more, with LaTeX
/// commands and math symbols dropped, so "$\\sqrt{49}$" and "√49" both give "49".
fn match_words(s: &str) -> Vec<String> {
    runs(s).into_iter().filter(|w| w.chars().count() >= 2).collect()
}
/// Every letter, digit and sign of a line, LaTeX commands dropped: "(1) $-4x - 12$" gives
/// "1-4x-12". Signs stay so "x = -9" and "x = 9" differ.
fn compact(s: &str) -> String {
    let mut out = String::new();
    let mut command = false;
    for c in s.chars() {
        if c == '\\' {
            command = true;
        } else if c.is_ascii_alphanumeric() {
            if !command {
                out.extend(c.to_lowercase());
            }
        } else {
            command = false;
            match c {
                '-' | '−' | '–' | '—' => out.push('-'),
                '+' | '=' => out.push(c),
                _ => {}
            }
        }
    }
    out
}
fn runs(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut command = false;
    for c in s.chars().chain([' ']) {
        if c == '\\' {
            command = true;
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else if c.is_alphanumeric() && c.is_ascii() {
            if !command {
                cur.extend(c.to_lowercase());
            }
        } else {
            command = false;
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        }
    }
    out
}
/// Lines up the model's layout with Tesseract's: printed blocks are matched to OCR lines by
/// their words, a scale and offset per axis is fitted from the matches and applied to every
/// block (handwriting included), and matched blocks take their line's exact place.
/// Returns false when there was nothing to anchor to.
fn anchor(blocks: &mut [Block], lines: &[Line]) -> bool {
    let mut pairs: Vec<(usize, usize)> = vec![];
    let mut used = vec![false; lines.len()];
    let lw: Vec<Vec<String>> = lines.iter().map(|l| match_words(&l.text)).collect();
    let mut scored = vec![];
    for (bi, b) in blocks.iter().enumerate().filter(|(_, b)| !b.handwritten) {
        let first = b.text.lines().next().unwrap_or("");
        let bw = match_words(first);
        if bw.is_empty() {
            continue;
        }
        for (li, words) in lw.iter().enumerate() {
            let shared = bw.iter().filter(|w| words.contains(w)).count();
            let long = bw.iter().any(|w| w.len() >= 3 && words.contains(w));
            let score = shared as f32 / bw.len().max(words.len()) as f32;
            if long && score >= 0.5 {
                scored.push((score, bi, li));
            }
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut taken = vec![false; blocks.len()];
    for (_, bi, li) in scored {
        if !taken[bi] && !used[li] {
            taken[bi] = true;
            used[li] = true;
            pairs.push((bi, li));
        }
    }
    if pairs.is_empty() {
        return false;
    }
    // Least squares on the top-left corners, per axis; an offset alone when the matches don't
    // spread enough to measure a scale.
    let fit = |model: Vec<f32>, truth: Vec<f32>| -> (f32, f32) {
        let n = model.len() as f32;
        let (mm, mt) = (model.iter().sum::<f32>() / n, truth.iter().sum::<f32>() / n);
        let var: f32 = model.iter().map(|m| (m - mm).powi(2)).sum();
        let cov: f32 = model.iter().zip(&truth).map(|(m, t)| (m - mm) * (t - mt)).sum();
        let spread = model.iter().cloned().fold(f32::MIN, f32::max) - model.iter().cloned().fold(f32::MAX, f32::min);
        if model.len() >= 2 && spread > 150.0 && var > 0.0 {
            let a = (cov / var).clamp(0.7, 1.4);
            (a, mt - a * mm)
        } else {
            (1.0, mt - mm)
        }
    };
    let (ax, bx) = fit(pairs.iter().map(|&(b, _)| blocks[b].bbox[0]).collect(), pairs.iter().map(|&(_, l)| lines[l].bbox[0]).collect());
    let (ay, by) = fit(pairs.iter().map(|&(b, _)| blocks[b].bbox[1]).collect(), pairs.iter().map(|&(_, l)| lines[l].bbox[1]).collect());
    for b in blocks.iter_mut() {
        let [x, y, w, h] = b.bbox;
        let (x, y) = ((ax * x + bx).clamp(0.0, 999.0), (ay * y + by).clamp(0.0, 999.0));
        b.bbox = [x, y, (ax * w).clamp(1.0, 1000.0 - x), (ay * h).clamp(1.0, 1000.0 - y)];
        b.line *= ay;
    }
    // Short printed blocks ("(1)", "6") have no words to match on: they take an unused run with
    // exactly their letters and digits, near where they sit. Unmatched blocks (these, handwriting,
    // print OCR misread) then move as their nearest matched neighbours did, since the model's
    // errors are local: a column or a row off together. Each round's matches pull the rest closer
    // for the next.
    let fitted: Vec<[f32; 2]> = blocks.iter().map(|b| [b.bbox[0], b.bbox[1]]).collect();
    let compacts: Vec<String> = lines.iter().map(|l| compact(&l.text)).collect();
    for _ in 0..3 {
        for bi in 0..blocks.len() {
            let b = &blocks[bi];
            let c = compact(&b.text);
            if b.handwritten || taken[bi] || c.is_empty() || b.text.lines().count() > 1 {
                continue;
            }
            // Exact for short text; long text (a printed equation Tesseract half misreads) may
            // differ in a few characters.
            let need = if c.chars().count() < 6 { 1.0 } else { 0.55 };
            let mut near: Vec<(f32, usize)> = (0..lines.len())
                .filter(|&li| !used[li])
                .map(|li| (li, similarity(&c, &compacts[li])))
                .filter(|&(_, sim)| sim >= need)
                .map(|(li, sim)| ((lines[li].bbox[0] - b.bbox[0]).abs(), (lines[li].bbox[1] - b.bbox[1]).abs(), sim, li))
                .filter(|&(dx, dy, ..)| dx < 60.0 && dy < 40.0)
                .map(|(dx, dy, sim, li)| (dx + 2.0 * dy + 40.0 * (1.0 - sim), li))
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            // The same answer often repeats ("x = 6"): take a run only when it is clearly the nearest.
            if let Some(&(cost, li)) = near.first()
                && near.get(1).is_none_or(|second| cost < second.0 * 0.5)
            {
                used[li] = true;
                taken[bi] = true;
                pairs.push((bi, li));
            }
        }
        let moved: Vec<([f32; 2], [f32; 2])> = pairs
            .iter()
            .map(|&(bi, li)| (fitted[bi], [lines[li].bbox[0] - fitted[bi][0], lines[li].bbox[1] - fitted[bi][1]]))
            .collect();
        for (bi, b) in blocks.iter_mut().enumerate() {
            if taken[bi] {
                continue;
            }
            let mut near: Vec<(f32, [f32; 2])> = moved
                .iter()
                .map(|(at, d)| (((at[0] - fitted[bi][0]).powi(2) + (at[1] - fitted[bi][1]).powi(2)).sqrt(), *d))
                .filter(|(dist, _)| *dist < 120.0)
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            near.truncate(3);
            let weight: f32 = near.iter().map(|(dist, _)| 1.0 / (dist + 5.0)).sum();
            if weight > 0.0 {
                let dx = near.iter().map(|(dist, d)| d[0] / (dist + 5.0)).sum::<f32>() / weight;
                let dy = near.iter().map(|(dist, d)| d[1] / (dist + 5.0)).sum::<f32>() / weight;
                b.bbox[0] = (fitted[bi][0] + dx).clamp(0.0, 999.0 - b.bbox[2].min(998.0));
                b.bbox[1] = (fitted[bi][1] + dy).clamp(0.0, 999.0 - b.bbox[3].min(998.0));
            }
        }
    }
    for (bi, li) in pairs {
        let b = &mut blocks[bi];
        let l = lines[li].bbox;
        if b.text.lines().count() <= 1 && !b.text.contains("$$") {
            // One line: the OCR box, a little taller so descenders and accents fit. A run that
            // only partly matches may have lost words: its width can't be trusted.
            let width = if similarity(&compact(&b.text), &compact(&lines[li].text)) >= 0.9 { l[2] } else { l[2].max(b.bbox[2]) };
            b.bbox = [l[0], l[1] - l[3] * 0.15, width.min(1000.0 - l[0]), l[3] * 1.3];
            b.line = l[3] * 1.3;
        } else {
            b.bbox[0] = l[0];
            b.bbox[1] = l[1];
        }
    }
    true
}
/// How alike two strings are: their longest common subsequence over the longer length, 0–1.
fn similarity(a: &str, b: &str) -> f32 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut row = vec![0u16; b.len() + 1];
    for x in &a {
        let mut diag = 0;
        for (j, y) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = if x == y { diag + 1 } else { up.max(row[j]) };
            diag = up;
        }
    }
    row[b.len()] as f32 / a.len().max(b.len()) as f32
}
/// Grows each block of the student's writing to cover the ink it belongs to. The model's boxes
/// for handwriting are loose; the ink isn't. Strokes outside printed text are grouped into
/// clusters, and each cluster goes to the handwriting block it overlaps most, or the nearest.
fn ink_extents(blocks: &mut [Block], gray: &image::GrayImage) {
    let (w, h) = (gray.width() as usize, gray.height() as usize);
    if w < 50 || h < 50 || !blocks.iter().any(|b| b.handwritten) {
        return;
    }
    let mut lum: Vec<u8> = gray.as_raw().clone();
    lum.select_nth_unstable(w * h / 2);
    let cut = (lum[w * h / 2] as i32 - 70).max(40) as u8;
    let mut ink: Vec<bool> = gray.as_raw().iter().map(|&v| v < cut).collect();
    let (sx, sy) = (w as f32 / 1000.0, h as f32 / 1000.0);
    let px = |b: &[f32; 4], pad: f32| {
        let x0 = ((b[0] - pad) * sx).max(0.0) as usize;
        let y0 = ((b[1] - pad) * sy).max(0.0) as usize;
        let x1 = (((b[0] + b[2] + pad) * sx).ceil() as usize).min(w);
        let y1 = (((b[1] + b[3] + pad) * sy).ceil() as usize).min(h);
        (x0, y0, x1, y1)
    };
    // Printed text is not the student's ink.
    for b in blocks.iter().filter(|b| !b.handwritten) {
        let (x0, y0, x1, y1) = px(&b.bbox, 3.0);
        for y in y0..y1 {
            ink[y * w + x0..y * w + x1].fill(false);
        }
    }
    // Strokes within a little of each other join: letters into words, lines of working into one.
    let r = (w as f32 * 0.008).ceil() as usize;
    let mut wide = vec![false; w * h];
    for y in 0..h {
        let mut last = None;
        for x in 0..w {
            if ink[y * w + x] {
                last = Some(x);
            }
            if last.is_some_and(|l| x - l <= r) {
                wide[y * w + x] = true;
            }
        }
        let mut last = None;
        for x in (0..w).rev() {
            if ink[y * w + x] {
                last = Some(x);
            }
            if last.is_some_and(|l| l - x <= r) {
                wide[y * w + x] = true;
            }
        }
    }
    let mut grown = vec![false; w * h];
    for x in 0..w {
        let mut last = None;
        for y in 0..h {
            if wide[y * w + x] {
                last = Some(y);
            }
            if last.is_some_and(|l| y - l <= r) {
                grown[y * w + x] = true;
            }
        }
        let mut last = None;
        for y in (0..h).rev() {
            if wide[y * w + x] {
                last = Some(y);
            }
            if last.is_some_and(|l| l - y <= r) {
                grown[y * w + x] = true;
            }
        }
    }
    // Clusters, by the box of their ink in thousandths of the page.
    let mut seen = vec![false; w * h];
    let mut clusters: Vec<[f32; 4]> = vec![];
    for start in 0..w * h {
        if !grown[start] || seen[start] {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1, mut count) = (w, h, 0, 0, 0usize);
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            if ink[i] {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                count += 1;
            }
            for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                if nx < w && ny < h && grown[ny * w + nx] && !seen[ny * w + nx] {
                    seen[ny * w + nx] = true;
                    stack.push(ny * w + nx);
                }
            }
        }
        // Specks, and scanner streaks running down the page, aren't writing.
        if count < 6 || y1 - y0 > h * 35 / 100 || x1 - x0 > w * 60 / 100 {
            continue;
        }
        clusters.push([x0 as f32 / sx, y0 as f32 / sy, (x1 - x0) as f32 / sx, (y1 - y0) as f32 / sy]);
    }
    let overlap = |a: &[f32; 4], b: &[f32; 4]| {
        let ix = ((a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0])).max(0.0);
        let iy = ((a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1])).max(0.0);
        ix * iy
    };
    let gap = |a: &[f32; 4], b: &[f32; 4]| {
        let dx = (a[0] - (b[0] + b[2])).max(b[0] - (a[0] + a[2])).max(0.0);
        let dy = (a[1] - (b[1] + b[3])).max(b[1] - (a[1] + a[3])).max(0.0);
        (dx * dx + dy * dy).sqrt()
    };
    let mut grown_boxes: Vec<Option<[f32; 4]>> = vec![None; blocks.len()];
    for c in &clusters {
        let hand = blocks.iter().enumerate().filter(|(_, b)| b.handwritten);
        let best = hand
            .clone()
            .map(|(i, b)| (i, overlap(c, &b.bbox)))
            .filter(|&(_, o)| o > 0.0)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
            .or_else(|| {
                hand.map(|(i, b)| (i, gap(c, &b.bbox)))
                    .filter(|&(_, d)| d < 45.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i)
            });
        if let Some(i) = best {
            let u = grown_boxes[i].get_or_insert(*c);
            let (x1, y1) = ((u[0] + u[2]).max(c[0] + c[2]), (u[1] + u[3]).max(c[1] + c[3]));
            u[0] = u[0].min(c[0]);
            u[1] = u[1].min(c[1]);
            u[2] = x1 - u[0];
            u[3] = y1 - u[1];
        }
    }
    for (b, g) in blocks.iter_mut().zip(grown_boxes) {
        if let Some(g) = g {
            b.bbox = g;
        }
    }
}
/// Models a provider offers. `vision` is false when the provider can't say which accept images,
/// in which case every model is listed.
pub struct Models {
    pub list: Vec<String>,
    pub vision: bool,
    /// A line to show under the list (for Claude: which sign-in it uses).
    pub note: Option<String>,
}
fn token_for(s: &Settings, key: &str) -> String {
    if !key.is_empty() {
        return key.into();
    }
    let env = match s.provider.as_str() {
        "OpenRouter" => "OPENROUTER_API_KEY",
        "Ollama Cloud" => "OLLAMA_API_KEY",
        _ => "DOCUMENTASHION_AI_KEY",
    };
    std::env::var(env).unwrap_or_default()
}
/// Asks the provider which models it has, keeping those that accept images where it says so.
pub fn models(s: &Settings, key: &str) -> Result<Models, String> {
    if s.provider == "Claude" {
        return crate::claude::models();
    }
    let token = token_for(s, key);
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .new_agent();
    let get = |url: &str| -> Result<Value, String> {
        let mut req = agent.get(url);
        if !token.is_empty() {
            req = req.header("Authorization", &format!("Bearer {token}"));
        }
        let body = req
            .call()
            .map_err(|e| format!("Couldn't reach {url}: {e}"))?
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&body).map_err(|e| e.to_string())
    };
    let base = s.endpoint.trim_end_matches('/');
    let mut list: Vec<String>;
    let mut vision = true;
    if s.provider == "Local Ollama" || s.provider == "Ollama Cloud" {
        let root = base.trim_end_matches("/v1");
        let tags = get(&format!("{root}/api/tags"))?;
        let names: Vec<String> = tags["models"]
            .as_array()
            .map(|a| a.iter().filter_map(|m| m["name"].as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        list = vec![];
        for name in names {
            let show = agent
                .post(&format!("{root}/api/show"))
                .header("Content-Type", "application/json")
                .header("Authorization", &format!("Bearer {token}"))
                .send(json!({"model": name}).to_string().into_bytes())
                .ok()
                .and_then(|mut r| r.body_mut().read_to_string().ok())
                .and_then(|b| serde_json::from_str::<Value>(&b).ok());
            match show.as_ref().and_then(|v| v["capabilities"].as_array()) {
                Some(c) => {
                    if c.iter().any(|c| c == "vision") {
                        list.push(name);
                    }
                }
                None => {
                    // An older server that doesn't report capabilities: list everything.
                    vision = false;
                    list.push(name);
                }
            }
        }
    } else {
        let v = get(&format!("{base}/models"))?;
        let items = v["data"].as_array().cloned().unwrap_or_default();
        list = vec![];
        for m in &items {
            let Some(id) = m["id"].as_str() else { continue };
            match m.pointer("/architecture/input_modalities").and_then(Value::as_array) {
                Some(mods) => {
                    if mods.iter().any(|x| x == "image") {
                        list.push(id.to_owned());
                    }
                }
                None => {
                    vision = false;
                    list.push(id.to_owned());
                }
            }
        }
        if !vision {
            // Mixed or unknown: show everything rather than hide a usable model.
            list = items.iter().filter_map(|m| m["id"].as_str().map(str::to_owned)).collect();
        }
    }
    list.sort();
    list.dedup();
    if list.is_empty() {
        return Err("The provider listed no models with image input.".into());
    }
    Ok(Models { list, vision, note: None })
}
pub type Cancel = Arc<AtomicBool>;
pub fn import_pdf(
    path: &std::path::Path,
    cancel: &AtomicBool,
) -> Result<Vec<std::path::PathBuf>, String> {
    let dir = root().join("imports").join(id());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut c = Command::new("pdftoppm");
    c.args(["-png", "-r", "300"])
        .arg(path)
        .arg(dir.join("page"));
    process(c, cancel)?;
    let mut pages: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    pages.sort();
    Ok(pages)
}
#[cfg(test)]
mod tests {
    use super::*;
    /// Rebuilds a real page with a vision model:
    /// `DOCUMENTASHION_LIVE_IMAGE=page.png DOCUMENTASHION_LIVE_MODEL=deepseek-v4.1-flash:1017-cloud cargo test --release live_rebuild -- --ignored`
    /// writes the result next to the image as `<image>.rebuild.json`, a project as `<image>.project.json`,
    /// and the model's reply as `<image>.reply.txt`. With `DOCUMENTASHION_LIVE_REUSE` set, the saved
    /// reply is laid out again without asking the model, for working on the anchoring.
    #[test]
    #[ignore]
    fn live_rebuild() {
        let img = std::env::var("DOCUMENTASHION_LIVE_IMAGE").unwrap();
        let page = Page { id: "live".into(), image: img.clone().into(), dpi: 300, ..Default::default() };
        let s = Settings { model: std::env::var("DOCUMENTASHION_LIVE_MODEL").unwrap_or_default(), provider: std::env::var("DOCUMENTASHION_LIVE_PROVIDER").unwrap_or_else(|_| "Local Ollama".into()), ..Default::default() };
        let t = Instant::now();
        let saved = format!("{img}.reply.txt");
        let (png, w, h) = page_png(&page).unwrap();
        let (iw, ih) = sent_size(w, h);
        let reply = match std::env::var_os("DOCUMENTASHION_LIVE_REUSE") {
            Some(_) => std::fs::read_to_string(&saved).unwrap(),
            None => {
                let r = chat(&s, "", &rebuild_prompt(&page, iw, ih), &png, true, Duration::from_secs(300)).unwrap();
                std::fs::write(&saved, &r).unwrap();
                r
            }
        };
        let mut blocks = parse_blocks(&reply, iw as f32, ih as f32).unwrap();
        let scan = read_scan(&page, &AtomicBool::new(false));
        let anchored = anchor(&mut blocks, &scan.lines);
        ink_extents(&mut blocks, scan.ink.as_ref().unwrap());
        let r = Rebuild { blocks, geometry: page.geometry(), model: s.model.clone(), anchored, grade: None };
        eprintln!("anchored: {}", r.anchored);
        eprintln!("{} blocks in {:.0?}", r.blocks.len(), t.elapsed());
        std::fs::write(format!("{img}.rebuild.json"), serde_json::to_vec_pretty(&r).unwrap()).unwrap();
        let project = Project { id: "live".into(), name: "Live rebuild".into(), pages: vec![page], key: vec![] };
        std::fs::write(format!("{img}.project.json"), serde_json::to_vec(&project).unwrap()).unwrap();
    }
    #[test]
    fn rebuild_reply_is_parsed_through_fences() {
        let reply = "Here you go:\n```json\n{\"blocks\":[{\"bbox\":[200,400,1200,500],\"handwritten\":true,\"text\":\"$x=4$\",\"conf\":0.4},{\"bbox\":[1,2,3],\"text\":\"bad box\"}]}\n```";
        let b = parse_blocks(reply, 2000.0, 2000.0).unwrap();
        assert_eq!(b.len(), 1);
        assert!(b[0].handwritten && b[0].conf < 0.5);
        assert_eq!(b[0].bbox, [100.0, 200.0, 500.0, 50.0]);
        assert!(parse_blocks("no layout here", 1.0, 1.0).is_err());
    }
    #[test]
    fn compact_keeps_signs_and_drops_latex() {
        assert_eq!(compact("(1) $-4x - 12$"), "1-4x-12");
        assert_eq!(compact("x=—9"), compact("$x = -9$"));
        assert_ne!(compact("$x = -9$"), compact("$x = 9$"));
        assert_eq!(compact(r"$\frac{1}{2}$"), "12");
    }
    #[test]
    fn match_words_ignore_latex() {
        assert_eq!(match_words(r"3. Evaluate $\sqrt{49} + 2^3$"), ["evaluate", "49"]);
        assert_eq!(match_words("3. Evaluate √49 + 2³"), ["evaluate", "49"]);
    }
    #[test]
    fn model_layout_is_anchored_to_ocr_lines() {
        // The model squeezed the page vertically by 0.9 and shifted it up; OCR has the truth.
        let truth = [(160.0, "1. Solve for x"), (305.0, "2. Simplify this"), (441.0, "3. Evaluate the sum"), (578.0, "4. What is the slope")];
        let lines: Vec<Line> = truth.iter().map(|(y, t)| Line { text: t.to_string(), bbox: [94.0, *y, 300.0, 13.0] }).collect();
        let mut blocks: Vec<Block> = truth
            .iter()
            .map(|(y, t)| Block { bbox: [88.0, y * 0.9 - 10.0, 300.0, 14.0], text: t.to_string(), ..Default::default() })
            .collect();
        blocks.push(Block { bbox: [120.0, 618.0 * 0.9 - 10.0, 100.0, 20.0], handwritten: true, text: "slope = -7".into(), ..Default::default() });
        assert!(anchor(&mut blocks, &lines));
        assert!((blocks[3].bbox[1] - (578.0 - 13.0 * 0.15)).abs() < 0.5);
        // The answer, which OCR can't anchor, moves with the fitted scale.
        assert!((blocks[4].bbox[1] - 618.0).abs() < 2.0, "{:?}", blocks[4].bbox);
        assert!(!anchor(&mut blocks.clone(), &[]));
    }
    #[test]
    fn scanner_process_failure_is_reported() {
        let result = process(Command::new("false"), &AtomicBool::new(false));
        assert!(result.unwrap_err().contains("Command failed"));
    }
    #[test]
    fn unreadable_files_are_skipped_not_fatal() {
        let dir = std::env::temp_dir().join(format!("doc-prep-{}", id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (good, bad) = (dir.join("good.png"), dir.join("notes.txt"));
        image::RgbImage::from_pixel(40, 40, image::Rgb([255, 255, 255])).save(&good).unwrap();
        std::fs::write(&bad, "not an image").unwrap();
        let (pages, note) = prepare(vec![good, bad.clone()], true, &AtomicBool::new(false)).unwrap();
        assert_eq!(pages.len(), 1);
        assert!(note.unwrap().contains("notes.txt"));
        assert!(prepare(vec![bad], false, &AtomicBool::new(false)).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn tesseract_tsv_becomes_lines_and_boxes() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t2550\t3300\t-1\t\n\
5\t1\t1\t1\t1\t1\t100\t200\t180\t40\t96.5\tName:\n\
5\t1\t1\t1\t1\t2\t300\t200\t120\t40\t41.0\t___\n\
5\t1\t1\t1\t2\t1\t100\t260\t90\t40\t93.1\tDate\n\
5\t1\t2\t1\t1\t1\t100\t400\t60\t40\t88.0\t1.\n\
5\t1\t2\t1\t1\t2\t170\t400\t60\t40\t-1\t \n";
        let (text, words) = parse_tsv(tsv);
        assert_eq!(text, "Name: ___\nDate\n\n1.");
        assert_eq!(words.len(), 4);
        assert_eq!(words[1].bbox, [300, 200, 120, 40]);
        assert_eq!(words[1].conf, 41.0);
    }
    #[test]
    fn cancelled_process_is_reaped() {
        let mut c = Command::new("sleep");
        c.arg("20");
        let start = Instant::now();
        let result = process(c, &AtomicBool::new(true));
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    /// A worksheet rendered at 300 DPI, fed in tilted with scanner background along one edge,
    /// taken through cleanup, Tesseract, a correction and PDF export.
    /// Needs `pdftoppm`, `pdftotext` and `tesseract` with English: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn tilted_worksheet_end_to_end() {
        use crate::{imaging, pdf};
        let dir = std::env::temp_dir().join(format!("doc-e2e-{}", id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lines = [
            "Algebra Worksheet 4",
            "Name: Jordan Lee      Date: March 12",
            "1. Solve for x when 3x + 5 = 20",
            "2. Simplify the expression 4(a + 2) - 3a",
            "3. A train travels 180 miles in 3 hours.",
            "   What is its average speed?",
            "4. Graph the line y = 2x - 1",
        ];
        let mut content = String::from("BT /F1 16 Tf 72 700 Td 22 TL\n");
        for l in lines {
            content.push_str(&format!("({l}) Tj T*\n"));
        }
        content.push_str("ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".into(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut doc = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![];
        for (i, o) in objects.iter().enumerate() {
            offsets.push(doc.len());
            doc.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = doc.len();
        doc.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for o in offsets {
            doc.extend(format!("{o:010} 00000 n \n").bytes());
        }
        doc.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
        std::fs::write(dir.join("sheet.pdf"), doc).unwrap();
        let pages = import_pdf(&dir.join("sheet.pdf"), &AtomicBool::new(false)).unwrap();
        let flat = image::open(&pages[0]).unwrap().to_rgb8();
        assert_eq!(flat.dimensions(), (2550, 3300));
        // Feed it in 2.5 degrees clockwise with a dark band of scanner background on the left.
        let mut fed = imaging::rotate(&flat, 2.5);
        for y in 0..fed.height() {
            for x in 0..70 {
                fed.put_pixel(x, y, image::Rgb([25, 25, 28]));
            }
        }
        let scan = dir.join("scan.png");
        fed.save(&scan).unwrap();

        let started = Instant::now();
        let (prepared, _) = prepare(vec![scan], true, &AtomicBool::new(false)).unwrap();
        println!("analysis took {:?}", started.elapsed());
        let mut project = Project::new();
        project.id = format!("e2e-{}", id());
        let (path, cleanup) = prepared.into_iter().next().unwrap();
        let a = cleanup.unwrap();
        println!("analysis: {a:?}");
        assert!((a.skew - 2.5).abs() < 0.2, "skew {}", a.skew);
        assert!(a.crop[0] > 0.025 && a.crop[0] < 0.06, "crop {:?}", a.crop);
        assert!(!a.blank);
        let tmp_root = dir.join("data");
        // SAFETY: tests in this module don't read XDG_DATA_HOME concurrently with this one.
        unsafe { std::env::set_var("XDG_DATA_HOME", &tmp_root) };
        project.import(path, 300, Some(a)).unwrap();
        let mut page = project.pages[0].clone();
        assert!(page.straighten && page.clean);

        let started = Instant::now();
        let reading = ocr(&page, &AtomicBool::new(false)).unwrap();
        println!("OCR took {:?}\n{}", started.elapsed(), reading.text);
        assert!(reading.text.contains("Algebra Worksheet 4"));
        assert!(reading.text.contains("180 miles"));
        let unsure = reading.words.iter().filter(|w| w.conf < crate::review::UNSURE).count();
        println!("{} words, {unsure} unsure", reading.words.len());

        // Accept OCR, then a teacher's correction: "March 12" becomes "March 13".
        page.text = reading.text.replace("March 12", "March 13");
        page.words = reading.words.clone();
        page.words_geometry = reading.geometry.clone();
        let rendered = page.render().unwrap();
        let out = dir.join("out.pdf");
        pdf::export(&Project { pages: vec![page.clone()], ..project.clone() }, &out).unwrap();
        let text = Command::new("pdftotext").arg(&out).arg("-").output().unwrap();
        let text = String::from_utf8_lossy(&text.stdout);
        assert!(text.contains("March 13") && text.contains("Simplify the expression"), "{text}");
        // The corrected word sits where the OCR saw the original one.
        let ocr_box = reading.words.iter().find(|w| w.text == "12").unwrap().bbox;
        let bbox = Command::new("pdftotext").arg("-bbox").arg(&out).arg("-").output().unwrap();
        let bbox = String::from_utf8_lossy(&bbox.stdout);
        let line = bbox.lines().find(|l| l.ends_with(">13</word>")).unwrap();
        let xmin: f32 = line.split('"').nth(1).unwrap().parse().unwrap();
        let expected = ocr_box[0] as f32 * 72.0 / 300.0;
        assert!((xmin - expected).abs() < 2.0, "13 at {xmin}pt, OCR box at {expected}pt");
        println!("rendered {}x{}", rendered.width(), rendered.height());
        // DOCUMENTASHION_E2E_KEEP=<dir> keeps the project for a headless UI preview.
        if let Some(keep) = std::env::var_os("DOCUMENTASHION_E2E_KEEP") {
            let keep = std::path::PathBuf::from(keep);
            std::fs::create_dir_all(&keep).unwrap();
            let image = keep.join("scan.png");
            std::fs::copy(&page.image, &image).unwrap();
            page.image = image;
            page.text = reading.text.clone();
            let kept = Project { name: "Algebra Worksheet 4".into(), pages: vec![page], ..project };
            std::fs::write(keep.join("project.json"), serde_json::to_vec_pretty(&kept).unwrap()).unwrap();
            std::fs::write(
                keep.join("proposal.txt"),
                reading.text.replace("Name:JordanLee", "Name: Jordan Lee").replace("March 12", "March 13")
                    .replace("Whatis", "What is").replace("y = 2x - 1", "y = 2x + 1 [unclear]"),
            )
            .unwrap();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
