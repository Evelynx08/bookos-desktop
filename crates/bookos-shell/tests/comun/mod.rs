//! Lo que comparten las pruebas de dibujo del shell.
//!
//! Vive en un subdirectorio a propósito: Cargo compila cada `.rs` suelto de
//! `tests/` como un binario aparte, y un `mod.rs` dentro de una carpeta no. Así
//! los ayudantes se comparten sin convertirse en una prueba vacía.

#![allow(dead_code)]

use bookos_shell::{Config, Shell};
use image::ImageEncoder as _;

pub const ANCHO: u32 = 1280;

/// Siempre con la configuración por defecto: `Shell::new` leería el
/// `panel.conf` del usuario y el test pasaría o fallaría según lo que tenga
/// puesto en su casa.
pub fn shell(escala: f32) -> Shell {
    static DATA: std::sync::Once = std::sync::Once::new();
    DATA.call_once(|| bookos_system::seed_test_state(bookos_system::State {
        network: serde_json::json!({"enabled":true,"ssid":"BookOS prueba","ethernet":{"connected":false},"networks":[{"ssid":"BookOS prueba","signal":80,"active":true}]}),
        bluetooth: serde_json::json!({"present":true,"enabled":true,"devices":[]}),
        audio: serde_json::json!({"output":{"volume":50,"muted":false},"input":{"volume":70,"muted":false}}),
        ..Default::default()
    }));
    // `BOOKOS_TEMA=claro` pinta todos los volcados con la paleta clara. Es la
    // forma de mirarla sin arrancar una sesión, y va aquí y no en cada test
    // porque el tema es del proceso: a medias no se puede ver.
    let mut config = Config::default();
    if std::env::var("BOOKOS_TEMA").as_deref() == Ok("claro") {
        config.tema = bookos_shell::tema::Tema::Claro;
    }
    Shell::con_config(ANCHO, escala, config)
}

pub fn pinta(escala: f32) -> (Vec<u8>, u32, u32) {
    let mut shell = shell(escala);
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let damage = shell.draw_panel(&mut buf);
    assert_eq!(damage.len(), 1, "el panel se repinta entero");
    (buf, w, h)
}

/// El brillo de un píxel del buffer, de 0 a 255.
pub fn luma(buf: &[u8], w: u32, x: u32, y: u32) -> i32 {
    let i = ((y * w + x) * 4) as usize;
    (buf[i] as i32 + buf[i + 1] as i32 + buf[i + 2] as i32) / 3
}

/// El brillo del fondo de una superficie: el valor que más se repite.
///
/// La moda y no una esquina concreta porque el margen no siempre está vacío —el
/// nombre del panel empieza casi pegado al borde—, y no la media porque un
/// texto claro sobre fondo oscuro la desplaza. Lo que domina en cualquiera de
/// estas superficies es el fondo, por definición.
pub fn fondo_luma(buf: &[u8], w: u32, h: u32) -> i32 {
    let mut cuentas = [0u32; 256];
    for y in 0..h {
        for x in 0..w {
            cuentas[luma(buf, w, x, y) as usize] += 1;
        }
    }
    cuentas
        .iter()
        .enumerate()
        .max_by_key(|(_, n)| **n)
        .map(|(v, _)| v as i32)
        .unwrap_or(0)
}

/// Si un píxel se aparta del fondo lo bastante como para ser algo dibujado.
///
/// El criterio es la **diferencia** con el fondo y no un umbral absoluto: en
/// tema oscuro la tinta sube el brillo y en claro lo baja, y un `> 60` fijo
/// daba por dibujado el panel entero en cuanto la paleta se aclaró.
///
/// 12 y no más: el logo del panel tiene paleta propia —lila y azul claro, no se
/// tiñe— y sobre el panel claro se separa del fondo solo 17 puntos de brillo.
pub fn distinto_del_fondo(luma_px: i32, fondo: i32) -> bool {
    (luma_px - fondo).abs() > 12
}

/// Cuántos píxeles no son el fondo en una franja vertical dada, en tanto por
/// mil. Sirve para preguntar "¿hay algo dibujado *ahí*?" sin fijar colores.
pub fn tinta(buf: &[u8], w: u32, h: u32, desde: u32, hasta: u32) -> u32 {
    let fondo = fondo_luma(buf, w, h);
    let mut con_tinta: u32 = 0;
    let mut total = 0;
    for y in 0..h {
        for x in desde..hasta.min(w) {
            total += 1;
            if distinto_del_fondo(luma(buf, w, x, y), fondo) {
                con_tinta += 1;
            }
        }
    }
    (con_tinta * 1000).checked_div(total).unwrap_or(0)
}

/// Pone `src` encima de `dst` con alfa, para componer el volcado igual que
/// hace el compositor. Los dos están en B,G,R,A premultiplicado.
pub fn mezclar(dst: &mut [u8], dw: u32, src: &[u8], sw: u32, sh: u32, x0: u32, y0: u32) {
    for y in 0..sh {
        for x in 0..sw {
            let (dx, dy) = (x + x0, y + y0);
            if dx >= dw {
                continue;
            }
            let si = ((y * sw + x) * 4) as usize;
            let di = ((dy * dw + dx) * 4) as usize;
            if di + 3 >= dst.len() {
                continue;
            }
            let a = src[si + 3] as u32;
            for c in 0..3 {
                // `src` ya viene premultiplicado, así que es sobre-encima
                // clásico: src + dst*(1-a).
                dst[di + c] =
                    (src[si + c] as u32 + dst[di + c] as u32 * (255 - a) / 255).min(255) as u8;
            }
        }
    }
}

/// Guarda el buffer como PNG si la variable está puesta.
pub fn volcar(variable: &str, buf: &[u8], w: u32, h: u32) {
    let Some(destino) = std::env::var_os(variable) else {
        return;
    };
    // El buffer sale en B,G,R,A y el PNG se escribe en R,G,B,A: sin este
    // cambio el volcado enseña los azules en naranja y manda a buscar un
    // fallo de color que no existe.
    let mut rgba = buf.to_vec();
    for p in rgba.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    let fichero = std::fs::File::create(&destino).unwrap();
    image::codecs::png::PngEncoder::new(std::io::BufWriter::new(fichero))
        .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
}
