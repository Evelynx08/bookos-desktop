//! Previsualiza los iconos del escritorio sobre el fondo, en PNG.
//!
//! `BOOKOS_ESCRITORIO=/tmp/escritorio-demo cargo run -p bookos-shell --example escritorio`
//! deja en `/tmp/escritorio/` la rejilla con una selección puesta y la banda
//! elástica encima. Existe porque los iconos se dibujan **sobre el fondo de
//! pantalla**: si el nombre se lee o no es la única pregunta que importa aquí,
//! y no se contesta leyendo el árbol de widgets.

use bookos_shell::{Config, Shell, escritorio::CELDA};

const ESCALA: f32 = 2.0;
/// Tamaño lógico de la pantalla de mentira.
const PANTALLA: (f32, f32) = (1440.0, 900.0);

fn main() {
    preparar_carpeta();
    std::fs::create_dir_all("/tmp/escritorio").unwrap();

    use bookos_shell::tema::Tema::*;
    for (etiqueta, tema) in [("oscuro", Oscuro), ("claro", Claro)] {
        let (fw, fh) = ((PANTALLA.0 * ESCALA) as u32, (PANTALLA.1 * ESCALA) as u32);
        let mut lienzo = fondo(fw, fh);

        let mut shell = Shell::con_config(
            PANTALLA.0 as u32,
            ESCALA,
            Config {
                tema,
                ..Config::default()
            },
        );
        shell.escritorio_pantalla(PANTALLA, ESCALA);
        // Dos seleccionados, para ver la pastilla de acento junto a la normal.
        shell.escritorio_mut().seleccionar(Some(1), false);
        shell.escritorio_mut().seleccionar(Some(2), true);

        let (cw, ch) = shell.escritorio_buffer_size();
        for i in 0..shell.escritorio().cuantos() {
            let Some((x, y, _, _)) = shell.escritorio().rect(i) else {
                continue;
            };
            let mut celda = vec![0u8; (cw * ch * 4) as usize];
            shell.draw_escritorio(i, &mut celda);
            pegar(
                &mut lienzo,
                (fw, fh),
                &celda,
                (cw, ch),
                ((x * ESCALA) as i32, (y * ESCALA) as i32),
            );
        }

        // Y la banda elástica, tal como la compone el compositor: relleno flojo
        // y un filo de acento.
        let acento = bookos_shell::tema::acento();
        let banda = (
            (CELDA.0 * 0.6 * ESCALA) as i32,
            (100.0 * ESCALA) as i32,
            (CELDA.0 * 2.4 * ESCALA) as i32,
            (CELDA.1 * 2.2 * ESCALA) as i32,
        );
        rectangulo(&mut lienzo, (fw, fh), banda, acento, 0.14);
        let filo = ESCALA as i32;
        for borde in [
            (banda.0, banda.1, banda.2, filo),
            (banda.0, banda.1 + banda.3 - filo, banda.2, filo),
            (banda.0, banda.1, filo, banda.3),
            (banda.0 + banda.2 - filo, banda.1, filo, banda.3),
        ] {
            rectangulo(&mut lienzo, (fw, fh), borde, acento, 0.9);
        }

        volcar(&format!("escritorio-{etiqueta}"), &lienzo, fw, fh);
    }
}

/// Unos cuantos ficheros de mentira para tener algo que dibujar.
fn preparar_carpeta() {
    let Some(dir) = std::env::var_os("BOOKOS_ESCRITORIO") else {
        println!("sin BOOKOS_ESCRITORIO se dibuja el escritorio de verdad");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(dir.join("Proyectos")).unwrap();
    for nombre in [
        "Notas de la reunión.txt",
        "captura de pantalla 2026-08-20.png",
        "informe-trimestral.pdf",
        "musica.mp3",
        "copia-de-seguridad.tar.gz",
    ] {
        let _ = std::fs::write(dir.join(nombre), b"");
    }
    let _ = std::fs::write(
        dir.join("konsole.desktop"),
        b"[Desktop Entry]\nType=Application\nName=Terminal\nExec=konsole\nIcon=utilities-terminal\n",
    );
}

/// El fondo de pantalla de BookOS si está, y si no, un degradado: lo que
/// importa es que debajo del nombre haya algo con color, no cuál.
fn fondo(w: u32, h: u32) -> Vec<u8> {
    let mut lienzo = vec![0u8; (w * h * 4) as usize];
    let candidato = std::env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home)
            .join("Descargas/BookOS/BookOS-Wallpapers/Wallpapers-0.6/Dark/blue_dark.png")
    });
    if let Some((pixeles, iw, ih)) = candidato
        .as_deref()
        .and_then(bookos_shell::decodificar_rgba)
    {
        for y in 0..h {
            for x in 0..w {
                // Vecino más próximo: es una previa, no una imagen final.
                let sx = (x as u64 * iw as u64 / w as u64).min(iw as u64 - 1) as u32;
                let sy = (y as u64 * ih as u64 / h as u64).min(ih as u64 - 1) as u32;
                let o = ((sy * iw + sx) * 4) as usize;
                let d = ((y * w + x) * 4) as usize;
                lienzo[d..d + 4].copy_from_slice(&pixeles[o..o + 4]);
            }
        }
        return lienzo;
    }
    for y in 0..h {
        for x in 0..w {
            let d = ((y * w + x) * 4) as usize;
            lienzo[d] = 20 + (x * 40 / w) as u8;
            lienzo[d + 1] = 60 + (y * 60 / h) as u8;
            lienzo[d + 2] = 110 + (y * 80 / h) as u8;
            lienzo[d + 3] = 255;
        }
    }
    lienzo
}

/// Compone una celda —B,G,R,A premultiplicado, tal como sale de iced— sobre el
/// lienzo, que va en R,G,B,A.
fn pegar(
    lienzo: &mut [u8],
    (w, h): (u32, u32),
    celda: &[u8],
    (cw, ch): (u32, u32),
    (px, py): (i32, i32),
) {
    for y in 0..ch {
        for x in 0..cw {
            let (dx, dy) = (px + x as i32, py + y as i32);
            if dx < 0 || dy < 0 || dx >= w as i32 || dy >= h as i32 {
                continue;
            }
            let o = ((y * cw + x) * 4) as usize;
            let (b, g, r, a) = (celda[o], celda[o + 1], celda[o + 2], celda[o + 3]);
            if a == 0 {
                continue;
            }
            let d = ((dy as u32 * w + dx as u32) * 4) as usize;
            let inv = 255 - a as u32;
            lienzo[d] = (r as u32 + lienzo[d] as u32 * inv / 255) as u8;
            lienzo[d + 1] = (g as u32 + lienzo[d + 1] as u32 * inv / 255) as u8;
            lienzo[d + 2] = (b as u32 + lienzo[d + 2] as u32 * inv / 255) as u8;
            lienzo[d + 3] = 255;
        }
    }
}

fn rectangulo(
    lienzo: &mut [u8],
    (w, h): (u32, u32),
    (rx, ry, rw, rh): (i32, i32, i32, i32),
    color: iced_core::Color,
    alfa: f32,
) {
    for y in ry.max(0)..(ry + rh).min(h as i32) {
        for x in rx.max(0)..(rx + rw).min(w as i32) {
            let d = ((y as u32 * w + x as u32) * 4) as usize;
            for (i, c) in [color.r, color.g, color.b].into_iter().enumerate() {
                let fondo = lienzo[d + i] as f32 / 255.0;
                lienzo[d + i] = ((c * alfa + fondo * (1.0 - alfa)) * 255.0) as u8;
            }
        }
    }
}

fn volcar(nombre: &str, buf: &[u8], w: u32, h: u32) {
    use image::ImageEncoder;
    let ruta = format!("/tmp/escritorio/{nombre}.png");
    let f = std::fs::File::create(&ruta).unwrap();
    image::codecs::png::PngEncoder::new(std::io::BufWriter::new(f))
        .write_image(buf, w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
    println!("{ruta}  {w}×{h}");
}
