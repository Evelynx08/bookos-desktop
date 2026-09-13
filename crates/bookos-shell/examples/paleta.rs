//! Vuelca la tarjeta de Apariencia con varios acentos.
//!
//! `cargo run -p bookos-shell --example paleta` deja en `/tmp/paleta/` una
//! imagen por acento. Existe para mirar una cosa que no se ve en un test: si la
//! fila de fondos, ordenada por afinidad, propone de verdad la pareja del
//! acento elegido, o si el orden salta a la vista como arbitrario.

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder as _};

use bookos_shell::tema::{Acento, Tema};
use bookos_shell::{Config, Emergente, Shell};

const ESCALA: f32 = 2.0;

fn main() {
    let salida = std::path::Path::new("/tmp/paleta");
    if let Err(err) = std::fs::create_dir_all(salida) {
        eprintln!("no se pudo crear {}: {err}", salida.display());
        return;
    }
    for acento in [
        Acento::Azul,
        Acento::Verde,
        Acento::Rojo,
        Acento::Morado,
        Acento::Grafito,
    ] {
        // Por la config y no con `tema::aplicar_acento`: `con_config` la
        // reaplica al construirse y pisaría lo que se dejara puesto antes.
        let config = Config {
            tema: Tema::Oscuro,
            acento,
            ..Config::default()
        };
        let mut shell = Shell::con_config(1280, ESCALA, config);
        shell.abrir(Emergente::apariencia());
        let Some((w, h)) = shell.emergente_buffer_size() else {
            eprintln!("la tarjeta no abrió superficie");
            return;
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_emergente(&mut buf);
        // B,G,R,A a R,G,B,A, igual que `volcar` en los tests.
        for p in buf.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        let ruta = salida.join(format!("{}.png", format!("{acento:?}").to_lowercase()));
        let Ok(fichero) = std::fs::File::create(&ruta) else {
            eprintln!("no se pudo crear {}", ruta.display());
            return;
        };
        match PngEncoder::new(std::io::BufWriter::new(fichero)).write_image(
            &buf,
            w,
            h,
            ExtendedColorType::Rgba8,
        ) {
            Ok(()) => println!("{}  {w}×{h}", ruta.display()),
            Err(err) => eprintln!("no se pudo escribir {}: {err}", ruta.display()),
        }
    }
}
