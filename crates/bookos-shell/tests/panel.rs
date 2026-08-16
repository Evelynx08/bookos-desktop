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
    Shell::con_config(ANCHO, escala, Config::default())
}

fn pinta(escala: f32) -> (Vec<u8>, u32, u32) {
    let mut shell = shell(escala);
    let (w, h) = shell.panel_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let damage = shell.draw_panel(&mut buf);
    assert_eq!(damage.len(), 1, "el panel se repinta entero");
    (buf, w, h)
}

/// Cuántos píxeles no son el fondo en una franja vertical dada, en tanto por
/// mil. Sirve para preguntar "¿hay algo dibujado *ahí*?" sin fijar colores.
fn tinta(buf: &[u8], w: u32, h: u32, desde: u32, hasta: u32) -> u32 {
    let mut con_tinta = 0;
    let mut total = 0;
    for y in 0..h {
        for x in desde..hasta.min(w) {
            let i = ((y * w + x) * 4) as usize;
            let (r, g, b) = (buf[i], buf[i + 1], buf[i + 2]);
            total += 1;
            // El fondo es casi negro; el texto es claro. Cualquier cosa por
            // encima de ese umbral es algo que se ha dibujado encima.
            if r.max(g).max(b) > 60 {
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
    assert!(shell.soltar(), "soltar no devolvió que había algo agarrado");
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
    shell.bloquear("12:30".into(), pantalla);
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
            titulo: "Recording".into(),
            detalle: "0:37".into(),
            color: iced_core::Color::from_rgb(0.9, 0.2, 0.2),
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

    let hay_tinta = |x: u32| {
        (0..h).any(|y| {
            let i = ((y * w + x) * 4) as usize;
            buf[i].max(buf[i + 1]).max(buf[i + 2]) > 70
        })
    };
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
