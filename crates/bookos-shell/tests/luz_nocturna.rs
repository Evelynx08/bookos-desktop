//! La luz nocturna en la tarjeta de brillo y en la Estación de Control.
//!
//! Va en su propio binario porque lo que se enseña sale de un global del
//! proceso (`marcar_luz_nocturna`), y en el binario de `panel.rs` los tests
//! corren a la vez y se lo pisarían.
//!
//! Con `BOOKOS_NOCTURNA_BRILLO_PNG`, `BOOKOS_NOCTURNA_CENTRO_PNG` y sus
//! `…_ENCENDIDA_PNG` guarda las tarjetas apagada y encendida.

mod comun;

use bookos_shell::{Accion, Emergente};

fn pintar(shell: &mut bookos_shell::Shell) -> (Vec<u8>, u32, u32) {
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    (buf, w, h)
}

#[test]
fn encenderla_se_ve_en_las_dos_tarjetas() {
    for (nombre, abrir) in [
        ("brillo", Emergente::brillo as fn() -> Emergente),
        ("centro", Emergente::centro),
    ] {
        let mut shell = comun::shell(1.0);
        bookos_shell::marcar_luz_nocturna(false);
        shell.abrir(abrir());
        let (apagada, w, h) = pintar(&mut shell);
        bookos_shell::marcar_luz_nocturna(true);
        shell.abrir(abrir());
        let (encendida, _, _) = pintar(&mut shell);
        let var = format!("BOOKOS_NOCTURNA_{}", nombre.to_uppercase());
        comun::volcar(&format!("{var}_PNG"), &apagada, w, h);
        comun::volcar(&format!("{var}_ENCENDIDA_PNG"), &encendida, w, h);
        assert_ne!(apagada, encendida, "{nombre}: encenderla no cambia nada");
    }
}

/// Pulsar la fila de la tarjeta de brillo pide el estado contrario.
#[test]
fn la_fila_de_la_tarjeta_alterna() {
    let mut shell = comun::shell(1.0);
    bookos_shell::marcar_luz_nocturna(false);
    shell.abrir(Emergente::brillo());
    let ((_, h), _, _) = shell.emergente_geometria().expect("tiene superficie");
    // El grupo de la luz nocturna es lo último de la tarjeta.
    assert_eq!(
        shell.emergente_pulsar(100.0, h as f32 - 40.0),
        Some(Accion::LuzNocturna(true))
    );
}
