//! El panel se pinta de verdad.
//!
//! Es una prueba de humo, no de píxeles: comprobar el color exacto de cada
//! punto ataría el test al tema y saltaría con cada retoque. Lo que se
//! comprueba es lo que de verdad se rompe sin avisar — que `draw_panel` no
//! entra en pánico dentro del proceso del compositor y que deja algo dibujado
//! en la franja donde van los estados.
//!
//! Con `BOOKOS_PANEL_PNG=/ruta/panel.png` además guarda lo pintado, que es la
//! forma más rápida de mirar el panel sin arrancar una sesión.

use bookos_shell::{Config, Shell};
use image::ImageEncoder as _;

const ANCHO: u32 = 1280;

/// Siempre con la configuración por defecto: `Shell::new` leería el
/// `panel.conf` del usuario y el test pasaría o fallaría según lo que tenga
/// puesto en su casa.
fn shell(escala: f32) -> Shell {
    // `BOOKOS_TEMA=claro` pinta todos los volcados con la paleta clara. Es la
    // forma de mirarla sin arrancar una sesión, y va aquí y no en cada test
    // porque el tema es del proceso: a medias no se puede ver.
    let mut config = Config::default();
    if std::env::var("BOOKOS_TEMA").as_deref() == Ok("claro") {
        config.tema = bookos_shell::tema::Tema::Claro;
    }
    Shell::con_config(ANCHO, escala, config)
}

fn pinta(escala: f32) -> (Vec<u8>, u32, u32) {
    let mut shell = shell(escala);
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let damage = shell.draw_panel(&mut buf);
    assert_eq!(damage.len(), 1, "el panel se repinta entero");
    (buf, w, h)
}

/// El brillo de un píxel del buffer, de 0 a 255.
fn luma(buf: &[u8], w: u32, x: u32, y: u32) -> i32 {
    let i = ((y * w + x) * 4) as usize;
    (buf[i] as i32 + buf[i + 1] as i32 + buf[i + 2] as i32) / 3
}

/// El brillo del fondo de una superficie: el valor que más se repite.
///
/// La moda y no una esquina concreta porque el margen no siempre está vacío —el
/// nombre del panel empieza casi pegado al borde—, y no la media porque un
/// texto claro sobre fondo oscuro la desplaza. Lo que domina en cualquiera de
/// estas superficies es el fondo, por definición.
fn fondo_luma(buf: &[u8], w: u32, h: u32) -> i32 {
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
fn distinto_del_fondo(luma_px: i32, fondo: i32) -> bool {
    (luma_px - fondo).abs() > 12
}

/// Cuántos píxeles no son el fondo en una franja vertical dada, en tanto por
/// mil. Sirve para preguntar "¿hay algo dibujado *ahí*?" sin fijar colores.
fn tinta(buf: &[u8], w: u32, h: u32, desde: u32, hasta: u32) -> u32 {
    let fondo = fondo_luma(buf, w, h);
    let mut con_tinta = 0;
    let mut total = 0;
    for y in 0..h {
        for x in desde..hasta.min(w) {
            total += 1;
            if distinto_del_fondo(luma(buf, w, x, y), fondo) {
                con_tinta += 1;
            }
        }
    }
    if total == 0 { 0 } else { con_tinta * 1000 / total }
}

/// Pone `src` encima de `dst` con alfa, para componer el volcado igual que
/// hace el compositor. Los dos están en B,G,R,A premultiplicado.
fn mezclar(dst: &mut [u8], dw: u32, src: &[u8], sw: u32, sh: u32, x0: u32, y0: u32) {
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
fn volcar(variable: &str, buf: &[u8], w: u32, h: u32) {
    let Some(destino) = std::env::var_os(variable) else {
        return;
    };
    // El buffer sale en B,G,R,A y el PNG se escribe en R,G,B,A: sin este
    // cambio el volcado enseña los azules en naranja y manda a buscar un
    // fallo de color que no existe.
    let mut rgba = buf.to_vec();
    for p in rgba.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    let fichero = std::fs::File::create(&destino).unwrap();
    image::codecs::png::PngEncoder::new(std::io::BufWriter::new(fichero))
        .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
}

#[test]
fn dibuja_los_estados_en_su_sitio() {
    let (buf, w, h) = pinta(1.0);

    volcar("BOOKOS_PANEL_PNG", &buf, w, h);

    // El nombre va a la izquierda y el reloj al extremo derecho; los estados
    // de en medio dependen de la máquina, así que no se exigen.
    assert!(tinta(&buf, w, h, 0, 120) > 0, "no se ve el nombre a la izquierda");
    assert!(
        tinta(&buf, w, h, w - 120, w) > 0,
        "no se ve el reloj a la derecha"
    );
}

/// El emergente de Sonido se abre desde el icono del volumen, se dibuja y su
/// deslizador responde al arrastre.
///
/// Con `BOOKOS_SONIDO_PNG=/ruta.png` guarda lo que pinta, para poder mirarlo.
#[test]
fn el_sonido_se_abre_y_su_deslizador_arrastra() {
    let mut shell = shell(1.0);
    // El centro del icono del volumen, sacado de las mismas zonas que usa el
    // compositor para repartir los clics.
    let (_, x0, x1) = shell
        .zonas_panel()
        .into_iter()
        .find(|(n, _, _)| *n == "volumen")
        .expect("el volumen está en el panel de serie");
    shell.panel_pulsado((x0 + x1) / 2.0, 8.0);
    assert!(shell.hay_emergente(), "el volumen no abrió su emergente");

    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_SONIDO_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el emergente sale en blanco");

    // Agarrar la píldora por la izquierda y arrastrarla hasta pasarse del
    // borde derecho: el nivel tiene que acabar al máximo, no a medias.
    shell.emergente_pulsar(20.0, 90.0);
    assert!(shell.emergente_agarrada(), "la píldora no quedó agarrada");
    shell.emergente_puntero(Some((10_000.0, 90.0)));
    assert!(shell.soltar().0, "soltar no devolvió que había algo agarrado");
    assert!(!shell.emergente_agarrada(), "sigue agarrada tras soltar");
}

/// El emergente del brillo se abre desde su icono y se dibuja.
///
/// Con `BOOKOS_BRILLO_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_brillo_se_abre_desde_su_icono() {
    let mut shell = shell(1.0);
    let Some((_, x0, x1)) = shell
        .zonas_panel()
        .into_iter()
        .find(|(n, _, _)| *n == "brillo")
    else {
        // Una máquina sin retroiluminación no dibuja el widget, y entonces no
        // hay nada que abrir: no es un fallo del shell.
        return;
    };
    shell.panel_pulsado((x0 + x1) / 2.0, 8.0);
    assert!(shell.hay_emergente(), "el brillo no abrió su emergente");
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_BRILLO_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el emergente sale en blanco");
}

/// Pasar de página en el launchpad anima: la rejilla entra deslizándose y
/// apareciendo, y el buffer se repinta mientras dure.
///
/// Con `BOOKOS_PAGINA_PNG=/ruta.png` guarda un fotograma **a mitad** de la
/// transición, que es el único que demuestra que hay algo entre las dos
/// páginas.
#[test]
fn el_launchpad_anima_el_cambio_de_pagina() {
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::launchpad((ANCHO as f32, 800.0)));
    assert!(shell.hay_emergente(), "no abrió el launchpad");
    // Un gesto de sobra para pasar el umbral de 20 px.
    if !shell.emergente_desplazar(60.0, 0.0) {
        // Con menos de treinta aplicaciones instaladas no hay segunda página.
        return;
    }
    assert!(shell.emergente_animando(), "el cambio de página no anima");
    assert!(
        shell.emergente_needs_paint(),
        "mientras anima hay que repintar el buffer en cada fotograma"
    );
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_PAGINA_PNG", &buf, w, h);

    // Y termina sola.
    std::thread::sleep(std::time::Duration::from_millis(320));
    assert!(!shell.emergente_animando(), "la transición no acaba");
}

/// Ninguna superficie del shell puede pedirle al compositor un tamaño lógico
/// que no dé píxeles enteros a la escala de la pantalla.
///
/// Si no cuadra, Smithay reescala la textura y el filtrado bilineal emborrona
/// todo el contenido: es lo que hacía que las tarjetas se vieran desenfocadas a
/// escala 1,75 mientras el panel salía nítido.
#[test]
fn las_superficies_caen_en_pixel_entero() {
    for escala in [1.0, 1.25, 1.5, 1.75, 2.0] {
        let mut shell = shell(escala);
        for (nombre, emergente) in [
            ("red", bookos_shell::Emergente::red()),
            ("bluetooth", bookos_shell::Emergente::bluetooth()),
            ("notificaciones", bookos_shell::Emergente::notificaciones()),
            ("centro", bookos_shell::Emergente::centro()),
            ("energia", bookos_shell::Emergente::energia()),
            ("calendario", bookos_shell::Emergente::calendario()),
            ("menu", bookos_shell::Emergente::menu()),
            ("acerca", bookos_shell::Emergente::acerca()),
        ] {
            shell.abrir(emergente);
            let ((lw, lh), _) = shell.emergente_geometria().expect("hay emergente");
            let (bw, bh) = shell.emergente_buffer_size().expect("hay buffer");
            assert_eq!(
                bw,
                (lw as f32 * escala).round() as u32,
                "{nombre} a {escala}: el ancho lógico {lw} no cuadra con el buffer {bw}"
            );
            assert_eq!(
                bh,
                (lh as f32 * escala).round() as u32,
                "{nombre} a {escala}: el alto lógico {lh} no cuadra con el buffer {bh}"
            );
            // Y el físico que pide el compositor tiene que ser entero.
            let pedido = lw as f32 * escala;
            assert!(
                (pedido - pedido.round()).abs() < 0.001,
                "{nombre} a {escala}: pide {pedido} px, que no es entero"
            );
        }
    }
}

/// El panel a la escala de un portátil HiDPI, que es donde se vería un
/// rasterizado a 1× ampliado después: el buffer tiene que salir con los píxeles
/// de verdad, no con los lógicos estirados.
///
/// Con `BOOKOS_PANEL_175_PNG=/ruta.png` lo guarda.
#[test]
fn el_panel_a_escala_fraccionaria_se_rasteriza_a_pixeles_reales() {
    let mut shell = shell(1.75);
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);
    volcar("BOOKOS_PANEL_175_PNG", &buf, w, h);
    // El buffer es físico: 1,75 veces el ancho lógico.
    assert_eq!(w, (ANCHO as f32 * 1.75).round() as u32);
    assert!(h >= 56, "el panel de 32 lógicos son 56 físicos, y salen {h}");
}

/// La otra batería, la de trazo monocromo, en el panel.
///
/// Con `BOOKOS_PANEL_SIMPLE_PNG=/ruta.png` guarda el panel con ella puesta,
/// para poder compararla al lado de la que dibuja el pictograma relleno.
#[test]
fn la_bateria_simple_se_pinta() {
    let mut config = Config::default();
    config.derecha[0] = "bateria-simple".into();
    let mut shell = Shell::con_config(ANCHO, 1.0, config);
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);
    volcar("BOOKOS_PANEL_SIMPLE_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, w / 2, w) > 0, "la mitad derecha sale vacía");
}

/// Cada widget del panel abre su tarjeta al pulsarlo.
///
/// Es lo que se ve desde fuera: que exista el módulo no significa que el clic
/// llegue. Aquí se pulsa el centro de cada zona y se mira qué se abrió.
#[test]
fn todos_los_widgets_abren_su_tarjeta() {
    let esperado = [
        ("bateria", "energia"),
        ("bluetooth", "bluetooth"),
        ("red", "red"),
        ("volumen", "sonido"),
        ("brillo", "brillo"),
        ("notificaciones", "notificaciones"),
        ("control", "centro"),
        ("reloj", "calendario"),
    ];
    let mut shell = shell(1.0);
    let zonas = shell.zonas_panel();
    for (widget, tarjeta) in esperado {
        let Some((_, x0, x1)) = zonas.iter().find(|(n, _, _)| *n == widget).copied() else {
            // Un widget que no se dibuja en esta máquina no tiene zona.
            continue;
        };
        shell.cerrar_emergente();
        shell.panel_pulsado((x0 + x1) / 2.0, 8.0);
        assert_eq!(
            shell.emergente_nombre(),
            Some(tarjeta),
            "pulsar «{widget}» no abrió «{tarjeta}»"
        );
    }
}

/// Las tarjetas de conectividad, con lo que haya en esta máquina.
///
/// Con `BOOKOS_RED_PNG` y `BOOKOS_BT_PNG` las guarda.
#[test]
fn las_tarjetas_de_conectividad_se_pintan() {
    for (emergente, var) in [
        (bookos_shell::Emergente::red(), "BOOKOS_RED_PNG"),
        (bookos_shell::Emergente::bluetooth(), "BOOKOS_BT_PNG"),
        (bookos_shell::Emergente::notificaciones(), "BOOKOS_NOTIF_PNG"),
        (bookos_shell::Emergente::centro(), "BOOKOS_CENTRO_PNG"),
        (bookos_shell::Emergente::energia(), "BOOKOS_ENERGIA_PNG"),
    ] {
        let mut shell = shell(1.0);
        shell.abrir(emergente);
        let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_emergente(&mut buf);
        volcar(var, &buf, w, h);
        assert!(tinta(&buf, w, h, 0, w) > 0, "{var} sale en blanco");
    }
}

/// El aviso de volumen, con la cápsula del diseño.
///
/// Con `BOOKOS_OSD_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_osd_se_pinta() {
    let mut shell = shell(1.0);
    shell.mostrar_osd("volumen-medio", Some(60), None);
    let (w, h) = shell.osd_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_osd(&mut buf);
    volcar("BOOKOS_OSD_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el aviso sale en blanco");

    // Y el de estado, que lleva texto en vez de barra.
    shell.mostrar_osd("touchpad", None, Some("Touchpad desactivado".into()));
    let (w, h) = shell.osd_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_osd(&mut buf);
    volcar("BOOKOS_OSD_TEXTO_PNG", &buf, w, h);

    // El del cambio de escritorio es de esa misma clase, y es lo único que se
    // ve al cambiar a uno vacío: sin ventanas no hay nada que deslizar.
    shell.mostrar_osd("escritorio-2", None, Some("Escritorio 2".into()));
    let (w, h) = shell.osd_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_osd(&mut buf);
    volcar("BOOKOS_OSD_ESCRITORIO_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el aviso del escritorio sale en blanco");
}

/// «Acerca de este PC» se dibuja con los datos de esta máquina.
///
/// Con `BOOKOS_ACERCA_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_acerca_se_pinta() {
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::acerca());
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_ACERCA_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "la tarjeta sale en blanco");
}

/// El menú del clic derecho sobre un icono del dock.
///
/// Con `BOOKOS_MENU_DOCK_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_clic_derecho_del_dock_abre_su_menu() {
    let mut shell = shell(1.0);
    // Sobre una aplicación abierta, para que salga también "Cerrar".
    shell.dock_ventanas(&["org.kde.dolphin".to_string()]);
    let centro_y = bookos_shell::DOCK_PAD + bookos_shell::DOCK_ICON / 2.0;
    // El primero es el launchpad, que no tiene menú; el segundo sí.
    let x_launchpad = bookos_shell::DOCK_PAD + bookos_shell::DOCK_ICON / 2.0;
    assert!(
        !shell.dock_menu(x_launchpad, centro_y),
        "el launchpad no debe tener menú contextual"
    );
    let x = x_launchpad + 2.0 * (bookos_shell::DOCK_ICON + 14.0);
    assert!(shell.dock_menu(x, centro_y), "no abrió el menú del dock");

    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_MENU_DOCK_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el menú sale en blanco");
}

/// Una aplicación abierta sin lanzador entra en el dock y se ve, que es lo que
/// hace falta para poder fijarla con el clic derecho.
///
/// Con `BOOKOS_DOCK_ABIERTA_PNG=/ruta.png` guarda el dock con ella dentro.
#[test]
fn una_ventana_sin_lanzador_sale_en_el_dock() {
    let mut shell = shell(1.0);
    let (w0, _) = shell.dock_buffer_size();
    assert!(
        shell.dock_ventanas(&["org.gnome.Calculator".to_string()]),
        "abrir una ventana desconocida tiene que cambiar el dock"
    );
    let (w, h) = shell.dock_buffer_size();
    assert!(w > w0, "el dock no creció: {w} no es mayor que {w0}");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_dock(&mut buf);
    volcar("BOOKOS_DOCK_ABIERTA_PNG", &buf, w, h);

    // Y al cerrarla, el dock vuelve a su ancho.
    assert!(shell.dock_ventanas(&[]));
    assert_eq!(shell.dock_buffer_size().0, w0);
}

/// El emergente de la energía se abre desde la batería y trae sus perfiles.
///
/// Con `BOOKOS_ENERGIA_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn la_energia_se_abre_desde_la_bateria() {
    let mut shell = shell(1.0);
    let Some((_, x0, x1)) = shell
        .zonas_panel()
        .into_iter()
        .find(|(n, _, _)| *n == "bateria")
    else {
        // Un sobremesa no dibuja el widget de la batería.
        return;
    };
    shell.panel_pulsado((x0 + x1) / 2.0, 8.0);
    assert!(shell.hay_emergente(), "la batería no abrió su emergente");
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_ENERGIA_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el emergente sale en blanco");
}

/// El centro de control, que es la tarjeta con más colores propios del shell:
/// círculos de acento, conmutadores apagados y las tarjetas interiores.
#[test]
fn el_centro_de_control_se_pinta() {
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::centro());
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_CENTRO_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el centro sale en blanco");
}

/// La separación entre widgets del panel, que es la tolerancia con la que se
/// reparten los clics.
fn view_hueco() -> f32 {
    20.0
}

/// Las zonas que el shell declara para el hit-test tienen que caer donde de
/// verdad se ha pintado cada widget.
///
/// Es la comprobación que hace fiable a `Widget::ancho`, que es una estimación
/// y no una medida: si un widget declara de menos, pierde clics en su borde; si
/// declara de más, se los roba al vecino. Aquí se pinta el panel y se mira si
/// hay tinta dentro de cada zona declarada.
#[test]
fn cada_widget_recibe_los_clics_donde_se_dibuja() {
    let escala = 1.0;
    let mut shell = shell(escala);
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);

    let zonas = shell.zonas_panel();
    assert!(!zonas.is_empty(), "el panel no declara ninguna zona");

    // La zona se comprueba con la tolerancia del hueco que separa los widgets.
    // `Widget::ancho` es una estimación —el texto lo mide iced al pintar, y
    // aquí hace falta antes— así que el error se acumula de derecha a
    // izquierda: con el reloj y la batería cambiando de anchura según la hora y
    // el porcentaje, la zona del cuarto widget puede irse unos píxeles. Lo que
    // tiene que cumplirse es que el clic caiga en **su** widget, y para eso
    // basta con que el dibujo esté dentro de la zona más el margen que da el
    // hueco. Exigirlo al píxel hacía fallar el test una vez de cada tres.
    let margen = view_hueco() / 2.0;
    for (nombre, x0, x1) in zonas {
        assert!(x1 > x0, "la zona de {nombre} está vacía: {x0}..{x1}");
        let a = (x0 - margen).max(0.0) as u32;
        let b = ((x1 + margen).max(0.0) as u32).min(w);
        let dentro = tinta(&buf, w, h, a, b);
        assert!(
            dentro > 0,
            "la zona de {nombre} ({a}..{b}) está en blanco: se declara donde no se pinta"
        );
    }
}

/// El panel se pinta a la escala del monitor, así que el buffer crece con
/// ella. Que el dibujo siga cayendo dentro es justo lo que se rompía cuando
/// se confundían píxeles lógicos y físicos.
#[test]
fn aguanta_una_escala_fraccionaria() {
    let (buf, w, h) = pinta(1.75);
    assert_eq!((w, h), ((ANCHO as f32 * 1.75) as u32, 56));
    assert!(tinta(&buf, w, h, 0, 200) > 0, "no se ve nada a esa escala");
}

/// El menú se abre desde el logo, se recorre y se cierra.
///
/// Con `BOOKOS_MENU_PNG=/ruta/menu.png` guarda el menú con una entrada
/// señalada.
#[test]
fn el_menu_se_abre_se_recorre_y_se_cierra() {
    use bookos_shell::TeclaPulsada;

    let mut shell = shell(1.0);
    assert!(!shell.hay_emergente());

    // Pulsar el nombre de la izquierda lo abre.
    shell.panel_pulsado(20.0, 16.0);
    assert!(shell.hay_emergente(), "pulsar el logo tiene que abrir el menú");

    let ((w, h), _ancla) = shell.emergente_geometria().unwrap();
    // Más ancho que los 210 del plasmoide desde que las filas llevan icono.
    assert_eq!(w, 236, "el ancho del menú");
    assert!(h > 0);

    // Bajar señala la primera entrada, y eso obliga a repintar.
    assert!(shell.emergente_tecla(TeclaPulsada::Abajo).0);
    // El realce tarda lo que dice el token de hover; sin esperarlo, el volcado
    // enseña la fila todavía sin encender.
    std::thread::sleep(bookos_shell::tema::D_HOVER);
    let (bw, bh) = shell.emergente_buffer_size().unwrap();
    let mut buf = vec![0u8; (bw * bh * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_MENU_PNG", &buf, bw, bh);
    // El acento del hover es sólido, así que tiene que haber píxeles azules.
    assert!(
        tinta(&buf, bw, bh, 0, bw) > 0,
        "el menú sale vacío"
    );

    // Y Esc lo cierra.
    assert!(shell.emergente_tecla(TeclaPulsada::Escape).0);
    assert!(!shell.hay_emergente(), "Esc tiene que cerrar el menú");
}

/// Pulsar una entrada del menú devuelve su acción y lo cierra.
#[test]
fn elegir_del_menu_lo_cierra() {
    let mut shell = shell(1.0);
    shell.panel_pulsado(20.0, 16.0);

    // La primera entrada, "Acerca de este PC": empieza tras el margen de 2 px
    // y mide 26 de alto.
    let accion = shell.emergente_pulsar(100.0, 2.0 + 13.0);
    assert!(accion.is_some(), "el centro de la primera fila no responde");
    assert!(!shell.hay_emergente(), "elegir algo tiene que cerrar el menú");

    // Y en un separador no hay nada que elegir, así que el menú sigue abierto.
    shell.panel_pulsado(20.0, 16.0);
    assert_eq!(shell.emergente_pulsar(100.0, 2.0 + 26.0 + 3.0), None);
    assert!(shell.hay_emergente(), "fallar la puntería no cierra el menú");
}

/// El calendario se abre desde el reloj del centro.
///
/// Con `BOOKOS_CALENDARIO_PNG=/ruta/cal.png` lo guarda.
#[test]
fn el_calendario_se_abre_desde_el_reloj() {
    let mut shell = shell(1.0);
    // El extremo derecho, que es donde va el reloj.
    shell.panel_pulsado(ANCHO as f32 - 30.0, 16.0);
    assert!(shell.hay_emergente(), "pulsar el reloj abre el calendario");

    let ((w, _h), _) = shell.emergente_geometria().unwrap();
    assert_eq!(w, 340, "el ancho es el de la tarjeta del plasmoide");

    let (bw, bh) = shell.emergente_buffer_size().unwrap();
    let mut buf = vec![0u8; (bw * bh * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_CALENDARIO_PNG", &buf, bw, bh);
    assert!(tinta(&buf, bw, bh, 0, bw) > 0, "el calendario sale vacío");

    // Pulsar el logo con el calendario abierto cambia al menú, no lo cierra.
    shell.panel_pulsado(20.0, 16.0);
    assert_eq!(shell.emergente_geometria().unwrap().0 .0, 236);
}

/// El launchpad se abre y se pinta con las aplicaciones de esta máquina.
///
/// Es una prueba de humo a propósito: cuántas aplicaciones hay depende del
/// sistema donde corra, así que lo que se comprueba es que se abre, tiene el
/// tamaño que dice y deja píxeles. La lógica de búsqueda y de páginas se prueba
/// aparte, con una lista fija.
///
/// Con `BOOKOS_LAUNCHPAD_PNG=/ruta/lp.png` lo guarda.
#[test]
fn el_launchpad_se_abre_y_se_pinta() {
    use bookos_shell::{Emergente, TeclaPulsada};

    let mut shell = shell(1.0);
    shell.abrir(Emergente::launchpad((ANCHO as f32, 800.0)));
    assert!(shell.hay_emergente());

    let (bw, bh) = shell.emergente_buffer_size().unwrap();
    let mut buf = vec![0u8; (bw * bh * 4) as usize];
    shell.draw_emergente(&mut buf);
    assert!(tinta(&buf, bw, bh, 0, bw) > 0, "el launchpad sale vacío");

    // El volcado compone lo que verá el usuario, no solo este buffer: el velo
    // y el realce los pinta el compositor, así que un PNG del buffer a secas
    // enseñaría una rejilla flotando en la nada y no diría nada del aspecto.
    let velo = shell.emergente_velo().expect("el launchpad pide velo");
    shell.emergente_puntero(Some((100.0, 200.0)));
    let realce = shell.emergente_realce();
    let mut compuesto = vec![0u8; (bw * bh * 4) as usize];
    for p in compuesto.chunks_exact_mut(4) {
        // El velo va sobre negro, que es el escritorio vacío.
        p[0] = (velo[2] * velo[3] * 255.0) as u8;
        p[1] = (velo[1] * velo[3] * 255.0) as u8;
        p[2] = (velo[0] * velo[3] * 255.0) as u8;
        p[3] = 255;
    }
    if let Some((rx, ry, rw, rh, marco)) = realce {
        let (mut rbuf, rw_px, rh_px) = {
            let (w, h) = (rw as u32, rh as u32);
            (vec![0u8; (w * h * 4) as usize], w, h)
        };
        shell.draw_realce(&mut rbuf, rw, rh, marco);
        mezclar(&mut compuesto, bw, &rbuf, rw_px, rh_px, rx as u32, ry as u32);
    }
    mezclar(&mut compuesto, bw, &buf, bw, bh, 0, 0);
    volcar("BOOKOS_LAUNCHPAD_PNG", &compuesto, bw, bh);

    // Escribir no lo cierra; Esc con búsqueda solo la limpia, y el segundo sí.
    assert!(shell.emergente_tecla(TeclaPulsada::Caracter('k')).0);
    assert!(shell.hay_emergente());
    assert!(shell.emergente_tecla(TeclaPulsada::Escape).0);
    assert!(shell.hay_emergente(), "el primer Esc solo limpia la búsqueda");
    assert!(shell.emergente_tecla(TeclaPulsada::Escape).0);
    assert!(!shell.hay_emergente(), "el segundo Esc sí cierra");
}

/// Señalar un icono tiene que **verse**. El hit-test ya se prueba aparte; lo
/// que aquí se comprueba es que el resaltado llega hasta los píxeles, que es
/// donde se queda un `hover` que se guarda pero no se dibuja.
///
/// Con `BOOKOS_DOCK_PNG=/ruta/dock.png` guarda el dock con el primer icono
/// señalado.
#[test]
fn el_icono_señalado_se_ve_distinto() {
    let mut shell = shell(1.0);
    let (w, h) = shell.dock_buffer_size();

    let mut apagado = vec![0u8; (w * h * 4) as usize];
    shell.draw_dock(&mut apagado);

    let centro = bookos_shell::DOCK_PAD + bookos_shell::DOCK_ICON / 2.0;
    assert!(
        shell.dock_hover(Some((centro, centro))),
        "señalar un icono nuevo tiene que pedir repintado"
    );
    // Y con una ventana abierta, para ver también el indicador en el volcado.
    shell.dock_ventanas(&["org.kde.dolphin".to_string()]);
    // La placa entra en 120 ms: sin esperarlos, lo que se pinta es el primer
    // fotograma de la animación, casi idéntico al icono apagado.
    std::thread::sleep(bookos_shell::tema::D_HOVER);
    let mut señalado = vec![0u8; (w * h * 4) as usize];
    shell.draw_dock(&mut señalado);
    volcar("BOOKOS_DOCK_PNG", &señalado, w, h);

    assert_ne!(apagado, señalado, "el icono señalado sale idéntico al normal");
    assert!(
        !shell.dock_hover(Some((centro, centro))),
        "señalar lo mismo dos veces no puede costar un repintado"
    );
    assert!(shell.dock_hover(None), "salir del dock tiene que apagarlo");
}

/// La foto de perfil del bloqueo sale de `panel.conf` y se recorta en círculo.
///
/// Con `BOOKOS_BLOQUEO_FOTO_PNG=/ruta.png` guarda lo que sale. La foto se
/// genera aquí —un cuadrado de color— en vez de traerse de la máquina: el test
/// tiene que valer en una donde nadie ha puesto ninguna.
#[test]
fn el_bloqueo_enseña_la_foto_de_la_configuracion() {
    let ruta = std::env::temp_dir().join("bookos-test-avatar.png");
    let foto = image::RgbaImage::from_fn(240, 160, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, 200, 255])
    });
    foto.save(&ruta).expect("se puede escribir en el temporal");

    let config = Config {
        avatar: Some(ruta.to_string_lossy().into_owned()),
        ..Config::default()
    };
    let mut shell = Shell::con_config(ANCHO, 1.0, config);
    let pantalla = (1645.0, 1029.0);
    shell.bloquear("12:30".into(), "lunes, 17 de agosto".into(), pantalla);
    let (w, h) = shell.bloqueo_buffer_size().expect("hay superficie de bloqueo");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_bloqueo(&mut buf);
    volcar("BOOKOS_BLOQUEO_FOTO_PNG", &buf, w, h);

    // El avatar es la mancha opaca de la columna central en la mitad de abajo:
    // se busca en vez de calcular su `y`, que depende de dónde acabe el texto.
    let alfa = |x: u32, y: u32| buf[((y * w + x) * 4 + 3) as usize];
    let centro_x = w / 2;
    let filas: Vec<u32> = ((h / 2)..(h * 4 / 5))
        .filter(|y| alfa(centro_x, *y) > 200)
        .collect();
    let (arriba, abajo) = (
        *filas.first().expect("no hay avatar en la columna central"),
        *filas.last().expect("no hay avatar en la columna central"),
    );
    let diametro = abajo - arriba;
    assert!(
        diametro > 60,
        "el avatar mide {diametro} px de alto: la foto no se pintó"
    );
    // Y es un círculo: la esquina de su cuadrado tiene que estar vacía. Sin el
    // recorte, la foto salía rectangular y llenaba también las esquinas.
    let radio = diametro / 2;
    let esquina = (
        centro_x + (radio as f32 * 0.78) as u32,
        arriba + (radio as f32 * 0.22) as u32,
    );
    assert!(
        alfa(esquina.0, esquina.1) < 40,
        "la esquina del avatar está pintada: la foto no se recortó en círculo"
    );
    let _ = std::fs::remove_file(&ruta);
}

/// La pantalla de bloqueo, pintada sobre el fondo del escritorio.
///
/// Con `BOOKOS_BLOQUEO_PNG=/ruta.png` guarda lo que sale, para compararlo con
/// el diseño. Va al tamaño lógico de la pantalla real —1645×1029, que es
/// 2880×1800 a escala 1,75— porque las medidas del diseño son absolutas: a la
/// mitad, el reloj y el avatar ocupan el doble de lo que les toca.
#[test]
fn el_bloqueo_se_pinta_con_reloj_avatar_y_campo() {
    use bookos_shell::bloqueo::{Estado, Medio};

    let pantalla = (1645.0, 1029.0);
    let mut shell = shell(1.0);
    shell.bloquear("12:30".into(), "lunes, 17 de agosto".into(), pantalla);
    assert!(shell.esta_bloqueado());
    if let Some(b) = shell.bloqueo_mut() {
        b.escritos = 6;
    }

    let (w, h) = shell.bloqueo_buffer_size().expect("hay superficie de bloqueo");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_bloqueo(&mut buf);
    volcar("BOOKOS_BLOQUEO_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "no se ha pintado nada");
    // El reloj vive en la franja de arriba: si no hay tinta ahí, se ha vuelto a
    // quedar sin dibujar, que es justo lo que pasó la primera vez.
    let alto_reloj = h / 4;
    let franja: u32 = (0..alto_reloj)
        .map(|y| {
            (0..w)
                .filter(|x| {
                    let i = ((y * w + x) * 4) as usize;
                    buf[i].max(buf[i + 1]).max(buf[i + 2]) > 60
                })
                .count() as u32
        })
        .sum();
    assert!(franja > 0, "el reloj no se dibujó");

    // Y con algo sonando, la tarjeta se añade sin cambiar el tamaño.
    if let Some(b) = shell.bloqueo_mut() {
        b.estado = Estado::Fallo;
        b.medios.push(Medio {
            bus: "org.mpris.MediaPlayer2.prueba".into(),
            titulo: "Recording".into(),
            detalle: "0:37".into(),
            color: iced_core::Color::from_rgb(0.9, 0.2, 0.2),
            progreso: Some(0.2),
            posicion: Some(37),
            duracion: Some(185),
            reproduciendo: true,
        });
    }
    let mut buf2 = vec![0u8; (w * h * 4) as usize];
    shell.draw_bloqueo(&mut buf2);
    volcar("BOOKOS_BLOQUEO_MEDIOS_PNG", &buf2, w, h);
    assert_eq!(
        shell.bloqueo_buffer_size(),
        Some((w, h)),
        "la tarjeta cambió el tamaño de la superficie"
    );

    // Y con el menú de apagado desplegado, que es la otra variante del diseño.
    if let Some(b) = shell.bloqueo_mut() {
        b.menu = true;
    }
    let mut buf3 = vec![0u8; (w * h * 4) as usize];
    shell.draw_bloqueo(&mut buf3);
    volcar("BOOKOS_BLOQUEO_MENU_PNG", &buf3, w, h);

    shell.desbloquear();
    assert!(!shell.esta_bloqueado());
}

/// El silencio puesto: la tarjeta crece y enseña las cuatro duraciones.
///
/// Con `BOOKOS_NOTIF_DND_PNG=/ruta.png` la guarda. Es el estado que no se ve al
/// abrir la tarjeta y donde estaba el error de altura: el hueco reservado tiene
/// que cuadrar con lo que se pinta.
#[test]
fn el_no_molestar_enseña_sus_duraciones() {
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::notificaciones());
    let (_, alto_antes) = shell.emergente_buffer_size().expect("tiene superficie");
    // El interruptor vive en la fila de «No molestar», a la derecha.
    shell.emergente_pulsar(335.0 - 21.0 - 10.0 - 20.0, 21.0 + 40.0 + 43.0 + 12.0 + 28.0);
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    assert!(h > alto_antes, "con el silencio puesto la tarjeta crece");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_NOTIF_DND_PNG", &buf, w, h);
    // La franja de abajo es la de los chips: si el hueco reservado sobrara,
    // saldría en negro.
    assert!(tinta(&buf, w, h, h - 30, w) > 0, "los chips no se pintan");
}

/// Las zonas de clic del panel caen donde está el dibujo.
///
/// Es el test que faltaba cuando las zonas se estimaban a 7,3 px por carácter:
/// el reloj declaraba 95 px y ocupaba 73, y ese error se acumulaba hacia la
/// izquierda hasta dejar a la batería 10 px fuera de su zona. Se comprueba
/// contra el reloj porque es el único widget que se dibuja en cualquier
/// máquina.
#[test]
fn las_zonas_del_panel_cuadran_con_lo_pintado() {
    let mut shell = shell(1.0);
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);

    let fondo = fondo_luma(&buf, w, h);
    let hay_tinta = |x: u32| (0..h).any(|y| distinto_del_fondo(luma(&buf, w, x, y), fondo));
    let (_, x0, x1) = shell
        .zonas_panel()
        .into_iter()
        .find(|(n, _, _)| *n == "reloj")
        .expect("el reloj sale siempre");
    // Toda la tinta de la derecha del panel tiene que estar dentro de la zona
    // del reloj: si sobresale, su ancho declarado se queda corto.
    let ultima = (0..w).rev().find(|x| hay_tinta(*x)).expect("hay reloj");
    let primera_del_reloj = (x0 as u32..w).find(|x| hay_tinta(*x)).expect("hay reloj");
    assert!(
        (ultima as f32) <= x1,
        "el reloj se pinta hasta {ultima} y su zona acaba en {x1}"
    );
    assert!(
        primera_del_reloj as f32 >= x0,
        "el reloj empieza en {primera_del_reloj}, antes de su zona ({x0})"
    );

    // Y las zonas se tocan entre sí: un hueco entre dos es un sitio del panel
    // donde pulsar no hace nada.
    let mut zonas = shell.zonas_panel();
    zonas.sort_by(|a, b| a.1.total_cmp(&b.1));
    for par in zonas.windows(2) {
        let (izq, der) = (&par[0], &par[1]);
        assert!(
            (der.1 - izq.2).abs() < 0.01,
            "hueco muerto entre «{}» y «{}»: {} .. {}",
            izq.0,
            der.0,
            izq.2,
            der.1
        );
    }
}

/// El indicador de escritorios, con el dato que solo el compositor conoce.
///
/// Con `BOOKOS_ESCRITORIOS_PNG=/ruta.png` guarda el panel con él puesto. Sin el
/// dato el widget no ocupa nada —es lo que hace que no deje un hueco durante el
/// arranque—, así que hay que dárselo antes de mirar.
#[test]
fn el_indicador_de_escritorios_se_pinta_y_responde() {
    let mut shell = shell(1.0);
    assert!(
        shell.zonas_panel().iter().all(|(n, x0, x1)| *n != "escritorios" || x1 - x0 < 0.01),
        "sin dato del compositor no puede ocupar sitio"
    );

    assert!(shell.escritorios(2, 4), "el dato nuevo pide repintar");
    assert!(!shell.escritorios(2, 4), "el mismo dato, no");

    let (_, x0, x1) = shell
        .zonas_panel()
        .into_iter()
        .find(|(n, _, _)| *n == "escritorios")
        .expect("ya tiene zona");
    assert!(x1 - x0 > 20.0, "cuatro puntos ocupan más de 20 px: {}", x1 - x0);

    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_panel(&mut buf);
    volcar("BOOKOS_ESCRITORIOS_PNG", &buf, w, h);
    assert!(
        tinta(&buf, w, h, x0 as u32, x1 as u32) > 0,
        "el indicador no se ve en su zona"
    );

    // Pulsar el primer punto lleva al primer escritorio, no abre ninguna
    // tarjeta: es el único widget del panel que responde por su cuenta.
    let accion = shell.panel_pulsado(x0 + 2.0, 8.0);
    assert_eq!(accion, Some(bookos_shell::Accion::Escritorio(0)));
    assert!(!shell.hay_emergente(), "no debe abrir ninguna tarjeta");

    let paso = (x1 - x0) / 4.0;
    let accion = shell.panel_pulsado(x0 + paso * 2.0 + 1.0, 8.0);
    assert_eq!(accion, Some(bookos_shell::Accion::VistaEscritorios));
}

#[test]
fn la_vista_de_escritorios_se_pinta_y_conserva_las_acciones_de_gestion() {
    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::escritorios(
        (ANCHO as f32, 720.0),
        0,
        vec!["Trabajo".into(), "Personal".into()],
    ));
    assert_eq!(shell.emergente_nombre(), Some("escritorios"));
    assert_eq!(shell.escritorios_miniaturas().len(), 2);

    let (bw, bh) = shell.emergente_buffer_size().unwrap();
    let mut buf = vec![0u8; (bw * bh * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_VISTA_ESCRITORIOS_PNG", &buf, bw, bh);
    assert!(tinta(&buf, bw, bh, 0, bw) > 0, "la vista sale vacía");

    // El + vive suelto en el extremo derecho de la franja, a 24 px del borde:
    // con 1280 px de ancho cae sobre x=1230, centrado con las miniaturas.
    assert_eq!(
        shell.emergente_pulsar(1230.0, 105.0),
        Some(bookos_shell::Accion::CrearEscritorio)
    );
    assert!(shell.hay_emergente(), "crear no debe cerrar la vista");
    assert!(shell.actualizar_vista_escritorios(
        0,
        vec!["Trabajo".into(), "Personal".into(), "Escritorio 3".into()]
    ));
    assert_eq!(shell.escritorios_miniaturas().len(), 3);
}

/// El conmutador de Alt+Tab: la fila de celdas con la elegida realzada.
///
/// Con `BOOKOS_CONMUTADOR_PNG=/ruta.png` guarda lo que pinta. Los iconos se
/// resuelven del tema del sistema, así que en una máquina pelada saldrán huecos
/// —lo que se comprueba aquí es la tarjeta, la selección y el rótulo.
#[test]
fn el_conmutador_se_pinta_y_recorre() {
    let mut shell = shell(1.0);
    let apps: Vec<_> = ["firefox", "konsole", "dolphin"]
        .into_iter()
        .map(|id| bookos_shell::entrada_de_ventana(id, ""))
        .collect();
    assert!(
        shell.abrir_conmutador(
            bookos_shell::conmutador::Modo::Aplicaciones,
            apps,
            (1280.0, 720.0),
        ),
        "con tres celdas se abre"
    );

    let (w, h) = shell.conmutador_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_conmutador(&mut buf);
    volcar("BOOKOS_CONMUTADOR_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el conmutador sale en blanco");

    // El primer paso del compositor lleva de la actual a la anterior.
    shell.conmutador_mover(1);
    assert_eq!(shell.cerrar_conmutador(), Some(1));
    assert!(!shell.hay_conmutador(), "cerrarlo lo suelta");

    // Con una sola celda no hay nada que conmutar.
    let una = vec![bookos_shell::entrada_de_ventana("firefox", "")];
    assert!(
        !shell.abrir_conmutador(
            bookos_shell::conmutador::Modo::Aplicaciones,
            una,
            (1280.0, 720.0),
        ),
        "con una sola no se abre"
    );
}

/// Dos ventanas de la **misma** aplicación: el caso que dejó Alt+Tab muerto.
///
/// Repiten icono y lo único que las distingue es el título, así que se comprueba
/// que la tarjeta se abre igual y que el índice elegido es el de la celda, no
/// algo derivado de la aplicación —que sería el mismo para las dos.
#[test]
fn dos_ventanas_de_la_misma_app_se_conmutan() {
    let mut shell = shell(1.0);
    let ventanas = vec![
        bookos_shell::entrada_de_ventana("org.kde.konsole", "~/proyecto"),
        bookos_shell::entrada_de_ventana("org.kde.konsole", "ssh servidor"),
    ];
    assert!(
        shell.abrir_conmutador(
            bookos_shell::conmutador::Modo::Ventanas,
            ventanas,
            (1280.0, 720.0),
        ),
        "dos ventanas de la misma aplicación tienen que abrir el conmutador"
    );

    let (w, h) = shell.conmutador_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_conmutador(&mut buf);
    volcar("BOOKOS_CONMUTADOR_VENTANAS_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "sale en blanco");

    // El compositor aplica el primer Tab después de abrir.
    shell.conmutador_mover(1);
    assert_eq!(shell.cerrar_conmutador(), Some(1));
}

/// Un título vacío no puede dejar la celda sin rótulo: se cae al nombre de la
/// aplicación. Hay clientes que tardan en poner el título.
#[test]
fn sin_titulo_se_usa_el_nombre_de_la_app() {
    let con = bookos_shell::entrada_de_ventana("konsole", "  ");
    assert!(!con.nombre.is_empty());
}

/// El menú de apagado del bloqueo **se despliega**: no aparece de golpe.
///
/// Se mide sobre el buffer recién dibujado y no sobre un volcado, y se compara
/// el primer fotograma con el de después de la animación: los tiempos exactos
/// dependen de lo que tarde el rasterizado, pero «al pulsar todavía no hay
/// menú» y «un cuarto de segundo después está entero» valen en cualquier
/// máquina.
#[test]
fn el_menu_del_bloqueo_se_despliega() {
    let pantalla = (1645.0, 1029.0);
    let mut shell = shell(1.0);
    shell.bloquear("12:30".into(), "lunes, 17 de agosto".into(), pantalla);
    let (w, h) = shell.bloqueo_buffer_size().expect("hay superficie de bloqueo");

    // El centro del botón de los tres puntos, en la esquina de abajo.
    shell.bloqueo_pulsado(50.0, pantalla.1 - 50.0);

    let alto_menu = |shell: &mut Shell| -> u32 {
        let mut buf = vec![0u8; (w * h * 4) as usize];
        shell.draw_bloqueo(&mut buf);
        let alfa = |y: u32| buf[((y * w + 60) * 4 + 3) as usize];
        // Solo por encima del botón: es donde se despliega el menú.
        let filas: Vec<u32> = (h / 2..h - 90).filter(|y| alfa(*y) > 30).collect();
        match (filas.first(), filas.last()) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        }
    };

    assert_eq!(alto_menu(&mut shell), 0, "el menú ya estaba entero al pulsar");
    std::thread::sleep(std::time::Duration::from_millis(250));
    let final_ = alto_menu(&mut shell);
    assert!(
        final_ > 100,
        "el menú no acabó de desplegarse: {final_} px de alto"
    );
}

/// La tarjeta de Apariencia se pinta, elige color y lo aplica a todo el shell.
///
/// Con `BOOKOS_APARIENCIA_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn la_apariencia_elige_acento_y_repinta_el_shell() {
    use bookos_shell::tema::{self, Acento, Tema};

    // El tema y el acento son globales del proceso y los demás tests corren a
    // la vez: se dejan como estaban al salir.
    let antes = (tema::actual(), tema::acento_actual());

    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::apariencia());
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_APARIENCIA_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "la tarjeta sale en blanco");

    // Pulsar el tercer color pide cambiarlo; el shell no lo aplica solo, eso
    // es cosa del compositor, que es quien además lo guarda.
    let ((lw, _), _) = shell.emergente_geometria().unwrap();
    let accion = shell.emergente_pulsar(lw as f32 / 2.0, 250.0);
    let Some(bookos_shell::Accion::Apariencia { tema: t, acento }) = accion else {
        panic!("pulsar en la rejilla tiene que pedir un cambio: {accion:?}");
    };
    assert_eq!(t, tema::actual(), "el tema no se ha tocado");

    // Y aplicarlo cambia el acento del proceso y obliga a repintar el panel.
    shell.aplicar_apariencia(t, Acento::Verde);
    assert_eq!(tema::acento_actual(), Acento::Verde);
    assert_eq!(tema::acento(), tema::Acento::Verde.color());
    assert!(shell.refresh(), "cambiar de acento tiene que repintar el panel");

    // Cambiar de tema es lo mismo por el otro camino, y lo que se pinta con la
    // otra paleta tiene que ser **otro dibujo**: es la comprobación de que el
    // acento llega hasta el píxel y no se queda en el estado.
    shell.aplicar_apariencia(Tema::Claro, acento);
    assert!(tema::es_claro());
    let mut claro = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut claro);
    volcar("BOOKOS_APARIENCIA_CLARO_PNG", &claro, w, h);
    assert_ne!(buf, claro, "la tarjeta se pinta igual con la otra paleta");

    shell.aplicar_apariencia(antes.0, antes.1);
}

/// El diálogo del botón de encendido se pinta y sus cinco opciones responden.
///
/// Con `BOOKOS_APAGAR_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_dialogo_de_energia_se_pinta_y_responde() {
    use bookos_shell::{Accion, TeclaPulsada};

    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::apagar());
    // Es un diálogo: lleva velo, o lo de debajo no se apaga.
    assert!(shell.emergente_velo().is_some(), "el diálogo sale sin velo");

    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_APAGAR_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el diálogo sale en blanco");

    // Cuatro a la derecha llegan a «Apagar», y ahí Intro apaga.
    for _ in 0..4 {
        assert!(shell.emergente_tecla(TeclaPulsada::Derecha).0);
    }
    let (_, accion) = shell.emergente_tecla(TeclaPulsada::Intro);
    assert!(
        matches!(accion, Some(Accion::Lanzar(ref cmd)) if cmd.contains("poweroff")),
        "Intro sobre «Apagar» tiene que apagar: {accion:?}"
    );
    // Y ejecutar cierra el diálogo.
    assert!(!shell.hay_emergente());

    // Esc lo cierra sin hacer nada.
    shell.abrir(bookos_shell::Emergente::apagar());
    let (repintar, accion) = shell.emergente_tecla(TeclaPulsada::Escape);
    assert!(repintar && accion.is_none());
    assert!(!shell.hay_emergente());
}

/// La tarjeta de notificaciones enseña la cola, y pulsar una la cierra.
///
/// Con `BOOKOS_NOTIF_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn las_notificaciones_se_listan_y_se_cierran() {
    use bookos_shell::notificaciones::Notificacion;
    use bookos_shell::Accion;

    let mut shell = shell(1.0);
    for (i, (app, resumen, cuerpo, critica)) in [
        ("Firefox", "Descarga terminada", "bookos-desktop.tar.gz", false),
        ("Batería", "Batería baja", "Quedan 7 minutos", true),
        ("Konsole", "Compilación lista", "cargo build: 0 errores", false),
    ]
    .into_iter()
    .enumerate()
    {
        shell.notificar(Notificacion::nueva(
            i as u32 + 1,
            app.into(),
            resumen.into(),
            cuerpo.into(),
            "",
            critica,
        ), -1);
    }
    assert_eq!(shell.notificaciones().len(), 3);

    shell.abrir_de_widget("notificaciones");
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_NOTIF_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "la tarjeta sale en blanco");

    // Pulsar la primera fila pide cerrar **esa**, y la tarjeta se queda abierta.
    // La lista empieza bajo la cabecera: margen (21) + cabecera (40).
    let accion = shell.emergente_pulsar(60.0, 75.0);
    let Some(Accion::CerrarNotificacion(id)) = accion else {
        panic!("pulsar una notificación tiene que cerrarla: {accion:?}");
    };
    assert_eq!(id, 3, "la más nueva va arriba");
    assert!(shell.hay_emergente(), "cerrar una no cierra la tarjeta");

    // El shell la retira y el contador baja.
    assert!(shell.cerrar_notificacion(id));
    assert_eq!(shell.notificaciones().len(), 2);

    // Y «Borrar todo» las quita todas, devolviendo a quién hay que avisar.
    let ids = shell.borrar_notificaciones();
    assert_eq!(ids.len(), 2);
    assert!(shell.notificaciones().is_empty());
}

/// El aviso de una notificación se pinta arriba a la derecha y se va solo.
///
/// Con `BOOKOS_TOAST_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_aviso_de_notificacion_se_pinta_y_se_descarta() {
    use bookos_shell::notificaciones::Notificacion;

    let mut shell = shell(1.0);
    shell.notificar(
        Notificacion::nueva(
            1,
            "Firefox".into(),
            "Descarga terminada".into(),
            "bookos-desktop.tar.gz".into(),
            "",
            false,
        ),
        -1,
    );
    let (w, h) = shell.toast_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_toast(&mut buf);
    volcar("BOOKOS_TOAST_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el aviso sale en blanco");
    assert!(shell.toast_vivo());

    // Pulsarlo lo manda a irse, pero la notificación se queda en la lista:
    // despedir el aviso no es haberla atendido.
    assert!(shell.descartar_toast());
    assert!(
        shell.toast_queda().is_some_and(|q| q <= bookos_shell::toast::SALIDA),
        "descartar tiene que dejarle solo la salida"
    );
    assert_eq!(shell.notificaciones().len(), 1);
}

/// «No molestar» sobrevive a cerrar la tarjeta, y mientras está puesto una
/// notificación normal no saca aviso pero una crítica sí.
#[test]
fn el_silencio_sobrevive_y_deja_pasar_lo_critico() {
    use bookos_shell::notificaciones::Notificacion;

    let mut shell = shell(1.0);
    shell.abrir_de_widget("notificaciones");
    // El interruptor de «No molestar», en la fila de su bloque.
    let (w, _) = shell.emergente_geometria().unwrap().0;
    shell.emergente_pulsar(w as f32 - 40.0, 145.0);
    assert!(
        shell.notificaciones_silenciadas(),
        "pulsar el interruptor tiene que silenciar"
    );

    // Cerrar la tarjeta no lo olvida: es lo que pasaba cuando el silencio vivía
    // dentro de la emergente, que se destruye al cerrarla.
    shell.cerrar_emergente();
    assert!(shell.notificaciones_silenciadas(), "el silencio se ha perdido");

    // Silenciado, una normal se guarda sin avisar…
    shell.notificar(
        Notificacion::nueva(1, "Firefox".into(), "Descarga".into(), String::new(), "", false),
        -1,
    );
    assert!(!shell.toast_vivo(), "con silencio no debe salir el aviso");
    assert_eq!(shell.notificaciones().len(), 1, "pero sí se guarda");

    // …y una crítica sale igual: para eso es crítica.
    shell.notificar(
        Notificacion::nueva(2, "Batería".into(), "Batería baja".into(), String::new(), "", true),
        -1,
    );
    assert!(shell.toast_vivo(), "una crítica tiene que salir aunque haya silencio");
}

/// El launchpad con carpetas: la tapa con miniaturas, y entrar y salir.
///
/// Con `BOOKOS_CARPETAS_PNG=/ruta.png` guarda la rejilla con la carpeta, y con
/// `BOOKOS_DENTRO_PNG=/ruta.png` lo que se ve dentro.
#[test]
fn el_launchpad_dibuja_carpetas() {
    // La configuración va a un directorio de usar y tirar: abrir el launchpad
    // lee `launchpad.conf`, y el test no puede depender de lo que tenga quien
    // compile ni escribirle nada.
    let dir = std::env::temp_dir().join("bookos-test-carpetas");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bookos")).unwrap();
    std::fs::write(
        dir.join("bookos/launchpad.conf"),
        "Utilidades = konsole, kate, dolphin\n",
    )
    .unwrap();
    // SAFETY: este test corre en su propio binario y nadie más toca la
    // variable; se restaura al terminar.
    let antes = std::env::var_os("XDG_CONFIG_HOME");
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };

    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::launchpad((1280.0, 800.0)));
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_CARPETAS_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "el launchpad sale en blanco");

    // La carpeta va la primera de la rejilla: pulsarla entra dentro y no lanza
    // nada.
    let accion = shell.emergente_pulsar(40.0, 120.0);
    assert!(accion.is_none(), "pulsar agarra, no lanza: {accion:?}");
    let (_, accion) = shell.soltar();
    assert!(accion.is_none(), "entrar en una carpeta no es una acción");
    assert!(shell.hay_emergente(), "y no cierra el launchpad");

    let mut dentro = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut dentro);
    volcar("BOOKOS_DENTRO_PNG", &dentro, w, h);
    assert_ne!(buf, dentro, "dentro de la carpeta se ve lo mismo que fuera");

    // Elegir un color de la fila tiñe la carpeta. Se comprueba **fuera**, que
    // es donde se ve la tapa: se elige, se sale con Esc y se vuelve a pintar.
    shell.emergente_pulsar(420.0, 78.0);
    shell.emergente_tecla(bookos_shell::TeclaPulsada::Escape);
    let mut teñida = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut teñida);
    volcar("BOOKOS_COLOR_PNG", &teñida, w, h);
    assert_ne!(buf, teñida, "la carpeta se pinta igual con color que sin él");

    // El Esc de arriba ya sacó de la carpeta —y no cerró el launchpad, que es
    // lo que se comprueba aquí—; el siguiente sí lo cierra.
    assert!(shell.hay_emergente(), "salir de la carpeta no puede cerrarlo");
    shell.emergente_tecla(bookos_shell::TeclaPulsada::Escape);
    assert!(!shell.hay_emergente());

    match antes {
        Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
        None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
    }
}

/// La barra de título se pinta y sus tres botones caen donde dice el hit-test.
///
/// Con `BOOKOS_BARRA_PNG=/ruta.png` guarda lo que sale, que es la forma de
/// mirar los glifos sin arrancar una sesión.
#[test]
fn la_barra_de_titulo_pinta_sus_botones() {
    use bookos_shell::decoracion::{boton_en, Boton, Estado};

    const ANCHO_BARRA: f32 = 640.0;
    let mut shell = shell(1.0);
    // Con el cursor sobre el cierre: es el estado que más cosas dibuja —el
    // círculo rojo y la tinta que se lee encima— y el que se rompería sin que
    // ningún otro test se enterase.
    let estado = Estado {
        titulo: "Konsole — bash".into(),
        activa: true,
        maximizada: false,
        señalado: Some(Boton::Cerrar),
        pulsado: None,
    };
    assert!(shell.barra_preparar(1, ANCHO_BARRA, estado), "hay que pintarla");
    let (w, h) = shell.barra_buffer_size(1).expect("la barra recién creada");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let damage = shell.draw_barra(1, &mut buf);
    assert_eq!(damage.len(), 1, "la barra se repinta entera");
    volcar("BOOKOS_BARRA_PNG", &buf, w, h);

    // Los tres botones dejan tinta en su zona.
    for (boton, nombre) in [
        (Boton::Minimizar, "minimizar"),
        (Boton::Maximizar, "maximizar"),
        (Boton::Cerrar, "cerrar"),
    ] {
        // El centro de la zona que el hit-test le asigna, buscado desde el
        // borde derecho: así el test comprueba a la vez el dibujo y la
        // geometría que usa el compositor para repartir los clics.
        let x = (0..ANCHO_BARRA as u32)
            .rev()
            .find(|x| boton_en(ANCHO_BARRA, *x as f32, h as f32 / 2.0) == Some(boton))
            .unwrap_or_else(|| panic!("el hit-test no encuentra {nombre}"));
        assert!(
            tinta(&buf, w, h, x.saturating_sub(10), x) > 0,
            "no se ve el botón de {nombre}"
        );
    }

    // Y el título, en el centro.
    assert!(
        tinta(&buf, w, h, w / 2 - 60, w / 2 + 60) > 0,
        "no se ve el título"
    );

    // Repetir con el mismo estado no obliga a repintar: es lo que evita
    // rasterizar una barra por ventana en cada frame.
    let igual = Estado {
        titulo: "Konsole — bash".into(),
        activa: true,
        maximizada: false,
        señalado: Some(Boton::Cerrar),
        pulsado: None,
    };
    assert!(!shell.barra_preparar(1, ANCHO_BARRA, igual), "repinta de más");
}

/// El buscador se abre, escribe, se recorre con las flechas y lanza con Intro.
///
/// Con `BOOKOS_BUSCADOR_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn el_buscador_busca_y_lanza() {
    use bookos_shell::{Accion, TeclaPulsada};

    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::buscador());
    assert!(shell.hay_emergente());
    assert_eq!(shell.emergente_nombre(), Some("buscador"));
    assert!(shell.emergente_usa_cristal(), "el buscador va sobre cristal");

    // Vacío es solo el campo: sin lista, la tarjeta no puede tener el alto de
    // seis filas vacías.
    let ((w, alto_vacio), _) = shell.emergente_geometria().unwrap();
    assert_eq!(w, 640);

    // «sh» encuentra al menos el ejecutable del PATH, que existe en cualquier
    // Linux; escribir tiene que hacer crecer la tarjeta.
    for c in "sh".chars() {
        assert!(shell.emergente_tecla(TeclaPulsada::Caracter(c)).0);
    }
    let ((_, alto), _) = shell.emergente_geometria().unwrap();
    assert!(alto > alto_vacio, "la lista no apareció al escribir");

    let (bw, bh) = shell.emergente_buffer_size().unwrap();
    let mut buf = vec![0u8; (bw * bh * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_BUSCADOR_PNG", &buf, bw, bh);
    assert!(tinta(&buf, bw, bh, 0, bw) > 0, "el buscador sale vacío");

    // Intro lanza lo elegido y cierra.
    let (repintar, accion) = shell.emergente_tecla(TeclaPulsada::Intro);
    assert!(repintar);
    assert!(
        matches!(accion, Some(Accion::Lanzar(_))),
        "Intro tiene que lanzar lo elegido, y devolvió {accion:?}"
    );
    assert!(!shell.hay_emergente(), "lanzar cierra el buscador");
}

/// Esc borra lo escrito antes de cerrar: lo primero que se quiere deshacer es
/// la búsqueda, no la ventana.
#[test]
fn el_escape_del_buscador_borra_antes_de_cerrar() {
    use bookos_shell::TeclaPulsada;

    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::buscador());
    shell.emergente_tecla(TeclaPulsada::Caracter('s'));
    assert!(shell.emergente_tecla(TeclaPulsada::Escape).0);
    assert!(shell.hay_emergente(), "el primer Esc solo borra lo escrito");
    shell.emergente_tecla(TeclaPulsada::Escape);
    assert!(!shell.hay_emergente(), "el segundo Esc cierra");
}
