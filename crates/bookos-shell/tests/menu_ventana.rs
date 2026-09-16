mod comun;

#[test]
fn menu_ventana_dibuja_y_envia_el_destino_exacto() {
    use bookos_shell::{Accion, Emergente};
    let mut shell = comun::shell(1.0);
    let opciones = vec![
        ("Siempre encima".into(), Accion::VentanaEncima),
        ("Mover a Escritorio 2".into(), Accion::VentanaEscritorio(1)),
        ("Mover a HDMI-A-1".into(), Accion::VentanaMonitor("HDMI-A-1".into())),
    ];
    shell.abrir(Emergente::menu_ventana(opciones.clone()));
    let (w, h) = shell.emergente_buffer_size().unwrap();
    let mut pixels = vec![0; (w * h * 4) as usize];
    shell.draw_emergente(&mut pixels);
    comun::volcar("BOOKOS_MENU_VENTANA_PNG", &pixels, w, h);
    assert!(comun::tinta(&pixels, w, h, 0, w) > 0);
    for (i, (_, accion)) in opciones.iter().enumerate() {
        shell.abrir(Emergente::menu_ventana(opciones.clone()));
        assert_eq!(shell.emergente_pulsar(100.0, 4.0 + 48.0 * i as f32 + 24.0), Some(accion.clone()));
    }
}
