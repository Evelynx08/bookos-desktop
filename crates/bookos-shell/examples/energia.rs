//! Previsualiza la batería —el widget del panel y su emergente— en PNG.
//!
//! `cargo run -p bookos-shell --example energia` deja en `/tmp/energia/` una
//! imagen por tema. Existe porque el widget vive dentro del compositor y
//! juzgar su alineación leyendo el árbol de widgets no funciona: hay que verlo.

use bookos_shell::{Config, Shell};

const ESCALA: f32 = 2.0;
const ANCHO: u32 = 1646;

fn volcar(nombre: &str, buf: &[u8], w: u32, h: u32) {
    use image::ImageEncoder;
    // El buffer sale en B,G,R,A y el PNG se escribe en R,G,B,A.
    let mut rgba = buf.to_vec();
    for p in rgba.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    let ruta = format!("/tmp/energia/{nombre}.png");
    let f = std::fs::File::create(&ruta).unwrap();
    image::codecs::png::PngEncoder::new(std::io::BufWriter::new(f))
        .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
    println!("{ruta}  {w}×{h}");
}

fn shell(tema: bookos_shell::tema::Tema) -> Shell {
    let config = Config {
        tema,
        derecha: vec!["bateria".into(), "red".into(), "volumen".into()],
        ..Config::default()
    };
    Shell::con_config(ANCHO, ESCALA, config)
}

fn main() {
    std::fs::create_dir_all("/tmp/energia").unwrap();
    use bookos_shell::tema::Tema::*;
    for (etiqueta, tema) in [("oscuro", Oscuro), ("claro", Claro)] {
        // El panel entero: así se ve la batería junto a sus vecinos, que es
        // donde se nota si el pictograma está desalineado o pesa de más.
        let mut s = shell(tema);
        s.refresh();
        let (w, h) = s.panel_buffer_size();
        let mut buf = vec![0u8; (w * h * 4) as usize];
        s.draw_panel(&mut buf);
        volcar(&format!("panel-{etiqueta}"), &buf, w, h);

        // Y el emergente que sale al pulsarlo.
        let mut s = shell(tema);
        s.refresh();
        s.abrir_de_widget("bateria");
        let Some((w, h)) = s.emergente_buffer_size() else {
            println!("energia-{etiqueta}: no se abrió el emergente");
            continue;
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        s.draw_emergente(&mut buf);
        volcar(&format!("energia-{etiqueta}"), &buf, w, h);
    }
}
