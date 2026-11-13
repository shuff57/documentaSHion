use crate::{
    diff,
    model::{Page, Project},
};
use std::path::Path;
fn font_objects(p: &Project) -> Vec<Vec<u8>> {
    let font = include_bytes!("../assets/SourceSans3-Regular.ttf");
    let face = ttf_parser::Face::parse(font, 0).unwrap();
    let mut mapping = std::collections::BTreeMap::new();
    for c in p.pages.iter().flat_map(|p| p.text.chars()).chain([' ']) {
        if let Some(g) = face.glyph_index(c) {
            mapping.insert(g.0, c);
        }
    }
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /DocUnicode def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    let entries: Vec<_> = mapping.into_iter().collect();
    for chunk in entries.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (g, c) in chunk {
            let mut buf = [0u16; 2];
            let hex = c
                .encode_utf16(&mut buf)
                .iter()
                .map(|n| format!("{n:04X}"))
                .collect::<String>();
            cmap.push_str(&format!("<{g:04X}> <{hex}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap CMapName currentdict /CMap defineresource pop end end");
    let mut fontstream = format!(
        "<< /Length {} /Length1 {} >>\nstream\n",
        font.len(),
        font.len()
    )
    .into_bytes();
    fontstream.extend(font);
    fontstream.extend(b"\nendstream");
    vec![vec![],vec![],b"<< /Type /Font /Subtype /Type0 /BaseFont /SourceSans3 /Encoding /Identity-H /DescendantFonts [4 0 R] /ToUnicode 7 0 R >>".to_vec(),b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SourceSans3 /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 5 0 R /CIDToGIDMap /Identity /DW 500 >>".to_vec(),b"<< /Type /FontDescriptor /FontName /SourceSans3 /Flags 32 /FontBBox [-1000 -500 2000 1500] /ItalicAngle 0 /Ascent 1000 /Descent -300 /CapHeight 700 /StemV 80 /FontFile2 6 0 R >>".to_vec(),fontstream,format!("<< /Length {} >>\nstream\n{cmap}\nendstream",cmap.len()).into_bytes()]
}
fn encoded(s: &str) -> String {
    let face =
        ttf_parser::Face::parse(include_bytes!("../assets/SourceSans3-Regular.ttf"), 0).unwrap();
    s.chars()
        .map(|c| format!("{:04X}", face.glyph_index(c).map(|g| g.0).unwrap_or(0)))
        .collect()
}
/// Every glyph is declared 500 units wide (/DW 500), so this is the width viewers use for selection.
const GLYPH_WIDTH: f32 = 0.5;
fn horizontal_scale(text: &str, size: f32, width: f32) -> f32 {
    (100.0 * width / (text.chars().count().max(1) as f32 * GLYPH_WIDTH * size)).clamp(1.0, 1000.0)
}
/// Invisible text over the page image. With current OCR boxes each word sits on its word in
/// the scan, so search hits and selection line up; otherwise lines are spread down the page.
fn text_layer(page: &Page, pw: f32, ph: f32, out: &mut String) {
    let scale = 72.0 / page.dpi.max(1) as f32;
    if page.words_current() {
        let placed = diff::place(&page.words, &page.text);
        if placed.iter().all(|p| p.bbox.is_some()) {
            let last = placed.len().saturating_sub(1);
            for (n, p) in placed.into_iter().enumerate() {
                let [x, y, w, h] = p.bbox.unwrap().map(|v| v as f32 * scale);
                // A trailing space keeps neighbouring words apart in search and copied text,
                // even where OCR boxes touch. The word itself is scaled to fill its box.
                let shown = if n < last { format!("{} ", p.text) } else { p.text.to_string() };
                let size = h.clamp(1.0, 200.0);
                // The box bottom includes descenders; the baseline sits a little above it.
                let baseline = ph - (y + h) + size * 0.2;
                out.push_str(&format!(
                    "/F1 {size:.2} Tf {:.2} Tz 1 0 0 1 {x:.2} {baseline:.2} Tm <{}> Tj\n",
                    horizontal_scale(p.text, size, w),
                    encoded(&shown)
                ));
            }
            return;
        }
    }
    let lines: Vec<_> = page.text.lines().collect();
    let step = (ph - 24.0) / lines.len().max(1) as f32;
    out.push_str("/F1 8 Tf\n");
    for (n, line) in lines.iter().enumerate() {
        out.push_str(&format!(
            "{:.2} Tz 1 0 0 1 12 {:.2} Tm <{}> Tj\n",
            horizontal_scale(line, 8.0, pw - 24.0).min(100.0),
            ph - 16.0 - n as f32 * step,
            encoded(line)
        ));
    }
}
pub fn export(p: &Project, path: &Path) -> Result<(), String> {
    if p.pages.is_empty() {
        return Err("Add pages before exporting.".into());
    }
    let mut objects = font_objects(p);
    let mut kids = vec![];
    for page in &p.pages {
        let img = page.render()?.to_rgb8();
        let (w, h) = img.dimensions();
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 90)
            .encode(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
            .map_err(|e| e.to_string())?;
        let image_id = objects.len() + 1;
        let mut stream=format!("<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",jpg.len()).into_bytes();
        stream.extend(jpg);
        stream.extend(b"\nendstream");
        objects.push(stream);
        let pw = w as f32 * 72.0 / page.dpi.max(1) as f32;
        let ph = h as f32 * 72.0 / page.dpi.max(1) as f32;
        let mut content = format!("q {pw} 0 0 {ph} 0 0 cm /Im Do Q\nBT 3 Tr\n");
        text_layer(page, pw, ph, &mut content);
        content.push_str("ET");
        let content_id = objects.len() + 1;
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
            .into_bytes(),
        );
        let page_id = objects.len() + 1;
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw} {ph}] /Resources << /Font << /F1 3 0 R >> /XObject << /Im {image_id} 0 R >> >> /Contents {content_id} 0 R >>").into_bytes());
        kids.push(format!("{page_id} 0 R"));
    }
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    objects[1] = format!(
        "<< /Type /Pages /Count {} /Kids [{}] >>",
        kids.len(),
        kids.join(" ")
    )
    .into_bytes();
    let mut bytes = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0];
    for (i, o) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend(o);
        bytes.extend(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for n in offsets.iter().skip(1) {
        bytes.extend(format!("{n:010} 00000 n \n").as_bytes());
    }
    bytes.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    #[test]
    fn exported_pdf_is_readable_and_searchable() {
        let dir = std::env::temp_dir().join(format!("doc-test-{}", id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("page.png");
        image::RgbImage::from_pixel(300, 400, image::Rgb([255, 255, 255]))
            .save(&path)
            .unwrap();
        let p = Project {
            id: "test".into(),
            name: "Test".into(),
            key: vec![],
            pages: vec![Page {
                image: path,
                text: "Algebra worksheet (2026) — café".into(),
                dpi: 300,
                ..Default::default()
            }],
        };
        let pdf = dir.join("test.pdf");
        export(&p, &pdf).unwrap();
        let output = std::process::Command::new("pdftotext")
            .arg(&pdf)
            .arg("-")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("Algebra worksheet (2026) — café")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn ocr_words_are_placed_on_their_boxes() {
        let dir = std::env::temp_dir().join(format!("doc-test-{}", id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("page.png");
        // 300 DPI: 1 pixel = 0.24 pt.
        image::RgbImage::from_pixel(2550, 3300, image::Rgb([255, 255, 255]))
            .save(&path)
            .unwrap();
        let mut page = Page {
            image: path,
            text: "Quiz 3\nSolve for x".into(),
            dpi: 300,
            words: ["Quiz", "2", "Solve", "for", "x"]
                .iter()
                .zip([(300, 400), (500, 400), (300, 600), (500, 600), (660, 600)])
                .map(|(t, (x, y))| Word {
                    text: t.to_string(),
                    conf: 90.0,
                    bbox: [x, y, if t.len() > 1 { 150 } else { 50 }, 50],
                })
                .collect(),
            ..Default::default()
        };
        page.words_geometry = page.geometry();
        let p = Project {
            id: "test".into(),
            name: "Test".into(),
            key: vec![],
            pages: vec![page],
        };
        let pdf = dir.join("test.pdf");
        export(&p, &pdf).unwrap();
        let out = std::process::Command::new("pdftotext")
            .args(["-bbox"])
            .arg(&pdf)
            .arg("-")
            .output()
            .unwrap();
        let html = String::from_utf8_lossy(&out.stdout);
        // The corrected "3" takes the place of the OCR's "2".
        for (word, x) in [("Quiz", 72.0), ("3", 120.0), ("Solve", 72.0), ("x", 158.4)] {
            let line = html
                .lines()
                .find(|l| l.ends_with(&format!(">{word}</word>")))
                .unwrap_or_else(|| panic!("{word} missing from {html}"));
            let xmin: f32 = line.split('"').nth(1).unwrap().parse().unwrap();
            assert!((xmin - x).abs() < 1.5, "{word} at {xmin}, expected {x}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
