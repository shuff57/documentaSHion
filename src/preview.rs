//! Headless rendering of the real egui UI for layout review in restricted environments.
use eframe::egui::{
    self,
    epaint::{ImageData, Primitive},
};
pub fn render(path: &str) {
    let ctx = egui::Context::default();
    let mut app = crate::app::App::new(&ctx, false);
    if let Ok(p) = std::env::var("DOCUMENTASHION_PREVIEW_PROJECT") {
        app.project = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
    }
    // A suggested correction for the selected page, to review highlight rendering.
    if let Ok(path) = std::env::var("DOCUMENTASHION_PREVIEW_PROPOSAL") {
        let page = &app.project.pages[0];
        app.proposal = Some(crate::app::Proposal {
            kind: "Correct".into(),
            text: std::fs::read_to_string(path).unwrap(),
            base: page.text.clone(),
            words: vec![],
            geometry: String::new(),
            view: None,
        });
    }
    if std::env::var_os("DOCUMENTASHION_PREVIEW_DARK").is_some() {
        app.settings.dark = true;
        crate::ui::theme::set_dark(true, &ctx);
    }
    if std::env::var_os("DOCUMENTASHION_PREVIEW_SETTINGS").is_some() {
        app.screen = crate::app::Screen::Settings;
    }
    if let Ok(screen) = std::env::var("DOCUMENTASHION_PREVIEW_SCREEN") {
        app.screen = match screen.as_str() {
            "library" => crate::app::Screen::Library,
            "settings" => crate::app::Screen::Settings,
            _ => crate::app::Screen::Document,
        };
    }
    if std::env::var_os("DOCUMENTASHION_PREVIEW_CROP").is_some() {
        app.crop_open = true;
    }
    if let Ok(n) = std::env::var("DOCUMENTASHION_PREVIEW_SECTION") {
        app.settings_section = n.parse().unwrap_or(0);
    }
    let mut textures = std::collections::HashMap::new();
    let mut last = None;
    for pass in 0..4 {
        let mut out = ctx.run_ui(
            egui::RawInput {
                time: Some(1.0 + pass as f64 * 0.3),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440., 920.),
                )),
                ..Default::default()
            },
            |ui| app.draw(ui),
        );
        for (id, deltas) in &out.textures_delta.set {
            for d in deltas {
                let ImageData::Color(img) = &d.image;
                if let Some([x, y]) = d.pos {
                    if let Some(old) = textures.get_mut(id) {
                        let old: &mut egui::ColorImage = old;
                        for iy in 0..img.size[1] {
                            for ix in 0..img.size[0] {
                                old.pixels[(iy + y) * old.size[0] + ix + x] =
                                    img.pixels[iy * img.size[0] + ix];
                            }
                        }
                    }
                } else {
                    textures.insert(*id, (**img).clone());
                }
            }
        }
        out.textures_delta.clear();
        last = Some(out);
    }
    let out = last.unwrap();
    let mut canvas = image::RgbaImage::from_pixel(1440, 920, image::Rgba([247, 244, 235, 255]));
    for clipped in ctx.tessellate(out.shapes, 1.0) {
        let Primitive::Mesh(mesh) = clipped.primitive else {
            continue;
        };
        let Some(tex) = textures.get(&mesh.texture_id) else {
            continue;
        };
        for tri in mesh.indices.chunks_exact(3) {
            let a = mesh.vertices[tri[0] as usize];
            let b = mesh.vertices[tri[1] as usize];
            let c = mesh.vertices[tri[2] as usize];
            let cross = |u: egui::Vec2, v: egui::Vec2| u.x * v.y - u.y * v.x;
            let area = cross(b.pos - a.pos, c.pos - a.pos);
            if area.abs() < 0.0001 {
                continue;
            }
            let minx = a
                .pos
                .x
                .min(b.pos.x)
                .min(c.pos.x)
                .max(clipped.clip_rect.min.x)
                .max(0.) as u32;
            let maxx = a
                .pos
                .x
                .max(b.pos.x)
                .max(c.pos.x)
                .min(clipped.clip_rect.max.x)
                .min(1439.) as u32;
            let miny = a
                .pos
                .y
                .min(b.pos.y)
                .min(c.pos.y)
                .max(clipped.clip_rect.min.y)
                .max(0.) as u32;
            let maxy = a
                .pos
                .y
                .max(b.pos.y)
                .max(c.pos.y)
                .min(clipped.clip_rect.max.y)
                .min(919.) as u32;
            for y in miny..=maxy {
                for x in minx..=maxx {
                    let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = cross(b.pos - p, c.pos - p) / area;
                    let w1 = cross(c.pos - p, a.pos - p) / area;
                    let w2 = 1. - w0 - w1;
                    if w0 < 0. || w1 < 0. || w2 < 0. {
                        continue;
                    }
                    let uv = a.uv.to_vec2() * w0 + b.uv.to_vec2() * w1 + c.uv.to_vec2() * w2;
                    let tx = ((uv.x * tex.size[0] as f32) as usize).min(tex.size[0] - 1);
                    let ty = ((uv.y * tex.size[1] as f32) as usize).min(tex.size[1] - 1);
                    let t = tex.pixels[ty * tex.size[0] + tx].to_array();
                    let ca = a.color.to_array();
                    let cb = b.color.to_array();
                    let cc = c.color.to_array();
                    let mut color = [0.; 4];
                    for k in 0..4 {
                        color[k] = (ca[k] as f32 * w0 + cb[k] as f32 * w1 + cc[k] as f32 * w2)
                            * t[k] as f32
                            / 255.;
                    }
                    let dest = canvas.get_pixel_mut(x, y);
                    let alpha = color[3] / 255.;
                    for k in 0..3 {
                        dest[k] = (color[k] + dest[k] as f32 * (1. - alpha)).min(255.) as u8;
                    }
                }
            }
        }
    }
    canvas.save(path).unwrap();
    println!("Rendered {path}");
}
