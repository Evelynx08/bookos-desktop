//! Cuánto cuesta repintar la pantalla de bloqueo, que va a pantalla completa:
//! se paga en cada tecla de la contraseña y en cada fotograma de la animación
//! del punto que aparece.
//!
//! `cargo run -p bookos-shell --example bloqueo_bench --release`
use std::time::Instant;

use bookos_shell::bloqueo::Estado;
use bookos_shell::{Config, Shell};
// Con la configuración real —`panel.conf`, avatar incluido—: la de serie no
// lleva avatar y mide de menos.

fn main() {
    // El panel del portátil, 2880×1800, a las dos escalas que se usan.
    for escala in [2.0f32, 1.75] {
        let pantalla = (2880.0 / escala, 1800.0 / escala);
        let mut shell = Shell::con_config(pantalla.0 as u32, escala, Config::cargar());
        shell.bloquear("10:42".into(), "lunes, 29 de septiembre".into(), pantalla);
        let (w, h) = shell.bloqueo_buffer_size().expect("bloqueo puesto");
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_bloqueo(&mut buf);
        let veces = 20;
        let t = Instant::now();
        for i in 0..veces {
            shell.bloqueo_estado(i % 12 + 1, Estado::Escribiendo);
            shell.draw_bloqueo(&mut buf);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / veces as f64;
        println!("escala {escala}: {w}×{h}, {ms:.2} ms por tecla");

        // Con música: el bloqueo añade el velo del color de la canción.
        shell.bloqueo_medio(Some(bookos_shell::medios::Sonando {
            bus: "org.mpris.MediaPlayer2.prueba".into(),
            aplicacion: "Prueba".into(),
            caratula: None,
            titulo: "Cielo de invierno".into(),
            artista: "La Habitación Roja".into(),
            reproduciendo: true,
            posicion: Some(78),
            duracion: Some(214),
        }));
        shell.draw_bloqueo(&mut buf);
        let t = Instant::now();
        for i in 0..veces {
            shell.bloqueo_estado(i % 12 + 1, Estado::Escribiendo);
            shell.draw_bloqueo(&mut buf);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / veces as f64;
        println!("           con música: {ms:.2} ms por tecla");
    }
}
