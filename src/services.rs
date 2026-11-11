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
pub fn process(mut cmd: Command, cancel: &AtomicBool) -> Result<String, String> {
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
        if cancel.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(300) {
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
pub fn ai(p: &Page, s: &Settings, key: &str, task: &str) -> Result<String, String> {
    if s.model.trim().is_empty() {
        return Err("Set a vision-capable model in Settings first.".into());
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    p.render()?
        .resize(2200, 3000, image::imageops::FilterType::Triangle)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let img = base64::engine::general_purpose::STANDARD.encode(bytes.into_inner());
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
    let local = s.provider == "Local Ollama";
    let base = s.endpoint.trim_end_matches('/');
    let (url, body) = if local {
        (
            format!("{base}/api/chat"),
            json!({"model":s.model,"stream":false,"messages":[{"role":"user","content":prompt,"images":[img]}]}),
        )
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
        .timeout_global(Some(Duration::from_secs(180)))
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
