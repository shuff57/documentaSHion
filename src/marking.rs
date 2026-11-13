//! Answer keys and suggested marks: reading a key typed as a list or from scanned key pages,
//! and asking the vision model to mark the student's answers on a rebuilt page.
use crate::{
    model::*,
    services::{chat, page_png},
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// Reads a typed key, one question per line: `3. $x = 4$ [2]`. The label comes first ("3", "4b",
/// "Q5", "2(a)"); points go last as `[2]`, `(2 pts)` or `| 2`, and default to 1. A line without a
/// label takes the next number.
pub fn parse_key(text: &str) -> Vec<KeyItem> {
    let mut out: Vec<KeyItem> = vec![];
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (label, rest) = split_label(line);
        let (answer, points) = split_points(rest);
        let question = label.unwrap_or_else(|| (out.len() + 1).to_string());
        out.push(KeyItem { question, answer: answer.trim().to_owned(), points });
    }
    out
}

/// The key as editable text, the form `parse_key` reads.
pub fn format_key(items: &[KeyItem]) -> String {
    items
        .iter()
        .map(|k| {
            // "a) 9": a lone letter needs the bracket to read back as a label.
            let sep = if k.question.starts_with(|c: char| c.is_ascii_digit()) { "." } else { ")" };
            if k.points == 1.0 {
                format!("{}{sep} {}", k.question, k.answer)
            } else {
                format!("{}{sep} {} [{}]", k.question, k.answer, points(k.points))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A leading question label and the rest of the line.
fn split_label(line: &str) -> (Option<String>, &str) {
    let mut depth = 0;
    let mut end = None;
    for (i, c) in line.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            c if c.is_alphanumeric() => {}
            '.' | ')' | ':' => {
                end = Some((i, i + 1));
                break;
            }
            c if c.is_whitespace() => {
                end = Some((i, i));
                break;
            }
            _ => return (None, line),
        }
    }
    let Some((a, b)) = end else { return (None, line) };
    let label = line[..a].trim_start_matches(['Q', 'q', '#']);
    // A label is short and numbered ("12", "4b", "2(a)"), or a lone letter ("a)").
    let numbered = label.chars().next().is_some_and(|c| c.is_ascii_digit()) && label.len() <= 6;
    let letter = label.len() == 1 && label.chars().all(|c| c.is_ascii_alphabetic()) && &line[a..b] == ")";
    if numbered || letter {
        (Some(label.to_owned()), &line[b..])
    } else {
        (None, line)
    }
}

/// The answer and its points, from a trailing `[2]`, `(2 pts)` or `| 2`.
fn split_points(rest: &str) -> (&str, f32) {
    let t = rest.trim_end();
    let num = |s: &str| s.trim().parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.0);
    if let Some(inner) = t.strip_suffix(']').and_then(|s| s.rfind('[').map(|i| (&s[..i], &s[i + 1..])))
        && let Some(v) = num(inner.1)
    {
        return (inner.0, v);
    }
    if let Some((head, inner)) = t.strip_suffix(')').and_then(|s| s.rfind('(').map(|i| (&s[..i], &s[i + 1..]))) {
        let lower = inner.to_lowercase();
        for unit in ["points", "point", "pts", "pt", "marks", "mark"] {
            if let Some(n) = lower.trim().strip_suffix(unit)
                && let Some(v) = num(n)
            {
                return (head, v);
            }
        }
    }
    if let Some(i) = t.rfind('|')
        && let Some(v) = num(&t[i + 1..])
    {
        return (&t[..i], v);
    }
    (t, 1.0)
}

/// Labels compare without case, punctuation or a leading "Q": "Q3." matches "3".
fn same_question(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let s: String = s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        match s.strip_prefix('q') {
            Some(r) if r.starts_with(|c: char| c.is_ascii_digit()) => r.to_owned(),
            _ => s,
        }
    };
    norm(a) == norm(b)
}

const KEY_PROMPT: &str = r#"This image is an answer key for a school assignment. List the answer to every question on it.
Return ONLY a JSON object, no commentary, in this form:
{"items":[{"question":"1","answer":"...","points":1}]}
Rules:
- question: the question's label exactly as printed, e.g. "3", "4b".
- answer: the final answer only, not the working. Write mathematics as LaTeX inside $...$, e.g. "$x=\frac{3}{2}$".
- points: the points shown for that question; 1 if none are shown.
- Keep the order of the page. Use [unclear] for anything you cannot read."#;

/// Reads an answer key from a scanned or imported page.
pub fn read_key(p: &Page, s: &Settings, key: &str) -> Result<Vec<KeyItem>, String> {
    let (img, _, _) = page_png(p)?;
    let reply = chat(s, key, KEY_PROMPT, &img, true, Duration::from_secs(180))?;
    parse_key_reply(&reply)
}

/// Reads an answer key from image or PDF files, page by page.
pub fn read_key_files(paths: Vec<PathBuf>, s: &Settings, key: &str, cancel: &AtomicBool) -> Result<Vec<KeyItem>, String> {
    let mut images = vec![];
    for p in paths {
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            images.extend(crate::services::import_pdf(&p, cancel)?);
        } else {
            images.push(p);
        }
    }
    let mut out = vec![];
    for image in images {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        out.extend(read_key(&Page { image, ..Default::default() }, s, key)?);
    }
    Ok(out)
}

/// The JSON object in a model's reply, tolerating code fences and text around it.
fn json_object(reply: &str) -> Option<Value> {
    let (a, b) = (reply.find('{')?, reply.rfind('}')?);
    serde_json::from_str(reply.get(a..=b)?).ok()
}

fn parse_key_reply(reply: &str) -> Result<Vec<KeyItem>, String> {
    let bad = || "The model's reply wasn't an answer key. Try again, or type the key instead.".to_string();
    let v = json_object(reply).ok_or_else(bad)?;
    let items: Vec<KeyItem> = v
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(bad)?
        .iter()
        .filter_map(|i| {
            let answer = text_of(i.get("answer")?);
            (!answer.trim().is_empty()).then(|| KeyItem {
                question: i.get("question").map(text_of).unwrap_or_default().trim().to_owned(),
                answer: answer.trim().to_owned(),
                points: i.get("points").and_then(Value::as_f64).map(|p| p as f32).filter(|p| *p >= 0.0).unwrap_or(1.0),
            })
        })
        .collect();
    if items.is_empty() { Err(bad()) } else { Ok(items) }
}

/// A string or number field as text; models write `"question": 3` as often as `"3"`.
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

const MARK_PROMPT: &str = r#"You are helping a teacher mark a student's work. The page is attached. It has already been read into these numbered parts (P: printed, S: the student's writing):
{PARTS}

{KEY}

Return ONLY a JSON object, no commentary, in this form:
{"marks":[{"question":"1","blocks":[4],"answer":"...","expected":"...","verdict":"right","points":1,"max":1,"conf":0.9,"note":"..."}]}
Rules:
- One mark per question{EACH}.
- blocks: the numbers of the S parts holding the student's answer to that question (empty if they wrote nothing).
- answer: the student's final answer as written. expected: the correct answer. Mathematics as LaTeX in $...$.
- verdict: "right", "partial", "wrong", or "blank" when nothing was written.
- Accept answers that are mathematically equivalent (3/6 and 1/2, x=2 and 2, terms in a different order) unless the question asks for a particular form.
- points: points earned, from 0 to max. Give partial points only when the working is substantially right but the answer is not.
- conf: how sure you are of this mark, 0 to 1. Be honest: low when the writing is hard to read, the answer is ambiguous, or you are unsure of the correct answer.
- note: one short sentence for the teacher on wrong and partial answers, e.g. what went wrong. Empty for right answers.
- Read the student's writing from the image when the parts above look misread."#;

/// Asks the model to mark the student's answers on a rebuilt page, against `key` or, when it is
/// empty, against its own solutions.
pub fn mark(p: &Page, r: &Rebuild, key: &[KeyItem], s: &Settings, api: &str) -> Result<Grade, String> {
    let parts = r
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| format!("[{i}] {}: {}", if b.handwritten { "S" } else { "P" }, b.text.replace('\n', " / ")))
        .collect::<Vec<_>>()
        .join("\n");
    let key_text = if key.is_empty() {
        "There is no answer key. Solve each question yourself, carefully and step by step in your head, then mark the student's answer against your solution. Put your answer in \"expected\". Give each question 1 point unless the page shows points.".to_string()
    } else {
        format!(
            "The answer key (question. answer [points]):\n{}\nMark against this key. \"max\" is the key's points for the question.",
            key.iter().map(|k| format!("{}. {} [{}]", k.question, k.answer, points(k.points))).collect::<Vec<_>>().join("\n")
        )
    };
    let prompt = MARK_PROMPT
        .replace("{PARTS}", &parts)
        .replace("{KEY}", &key_text)
        .replace("{EACH}", if key.is_empty() { " on the page" } else { " in the key, in the key's order" });
    let (img, _, _) = page_png(p)?;
    let reply = chat(s, api, &prompt, &img, true, Duration::from_secs(300))?;
    let mut g = parse_marks(&reply, key, r.blocks.len())?;
    g.model = s.model.clone();
    Ok(g)
}

/// Reads the model's marks. With a key, its points are the maximum, and questions the model
/// skipped are added as blank and flagged for checking.
fn parse_marks(reply: &str, key: &[KeyItem], blocks: usize) -> Result<Grade, String> {
    let bad = || "The model's reply wasn't a set of marks. Try again.".to_string();
    let v = json_object(reply).ok_or_else(bad)?;
    let list = v.get("marks").and_then(Value::as_array).ok_or_else(bad)?;
    let num = |v: Option<&Value>| v.and_then(Value::as_f64).map(|f| f as f32).filter(|f| f.is_finite());
    let mut marks: Vec<Mark> = vec![];
    for m in list {
        let question = m.get("question").map(text_of).unwrap_or_default().trim().to_owned();
        if question.is_empty() || marks.iter().any(|k| same_question(&k.question, &question)) {
            continue;
        }
        let verdict = match m.get("verdict").map(text_of).unwrap_or_default().to_lowercase().trim() {
            "right" | "correct" => Verdict::Right,
            "partial" | "partially correct" | "partly right" => Verdict::Partial,
            "blank" | "missing" | "unanswered" | "none" => Verdict::Blank,
            _ => Verdict::Wrong,
        };
        let keyed = key.iter().find(|k| same_question(&k.question, &question));
        if !key.is_empty() && keyed.is_none() {
            continue; // Marks only what the key asks for.
        }
        let max = keyed.map(|k| k.points).or(num(m.get("max"))).filter(|v| *v > 0.0).unwrap_or(1.0);
        let pts = match verdict {
            Verdict::Right => max,
            Verdict::Wrong | Verdict::Blank => 0.0,
            Verdict::Partial => num(m.get("points")).unwrap_or(max / 2.0).clamp(0.0, max),
        };
        marks.push(Mark {
            question: keyed.map(|k| k.question.clone()).unwrap_or(question),
            answer: m.get("answer").map(text_of).unwrap_or_default(),
            expected: keyed.map(|k| k.answer.clone()).unwrap_or_else(|| m.get("expected").map(text_of).unwrap_or_default()),
            verdict,
            points: pts,
            max,
            conf: num(m.get("conf")).unwrap_or(1.0).clamp(0.0, 1.0),
            note: m.get("note").map(text_of).unwrap_or_default(),
            blocks: m
                .get("blocks")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_u64).map(|i| i as usize).filter(|&i| i < blocks).collect())
                .unwrap_or_default(),
            overridden: false,
        });
    }
    if !key.is_empty() {
        // Key order, with anything the model missed added for the teacher to check.
        marks = key
            .iter()
            .map(|k| {
                marks.iter().find(|m| same_question(&m.question, &k.question)).cloned().unwrap_or_else(|| Mark {
                    question: k.question.clone(),
                    expected: k.answer.clone(),
                    verdict: Verdict::Blank,
                    max: k.points,
                    conf: 0.0,
                    note: "Not found on this page. Check it.".into(),
                    ..Default::default()
                })
            })
            .collect();
    }
    if marks.is_empty() {
        return Err(bad());
    }
    Ok(Grade { marks, solved: key.is_empty(), model: String::new() })
}

/// The points that go with a verdict the teacher picked.
pub fn points_for(verdict: Verdict, current: f32, max: f32) -> f32 {
    match verdict {
        Verdict::Right => max,
        Verdict::Wrong | Verdict::Blank => 0.0,
        Verdict::Partial if current > 0.0 && current < max => current,
        Verdict::Partial => max / 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Reads a key from a page, then marks a rebuilt student page against it:
    /// `DOCUMENTASHION_LIVE_KEY=key.png DOCUMENTASHION_LIVE_IMAGE=student.png DOCUMENTASHION_LIVE_MODEL=… cargo test --release live_grade -- --ignored --nocapture`
    /// The student page must have been rebuilt first (`live_rebuild` writes `<image>.rebuild.json`);
    /// the marked copy is written back to that file, and the key to `<key>.key.txt`.
    #[test]
    #[ignore]
    fn live_grade() {
        let s = Settings { model: std::env::var("DOCUMENTASHION_LIVE_MODEL").unwrap(), provider: std::env::var("DOCUMENTASHION_LIVE_PROVIDER").unwrap_or_else(|_| "Local Ollama".into()), ..Default::default() };
        let key_img = std::env::var("DOCUMENTASHION_LIVE_KEY").unwrap();
        let img = std::env::var("DOCUMENTASHION_LIVE_IMAGE").unwrap();
        let saved = format!("{key_img}.key.txt");
        let t = std::time::Instant::now();
        let key = match std::fs::read_to_string(&saved) {
            Ok(text) => parse_key(&text),
            Err(_) => {
                let k = read_key(&Page { image: key_img.clone().into(), dpi: 300, ..Default::default() }, &s, "").unwrap();
                std::fs::write(&saved, format_key(&k)).unwrap();
                eprintln!("key read in {:.0?}", t.elapsed());
                k
            }
        };
        eprintln!("{} questions in the key", key.len());
        let path = format!("{img}.rebuild.json");
        let mut r: Rebuild = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let page = Page { image: img.into(), dpi: 300, ..Default::default() };
        let t = std::time::Instant::now();
        let g = mark(&page, &r, &key, &s, "").unwrap();
        let (got, of) = g.score();
        eprintln!("marked in {:.0?}: {got} / {of}, {} to check", t.elapsed(), g.unsure());
        for m in &g.marks {
            eprintln!("Q{} {:?} {}/{} conf {:.2} | {} | key {} | {}", m.question, m.verdict, m.points, m.max, m.conf, m.answer, m.expected, m.note);
        }
        r.grade = Some(g);
        std::fs::write(&path, serde_json::to_vec_pretty(&r).unwrap()).unwrap();
    }
    #[test]
    fn typed_keys_are_read_line_by_line() {
        let k = parse_key("1. 13\n2) $x = 4$ [2]\nQ3: slope = -2 (3 pts)\n4(b) 7 | 0.5\n\nno label here\na) 9");
        let got: Vec<(&str, &str, f32)> = k.iter().map(|k| (k.question.as_str(), k.answer.as_str(), k.points)).collect();
        assert_eq!(
            got,
            vec![("1", "13", 1.0), ("2", "$x = 4$", 2.0), ("3", "slope = -2", 3.0), ("4(b)", "7", 0.5), ("5", "no label here", 1.0), ("a", "9", 1.0)]
        );
        assert_eq!(parse_key(&format_key(&k)), k);
    }
    #[test]
    fn an_answer_in_brackets_is_not_points() {
        let k = parse_key("1. (2, 3)\n2. $[0, 1]$");
        assert_eq!((k[0].answer.as_str(), k[0].points), ("(2, 3)", 1.0));
        assert_eq!((k[1].answer.as_str(), k[1].points), ("$[0, 1]$", 1.0));
    }
    #[test]
    fn marks_follow_the_key() {
        let key = parse_key("1. 13\n2. 7 [2]\n3. 4");
        let reply = r#"```json
{"marks":[{"question":"Q2","blocks":[5,99],"answer":"$13$","verdict":"partial","points":5,"max":9,"conf":0.4},
{"question":1,"blocks":[2],"answer":"13","verdict":"correct","points":0,"conf":0.95},
{"question":"9","verdict":"right"}]}
```"#;
        let g = parse_marks(reply, &key, 10).unwrap();
        assert!(!g.solved);
        let q: Vec<&str> = g.marks.iter().map(|m| m.question.as_str()).collect();
        assert_eq!(q, ["1", "2", "3"]);
        // Right earns the key's points whatever the model wrote; partial stays within the maximum.
        assert_eq!((g.marks[0].verdict, g.marks[0].points), (Verdict::Right, 1.0));
        assert_eq!((g.marks[1].points, g.marks[1].max, g.marks[1].blocks.clone()), (2.0, 2.0, vec![5]));
        assert!(g.marks[1].unsure());
        // The question the model skipped is added and flagged.
        assert_eq!((g.marks[2].verdict, g.marks[2].expected.as_str()), (Verdict::Blank, "4"));
        assert!(g.marks[2].unsure());
        assert_eq!(g.score(), (3.0, 4.0));
    }
    #[test]
    fn without_a_key_the_models_solutions_are_kept() {
        let reply = r#"{"marks":[{"question":"1","expected":"$x=3$","answer":"$x=2$","verdict":"wrong","points":1,"max":1,"note":"Sign error"}]}"#;
        let g = parse_marks(reply, &[], 3).unwrap();
        assert!(g.solved);
        assert_eq!((g.marks[0].expected.as_str(), g.marks[0].points), ("$x=3$", 0.0));
        assert!(parse_marks("nothing", &[], 3).is_err());
    }
    #[test]
    fn keys_read_from_a_page_tolerate_numbers() {
        let k = parse_key_reply(r#"{"items":[{"question":3,"answer":"$\\frac{1}{2}$","points":2},{"question":"4","answer":""}]}"#).unwrap();
        assert_eq!(k, vec![KeyItem { question: "3".into(), answer: "$\\frac{1}{2}$".into(), points: 2.0 }]);
    }
}
