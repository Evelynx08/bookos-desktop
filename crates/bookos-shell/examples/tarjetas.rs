//! Cuánto cuesta repintar el panel y la tarjeta de cada widget.
//!
//! `cargo run -p bookos-shell --example tarjetas --release`
//!
//! `widgets` mide refrescar y las zonas de clic; esto mide el **dibujo**, que es
//! lo que se paga mientras se usa una tarjeta: cada movimiento del ratón sobre
//! una que no lleva realce aparte la repinta entera (`Shell::emergente_puntero`).
//! Mismo panel que el portátil: 2880 px a escala 1,75.

use std::time::Instant;

use bookos_shell::{Config, Shell};

const ANCHO: u32 = 1440;
const ESCALA: f32 = 2.0;
const TODOS: &[&str] = &[
    "reloj",
    "bateria",
    "red",
    "bluetooth",
    "volumen",
    "brillo",
    "notificaciones",
    "control",
    "escritorios",
];

fn main() {
    println!(
        "{:<15} {:>12} {:>12} {:>14}",
        "widget", "panel", "tarjeta", "buffer"
    );
    println!("{}", "-".repeat(56));
    for nombre in TODOS {
        let config = Config {
            centro: None,
            derecha: vec![nombre.to_string()],
            ..Config::default()
        };
        let mut shell = Shell::con_config(ANCHO, ESCALA, config);
        let (pw, ph) = shell.panel_buffer_size();
        let mut panel = vec![0u8; (pw * ph * 4) as usize];
        let veces = 30;
        shell.draw_panel(&mut panel);
        let t = Instant::now();
        for _ in 0..veces {
            shell.draw_panel(&mut panel);
        }
        let ms_panel = t.elapsed().as_secs_f64() * 1000.0 / veces as f64;

        shell.abrir_de_widget(nombre);
        let Some((w, h)) = shell.emergente_buffer_size() else {
            println!("{nombre:<15} {ms_panel:>10.2}ms {:>12}", "sin tarjeta");
            continue;
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_emergente(&mut buf);
        // Lo que pasa al mover el ratón por encima: un punto nuevo y repintar.
        let t = Instant::now();
        for i in 0..veces {
            let y = 20.0 + (i % 10) as f32 * 30.0;
            shell.emergente_puntero(Some((60.0, y)));
            shell.draw_emergente(&mut buf);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / veces as f64;
        println!(
            "{nombre:<15} {ms_panel:>10.2}ms {ms:>10.2}ms {:>14}",
            format!("{w}×{h}")
        );
    }

    // La base: un solo rectángulo redondeado del tamaño de la tarjeta del
    // centro de control. Lo que pase de aquí es contenido, no tamaño.
    let mut shell = Shell::con_config(ANCHO, ESCALA, Config::default());
    println!();
    for (fw, fh) in [(616usize, 1099usize), (308, 550), (154, 275)] {
        let (lw, lh) = (
            bookos_shell::a_pixel_entero(fw as f32 / ESCALA, ESCALA),
            bookos_shell::a_pixel_entero(fh as f32 / ESCALA, ESCALA),
        );
        let (fw, fh) = (
            (lw * ESCALA).round() as usize,
            (lh * ESCALA).round() as usize,
        );
        let mut buf = vec![0u8; fw * fh * 4];
        let veces = 30;
        let t = Instant::now();
        for _ in 0..veces {
            shell.draw_realce(&mut buf, lw, lh, false);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / veces as f64;
        println!("base {fw}×{fh}, un rectángulo: {ms:.2} ms");
    }
}
