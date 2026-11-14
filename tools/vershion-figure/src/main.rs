//! `vershion-figure`: reads `{"id": "coordinate-grid", "params": {…}}` on stdin and prints that
//! verSHion figure as SVG. Parameters left out keep the figure's starting values.
//!
//! `vershion-figure --describe [id …]` prints each figure's parameters instead, one per line:
//! `id  key  kind  default  label`.
use serde_json::Value as J;
use std::io::Read;
use vershion::figures::{Kind, Params, Value, registry, start_params};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--describe") {
        describe(&args[1..]);
        return;
    }
    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        fail(&e.to_string());
    }
    let req: J = serde_json::from_str(&input).unwrap_or_else(|e| fail(&format!("bad request: {e}")));
    let id = req.get("id").and_then(J::as_str).unwrap_or_else(|| fail("the request has no figure id"));
    let def = registry().into_iter().find(|d| d.id == id).unwrap_or_else(|| fail(&format!("no figure called {id}")));
    let defs = (def.params)();
    let mut p: Params = start_params(&def);
    if let Some(map) = req.get("params").and_then(J::as_object) {
        for (k, v) in map {
            let Some(d) = defs.iter().find(|d| d.key == k) else {
                fail(&format!("{id} has no parameter {k}"));
            };
            p.0.insert(k.clone(), value(&d.kind, v).unwrap_or_else(|| fail(&format!("{id}.{k}: {v} is not a valid value"))));
        }
    }
    p.normalize(&defs, false);
    print!("{}", (def.render)(&p));
}

fn value(kind: &Kind, v: &J) -> Option<Value> {
    Some(match kind {
        Kind::Float { .. } => Value::F(v.as_f64()?),
        Kind::Int { .. } => Value::I(v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64))?),
        Kind::Bool => Value::B(v.as_bool()?),
        _ => Value::S(match v {
            J::String(s) => s.clone(),
            other => other.to_string(),
        }),
    })
}

fn describe(ids: &[String]) {
    for def in registry() {
        if !ids.is_empty() && !ids.iter().any(|i| i == def.id) {
            continue;
        }
        for d in (def.params)() {
            let kind = match &d.kind {
                Kind::Float { min, max, .. } => format!("number {min}..{max}"),
                Kind::Int { min, max } => format!("integer {min}..{max}"),
                Kind::Bool => "true|false".into(),
                Kind::Choice(v) => v.join("|"),
                Kind::Color(v) => format!("colour {}", v.join("|")),
                Kind::Text => "text".into(),
                Kind::Long => "long text".into(),
                Kind::List => "list".into(),
                Kind::Data { columns } => format!("data, {columns} columns"),
            };
            let default = match &d.default {
                Value::F(f) => f.to_string(),
                Value::I(i) => i.to_string(),
                Value::B(b) => b.to_string(),
                Value::S(s) => format!("{s:?}"),
            };
            println!("{}\t{}\t{}\t{}\t{}", def.id, d.key, kind, default, d.label);
        }
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("vershion-figure: {msg}");
    std::process::exit(1);
}
