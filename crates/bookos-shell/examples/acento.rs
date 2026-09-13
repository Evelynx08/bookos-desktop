//! Vuelca el centro de control con cada uno de los diez acentos.
//!
//! `cargo run -p bookos-shell --example acento` deja en `/tmp/acento/` una
//! imagen por acento y por tema. Existe porque «el widget sigue al acento» no
//! se puede comprobar leyendo el código: lo que hay que ver es si el conmutador
//! apagado se distingue del encendido con los diez, y si la tinta de encima se
//! lee sobre el amarillo tanto como sobre el azul.
//!
//! El acento es un estático del proceso, así que las variantes se pintan en
//! serie y no en paralelo.

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder as _};

use bookos_shell::tema::{Acento, Tema};
use bookos_shell::{Config, Emergente, Shell};

const ESCALA: f32 = 2.0;

fn main() {
    let salida = std::path::Path::new("/tmp/acento");
    if let Err(err) = std::fs::create_dir_all(salida) {
        eprintln!("no se pudo crear {}: {err}", salida.display());
        return;
    }

    for tema in [Tema::Oscuro, Tema::Claro] {
        for acento in Acento::TODOS {
            // El tema y el acento van **en la configuración**, no puestos
            // antes con `tema::aplicar_*`: `Shell::con_config` los reaplica
            // desde la config al construirse y pisaría lo que se hubiera
            // dejado puesto. Comprobado — con `aplicar_acento` antes, los diez
            // volcados salían azules.
            let config = Config {
                tema,
                acento,
                ..Config::default()
            };
            let mut shell = Shell::con_config(1280, ESCALA, config);
            shell.abrir(Emergente::centro());
            let Some((w, h)) = shell.emergente_buffer_size() else {
                eprintln!("el centro no abrió superficie");
                return;
            };
            let mut buf = vec![0u8; (w * h * 4) as usize];
            shell.draw_emergente(&mut buf);

            // El buffer sale en B,G,R,A y el PNG va en R,G,B,A: sin el cambio
            // el volcado enseña los azules en naranja y manda a buscar un
            // fallo de color que no existe. Es lo mismo que hace `volcar` en
            // los tests.
            for p in buf.chunks_exact_mut(4) {
                p.swap(0, 2);
            }

            let nombre = format!(
                "{}-{}.png",
                format!("{acento:?}").to_lowercase(),
                if matches!(tema, Tema::Claro) {
                    "claro"
                } else {
                    "oscuro"
                }
            );
            let ruta = salida.join(&nombre);
            let fichero = match std::fs::File::create(&ruta) {
                Ok(f) => f,
                Err(err) => {
                    eprintln!("no se pudo crear {}: {err}", ruta.display());
                    return;
                }
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
}
