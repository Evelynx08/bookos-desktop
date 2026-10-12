//! La tarjeta «Acerca de este PC»: el ojo del número de serie y la ✕.
//!
//! Con `BOOKOS_ACERCA_PNG` y `BOOKOS_ACERCA_REVELADO_PNG` guarda la tarjeta
//! con el número oculto y a la vista.

mod comun;

use bookos_shell::{Accion, Emergente};

fn pintar(shell: &mut bookos_shell::Shell) -> (Vec<u8>, u32, u32) {
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    (buf, w, h)
}

#[test]
fn el_ojo_destapa_el_numero_de_serie_y_lo_vuelve_a_tapar() {
    let mut shell = comun::shell(1.0);
    shell.abrir(Emergente::acerca());
    let (oculto, w, h) = pintar(&mut shell);
    comun::volcar("BOOKOS_ACERCA_PNG", &oculto, w, h);

    // Sobre los puntos del número de serie: cuarto renglón de la ficha, que
    // empieza a 310 px del borde, justo a la derecha de las etiquetas.
    let (x, y) = (212.0, 401.0);
    assert_eq!(shell.emergente_pulsar(x, y), None, "el ojo no cierra nada");
    let (revelado, _, _) = pintar(&mut shell);
    comun::volcar("BOOKOS_ACERCA_REVELADO_PNG", &revelado, w, h);
    assert_ne!(oculto, revelado, "pulsar el ojo no cambia la tarjeta");

    shell.emergente_pulsar(x, y);
    let (otra_vez, _, _) = pintar(&mut shell);
    assert_eq!(oculto, otra_vez, "el segundo clic no lo vuelve a tapar");
}

#[test]
fn la_aspa_cierra_la_tarjeta() {
    let mut shell = comun::shell(1.0);
    shell.abrir(Emergente::acerca());
    // El centro de la ✕: 24 px a 16 de la esquina de una tarjeta de 420.
    let (x, y) = (392.0, 28.0);
    assert_eq!(shell.emergente_pulsar(x, y), Some(Accion::CerrarEmergente));
}
