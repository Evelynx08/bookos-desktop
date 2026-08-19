//! Previsualiza la isla de actividades en PNG, sin compositor.
//!
//! `cargo run -p bookos-shell --example actividad` deja en `/tmp/actividad/`
//! una imagen por variante y por tema. Existe porque el diseño de la isla no se
//! puede juzgar leyendo el árbol de widgets: hay que **verla**, y arrancar la
//! sesión entera para mirar una tarjeta es el camino largo.
//!
//! El fondo del PNG es transparente, igual que el buffer real: la isla cuelga
//! del borde superior de la pantalla y su sombra tiene que verse contra lo que
//! haya debajo.

use bookos_shell::actividad::{Clase, Estado, ItemCola, Portada};
use bookos_shell::{Config, Shell};

const ESCALA: f32 = 2.0;

fn base(clase: Clase) -> Estado {
    Estado {
        app_id: match clase {
            Clase::Player => "com.bookos.player",
            Clase::Timer => "com.bookos.clock",
            Clase::Recorder => "com.bookos.voicerecorder",
        }
        .into(),
        clase,
        activo: true,
        pausado: false,
        titulo: "Cielo de invierno".into(),
        subtitulo: "La Habitación Roja".into(),
        posicion_ms: 78_000,
        duracion_ms: 214_000,
        restante_ms: 154_000,
        volumen: 65,
        nivel: 0.7,
        aleatorio: true,
        repetir: false,
        portada: Some(portada()),
        cola: vec![
            item("1", "Cielo de invierno", "La Habitación Roja", true, true),
            item("2", "Ayer", "Los Planetas", false, false),
            item("3", "Mediterráneo", "Serrat", true, false),
            item("4", "La estatua del jardín botánico", "Radio Futura", false, false),
        ],
    }
}

fn item(id: &str, titulo: &str, artista: &str, favorita: bool, actual: bool) -> ItemCola {
    ItemCola {
        id: id.into(),
        titulo: titulo.into(),
        artista: artista.into(),
        duracion_ms: 234_000,
        favorita,
        actual,
    }
}

/// Una portada inventada: un degradado diagonal, que es lo que hace falta para
/// ver si el recorte y la sombra del arte están bien.
fn portada() -> Portada {
    let (w, h) = (300u32, 300u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let t = (x + y) as f32 / (w + h) as f32;
            rgba.extend_from_slice(&[
                (30.0 + 200.0 * t) as u8,
                (90.0 + 60.0 * (1.0 - t)) as u8,
                (200.0 - 60.0 * t) as u8,
                255,
            ]);
        }
    }
    Portada { rgba, width: w, height: h }
}

fn volcar(nombre: &str, buf: &[u8], w: u32, h: u32) {
    use image::ImageEncoder;
    // El buffer sale en B,G,R,A y el PNG se escribe en R,G,B,A.
    let mut rgba = buf.to_vec();
    for p in rgba.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    let ruta = format!("/tmp/actividad/{nombre}.png");
    let f = std::fs::File::create(&ruta).unwrap();
    image::codecs::png::PngEncoder::new(std::io::BufWriter::new(f))
        .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
    println!("{ruta}  {w}×{h}");
}

/// Pinta una variante. `clics` son pulsaciones lógicas que se dan antes de
/// dibujar, para llegar a la vista abierta o a la cola por el mismo camino que
/// las daría el usuario.
fn pinta(nombre: &str, tema: bookos_shell::tema::Tema, estado: Estado, clics: &[(f32, f32)]) {
    // El temporizador solo sale si la configuración lo deja verse siempre; sin
    // esto la vista del temporizador en marcha no se puede previsualizar.
    let actividades = bookos_shell::ConfigActividades {
        temporizador_siempre: true,
        ..Default::default()
    };
    let config = Config { tema, actividades, ..Config::default() };
    let mut shell = Shell::con_config(1646, ESCALA, config);
    shell.publicar_actividad(estado);
    for (x, y) in clics {
        shell.actividad_pulsar(*x, *y);
    }
    let Some((w, h)) = shell.actividad_buffer_size() else {
        println!("{nombre}: la isla no se publicó");
        return;
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_actividad(&mut buf);
    volcar(nombre, &buf, w, h);
}

fn main() {
    std::fs::create_dir_all("/tmp/actividad").unwrap();
    use bookos_shell::tema::Tema::*;

    // El primer clic cae en el centro de la tarjeta compacta y la abre; el
    // segundo, en la esquina inferior izquierda del reproductor, saca la cola.
    for (etiqueta, tema) in [("oscuro", Oscuro), ("claro", Claro)] {
        pinta(&format!("player-compacto-{etiqueta}"), tema, base(Clase::Player), &[]);
        pinta(&format!("player-abierto-{etiqueta}"), tema, base(Clase::Player), &[(180.0, 55.0)]);
        pinta(
            &format!("player-cola-{etiqueta}"),
            tema,
            base(Clase::Player),
            // El segundo clic cae en el botón de la cola de la fila del
            // progreso: 26 de margen + la posición que le dan las constantes.
            &[(180.0, 55.0), (26.0 + 370.0, 26.0 + 205.0)],
        );
        pinta(&format!("timer-compacto-{etiqueta}"), tema, base(Clase::Timer), &[]);
        pinta(&format!("timer-abierto-{etiqueta}"), tema, base(Clase::Timer), &[(180.0, 55.0)]);
        let mut vencido = base(Clase::Timer);
        vencido.restante_ms = -7_000;
        pinta(&format!("timer-vencido-{etiqueta}"), tema, vencido, &[(180.0, 55.0)]);
        pinta(&format!("grabadora-compacta-{etiqueta}"), tema, base(Clase::Recorder), &[]);
        pinta(&format!("grabadora-abierta-{etiqueta}"), tema, base(Clase::Recorder), &[(180.0, 55.0)]);
        let mut pausado = base(Clase::Player);
        pausado.pausado = true;
        pausado.portada = None;
        pinta(&format!("player-sin-portada-{etiqueta}"), tema, pausado, &[(180.0, 55.0)]);
    }
}
