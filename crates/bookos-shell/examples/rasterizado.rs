//! Qué cuesta cada pieza del rasterizado de una tarjeta, con tiny-skia a
//! pelo: 704×896, la del centro de control a escala 2.
//!
//! `cargo run -p bookos-shell --example rasterizado --release`
use std::time::Instant;
use tiny_skia::{Color, FillRule, Mask, Paint, PathBuilder, Pixmap, Rect, Transform};

fn medir(nombre: &str, mut f: impl FnMut()) {
    f();
    let veces = 60;
    let t = Instant::now();
    for _ in 0..veces {
        f();
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / veces as f64;
    println!("{nombre:<44} {ms:>6.2} ms");
}

fn redondeado(w: f32, h: f32, r: f32) -> tiny_skia::Path {
    let mut pb = PathBuilder::new();
    pb.move_to(r, 0.0);
    pb.line_to(w - r, 0.0);
    pb.quad_to(w, 0.0, w, r);
    pb.line_to(w, h - r);
    pb.quad_to(w, h, w - r, h);
    pb.line_to(r, h);
    pb.quad_to(0.0, h, 0.0, h - r);
    pb.line_to(0.0, r);
    pb.quad_to(0.0, 0.0, r, 0.0);
    pb.close();
    pb.finish().expect("camino válido")
}

fn main() {
    let (w, h) = (704u32, 896u32);
    let mut pixmap = Pixmap::new(w, h).expect("tamaño válido");
    let camino = redondeado(w as f32, h as f32, 36.0);
    let opaco = {
        let mut p = Paint::default();
        p.set_color(Color::from_rgba8(28, 28, 30, 255));
        p.anti_alias = true;
        p
    };
    let rect = Rect::from_xywh(0.0, 0.0, w as f32, h as f32).expect("rect");

    medir("fill(transparente)", || pixmap.fill(Color::TRANSPARENT));
    medir("fill_rect opaco sin antialias", || {
        let mut p = opaco.clone();
        p.anti_alias = false;
        pixmap.fill_rect(rect, &p, Transform::identity(), None);
    });
    medir("fill_path redondeado opaco con antialias", || {
        pixmap.fill_path(
            &camino,
            &opaco,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    });
    medir("Mask::new del tamaño del buffer", || {
        std::hint::black_box(Mask::new(w, h));
    });
    let mut m = Mask::new(w, h).expect("máscara del tamaño del pixmap");
    let entera = PathBuilder::from_rect(rect);
    medir("adjust_clip_mask (clear + fill_path rect)", || {
        m.clear();
        m.fill_path(&entera, FillRule::EvenOdd, false, Transform::identity());
    });
    let logico = redondeado(w as f32 / 2.0, h as f32 / 2.0, 18.0);
    medir("fill_path redondeado, lógico × escala 2", || {
        pixmap.fill_path(
            &logico,
            &opaco,
            FillRule::EvenOdd,
            Transform::from_scale(2.0, 2.0),
            None,
        );
    });
    let translucido = {
        let mut p = opaco.clone();
        p.set_color(Color::from_rgba8(255, 255, 255, 30));
        p
    };
    medir("fill_path redondeado TRANSLÚCIDO, × escala 2", || {
        pixmap.fill_path(
            &logico,
            &translucido,
            FillRule::EvenOdd,
            Transform::from_scale(2.0, 2.0),
            None,
        );
    });
    let borde = redondeado(w as f32 / 2.0 - 1.0, h as f32 / 2.0 - 1.0, 17.5);
    medir("stroke_path del borde, 1 lógico × escala 2", || {
        pixmap.stroke_path(
            &borde,
            &opaco,
            &tiny_skia::Stroke {
                width: 1.0,
                ..Default::default()
            },
            Transform::from_scale(2.0, 2.0).pre_translate(0.5, 0.5),
            None,
        );
    });
    let mut mascara = Mask::new(w, h).expect("máscara del tamaño del pixmap");
    mascara.fill_path(&camino, FillRule::Winding, true, Transform::identity());
    medir("fill_rect opaco a través de máscara", || {
        pixmap.fill_rect(rect, &opaco, Transform::identity(), Some(&mascara));
    });
}
