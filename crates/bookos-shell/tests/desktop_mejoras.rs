mod comun;
use bookos_shell::{Accion, Emergente, TeclaPulsada, confirmacion::Energia};

#[test]
fn confirmacion_hig_y_cancelacion() {
    let mut shell = comun::shell(1.75);
    shell.abrir(Emergente::confirmar_energia(Energia::Apagar));
    let (w, h) = shell.emergente_buffer_size().unwrap();
    // Canvas alinea el ancho lógico para evitar columnas HiDPI incompletas.
    assert!((w as f32 / 1.75 - 310.0).abs() <= 4.0);
    assert_eq!(h, 364);
    let mut buf = vec![0; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    comun::volcar("BOOKOS_DUMP_CONFIRMACION", &buf, w, h);
    assert!(matches!(
        shell.emergente_tecla(TeclaPulsada::Intro),
        (true, None)
    ));
    assert!(shell.emergente_nombre().is_none());
    shell.abrir(Emergente::confirmar_energia(Energia::Bloquear));
    shell.emergente_tecla(TeclaPulsada::Tabulador);
    assert!(matches!(
        shell.emergente_tecla(TeclaPulsada::Intro),
        (true, Some(Accion::EnergiaConfirmada(Energia::Bloquear)))
    ));
}

#[test]
fn cambiar_dock_redimensiona_y_repinta_a_escala_fraccionaria() {
    let mut shell = comun::shell(1.75);
    for tamano in [32, 80, 50] {
        shell.dock_tamano(tamano);
        assert!(shell.dock_needs_paint());
        let (w, h) = shell.dock_buffer_size();
        let mut buf = vec![0; (w * h * 4) as usize];
        assert!(!shell.draw_dock(&mut buf).is_empty());
        assert!(buf.as_chunks::<4>().0.iter().any(|p| p[3] != 0));
        assert!(!shell.dock_needs_paint());
        assert_eq!(shell.dock().tamano_actual(), tamano);
    }
}
