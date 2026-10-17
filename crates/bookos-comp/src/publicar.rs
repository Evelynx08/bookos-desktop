//! Deja la apariencia de la sesión donde la pantalla de acceso puede leerla.
//!
//! El greeter corre como otro usuario y antes de que nadie entre: no llega a
//! `~/.config` ni al fondo si está en la carpeta personal. Aquí se copia lo
//! justo a `bookos_shell::APARIENCIA_PUBLICA/<login>/`, legible por todos.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::fondo::Eleccion;

/// Lo que se publica de `panel.conf`. El resto —dock, atajos, escritorios— no
/// le hace falta al greeter y no tiene por qué leerlo cualquiera.
const CLAVES: [&str; 6] = [
    "tema",
    "tema_claro_desde",
    "tema_oscuro_desde",
    "acento",
    "efectos",
    "escala",
];
const MAXIMO: u64 = 32 * 1024 * 1024;

static ACTIVA: AtomicBool = AtomicBool::new(false);

/// Solo publica la sesión real. Anidado para desarrollar, con un `panel.conf`
/// de prueba, pisaría la apariencia de verdad del usuario.
pub fn activar() {
    ACTIVA.store(true, Ordering::Relaxed);
}

pub fn publicar(eleccion: &Eleccion) {
    let raiz = match std::env::var_os("BOOKOS_APARIENCIA_PUBLICA") {
        Some(raiz) => PathBuf::from(raiz),
        None if ACTIVA.load(Ordering::Relaxed) => PathBuf::from(bookos_shell::APARIENCIA_PUBLICA),
        None => return,
    };
    let fondos = [
        ("fondo-claro", crate::fondo::ruta_para(eleccion, true)),
        ("fondo-oscuro", crate::fondo::ruta_para(eleccion, false)),
    ];
    let lanzado = std::thread::Builder::new()
        .name("bookos-publicar".into())
        .spawn(move || {
            let carpeta = raiz.join(crate::autenticar::usuario());
            if let Err(error) = escribir(&carpeta, &fondos) {
                // Sin el greeter instalado la raíz no existe, y no es un fallo.
                tracing::debug!(?carpeta, "apariencia sin publicar: {error}");
            }
        });
    if let Err(error) = lanzado {
        tracing::warn!("no se pudo lanzar la publicación de la apariencia: {error}");
    }
}

fn escribir(carpeta: &Path, fondos: &[(&str, Option<PathBuf>)]) -> std::io::Result<()> {
    if !carpeta.is_dir() {
        std::fs::create_dir(carpeta)?;
    }
    std::fs::set_permissions(carpeta, std::fs::Permissions::from_mode(0o755))?;

    let config = bookos_shell::ruta_config()
        .and_then(|ruta| std::fs::read_to_string(ruta).ok())
        .unwrap_or_default();
    let publico: String = config
        .lines()
        .filter(|linea| {
            let clave = linea.split('=').next().unwrap_or_default().trim();
            CLAVES.contains(&clave) || clave.starts_with("bloqueo_reloj")
        })
        .flat_map(|linea| [linea, "\n"])
        .collect();
    reemplazar(&carpeta.join("panel.conf"), |temporal| {
        std::fs::write(temporal, &publico)
    })?;

    // Los colores que Settings saca del fondo: es lo que hace que la pantalla
    // de acceso tenga el mismo acento que el escritorio de quien va a entrar.
    let paleta = bookos_shell::ruta_config().map(|ruta| ruta.with_file_name("palette.css"));
    copiar(paleta.as_deref(), &carpeta.join("palette.css"))?;
    for (nombre, origen) in fondos {
        copiar(origen.as_deref(), &carpeta.join(nombre))?;
    }
    let casa = std::env::var("HOME").unwrap_or_default();
    let avatar = [
        bookos_shell::Config::cargar()
            .avatar
            .map(|ruta| ruta.replacen('~', &casa, 1))
            .unwrap_or_default(),
        format!("{casa}/.face"),
        format!("{casa}/.face.icon"),
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|ruta| ruta.is_file());
    copiar(avatar.as_deref(), &carpeta.join("avatar"))
}

/// Copia `origen` si cambió, o retira lo publicado si ya no hay nada que
/// enseñar: un vídeo, un fichero enorme o ninguno. El greeter cae entonces al
/// fondo de serie en vez de quedarse con el de la semana pasada.
fn copiar(origen: Option<&Path>, destino: &Path) -> std::io::Result<()> {
    let valido = origen
        .filter(|ruta| !crate::fondo::es_video(ruta))
        .and_then(|ruta| Some((ruta, std::fs::metadata(ruta).ok()?)))
        .filter(|(_, datos)| datos.is_file() && datos.len() <= MAXIMO);
    let Some((origen, datos)) = valido else {
        return match std::fs::remove_file(destino) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    };
    let al_dia = std::fs::metadata(destino).is_ok_and(|puesto| {
        puesto.len() == datos.len()
            && matches!((puesto.modified(), datos.modified()), (Ok(p), Ok(o)) if p >= o)
    });
    if al_dia {
        return Ok(());
    }
    reemplazar(destino, |temporal| {
        std::fs::copy(origen, temporal).map(drop)
    })
}

/// Escribe al lado y renombra, para que el greeter nunca lea medio fichero.
fn reemplazar(
    destino: &Path,
    escribir: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let temporal = destino.with_extension("parcial");
    escribir(&temporal)?;
    std::fs::set_permissions(&temporal, std::fs::Permissions::from_mode(0o644))?;
    std::fs::rename(&temporal, destino)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publica_lo_que_cambia_y_retira_lo_que_ya_no_hay() -> std::io::Result<()> {
        let raiz = std::env::temp_dir().join(format!("bookos-publicar-{}", std::process::id()));
        std::fs::create_dir_all(&raiz)?;
        let origen = raiz.join("origen.png");
        let destino = raiz.join("fondo-claro");
        std::fs::write(&origen, b"imagen")?;
        copiar(Some(&origen), &destino)?;
        assert_eq!(std::fs::read(&destino)?, b"imagen");
        assert_eq!(
            std::fs::metadata(&destino)?.permissions().mode() & 0o777,
            0o644
        );
        let video = raiz.join("origen.mp4");
        std::fs::write(&video, b"video")?;
        copiar(Some(&video), &destino)?;
        assert!(!destino.exists());
        copiar(None, &destino)?;
        std::fs::remove_dir_all(&raiz)
    }
}
