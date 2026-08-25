use bookos_shell::{Config, Shell};

/// Abrir el conmutador dos veces: la segunda tiene que ser barata.
///
/// Es lo que demuestra que la caché de iconos sirve. Sin ella, cada apertura
/// devolvía handles nuevos y iced volvía a rasterizar los SVG enteros: medido en
/// release, 46,6 ms **cada vez**.
#[test]
fn el_conmutador_no_rasteriza_dos_veces() {
    let apps = || -> Vec<_> {
        ["firefox", "konsole", "dolphin", "kate", "kcalc"]
            .into_iter()
            .map(|id| bookos_shell::entrada_de_ventana(id, id))
            .collect()
    };
    let mut shell = Shell::con_config(1280, 1.0, Config::default());

    let mut abrir_y_pintar = || {
        shell.abrir_conmutador(
            bookos_shell::conmutador::Modo::Aplicaciones,
            apps(),
            (1280.0, 720.0),
        );
        let (w, h) = shell.conmutador_buffer_size().expect("tiene superficie");
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let t = std::time::Instant::now();
        shell.draw_conmutador(&mut buf);
        let tardado = t.elapsed();
        shell.cerrar_conmutador();
        tardado
    };

    let primera = abrir_y_pintar();
    let segunda = abrir_y_pintar();
    println!(
        "conmutador: primera apertura {:.2} ms, segunda {:.2} ms",
        primera.as_secs_f64() * 1000.0,
        segunda.as_secs_f64() * 1000.0
    );
    // El umbral es holgado a propósito: lo que se comprueba es que la segunda no
    // vuelve a rasterizar, no un número exacto que dependería de la máquina.
    assert!(
        segunda < primera / 4,
        "la segunda apertura debería reutilizar el rasterizado: {:.2} ms contra {:.2} ms",
        segunda.as_secs_f64() * 1000.0,
        primera.as_secs_f64() * 1000.0
    );
}

/// Dibujar el panel y el dock seguidos cuesta lo que cuestan los dos, no el
/// triple.
///
/// Dentro de cada `Renderer` de iced vive la caché de rasterizado —los SVG ya
/// parseados y los pixmaps ya pintados— y **se purga al final de cada `draw`**:
/// `iced_tiny_skia` tira todo lo que no haya salido en el último dibujo. Con un
/// renderizador compartido, el panel tiraba los iconos del dock y el dock los
/// del panel, así que cada fotograma reparseaba los SVG de los dos: medido en
/// release, 1,19 + 1,39 ms por separado contra **7,61 ms alternándolos**. Con
/// uno para cada uno, 2,53 ms, que es la suma exacta.
///
/// Es el caso que se paga **en todos los fotogramas**, así que se vigila aquí.
#[test]
fn el_panel_y_el_dock_no_se_tiran_la_cache() {
    let mut shell = Shell::con_config(2881, 1.75, Config::default());
    let (pw, ph) = shell.panel_buffer_size();
    let (dw, dh) = shell.dock_buffer_size();
    let mut panel = vec![0u8; (pw * ph * 4) as usize];
    let mut dock = vec![0u8; (dw * dh * 4) as usize];

    // El bucle va escrito a mano y no en un ayudante: `shell` y los dos buffers
    // se toman prestados a la vez, y envolverlo en un cierre solo servía para
    // pelearse con el borrow checker.
    const VECES: u32 = 20;
    let media = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0 / VECES as f64;

    // Una pasada en vacío de cada cosa: la primera carga fuentes e iconos.
    shell.draw_panel(&mut panel);
    shell.draw_dock(&mut dock);

    let t = std::time::Instant::now();
    for _ in 0..VECES {
        shell.draw_panel(&mut panel);
    }
    let solo_panel = media(t);

    let t = std::time::Instant::now();
    for _ in 0..VECES {
        shell.draw_dock(&mut dock);
    }
    let solo_dock = media(t);

    let t = std::time::Instant::now();
    for _ in 0..VECES {
        shell.draw_panel(&mut panel);
        shell.draw_dock(&mut dock);
    }
    let juntos = media(t);
    let solos = solo_panel + solo_dock;

    println!("panel+dock: {solos:.2} ms por separado, {juntos:.2} ms alternados");

    // El umbral es holgado a propósito: lo que se comprueba es que no se
    // rasteriza dos veces, no un número que dependería de la máquina. Con la
    // regresión el factor era 2,9.
    assert!(
        juntos < solos * 1.6,
        "el panel y el dock se están tirando la caché: {juntos:.2} ms alternados contra {solos:.2} ms por separado"
    );
}

/// Refrescar el panel entero tiene que costar **céntimos de milisegundo**.
///
/// El panel se refresca con cada evento del kernel, y llegan en ráfagas: al
/// mover el brillo con la tecla, uno por paso. Lo que se vigila aquí es que
/// nadie vuelva a meter en ese camino una consulta al firmware:
///
/// - `actual_brightness` cuesta 0,512 ms medidos —le pregunta al hardware—
///   frente a los 0,030 de `brightness`;
/// - `platform_profile` cuesta 0,711 ms, porque es una llamada a la ACPI;
/// - `current_now` cuesta 0,531 ms, que es el controlador embebido.
///
/// Con las tres dentro, refrescar costaba 1,9 ms; ahora son 0,04. El umbral es
/// holgado a propósito —esto corre sin optimizar y en máquinas distintas—: lo
/// que caza es el orden de magnitud, no un número.
#[test]
fn refrescar_el_panel_no_toca_el_firmware() {
    let mut shell = Shell::con_config(1646, 1.75, Config::default());
    // La primera lee lo que toque leer una vez por sesión.
    shell.refresh();

    let veces = 200;
    let t = std::time::Instant::now();
    for _ in 0..veces {
        shell.refresh();
    }
    let media = t.elapsed().as_secs_f64() * 1000.0 / veces as f64;
    println!("refrescar el panel: {media:.3} ms de media");
    assert!(
        media < 0.5,
        "refrescar cuesta {media:.3} ms: alguien ha vuelto a leer del firmware en cada evento"
    );
}

/// Pintar **una celda** del escritorio tiene que costar céntimos de
/// milisegundo.
///
/// Es lo que sostiene la decisión de darle una superficie a cada icono en vez
/// de una capa a pantalla completa: al barrer con la banda elástica se repintan
/// las celdas que el borde acaba de cruzar, una o dos por movimiento del ratón.
/// Con una sola capa de 2881×1801 cada movimiento costaría los 23 ms que mide
/// el comentario de `Shell::paint`, o sea 43 fotogramas por segundo tirados a
/// la basura por mover el ratón.
///
/// Lo que se dibuja es la primera celda del escritorio de quien ejecuta el
/// test, así que el número de abajo depende de si hay algo en la carpeta: con
/// ella vacía —la celda es un buffer transparente de 168×168— salen 0,018 ms
/// sin optimizar. El umbral caza el orden de magnitud, que es lo que sostiene
/// la decisión de repartir el escritorio en superficies pequeñas.
#[test]
fn pintar_una_celda_del_escritorio_es_barato() {
    let mut shell = Shell::con_config(1646, 1.75, Config::default());
    let (w, h) = shell.escritorio_buffer_size();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    // La primera vuelta carga fuentes y rasteriza el icono; lo que se mide es
    // el repintado de después, que es el del barrido.
    shell.draw_escritorio(0, &mut buf);

    let veces = 50;
    let t = std::time::Instant::now();
    for _ in 0..veces {
        shell.draw_escritorio(0, &mut buf);
    }
    let media = t.elapsed().as_secs_f64() * 1000.0 / veces as f64;
    println!("pintar una celda del escritorio ({w}x{h}): {media:.3} ms de media");
    assert!(
        media < 2.0,
        "una celda cuesta {media:.3} ms: la selección con banda se notaría"
    );
}

/// Abrir la tarjeta de Apariencia no puede costar lo que cuesta decodificar los
/// fondos.
///
/// La sección de fondos enseña una miniatura por familia instalada. Con los
/// PNG de 2880×1800 serían **152 ms** medidos para las cuatro —50, 52, 25 y
/// 24—, y eso es tiempo del hilo del compositor: el escritorio entero se
/// quedaría clavado al abrir la tarjeta. Por eso la miniatura es el SVG de al
/// lado, de cuatro kilobytes, que rasteriza resvg al tamaño pedido.
///
/// El umbral es holgado a propósito: lo que se comprueba es que no se está
/// decodificando la foto, no un número exacto que dependería de la máquina.
#[test]
fn abrir_apariencia_no_decodifica_los_fondos() {
    let mut shell = Shell::con_config(1280, 1.0, Config::default());

    let mut abrir_y_pintar = || {
        shell.abrir(bookos_shell::Emergente::apariencia());
        let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let t = std::time::Instant::now();
        shell.draw_emergente(&mut buf);
        let tardado = t.elapsed();
        shell.cerrar_emergente();
        tardado
    };

    let primera = abrir_y_pintar();
    let segunda = abrir_y_pintar();
    println!(
        "apariencia: primera apertura {:.2} ms, segunda {:.2} ms",
        primera.as_secs_f64() * 1000.0,
        segunda.as_secs_f64() * 1000.0
    );
    assert!(
        primera.as_millis() < 120,
        "abrir Apariencia cuesta {:.2} ms: alguien está decodificando los fondos",
        primera.as_secs_f64() * 1000.0
    );
}
