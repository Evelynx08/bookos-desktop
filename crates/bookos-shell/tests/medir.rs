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
