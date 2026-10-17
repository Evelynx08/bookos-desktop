//! Previsualiza la entrada y salida reales sin arrancar una sesión completa.
//! `cargo run -p bookos-shell --example avisos` guarda los fotogramas en /tmp/avisos.

use std::time::{Duration, Instant};

use bookos_shell::notificaciones::Notificacion;
use bookos_shell::{Config, Shell};
use tiny_skia::{Pixmap, PixmapPaint, Transform};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let salida = std::path::Path::new("/tmp/avisos");
    std::fs::create_dir_all(salida)?;
    let mut shell = Shell::con_config(640, 1.0, Config::default());
    shell.notificar(
        Notificacion::nueva(
            1,
            "BookOS".into(),
            "Descarga terminada".into(),
            "El archivo ya está disponible".into(),
            "",
            false,
        ),
        -1,
    );
    let (w, h) = shell
        .toast_buffer_size(1)
        .expect("el aviso tiene superficie");
    let mut buffer = vec![0; (w * h * 4) as usize];
    shell.draw_toast(1, &mut buffer);
    for p in buffer.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    let tarjeta = Pixmap::from_vec(
        buffer,
        tiny_skia::IntSize::from_wh(w, h).expect("tamaño del aviso"),
    )
    .expect("buffer completo del aviso");
    let inicio = Instant::now();
    for i in 0..=40 {
        let instante = inicio + Duration::from_millis(i * 16);
        std::thread::sleep(instante.saturating_duration_since(Instant::now()));
        if i == 25 {
            shell.descartar_toast(1);
        }
        let (alfa, escala, y) = shell
            .toast_estado(1)
            .expect("aviso conservado durante la salida");
        let x = shell
            .toast_desplazamiento_horizontal(1)
            .expect("aviso con posición");
        let mut escena = Pixmap::new(640, 240).expect("lienzo de tamaño fijo");
        escena.fill(tiny_skia::Color::from_rgba8(25, 28, 33, 255));
        escena.draw_pixmap(
            0,
            0,
            tarjeta.as_ref(),
            &PixmapPaint {
                opacity: alfa,
                ..Default::default()
            },
            Transform::from_row(
                escala,
                0.0,
                0.0,
                escala,
                220.0 + x + w as f32 * (1.0 - escala) / 2.0,
                32.0 + y + h as f32 * (1.0 - escala) / 2.0,
            ),
            None,
        );
        escena.save_png(salida.join(format!("{i:02}.png")))?;
    }
    println!("fotogramas guardados en {}", salida.display());
    Ok(())
}
