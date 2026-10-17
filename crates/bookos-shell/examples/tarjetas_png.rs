//! Tarjeta de cada widget en PNG, tema claro y oscuro, sobre un fondo de color.
//!
//! `cargo run -p bookos-shell --example tarjetas_png` → `/tmp/tarjetas/`.

use bookos_shell::tema::Tema;
use bookos_shell::{Config, Shell};

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
    std::fs::create_dir_all("/tmp/tarjetas").expect("carpeta de salida");
    for (etiqueta, tema) in [("oscuro", Tema::Oscuro), ("claro", Tema::Claro)] {
        for nombre in TODOS {
            let config = Config {
                tema,
                centro: None,
                derecha: vec![nombre.to_string()],
                ..Config::default()
            };
            let mut shell = Shell::con_config(1440, ESCALA, config);
            shell.abrir_de_widget(nombre);
            let Some((w, h)) = shell.emergente_buffer_size() else {
                continue;
            };
            let mut buf = vec![0u8; (w * h * 4) as usize];
            shell.draw_emergente(&mut buf);
            // B,G,R,A premultiplicado sobre un azul medio, como un fondo.
            let rgba: Vec<u8> = buf
                .as_chunks::<4>().0.iter()
                .flat_map(|p| {
                    let inv = 255 - p[3] as u32;
                    let f = [40u32, 90, 160];
                    [
                        (p[2] as u32 + f[0] * inv / 255) as u8,
                        (p[1] as u32 + f[1] * inv / 255) as u8,
                        (p[0] as u32 + f[2] * inv / 255) as u8,
                        255,
                    ]
                })
                .collect();
            image::save_buffer(
                format!("/tmp/tarjetas/{nombre}-{etiqueta}.png"),
                &rgba,
                w,
                h,
                image::ExtendedColorType::Rgba8,
            )
            .expect("PNG escrito");
        }
    }
}
