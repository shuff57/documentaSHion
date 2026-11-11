//! Word-level highlights for reviewing text: what a suggestion changes, and which words need a
//! second look against the scan.
use crate::{
    diff::{self, Op},
    model::Word,
};
use eframe::egui::{Color32, FontId, Stroke, text::LayoutJob, text::TextFormat};

/// Below this Tesseract confidence a word is highlighted as unsure.
pub const UNSURE: f32 = 60.0;

pub struct Palette {
    ink: Color32,
    added: Color32,
    removed: Color32,
    flag: Color32,
    flag_line: Color32,
}
impl Palette {
    pub fn added_color(&self) -> Color32 {
        self.added
    }
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                ink: Color32::from_rgb(0xf2, 0xe8, 0xdd),
                added: Color32::from_rgb(0x2d, 0x4a, 0x31),
                removed: Color32::from_rgb(0xe8, 0x9a, 0x8c),
                flag: Color32::from_rgb(0x5c, 0x47, 0x1c),
                flag_line: Color32::from_rgb(0xe0, 0xb0, 0x4c),
            }
        } else {
            Self {
                ink: Color32::from_rgb(0x33, 0x2a, 0x24),
                added: Color32::from_rgb(0xd5, 0xe8, 0xcf),
                removed: Color32::from_rgb(0xa1, 0x3d, 0x2d),
                flag: Color32::from_rgb(0xf8, 0xe2, 0xa8),
                flag_line: Color32::from_rgb(0xb0, 0x7a, 0x12),
            }
        }
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct Summary {
    pub added: usize,
    pub removed: usize,
    /// Added words containing digits: changed answers, dates and scores deserve a check.
    pub numbers: usize,
    pub unclear: usize,
    /// Words kept from OCR that Tesseract was unsure of.
    pub unsure: usize,
}
impl Summary {
    pub fn describe(&self) -> String {
        let mut parts = vec![];
        if self.added + self.removed == 0 {
            parts.push("No word changes".to_string());
        } else {
            parts.push(format!("{} added · {} removed", self.added, self.removed));
        }
        if self.numbers > 0 {
            parts.push(format!("{} with numbers", self.numbers));
        }
        if self.unclear > 0 {
            parts.push(format!("{} [unclear]", self.unclear));
        }
        if self.unsure > 0 {
            parts.push(format!("{} unsure OCR", self.unsure));
        }
        parts.join(" · ")
    }
}

fn needs_check(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_digit()) || word.contains("[unclear")
}

fn separator(breaks: u8, first: bool) -> &'static str {
    match (first, breaks) {
        (true, _) => "",
        (_, 0) => " ",
        (_, 1) => "\n",
        _ => "\n\n",
    }
}

/// The suggestion's words in order, with removed words struck through and added words shaded.
/// `words` are the suggestion's own OCR words, if it came from Tesseract.
pub fn changes(base: &str, new: &str, words: &[Word], font: FontId, p: &Palette) -> (LayoutJob, Summary) {
    let (a, b) = (diff::tokens(base), diff::tokens(new));
    let at: Vec<&str> = a.iter().map(|t| t.text).collect();
    let bt: Vec<&str> = b.iter().map(|t| t.text).collect();
    let conf: Vec<Option<f32>> = diff::place(words, new).iter().map(|w| w.conf).collect();
    let plain = TextFormat::simple(font.clone(), p.ink);
    let mut job = LayoutJob::default();
    let mut s = Summary::default();
    // Line breaks already shown before a removed word aren't repeated before its replacement.
    let mut shown_breaks = 0u8;
    for (n, op) in diff::diff(&at, &bt).into_iter().enumerate() {
        let (text, mut breaks, mut format) = match op {
            Op::Same(_, j) => (b[j].text, b[j].breaks, plain.clone()),
            Op::Removed(i) => {
                s.removed += 1;
                let mut f = TextFormat::simple(font.clone(), p.removed);
                f.strikethrough = Stroke::new(1.5, p.removed);
                (a[i].text, a[i].breaks, f)
            }
            Op::Added(j) => {
                s.added += 1;
                let mut f = plain.clone();
                f.background = p.added;
                if needs_check(b[j].text) {
                    s.numbers += b[j].text.chars().any(|c| c.is_ascii_digit()) as usize;
                    f.background = p.flag;
                }
                (b[j].text, b[j].breaks, f)
            }
        };
        if let Op::Same(_, j) | Op::Added(j) = op {
            if b[j].text.contains("[unclear") {
                s.unclear += 1;
                format.background = p.flag;
            }
            if matches!(op, Op::Same(..)) && conf[j].is_some_and(|c| c < UNSURE) {
                s.unsure += 1;
                format.underline = Stroke::new(2.0, p.flag_line);
            }
        }
        if breaks <= shown_breaks {
            breaks = 0;
        }
        shown_breaks = if matches!(op, Op::Removed(_)) { shown_breaks.max(breaks) } else { 0 };
        job.append(separator(breaks, n == 0), 0.0, plain.clone());
        job.append(text, 0.0, format);
    }
    (job, s)
}

/// Lays out editable text exactly as typed, underlining words Tesseract was unsure of and
/// shading [unclear] markers.
pub fn unsure_words(text: &str, words: &[Word], font: FontId, p: &Palette) -> LayoutJob {
    let plain = TextFormat::simple(font, p.ink);
    let mut job = LayoutJob::default();
    let placed = diff::place(words, text);
    let mut at = 0;
    for (t, w) in diff::tokens(text).iter().zip(&placed) {
        let start = t.text.as_ptr() as usize - text.as_ptr() as usize;
        job.append(&text[at..start], 0.0, plain.clone());
        let mut f = plain.clone();
        if w.conf.is_some_and(|c| c < UNSURE) {
            f.underline = Stroke::new(2.0, p.flag_line);
        }
        if t.text.contains("[unclear") {
            f.background = p.flag;
        }
        job.append(t.text, 0.0, f);
        at = start + t.text.len();
    }
    job.append(&text[at..], 0.0, plain);
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_flags_numbers_and_unclear() {
        let (job, s) = changes(
            "Total: 18 points\nName Jon",
            "Total: 16 points\nName [unclear]",
            &[],
            FontId::proportional(14.0),
            &Palette::new(false),
        );
        assert_eq!((s.added, s.removed, s.numbers, s.unclear), (2, 2, 1, 1));
        assert_eq!(job.text, "Total: 18 16 points\nName Jon [unclear]");
    }
    #[test]
    fn a_replaced_line_start_stays_on_its_line() {
        let (job, _) = changes(
            "Title\n\nName:JordanLee Date",
            "Title\n\nName: Jordan Lee Date",
            &[],
            FontId::proportional(14.0),
            &Palette::new(false),
        );
        assert_eq!(job.text, "Title\n\nName:JordanLee Name: Jordan Lee Date");
    }
    #[test]
    fn layout_matches_the_text_exactly() {
        let text = "  Name:  ___\n\nDate ";
        let words = [
            Word { text: "Name:".into(), conf: 95.0, bbox: [0; 4] },
            Word { text: "___".into(), conf: 30.0, bbox: [0; 4] },
            Word { text: "Date".into(), conf: 90.0, bbox: [0; 4] },
        ];
        let job = unsure_words(text, &words, FontId::proportional(14.0), &Palette::new(true));
        assert_eq!(job.text, text);
        let underlined: Vec<&str> = job
            .sections
            .iter()
            .filter(|s| s.format.underline.width > 0.0)
            .map(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0])
            .collect();
        assert_eq!(underlined, ["___"]);
    }
}
