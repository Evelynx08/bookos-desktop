//! El logo del panel cambia de color con el canal de actualizaciones.
//!
//! Va en su propio binario porque el canal es un global del proceso y en el
//! de `panel.rs` los tests corren a la vez y se lo pisarían.
//!
//! Con `BOOKOS_CANAL_ESTABLE_PNG`, `…_BETA_PNG` y `…_DEV_PNG` guarda el panel
//! de cada uno.

mod comun;

use bookos_shell::marca::{self, Canal};

#[test]
fn cada_canal_pinta_su_logo() {
    let mut paneles = Vec::new();
    for (nombre, canal) in [
        ("estable", Canal::Estable),
        ("beta", Canal::Beta),
        ("dev", Canal::Dev),
    ] {
        marca::aplicar_canal(canal);
        let (buf, w, h) = comun::pinta(2.0);
        let var = format!("BOOKOS_CANAL_{}_PNG", nombre.to_uppercase());
        comun::volcar(&var, &buf, w, h);
        paneles.push(buf);
    }
    assert_ne!(paneles[0], paneles[1], "beta pinta igual que estable");
    assert_ne!(paneles[1], paneles[2], "dev pinta igual que beta");
}
