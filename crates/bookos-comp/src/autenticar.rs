//! Comprobar la contraseña del usuario para salir del bloqueo.
//!
//! ## Por qué `unix_chkpwd` y no PAM directamente
//!
//! Hablar con PAM desde aquí significa enlazar `libpam` y llevar una
//! conversación con callbacks desde un proceso de un solo hilo que además no
//! puede bloquearse: PAM tarda **segundos** a propósito cuando la contraseña
//! falla, y el compositor no puede quedarse parado mientras tanto. Es una
//! dependencia nueva y un hilo más para preguntar «sí o no».
//!
//! `unix_chkpwd` es el ayudante SUID que el propio `pam_unix` usa cuando no es
//! root: recibe el usuario como argumento y la contraseña por la entrada
//! estándar terminada en NUL, y responde con el código de salida —0 si es
//! correcta—. Viene con `pam` en cualquier distribución.
//!
//! **Lo que esto no cubre**: métodos que no sean la contraseña local (huella,
//! LDAP, tarjeta). Un equipo con `pam_fprintd` desbloquea con el dedo en otros
//! escritorios y aquí no, y `pam_faillock` no cuenta estos intentos.
//!
//! El proceso se lanza sin esperar y se recoge después. El bloqueo enseña
//! «comprobando» mientras tanto, que es justo el tiempo que PAM se toma en un
//! fallo.

use std::io::Write;
use std::process::{Child, Command, Stdio};

/// Una comprobación en marcha.
pub struct Comprobacion {
    hijo: Child,
}

impl Comprobacion {
    /// Lanza la comprobación. `None` si no hay ayudante en el sistema, en cuyo
    /// caso el bloqueo no puede autenticar a nadie y hay que decirlo.
    pub fn lanzar(usuario: &str, contrasena: &str) -> Option<Self> {
        let ruta = ["/usr/sbin/unix_chkpwd", "/sbin/unix_chkpwd"]
            .into_iter()
            .find(|r| std::path::Path::new(r).exists())?;
        let mut hijo = Command::new(ruta)
            .arg(usuario)
            // `nullok` es lo que le pasa `pam_unix` por omisión; sin este
            // argumento el ayudante no lee la contraseña de la entrada.
            .arg("nullok")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        // La contraseña va con el NUL final que el ayudante espera como marca
        // de fin; sin él se queda esperando más entrada.
        let escrita = hijo.stdin.as_mut().is_some_and(|entrada| {
            entrada.write_all(contrasena.as_bytes()).is_ok()
                && entrada.write_all(&[0]).is_ok()
        });
        // Cerrar la tubería: el ayudante no arranca la comprobación hasta ver
        // el final del fichero.
        drop(hijo.stdin.take());
        if !escrita {
            let _ = hijo.kill();
            return None;
        }
        Some(Self { hijo })
    }

    /// ¿Ya contestó? `None` mientras siga pensando.
    pub fn resultado(&mut self) -> Option<bool> {
        match self.hijo.try_wait() {
            Ok(Some(estado)) => Some(estado.success()),
            Ok(None) => None,
            // Si no se puede ni esperar al hijo, se da por fallida: dejar pasar
            // ante la duda es lo único que un bloqueo no puede hacer.
            Err(_) => Some(false),
        }
    }
}

/// Con qué cuenta se autentica.
pub fn usuario() -> String {
    std::env::var("USER").unwrap_or_else(|_| "root".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una contraseña que no es tiene que dar `false`, no colarse.
    ///
    /// Es una llamada real al ayudante del sistema: si no está instalado, no
    /// hay nada que comprobar y el test se salta solo.
    #[test]
    fn una_contrasena_falsa_no_pasa() {
        let Some(mut c) = Comprobacion::lanzar(&usuario(), "no-es-esta-contrasena-12345") else {
            return;
        };
        // `pam_unix` se toma su tiempo al fallar a propósito; dos segundos
        // sobran en esta máquina —medido, contesta en torno a 1,7 s.
        for _ in 0..60 {
            if let Some(vale) = c.resultado() {
                assert!(!vale, "una contraseña inventada ha pasado la comprobación");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("el ayudante no contestó en tres segundos");
    }
}
