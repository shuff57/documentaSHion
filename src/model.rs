use crate::imaging::{self, Analysis};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
pub fn id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string()
}
fn default_dpi() -> u32 {
    300
}
fn yes() -> bool {
    true
}
pub fn root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".local/share"))
        .join("documentashion")
}
/// One OCR word, boxed in the pixels of the page as rendered when OCR ran.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Word {
    pub text: String,
    /// Tesseract confidence, 0–100.
    pub conf: f32,
    /// x, y, width, height.
    pub bbox: [u32; 4],
}
/// One element of a rebuilt page: a run of printed text or of the student's writing, where it
/// sits on the scan. Math inside `text` is LaTeX between `$…$` (inline) or `$$…$$` (display).
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Block {
    /// x, y, width, height in thousandths of the rendered page.
    pub bbox: [f32; 4],
    /// The student wrote it (drawn in the answer colour), rather than it being printed.
    #[serde(default)]
    pub handwritten: bool,
    pub text: String,
    /// Height of one line of text, thousandths of the page height; 0 when the model gave none.
    #[serde(default)]
    pub line: f32,
    /// How sure the model is of its reading, 0–1.
    #[serde(default = "sure")]
    pub conf: f32,
}
fn sure() -> f32 {
    1.0
}
/// A digital copy of a page: its elements in place, with handwriting as typed text and math.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Rebuild {
    pub blocks: Vec<Block>,
    /// `Page::geometry()` it was made on; positions are stale if the page was turned or cropped since.
    pub geometry: String,
    /// The model that produced it.
    #[serde(default)]
    pub model: String,
    /// Positions were lined up with Tesseract's reading of the page; otherwise they are the
    /// model's estimate.
    #[serde(default)]
    pub anchored: bool,
    /// Suggested marks for the student's answers, once marked.
    #[serde(default)]
    pub grade: Option<Grade>,
}
impl Rebuild {
    /// Blocks below this confidence are flagged for checking.
    pub const UNSURE: f32 = 0.6;
    pub fn unsure(&self) -> usize {
        self.blocks
            .iter()
            .filter(|b| b.conf < Self::UNSURE || b.text.contains("[unclear]"))
            .count()
    }
}
fn one() -> f32 {
    1.0
}
/// One question of an answer key.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct KeyItem {
    /// The question's label as printed: "3", "4b".
    pub question: String,
    /// The expected answer; math as LaTeX in `$…$`.
    pub answer: String,
    #[serde(default = "one")]
    pub points: f32,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Right,
    Partial,
    #[default]
    Wrong,
    /// Nothing written.
    Blank,
}
/// A suggested mark for one question.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Mark {
    pub question: String,
    /// The student's answer, as read.
    #[serde(default)]
    pub answer: String,
    /// The answer from the key, or the model's own solution when there is no key.
    #[serde(default)]
    pub expected: String,
    pub verdict: Verdict,
    pub points: f32,
    pub max: f32,
    /// How sure the model is of the mark, 0–1.
    #[serde(default = "sure")]
    pub conf: f32,
    /// A short reason, for the teacher.
    #[serde(default)]
    pub note: String,
    /// The rebuild's blocks holding the student's answer: where the mark is drawn.
    #[serde(default)]
    pub blocks: Vec<usize>,
    /// The teacher changed the suggested mark.
    #[serde(default)]
    pub overridden: bool,
}
impl Mark {
    /// Marks below this confidence are flagged for checking.
    pub const UNSURE: f32 = 0.6;
    pub fn unsure(&self) -> bool {
        !self.overridden && self.conf < Self::UNSURE
    }
}
/// Suggested marks for a page.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Grade {
    pub marks: Vec<Mark>,
    /// There was no answer key: the model solved the questions itself.
    #[serde(default)]
    pub solved: bool,
    #[serde(default)]
    pub model: String,
}
impl Grade {
    /// Points scored and points possible.
    pub fn score(&self) -> (f32, f32) {
        self.marks.iter().fold((0.0, 0.0), |(s, m), k| (s + k.points, m + k.max))
    }
    pub fn unsure(&self) -> usize {
        self.marks.iter().filter(|m| m.unsure()).count()
    }
}
/// A score as text, without needless decimals: "7.5 / 10".
pub fn points(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r}") }
}
/// Page state that undo restores. Older projects stored a `(text, rewrite, rotation, clean)` tuple.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(from = "SnapshotRepr")]
pub struct Snapshot {
    pub text: String,
    pub rewrite: String,
    pub rotation: u8,
    pub clean: bool,
    pub straighten: bool,
    pub crop: [f32; 4],
    pub rebuild: Option<Rebuild>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum SnapshotRepr {
    Legacy(String, String, u8, bool),
    Current {
        text: String,
        rewrite: String,
        rotation: u8,
        clean: bool,
        #[serde(default)]
        straighten: bool,
        #[serde(default)]
        crop: [f32; 4],
        #[serde(default)]
        rebuild: Option<Rebuild>,
    },
}
impl From<SnapshotRepr> for Snapshot {
    fn from(r: SnapshotRepr) -> Self {
        match r {
            SnapshotRepr::Legacy(text, rewrite, rotation, clean) => Self {
                text,
                rewrite,
                rotation,
                clean,
                ..Default::default()
            },
            SnapshotRepr::Current {
                text,
                rewrite,
                rotation,
                clean,
                straighten,
                crop,
                rebuild,
            } => Self {
                text,
                rewrite,
                rotation,
                clean,
                straighten,
                crop,
                rebuild,
            },
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Page {
    pub id: String,
    pub image: PathBuf,
    pub text: String,
    pub rewrite: String,
    /// Quarter turns clockwise, applied after cropping and straightening.
    pub rotation: u8,
    pub clean: bool,
    #[serde(default = "default_dpi")]
    pub dpi: u32,
    /// Detected skew of the original scan in degrees, clockwise positive.
    #[serde(default)]
    pub skew: f32,
    /// Whether the detected skew is corrected.
    #[serde(default)]
    pub straighten: bool,
    /// Trim per edge as a fraction of the original, before straightening: left, top, right, bottom.
    #[serde(default)]
    pub crop: [f32; 4],
    /// The scan looked blank when it was imported (for example the back of a single-sided sheet).
    #[serde(default)]
    pub blank: bool,
    /// Word boxes from the last accepted Tesseract result, used to place PDF text.
    #[serde(default)]
    pub words: Vec<Word>,
    /// `geometry()` when `words` were measured; boxes are stale if it changed.
    #[serde(default)]
    pub words_geometry: String,
    /// The accepted rebuilt copy of the page, if any.
    #[serde(default)]
    pub rebuild: Option<Rebuild>,
    pub history: Vec<Snapshot>,
}
impl Page {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            rewrite: self.rewrite.clone(),
            rotation: self.rotation,
            clean: self.clean,
            straighten: self.straighten,
            crop: self.crop,
            rebuild: self.rebuild.clone(),
        }
    }
    pub fn checkpoint(&mut self) {
        self.push_history(self.snapshot());
    }
    pub fn push_history(&mut self, s: Snapshot) {
        if self.history.len() >= 100 {
            self.history.remove(0);
        }
        self.history.push(s);
    }
    /// The state before the next edit, for edits applied in place (typing).
    pub fn before_edit(&self) -> Snapshot {
        self.snapshot()
    }
    pub fn undo(&mut self) {
        if let Some(s) = self.history.pop() {
            self.text = s.text;
            self.rewrite = s.rewrite;
            self.rotation = s.rotation;
            self.clean = s.clean;
            self.straighten = s.straighten;
            self.crop = s.crop;
            self.rebuild = s.rebuild;
        }
    }
    /// The accepted rebuild, when it still lines up with the page as rendered now.
    pub fn rebuild_current(&self) -> Option<&Rebuild> {
        self.rebuild.as_ref().filter(|r| r.geometry == self.geometry())
    }
    /// Identifies the rendered pixel grid; OCR boxes are only valid for the geometry they were measured on.
    pub fn geometry(&self) -> String {
        let skew = if self.straighten { self.skew } else { 0.0 };
        format!(
            "r{} s{:.3} c{:.4},{:.4},{:.4},{:.4}",
            self.rotation % 4,
            skew,
            self.crop[0],
            self.crop[1],
            self.crop[2],
            self.crop[3]
        )
    }
    pub fn words_current(&self) -> bool {
        !self.words.is_empty() && self.words_geometry == self.geometry()
    }
    /// A key for cached textures: changes whenever the rendered image would.
    pub fn render_key(&self) -> String {
        format!("{}-{}-{}", self.id, self.geometry(), self.clean)
    }
    pub fn render(&self) -> Result<image::DynamicImage, String> {
        self.render_within(None)
    }
    /// Like `render`, but first shrinks the original to fit `max` pixels on its longest side.
    /// Thumbnails and the on-screen preview use this to avoid straightening full-resolution scans.
    pub fn render_within(&self, max: Option<u32>) -> Result<image::DynamicImage, String> {
        let mut i = image::open(&self.image).map_err(|e| e.to_string())?;
        if let Some(m) = max.filter(|&m| i.width().max(i.height()) > m) {
            i = i.resize(m, m, image::imageops::FilterType::Triangle);
        }
        i = imaging::apply_crop(i, self.crop);
        if self.straighten && self.skew.abs() >= 0.05 {
            i = imaging::rotate(&i.to_rgb8(), -self.skew).into();
        }
        i = match self.rotation % 4 {
            1 => i.rotate90(),
            2 => i.rotate180(),
            3 => i.rotate270(),
            _ => i,
        };
        if self.clean {
            i = i.adjust_contrast(12.0);
        }
        Ok(i)
    }
    /// Crop of the edge shown on screen as `side` (0 left, 1 top, 2 right, 3 bottom),
    /// mapped to the stored, pre-rotation edge.
    pub fn crop_index(&self, side: usize) -> usize {
        (side + 4 - (self.rotation as usize % 4)) % 4
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub pages: Vec<Page>,
    /// The answer key AI grade marks against; empty means the model solves the questions.
    #[serde(default)]
    pub key: Vec<KeyItem>,
}
impl Project {
    pub fn new() -> Self {
        Self {
            id: id(),
            name: "Untitled scan".into(),
            pages: vec![],
            key: vec![],
        }
    }
    pub fn dir(&self) -> PathBuf {
        root().join("projects").join(&self.id)
    }
    pub fn save(&self) -> Result<(), String> {
        let d = self.dir();
        fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        let p = d.join("project.json.tmp");
        fs::write(&p, serde_json::to_vec_pretty(self).unwrap()).map_err(|e| e.to_string())?;
        fs::rename(p, d.join("project.json")).map_err(|e| e.to_string())
    }
    /// Copy an image into the project. `cleanup` holds the automatic cleanup to apply, if any;
    /// it is stored as page settings so the original file stays untouched and each step can be undone.
    pub fn import(
        &mut self,
        p: PathBuf,
        dpi: u32,
        cleanup: Option<Analysis>,
    ) -> Result<(), String> {
        let id = id();
        fs::create_dir_all(self.dir()).map_err(|e| e.to_string())?;
        let dest = self.dir().join(format!(
            "{id}.{}",
            p.extension().and_then(|s| s.to_str()).unwrap_or("png")
        ));
        fs::copy(p, &dest).map_err(|e| e.to_string())?;
        let a = cleanup.unwrap_or_default();
        self.pages.push(Page {
            id,
            image: dest,
            clean: cleanup.is_some(),
            dpi,
            skew: a.skew,
            straighten: cleanup.is_some() && a.skew != 0.0,
            crop: a.crop,
            blank: a.blank,
            ..Default::default()
        });
        Ok(())
    }
}
pub fn library() -> Vec<Project> {
    fs::read_dir(root().join("projects"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| fs::read(e.path().join("project.json")).ok())
        .filter_map(|b| serde_json::from_slice(&b).ok())
        .collect()
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub dark: bool,
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub device: String,
    pub duplex: bool,
    pub dpi: u32,
    /// Straighten, trim dark borders, flag blank pages and raise contrast on new pages.
    #[serde(default = "yes")]
    pub auto_cleanup: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: false,
            provider: "Local Ollama".into(),
            endpoint: "http://127.0.0.1:11434".into(),
            model: String::new(),
            device: String::new(),
            duplex: true,
            dpi: 300,
            auto_cleanup: true,
        }
    }
}
impl Settings {
    pub fn load() -> Self {
        fs::read(root().join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<(), String> {
        fs::create_dir_all(root()).map_err(|e| e.to_string())?;
        fs::write(
            root().join("settings.json"),
            serde_json::to_vec_pretty(self).unwrap(),
        )
        .map_err(|e| e.to_string())
    }
    pub fn needs_key(&self) -> bool {
        // Claude uses the Claude CLI's own sign-in.
        self.provider != "Local Ollama" && self.provider != "Claude"
    }
    /// Keychain account for the current provider. Custom endpoints get one key each.
    pub fn key_account(&self) -> String {
        if self.provider == "Custom" {
            format!("Custom {}", self.endpoint.trim().trim_end_matches('/'))
        } else {
            self.provider.clone()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrections_are_reversible() {
        let mut p = Page {
            text: "original".into(),
            ..Default::default()
        };
        p.checkpoint();
        p.text = "corrected".into();
        p.rewrite = "rewritten".into();
        p.rotation = 1;
        p.clean = true;
        p.straighten = true;
        p.crop = [0.1, 0.0, 0.0, 0.0];
        p.undo();
        assert_eq!(p.text, "original");
        assert_eq!(p.rewrite, "");
        assert_eq!(p.rotation, 0);
        assert!(!p.clean);
        assert!(!p.straighten);
        assert_eq!(p.crop, [0.0; 4]);
    }
    #[test]
    fn project_roundtrip_preserves_original_and_rewrite() {
        let p = Project {
            id: "fixture".into(),
            name: "Algebra".into(),
            key: vec![],
            pages: vec![Page {
                image: "original.png".into(),
                text: "2 + 2".into(),
                rewrite: "Two plus two".into(),
                ..Default::default()
            }],
        };
        let out: Project = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert_eq!(out.pages[0].image, PathBuf::from("original.png"));
        assert_eq!(out.pages[0].text, "2 + 2");
        assert_eq!(out.pages[0].rewrite, "Two plus two");
    }
    #[test]
    fn alpha_projects_still_load() {
        let json = r#"{"id":"1","name":"Old","pages":[{"id":"p","image":"a.png","text":"new",
            "rewrite":"","rotation":1,"clean":true,"dpi":300,"history":[["old","",0,false]]}]}"#;
        let mut p: Project = serde_json::from_str(json).unwrap();
        let page = &mut p.pages[0];
        assert!(!page.straighten && page.words.is_empty());
        page.undo();
        assert_eq!((page.text.as_str(), page.rotation, page.clean), ("old", 0, false));
    }
    #[test]
    fn crop_follows_the_edge_on_screen() {
        let mut p = Page::default();
        assert_eq!(p.crop_index(0), 0);
        p.rotation = 1; // a quarter turn clockwise puts the original left edge on top
        assert_eq!(p.crop_index(1), 0);
        assert_eq!(p.crop_index(2), 1);
        p.rotation = 3;
        assert_eq!(p.crop_index(3), 0);
    }
    #[test]
    fn rotating_invalidates_ocr_boxes() {
        let mut p = Page {
            words: vec![Word::default()],
            ..Default::default()
        };
        p.words_geometry = p.geometry();
        assert!(p.words_current());
        p.rotation = 1;
        assert!(!p.words_current());
    }
}
