//! Qué se está reproduciendo, para la tarjeta del centro de control.
//!
//! **MPRIS con `busctl`, no con un cliente de D-Bus.** La especificación
//! (`org.mpris.MediaPlayer2.Player`) es el estándar que implementan todos los
//! reproductores; hablarla desde aquí exigiría una dependencia nueva y un hilo
//! dentro del compositor. `busctl` viene con systemd y es el mismo camino que
//! ya se usa para el brillo y para los perfiles de energía.
//!
//! **Solo se consulta con la tarjeta abierta.** Son tres procesos por
//! consulta —listar, estado y metadatos—, así que en reposo esto no corre: el
//! centro de control cerrado no existe.
//!
//! Lo que no hay: carátula. La trae `mpris:artUrl`, casi siempre como fichero
//! en `/tmp`, y descodificar un JPEG por cada canción para pintarlo de fondo es
//! trabajo que aún no se ha medido. La tarjeta se dibuja con el color del
//! sistema mientras tanto.

use std::process::{Command, Stdio};

/// Lo que está sonando ahora mismo.
#[derive(Debug, Clone, PartialEq)]
pub struct Sonando {
    /// El nombre del bus, para mandarle las órdenes: `org.mpris.MediaPlayer2.vlc`.
    pub bus: String,
    /// Cómo se llama el reproductor, ya legible: `vlc`, `firefox`.
    pub aplicacion: String,
    pub titulo: String,
    pub artista: String,
    pub reproduciendo: bool,
    /// Posición y duración en segundos, si el reproductor las publica.
    pub posicion: Option<u64>,
    pub duracion: Option<u64>,
}

impl Sonando {
    /// Lo que sonaría en la tarjeta: el primer reproductor que hay en el bus.
    ///
    /// Si hay varios se coge el primero por orden alfabético en vez de intentar
    /// adivinar cuál mira el usuario. Elegir «el que está sonando» exige mirar
    /// el estado de todos, que son dos procesos más por reproductor.
    pub fn leer() -> Option<Self> {
        let bus = primer_reproductor()?;
        let estado = propiedad(&bus, "PlaybackStatus")?;
        let metadatos = propiedad(&bus, "Metadata").unwrap_or_default();
        let titulo = campo(&metadatos, "xesam:title")?;
        Some(Self {
            aplicacion: bus
                .rsplit('.')
                .next()
                .unwrap_or(&bus)
                .replace("instance", "")
                .to_string(),
            artista: campo(&metadatos, "xesam:artist").unwrap_or_default(),
            reproduciendo: estado.contains("Playing"),
            posicion: propiedad(&bus, "Position")
                .and_then(|s| numero(&s))
                // Los micros del `Position` de MPRIS a segundos.
                .map(|us| us / 1_000_000),
            duracion: campo_numero(&metadatos, "mpris:length").map(|us| us / 1_000_000),
            titulo,
            bus,
        })
    }

    /// La orden que hay que ejecutar para un botón de la tarjeta.
    pub fn orden(&self, que: Orden) -> String {
        let metodo = match que {
            Orden::Anterior => "Previous",
            Orden::Siguiente => "Next",
            Orden::Alternar => "PlayPause",
        };
        format!(
            "busctl --user call {} /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player {metodo}",
            self.bus
        )
    }

    /// De 0 a 1, para la barra de progreso. `None` si el reproductor no publica
    /// las dos cosas —los navegadores a menudo no dan `Position`—.
    pub fn avance(&self) -> Option<f32> {
        let (p, d) = (self.posicion?, self.duracion?);
        (d > 0).then(|| (p as f32 / d as f32).clamp(0.0, 1.0))
    }
}

/// Los tres botones de la tarjeta.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Orden {
    Anterior,
    Alternar,
    Siguiente,
}

/// Formatea unos segundos como `m:ss`.
pub fn reloj(segundos: u64) -> String {
    format!("{}:{:02}", segundos / 60, segundos % 60)
}

fn primer_reproductor() -> Option<String> {
    let salida = Command::new("busctl")
        .args(["--user", "list", "--no-legend"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let texto = String::from_utf8_lossy(&salida.stdout);
    let mut nombres: Vec<&str> = texto
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
        .collect();
    nombres.sort();
    nombres.first().map(|s| s.to_string())
}

fn propiedad(bus: &str, nombre: &str) -> Option<String> {
    let salida = Command::new("busctl")
        .args([
            "--user",
            "get-property",
            bus,
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
            nombre,
        ])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|s| s.status.success())?;
    Some(String::from_utf8_lossy(&salida.stdout).into_owned())
}

/// Saca un campo de texto del volcado de `Metadata`.
///
/// `busctl` imprime el diccionario en una línea: `... "xesam:title" s "Canción"
/// "xesam:artist" as 1 "Artista" ...`. Se busca la clave y se coge la siguiente
/// cadena entrecomillada, saltándose el tipo. En vez de analizar el formato
/// entero —que cambia entre versiones— solo se leen los tres campos que se
/// enseñan.
fn campo(volcado: &str, clave: &str) -> Option<String> {
    let resto = volcado.split_once(&format!("\"{clave}\""))?.1;
    let mut trozos = resto.split('"');
    // El primer trozo es el tipo y lo que haya entre medias; el segundo, el
    // valor.
    let valor = trozos.nth(1)?.trim();
    (!valor.is_empty()).then(|| valor.to_string())
}

/// Igual, pero para un campo numérico: `"mpris:length" x 253000000`.
fn campo_numero(volcado: &str, clave: &str) -> Option<u64> {
    let resto = volcado.split_once(&format!("\"{clave}\""))?.1;
    numero(resto)
}

/// El primer número entero que aparece en un trozo de salida de `busctl`.
fn numero(texto: &str) -> Option<u64> {
    let mut digitos = String::new();
    for c in texto.chars() {
        if c.is_ascii_digit() {
            digitos.push(c);
        } else if !digitos.is_empty() {
            break;
        }
    }
    digitos.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El volcado real de `busctl` para una canción, con el artista dentro de
    /// un array —que es como lo publica MPRIS.
    const VOLCADO: &str = r#"a{sv} 5 "mpris:trackid" o "/org/mpris/MediaPlayer2/track/3" "mpris:length" x 253000000 "mpris:artUrl" s "file:///tmp/caratula.jpg" "xesam:title" s "Nombre de la canción" "xesam:artist" as 1 "Nombre Artista""#;

    #[test]
    fn saca_titulo_y_artista() {
        assert_eq!(
            campo(VOLCADO, "xesam:title").as_deref(),
            Some("Nombre de la canción")
        );
        assert_eq!(
            campo(VOLCADO, "xesam:artist").as_deref(),
            Some("Nombre Artista")
        );
    }

    #[test]
    fn saca_la_duracion_en_microsegundos() {
        assert_eq!(campo_numero(VOLCADO, "mpris:length"), Some(253_000_000));
    }

    /// Un campo que no está no puede devolver el del vecino.
    #[test]
    fn un_campo_ausente_es_none() {
        assert_eq!(campo(VOLCADO, "xesam:album"), None);
        assert_eq!(campo_numero("", "mpris:length"), None);
    }

    #[test]
    fn el_reloj_lleva_dos_cifras_en_los_segundos() {
        assert_eq!(reloj(43), "0:43");
        assert_eq!(reloj(300), "5:00");
        assert_eq!(reloj(3), "0:03");
    }
}
