//! Claude through the Claude Code CLI and the user's own sign-in (their Claude plan), so no API
//! key is needed. Each request runs `claude -p` headlessly with every built-in tool switched off,
//! the user's settings, hooks and MCP servers skipped, and the page image passed in the message.
use crate::services::Models;
use base64::Engine;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Model aliases the CLI resolves to the newest model of each family.
pub const MODELS: [&str; 2] = ["sonnet", "opus"];

/// The `claude` program: on PATH, or where the installer puts it (an app started from the
/// desktop often has a shorter PATH than a terminal).
pub fn bin() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join("claude"));
    let known = home
        .into_iter()
        .flat_map(|h| [h.join(".local/bin/claude"), h.join(".claude/local/claude")]);
    on_path.chain(known).find(|p| p.is_file())
}

/// A `claude` command that uses the signed-in plan: credentials set for other tools (an API key,
/// a proxy URL, a parent Claude Code session) would otherwise take precedence over the sign-in.
fn command() -> Result<Command, String> {
    let exe = bin().ok_or("The Claude CLI isn't installed. Install Claude Code, then press Sign in.")?;
    let mut cmd = Command::new(exe);
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy();
        if k.starts_with("ANTHROPIC_") || k.starts_with("CLAUDE_CODE_") || k == "CLAUDECODE" {
            cmd.env_remove(k.as_ref());
        }
    }
    // A neutral folder, so no project's CLAUDE.md is picked up.
    cmd.current_dir(std::env::temp_dir());
    Ok(cmd)
}

/// Runs `cmd`, feeding it `input`, and returns stdout; kills it after `limit`.
fn run(mut cmd: Command, input: Vec<u8>, limit: Duration) -> Result<String, String> {
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start the Claude CLI: {e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let a = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let b = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Claude didn't answer within {} seconds.", limit.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let _ = writer.join();
    let (out, err) = (a.join().unwrap_or_default(), b.join().unwrap_or_default());
    if status.success() || !out.trim().is_empty() {
        Ok(out)
    } else {
        Err(format!("The Claude CLI failed ({status}): {}", err.trim()))
    }
}

/// "Signed in · Max plan", or an error saying how to sign in.
pub fn status() -> Result<String, String> {
    let mut cmd = command()?;
    cmd.args(["auth", "status"]);
    let out = run(cmd, vec![], Duration::from_secs(20))?;
    let v: Value = serde_json::from_str(out.trim()).map_err(|_| format!("Unexpected sign-in status: {}", out.trim()))?;
    if v["loggedIn"] != true {
        return Err("Not signed in to Claude. Press Sign in, finish in the browser, then refresh.".into());
    }
    Ok(match v["subscriptionType"].as_str() {
        Some(plan) if !plan.is_empty() => format!("Signed in with your Claude {} plan.", title(plan)),
        _ => match v["authMethod"].as_str() {
            Some("claude.ai") => "Signed in with your Claude plan.".into(),
            _ => "Signed in (Anthropic Console: usage is billed to that account).".into(),
        },
    })
}

fn title(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The models to offer, once the sign-in is confirmed.
pub fn models() -> Result<Models, String> {
    let note = status()?;
    Ok(Models { list: MODELS.iter().map(|m| m.to_string()).collect(), vision: true, note: Some(note) })
}

/// Opens a terminal running `claude auth login`, which hands over to the browser.
pub fn sign_in() -> Result<(), String> {
    let exe = bin().ok_or("The Claude CLI isn't installed. Install Claude Code first: https://claude.com/claude-code")?;
    let login = format!("'{}' auth login; echo; read -p 'Signed in. Press Enter to close.' _", exe.display());
    let terminals: [(&str, &[&str]); 4] = [
        ("konsole", &["-e", "bash", "-c"]),
        ("gnome-terminal", &["--", "bash", "-c"]),
        ("xfce4-terminal", &["-x", "bash", "-c"]),
        ("xterm", &["-e", "bash", "-c"]),
    ];
    for (term, args) in terminals {
        let mut cmd = Command::new(term);
        cmd.args(args).arg(&login);
        for (k, _) in std::env::vars_os() {
            let k = k.to_string_lossy();
            if k.starts_with("ANTHROPIC_") || k.starts_with("CLAUDE_CODE_") || k == "CLAUDECODE" {
                cmd.env_remove(k.as_ref());
            }
        }
        if cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok() {
            return Ok(());
        }
    }
    Err("Couldn't open a terminal. Run `claude auth login` in one yourself, then refresh.".into())
}

/// One prompt and one page image (base64 PNG) to `model`; returns Claude's reply text.
pub fn chat(model: &str, prompt: &str, img_png_b64: &str, limit: Duration) -> Result<String, String> {
    // Re-encoded as JPEG at the same size: a full-page PNG can pass the per-image upload limit.
    let png = base64::engine::general_purpose::STANDARD.decode(img_png_b64).map_err(|e| e.to_string())?;
    let img = image::load_from_memory(&png).map_err(|e| e.to_string())?;
    let mut jpg = std::io::Cursor::new(Vec::new());
    img.to_rgb8()
        .write_to(&mut jpg, image::ImageFormat::Jpeg)
        .map_err(|e| e.to_string())?;
    let data = base64::engine::general_purpose::STANDARD.encode(jpg.into_inner());
    let message = json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": data}},
        {"type": "text", "text": prompt},
    ]}});
    let mut cmd = command()?;
    cmd.args([
        "-p",
        "--input-format", "stream-json",
        "--output-format", "stream-json",
        "--verbose",
        "--model", model,
        "--tools", "",
        "--restricted",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--system-prompt",
        "You read scanned school pages and reply exactly in the format the user asks for.",
    ]);
    let out = run(cmd, format!("{message}\n").into_bytes(), limit)?;
    let result = out
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v["type"] == "result")
        .ok_or("The Claude CLI ended without an answer.")?;
    // Diagnostics: DOCUMENTASHION_CLAUDE_LOG=file appends each result (model, tokens, timing).
    if let Some(path) = std::env::var_os("DOCUMENTASHION_CLAUDE_LOG") {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{result}");
        }
    }
    let text = result["result"].as_str().unwrap_or_default();
    if result["is_error"] == true {
        let why = if text.is_empty() { result["subtype"].as_str().unwrap_or("unknown error") } else { text };
        return Err(if why.contains("login") || why.contains("auth") {
            format!("Claude needs you to sign in again (Settings, AI provider): {why}")
        } else {
            format!("Claude couldn't answer: {why}")
        });
    }
    if text.trim().is_empty() {
        return Err("Claude returned no text.".into());
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a real request through the signed-in CLI (uses plan usage):
    /// `DOCUMENTASHION_LIVE_IMAGE=crop.png cargo test --release claude_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn claude_live() {
        println!("{:?}", status());
        let img = std::fs::read(std::env::var("DOCUMENTASHION_LIVE_IMAGE").unwrap()).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(img);
        let t = Instant::now();
        let r = chat("sonnet", r#"Transcribe the handwriting exactly as written, one line each. Reply with ONLY JSON: {"lines": ["..."]}"#, &b64, Duration::from_secs(300));
        println!("{:.1}s {r:?}", t.elapsed().as_secs_f32());
        assert!(r.is_ok());
    }
}
