//! Writes a [`Sheet`] as a `.docx` (WordprocessingML). Sizes in the sheet are points; Word
//! measures paragraph spacing, tabs and widths in twentieths of a point (twips), font sizes in
//! half points and drawings in EMU.
use super::*;
use std::{fmt::Write as _, io::Write as _, path::Path};

const TWIP: f32 = 20.0;
const EMU: f32 = 12700.0;

fn tw(pt: f32) -> i64 {
    (pt * TWIP).round() as i64
}
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The pictures of one part (body, header or footer): each needs a relationship in that part.
struct Media {
    /// Every picture in the package, named `imageN.png`.
    files: Vec<(String, Vec<u8>)>,
    /// This part's relationships: id and file name.
    rels: Vec<(String, String)>,
}
impl Media {
    /// Adds a PNG and returns its relationship id in the current part.
    fn add(&mut self, png: Vec<u8>) -> String {
        let name = format!("image{}.png", self.files.len() + 1);
        self.files.push((name.clone(), png));
        let rid = format!("rIdImg{}", self.files.len());
        self.rels.push((rid.clone(), name));
        rid
    }
    /// The current part's relationships file, and a fresh list for the next part.
    fn take_rels(&mut self, extra: &str) -> String {
        let mut rels = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        );
        rels.push_str(extra);
        for (rid, name) in self.rels.drain(..) {
            let _ = write!(
                rels,
                r#"<Relationship Id="{rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{name}"/>"#
            );
        }
        rels.push_str("</Relationships>");
        rels
    }
}

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#;

type Draw<'a> = &'a dyn Fn(&Figure) -> Result<Vec<u8>, String>;

/// Builds the document and writes it to `out`. `figure` draws a [`Figure`] as PNG bytes.
pub fn write(sheet: &Sheet, out: &Path, figure: Draw) -> Result<(), String> {
    write_as(sheet, out, figure, false)
}

/// The text of the marker [`write_as`] puts at the start of each paragraph, spacer and figure
/// when probing, numbered in [`super::anchors`] order:
/// `part` is `B` (body), `H` (header) or `F` (footer).
///
/// The marker is drawn with no advance, so its letters overlap and a PDF reader may give them
/// in any order; each digit therefore has its own letters: hundreds `0`–`9`, tens `a`–`j`,
/// units `k`–`t`.
pub fn marker(part: char, i: usize) -> String {
    let digit = |base: u8, d: usize| char::from(base + (d % 10) as u8);
    let hundreds = if i >= 100 { digit(b'0', i / 100).to_string() } else { String::new() };
    format!("Q{part}{hundreds}{}{}Z", digit(b'a', i / 10), digit(b'k', i))
}

/// Like [`write`]; `probe` starts every paragraph, spacer and figure, in tables too, with a
/// white one-point [`marker`] of
/// no width, so the block's first line can be found in a PDF of the document.
pub fn write_as(sheet: &Sheet, out: &Path, figure: Draw, probe: bool) -> Result<(), String> {
    // The next marker of each part, when probing.
    let mut head_mark = probe.then_some(('H', 0));
    let mut foot_mark = probe.then_some(('F', 0));
    let mut body_mark = probe.then_some(('B', 0));
    let mut media = Media { files: vec![], rels: vec![] };
    let mut ids = 1u32;
    let pg = &sheet.page;
    let (w, h) = if pg.landscape { (pg.width.max(pg.height), pg.width.min(pg.height)) } else { (pg.width, pg.height) };
    let width = pg.text_width();

    // Header: the graph paper, anchored to the page behind the text, then the header blocks.
    let mut header = String::new();
    // Not when probing: the fit compares what is printed on the sheet, not the paper.
    if let Some(g) = pg.background.as_ref().filter(|_| !probe) {
        let rid = media.add(grid_png(g, w, h)?);
        let _ = write!(header, r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="20" w:lineRule="exact"/></w:pPr>"#);
        behind(&mut header, w, h, &rid, &mut ids);
        header.push_str("</w:p>");
    }
    for b in &sheet.header {
        block(&mut header, b, sheet, width, &mut media, &mut ids, figure, &mut head_mark)?;
    }
    let has_header = !header.is_empty();
    let header_rels = media.take_rels("");
    let mut footer = String::new();
    for b in &sheet.footer {
        block(&mut footer, b, sheet, width, &mut media, &mut ids, figure, &mut foot_mark)?;
    }
    let has_footer = !footer.is_empty();
    let footer_rels = media.take_rels("");

    let mut body = String::new();
    for b in &sheet.blocks {
        block(&mut body, b, sheet, width, &mut media, &mut ids, figure, &mut body_mark)?;
    }
    // A body must end with a paragraph, and one after a final table keeps it editable.
    body.push_str(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="20" w:lineRule="exact"/></w:pPr></w:p>"#);
    let [mt, mr, mb, ml] = pg.margins;
    let orient = if pg.landscape { r#" w:orient="landscape""# } else { "" };
    let mut refs = String::new();
    let mut extra = String::new();
    if has_header {
        refs.push_str(r#"<w:headerReference w:type="default" r:id="rIdHeader"/>"#);
        extra.push_str(r#"<Relationship Id="rIdHeader" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#);
    }
    if has_footer {
        refs.push_str(r#"<w:footerReference w:type="default" r:id="rIdFooter"/>"#);
        extra.push_str(r#"<Relationship Id="rIdFooter" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>"#);
    }
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document {NS}><w:body>{body}<w:sectPr>{refs}<w:pgSz w:w="{}" w:h="{}"{orient}/><w:pgMar w:top="{}" w:right="{}" w:bottom="{}" w:left="{}" w:header="{}" w:footer="{}" w:gutter="0"/></w:sectPr></w:body></w:document>"#,
        tw(w),
        tw(h),
        tw(mt),
        tw(mr),
        tw(mb),
        tw(ml),
        tw(pg.header_at),
        tw(pg.footer_at)
    );
    extra.push_str(r#"<Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#);
    let rels = media.take_rels(&extra);
    let font = esc(&sheet.font);
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="{font}" w:hAnsi="{font}" w:cs="{font}" w:eastAsia="{font}"/><w:sz w:val="{s}"/><w:szCs w:val="{s}"/><w:lang w:val="en-US"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style><w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style></w:styles>"#,
        s = (sheet.size * 2.0).round() as i64
    );
    let mut types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>"#,
    );
    if has_header {
        types.push_str(r#"<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>"#);
    }
    if has_footer {
        types.push_str(r#"<Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>"#);
    }
    types.push_str("</Types>");
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut put = |name: &str, data: &[u8]| -> Result<(), String> {
        zip.start_file(name, opt).map_err(|e| e.to_string())?;
        zip.write_all(data).map_err(|e| e.to_string())
    };
    put("[Content_Types].xml", types.as_bytes())?;
    put("_rels/.rels", root_rels.as_bytes())?;
    put("word/document.xml", doc.as_bytes())?;
    put("word/styles.xml", styles.as_bytes())?;
    put("word/_rels/document.xml.rels", rels.as_bytes())?;
    if has_header {
        // A header must hold at least one paragraph.
        put("word/header1.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr {NS}>{header}</w:hdr>"#).as_bytes())?;
        put("word/_rels/header1.xml.rels", header_rels.as_bytes())?;
    }
    if has_footer {
        put("word/footer1.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr {NS}>{footer}</w:ftr>"#).as_bytes())?;
        put("word/_rels/footer1.xml.rels", footer_rels.as_bytes())?;
    }
    for (name, data) in &media.files {
        put(&format!("word/media/{name}"), data)?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// Graph paper for a `w`×`h` point page, as a transparent PNG at 200 dpi.
fn grid_png(g: &GridPaper, w: f32, h: f32) -> Result<Vec<u8>, String> {
    let k = 200.0 / 72.0;
    let (pw, ph) = ((w * k).round() as u32, (h * k).round() as u32);
    let c = u32::from_str_radix(g.color.trim_start_matches('#'), 16).unwrap_or(0xD5DCE6);
    let ink = image::Rgba([(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
    let mut img = image::RgbaImage::new(pw, ph);
    let thick = (g.width * k).round().max(1.0) as u32;
    let step = g.spacing.max(2.0);
    let mut x = g.offset[0].rem_euclid(step);
    while x < w {
        let px = (x * k).round() as u32;
        for dx in 0..thick {
            for y in 0..ph {
                if px + dx < pw {
                    img.put_pixel(px + dx, y, ink);
                }
            }
        }
        x += step;
    }
    let mut y = g.offset[1].rem_euclid(step);
    while y < h {
        let py = (y * k).round() as u32;
        for dy in 0..thick {
            for x in 0..pw {
                if py + dy < ph {
                    img.put_pixel(x, py + dy, ink);
                }
            }
        }
        y += step;
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    img.write_to(&mut bytes, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(bytes.into_inner())
}

/// A picture covering the page, behind the text.
fn behind(out: &mut String, w: f32, h: f32, rid: &str, ids: &mut u32) {
    let (cx, cy) = ((w * EMU).round() as i64, (h * EMU).round() as i64);
    let id = *ids;
    *ids += 1;
    let _ = write!(
        out,
        r#"<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="0" behindDoc="1" locked="1" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="{cx}" cy="{cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="{id}" name="Graph paper {id}"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{id}" name="Graph paper {id}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#
    );
}

fn block(
    out: &mut String,
    b: &Block,
    sheet: &Sheet,
    width: f32,
    media: &mut Media,
    ids: &mut u32,
    figure: Draw,
    mark: &mut Option<(char, usize)>,
) -> Result<(), String> {
    // This block's marker, if probing: tables and breaks take none (a table's paragraphs do).
    fn take(mark: &mut Option<(char, usize)>) -> Option<String> {
        mark.as_mut().map(|(part, n)| {
            *n += 1;
            marker(*part, *n - 1)
        })
    }
    match b {
        Block::P(p) => para(out, p, sheet, take(mark).as_deref()),
        Block::Space { height } => {
            let _ = write!(
                out,
                r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="{}" w:lineRule="exact"/></w:pPr>{}</w:p>"#,
                tw(height.max(1.0)),
                take(mark).as_deref().map(marker_run).unwrap_or_default()
            );
        }
        Block::Break => out.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#),
        Block::Figure(f) => {
            let png = figure(f)?;
            let rid = media.add(png);
            let _ = write!(
                out,
                r#"<w:p><w:pPr><w:spacing w:before="{}" w:after="0"/>{}{}</w:pPr>"#,
                tw(f.before),
                if f.indent != 0.0 { format!(r#"<w:ind w:left="{}"/>"#, tw(f.indent)) } else { String::new() },
                jc(f.align)
            );
            out.push_str(&take(mark).as_deref().map(marker_run).unwrap_or_default());
            drawing(out, f, &rid, ids);
            out.push_str("</w:p>");
        }
        Block::Table(t) => table(out, t, sheet, width, media, ids, figure, mark)?,
    }
    Ok(())
}

fn jc(a: Align) -> &'static str {
    match a {
        Align::Left => "",
        Align::Center => r#"<w:jc w:val="center"/>"#,
        Align::Right => r#"<w:jc w:val="right"/>"#,
        Align::Justify => r#"<w:jc w:val="both"/>"#,
    }
}

fn border(side: &str, r: &Rule) -> String {
    let val = match r.style {
        RuleStyle::Solid => "single",
        RuleStyle::Dashed => "dashed",
        RuleStyle::Dotted => "dotted",
        RuleStyle::Double => "double",
    };
    // Border widths are eighths of a point.
    format!(
        r#"<w:{side} w:val="{val}" w:sz="{}" w:space="1" w:color="{}"/>"#,
        (r.width * 8.0).round().clamp(2.0, 96.0) as i64,
        esc(&r.color)
    )
}

/// A white, one-point marker that takes no room on the line (see [`marker`]). The space after it
/// keeps it a word of its own in the PDF's text.
fn marker_run(text: &str) -> String {
    format!(
        r#"<w:r><w:rPr><w:color w:val="FFFFFF"/><w:spacing w:val="-12"/><w:sz w:val="2"/><w:szCs w:val="2"/></w:rPr><w:t xml:space="preserve">{text} </w:t></w:r>"#
    )
}

fn para(out: &mut String, p: &Para, sheet: &Sheet, mark: Option<&str>) {
    out.push_str("<w:p><w:pPr><w:keepLines/>");
    if p.rule_above.is_some() || p.rule_below.is_some() {
        out.push_str("<w:pBdr>");
        if let Some(r) = &p.rule_above {
            out.push_str(&border("top", r));
        }
        if let Some(r) = &p.rule_below {
            out.push_str(&border("bottom", r));
        }
        out.push_str("</w:pBdr>");
    }
    if !p.tabs.is_empty() {
        out.push_str("<w:tabs>");
        for t in &p.tabs {
            let val = match t.align {
                TabAlign::Left => "left",
                TabAlign::Center => "center",
                TabAlign::Right => "right",
                TabAlign::Decimal => "decimal",
            };
            let leader = match t.leader {
                Leader::None => "",
                Leader::Dot => r#" w:leader="dot""#,
                Leader::Underscore => r#" w:leader="underscore""#,
                Leader::Hyphen => r#" w:leader="hyphen""#,
            };
            let _ = write!(out, r#"<w:tab w:val="{val}"{leader} w:pos="{}"/>"#, tw(t.pos));
        }
        out.push_str("</w:tabs>");
    }
    let line = match p.line {
        Some(l) => format!(r#" w:line="{}" w:lineRule="exact""#, tw(l)),
        None => r#" w:line="240" w:lineRule="auto""#.into(),
    };
    let _ = write!(out, r#"<w:spacing w:before="{}" w:after="{}"{line}/>"#, tw(p.before), tw(p.after));
    if p.indent != 0.0 || p.hanging != 0.0 {
        let _ = write!(out, r#"<w:ind w:left="{}" w:hanging="{}"/>"#, tw(p.indent), tw(p.hanging));
    }
    out.push_str(jc(p.align));
    out.push_str("</w:pPr>");
    if let Some(m) = mark {
        out.push_str(&marker_run(m));
    }
    for r in &p.runs {
        run(out, r, sheet);
    }
    out.push_str("</w:p>");
}

fn rpr(t: &Text, sheet: &Sheet) -> String {
    let mut s = String::from("<w:rPr>");
    if let Some(f) = &t.font {
        let f = esc(f);
        let _ = write!(s, r#"<w:rFonts w:ascii="{f}" w:hAnsi="{f}" w:cs="{f}" w:eastAsia="{f}"/>"#);
    }
    if t.bold {
        s.push_str("<w:b/><w:bCs/>");
    }
    if t.italic {
        s.push_str("<w:i/><w:iCs/>");
    }
    if t.underline {
        s.push_str(r#"<w:u w:val="single"/>"#);
    }
    if let Some(c) = &t.color {
        let _ = write!(s, r#"<w:color w:val="{}"/>"#, esc(c.trim_start_matches('#')));
    }
    let size = t.size.unwrap_or(sheet.size);
    let _ = write!(s, r#"<w:sz w:val="{0}"/><w:szCs w:val="{0}"/>"#, (size * 2.0).round() as i64);
    if t.sup {
        s.push_str(r#"<w:vertAlign w:val="superscript"/>"#);
    } else if t.sub {
        s.push_str(r#"<w:vertAlign w:val="subscript"/>"#);
    }
    s.push_str("</w:rPr>");
    s
}

fn run(out: &mut String, r: &Run, sheet: &Sheet) {
    match r {
        Run::Tab { .. } => out.push_str("<w:r><w:tab/></w:r>"),
        Run::Br { .. } => out.push_str("<w:r><w:br/></w:r>"),
        Run::Math { math, display, size } => out.push_str(&omml(math, *display, size.unwrap_or(sheet.size))),
        Run::Text(t) => {
            let props = rpr(t, sheet);
            // Tabs and line breaks typed inside the text become Word's own.
            for (i, line) in t.text.split('\n').enumerate() {
                if i > 0 {
                    let _ = write!(out, "<w:r>{props}<w:br/></w:r>");
                }
                for (j, piece) in line.split('\t').enumerate() {
                    if j > 0 {
                        let _ = write!(out, "<w:r>{props}<w:tab/></w:r>");
                    }
                    if !piece.is_empty() {
                        let _ = write!(out, r#"<w:r>{props}<w:t xml:space="preserve">{}</w:t></w:r>"#, esc(piece));
                    }
                }
            }
        }
    }
}

/// An equation as Word math, through equaSHion's LaTeX reader and OMML writer, at `size` points.
pub fn omml(latex: &str, display: bool, size: f32) -> String {
    let mut m = equashion_math::from_linear(latex.trim());
    m.para = display;
    let xml = equashion_math::to_omml(&m);
    // Each math run carries the size; `w:rPr` goes after any `m:rPr` and before `m:t`.
    let sz = format!(
        r#"<w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/><w:sz w:val="{0}"/><w:szCs w:val="{0}"/></w:rPr>"#,
        (size * 2.0).round() as i64
    );
    let mut out = String::with_capacity(xml.len() * 2);
    let mut rest = xml.as_str();
    while let Some(i) = rest.find("<m:r>") {
        out.push_str(&rest[..i + 5]);
        rest = &rest[i + 5..];
        if rest.starts_with("<m:rPr") {
            let end = rest.find("</m:rPr>").map(|e| e + 8).unwrap_or(0);
            out.push_str(&rest[..end]);
            rest = &rest[end..];
        }
        if !rest.starts_with("<w:rPr>") {
            out.push_str(&sz);
        }
    }
    out.push_str(rest);
    out
}

fn drawing(out: &mut String, f: &Figure, rid: &str, ids: &mut u32) {
    let (cx, cy) = ((f.width * EMU).round() as i64, (f.height * EMU).round() as i64);
    let id = *ids;
    *ids += 1;
    let alt = esc(&f.alt);
    let _ = write!(
        out,
        r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{id}" name="Figure {id}" descr="{alt}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{id}" name="Figure {id}" descr="{alt}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    );
}

fn table(
    out: &mut String,
    t: &Table,
    sheet: &Sheet,
    _width: f32,
    media: &mut Media,
    ids: &mut u32,
    figure: Draw,
    mark: &mut Option<(char, usize)>,
) -> Result<(), String> {
    let total: f32 = t.widths.iter().sum();
    let line = r#"w:val="single" w:sz="6" w:space="0" w:color="000000""#;
    let none = r#"w:val="nil""#;
    let (outer, inner) = match t.borders {
        Borders::None => (none, none),
        Borders::All => (line, line),
        Borders::Outer => (line, none),
        Borders::Inner => (none, line),
    };
    // Boxed tables get a little padding; layout grids none, so their text lines up with the page.
    let pad = if t.borders == Borders::None { 0 } else { tw(4.0) };
    let _ = write!(
        out,
        r#"<w:tbl><w:tblPr><w:tblW w:w="{}" w:type="dxa"/>{}<w:tblBorders><w:top {outer}/><w:left {outer}/><w:bottom {outer}/><w:right {outer}/><w:insideH {inner}/><w:insideV {inner}/></w:tblBorders><w:tblLayout w:type="fixed"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="{pad}" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="{pad}" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid>"#,
        tw(total),
        jc(t.align)
    );
    for w in &t.widths {
        let _ = write!(out, r#"<w:gridCol w:w="{}"/>"#, tw(*w));
    }
    out.push_str("</w:tblGrid>");
    for row in &t.rows {
        out.push_str("<w:tr><w:trPr><w:cantSplit/>");
        if let Some(h) = row.height {
            let _ = write!(out, r#"<w:trHeight w:val="{}" w:hRule="atLeast"/>"#, tw(h));
        }
        out.push_str("</w:trPr>");
        let mut col = 0usize;
        for c in &row.cells {
            let span = c.span.max(1) as usize;
            let w: f32 = t.widths.iter().skip(col).take(span).sum();
            col += span;
            let _ = write!(out, r#"<w:tc><w:tcPr><w:tcW w:w="{}" w:type="dxa"/>"#, tw(w));
            if span > 1 {
                let _ = write!(out, r#"<w:gridSpan w:val="{span}"/>"#);
            }
            if let Some(s) = &c.shade {
                let _ = write!(out, r#"<w:shd w:val="clear" w:color="auto" w:fill="{}"/>"#, esc(s.trim_start_matches('#')));
            }
            match c.valign {
                VAlign::Top => {}
                VAlign::Center => out.push_str(r#"<w:vAlign w:val="center"/>"#),
                VAlign::Bottom => out.push_str(r#"<w:vAlign w:val="bottom"/>"#),
            }
            out.push_str("</w:tcPr>");
            let mut inner = String::new();
            for b in &c.blocks {
                block(&mut inner, b, sheet, w, media, ids, figure, mark)?;
            }
            // A cell must end with a paragraph.
            if !inner.ends_with("</w:p>") {
                inner.push_str(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0"/></w:pPr></w:p>"#);
            }
            out.push_str(&inner);
            out.push_str("</w:tc>");
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_runs_carry_their_size() {
        let x = omml("\\frac{2}{3}x-4", false, 13.0);
        assert!(x.contains("<m:f>"), "{x}");
        assert!(x.contains(r#"<w:sz w:val="26"/>"#), "{x}");
        assert_eq!(x.matches("<m:r>").count(), x.matches("<w:rPr>").count());
    }

    #[test]
    fn a_sheet_round_trips_through_json() {
        let src = r#"{"blocks":[{"type":"p","align":"center","runs":[{"text":"Title","bold":true,"size":22},{"tab":true},{"math":"x^{2}"}]},{"type":"space","height":12}]}"#;
        let s: Sheet = serde_json::from_str(src).unwrap();
        assert_eq!(s.blocks.len(), 2);
        let Block::P(p) = &s.blocks[0] else { panic!() };
        assert!(matches!(p.runs[1], Run::Tab { .. }));
        assert!(matches!(p.runs[2], Run::Math { .. }));
    }
}
