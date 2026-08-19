//! El volumen de salida: el icono del altavoz según el nivel.
//!
//! **Por qué `wpctl` y no una lectura de fichero.** El volumen no está en
//! sysfs: vive dentro de PipeWire y solo se puede preguntar por su API o por su
//! herramienta de línea de órdenes. Hablar el protocolo de PipeWire desde aquí
//! significaría una dependencia nueva y un hilo más dentro del compositor, y a
//! cambio de eso el panel enseña un icono.
//!
//! **Y por qué no hay temporizador.** Preguntar cada dos segundos por si acaso
//! es justo lo que este escritorio no hace. El nivel se relee cuando el panel
//! ya se está refrescando por otro motivo. El precio, dicho claro: si mueves el
//! volumen desde otro programa, el icono del panel puede tardar en enterarse.
//!
//! **La consulta no espera.** Medido: `wpctl get-volume` tarda **21 ms** en
//! contestar, más de un frame entero. Esperarlo dentro de `refrescar` bloqueaba
//! el compositor en cada evento de udev — el refresco del panel completo pasó
//! de 3 a 24 ms el día que se añadió este widget. Así que el proceso se lanza y
//! se recoge en la pasada siguiente: el fork cuesta menos de 1 ms y el valor
//! llega un refresco tarde, que para un icono no lo nota nadie.

use std::process::{Child, Command, Stdio};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::Widget;

pub struct Volumen {
    /// De 0 a 100. `None` mientras no se haya podido leer: sin PipeWire
    /// delante, el widget no se dibuja en vez de mentir con un 0 %.
    nivel: Option<u8>,
    silenciado: bool,
    icono: Option<Icono>,
    /// Con qué nombre se cargó, para no rasterizarlo otra vez sin motivo.
    icono_nombre: &'static str,
    /// La consulta lanzada y todavía sin contestar.
    pendiente: Option<Child>,
}

impl Volumen {
    pub fn new() -> Self {
        let mut v = Self {
            nivel: None,
            silenciado: false,
            icono: None,
            icono_nombre: "",
            pendiente: None,
        };
        // Se **lanza** y no se espera: quien la recoge es `esperar_arranque`,
        // ya con las de los demás widgets corriendo a la vez. Esperarla aquí
        // ponía en fila los 19 ms de esta con los 19 de bluetooth.
        v.pendiente = lanzar("@DEFAULT_AUDIO_SINK@");
        v
    }

    /// Recoge la consulta del arranque, esperándola si hace falta.
    fn recoger_arranque(&mut self) {
        let Some(hijo) = self.pendiente.take() else {
            return;
        };
        if let Some((nivel, silenciado)) = hijo
            .wait_with_output()
            .ok()
            .filter(|s| s.status.success())
            .and_then(|s| interpretar(&String::from_utf8_lossy(&s.stdout)))
        {
            self.aplicar(nivel, silenciado);
        }
    }

    /// Guarda lo leído y elige el icono. `true` si cambió algo que se ve.
    fn aplicar(&mut self, nivel: u8, silenciado: bool) -> bool {
        if Some(nivel) == self.nivel && silenciado == self.silenciado {
            return false;
        }
        self.nivel = Some(nivel);
        self.silenciado = silenciado;
        self.actualizar_icono();
        true
    }

    /// Recoge la consulta anterior si ya contestó y deja lanzada la siguiente.
    ///
    /// Nunca espera: es lo que mantiene el refresco del panel por debajo del
    /// milisegundo.
    fn releer(&mut self) -> bool {
        let mut cambio = false;
        if let Some(hijo) = &mut self.pendiente {
            match hijo.try_wait() {
                // Ya terminó: `wait_with_output` no bloquea y devuelve lo leído.
                Ok(Some(_)) => {
                    let hijo = self.pendiente.take().expect("acabamos de verlo dentro");
                    if let Some((nivel, silenciado)) = hijo
                        .wait_with_output()
                        .ok()
                        .filter(|s| s.status.success())
                        .and_then(|s| interpretar(&String::from_utf8_lossy(&s.stdout)))
                    {
                        cambio = self.aplicar(nivel, silenciado);
                    }
                }
                // Sigue en marcha: se le deja acabar y no se lanza otro, que si
                // no se acumularían tantos procesos como eventos lleguen.
                Ok(None) => return false,
                Err(_) => self.pendiente = None,
            }
        }
        self.pendiente = lanzar("@DEFAULT_AUDIO_SINK@");
        cambio
    }

    /// Los cuatro escalones del plasmoide: sin ondas por debajo del 40 %, una
    /// hasta el 75 % y dos por encima. Con el aspa si está silenciado o a cero.
    fn actualizar_icono(&mut self) {
        let nombre = match (self.silenciado, self.nivel.unwrap_or(0)) {
            (true, _) | (_, 0) => "volumen-silencio",
            (_, n) if n < 40 => "volumen-bajo",
            (_, n) if n < 75 => "volumen-medio",
            _ => "volumen-alto",
        };
        if nombre == self.icono_nombre {
            return;
        }
        self.icono = icono::propio(nombre);
        self.icono_nombre = nombre;
    }
}

impl Widget for Volumen {
    fn nombre(&self) -> &'static str {
        "volumen"
    }

    fn esperar_arranque(&mut self) {
        self.recoger_arranque();
    }

    fn refrescar(&mut self) -> bool {
        self.releer()
    }

    /// Por lo mismo que en el bluetooth: sin icono no se dibuja nada, así que
    /// tampoco puede ocupar sitio ni recibir clics.
    fn ancho(&self) -> f32 {
        match self.icono {
            Some(_) => tema::ICONO_PANEL,
            None => 0.0,
        }
    }

    /// Solo el icono, como el plasmoide en su forma compacta: el número vive en
    /// el emergente, y una fila de porcentajes seguidos —brillo, volumen,
    /// batería— se lee peor que un icono que ya dice el nivel por su forma.
    fn ver(&self) -> PanelElement<'_> {
        let (Some(_), Some(ic)) = (self.nivel, &self.icono) else {
            return crate::widget::vacio();
        };
        // Silenciado en gris: es la misma señal que da el plasmoide, y se
        // distingue del aspa a distancia de un vistazo.
        let color = if self.silenciado {
            tema::TEXTO2
        } else {
            TEXT()
        };
        icono::ver_teñido(ic, tema::ICONO_PANEL, Some(color))
    }
}

/// Lanza la consulta sin esperarla.
///
/// Si PipeWire no está levantado, `wpctl` falla al contestar y el resultado se
/// descarta: es la diferencia entre "el volumen está a cero" y "aquí no hay con
/// quién hablar", y el panel enseña cosas distintas en cada caso.
fn lanzar(destino: &str) -> Option<Child> {
    Command::new("wpctl")
        .arg("get-volume")
        .arg(destino)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// La misma consulta, esperándola.
///
/// Solo para abrir: el emergente de Sonido necesita los dos niveles antes de
/// dibujarse por primera vez, y el widget del panel su valor inicial. En el
/// camino de refresco no se usa — ahí cuesta 21 ms.
pub(crate) fn consultar(destino: &str) -> Option<(u8, bool)> {
    let salida = Command::new("wpctl")
        .arg("get-volume")
        .arg(destino)
        .output()
        .ok()?;
    if !salida.status.success() {
        return None;
    }
    interpretar(&String::from_utf8_lossy(&salida.stdout))
}

/// Saca el par (nivel, silenciado) de la línea que escupe `wpctl`.
fn interpretar(linea: &str) -> Option<(u8, bool)> {
    let resto = linea.split_once("Volume:")?.1;
    let numero = resto.split_whitespace().next()?;
    let valor: f32 = numero.parse().ok()?;
    // PipeWire permite pasar del 100 % —hasta 1.5 con la sobreamplificación—
    // y el icono no tiene un escalón para eso: se recorta.
    let nivel = (valor * 100.0).round().clamp(0.0, 100.0) as u8;
    Some((nivel, linea.contains("[MUTED]")))
}

#[cfg(test)]
mod tests {
    use super::interpretar;

    #[test]
    fn lee_la_salida_de_wpctl() {
        assert_eq!(interpretar("Volume: 0.35\n"), Some((35, false)));
        assert_eq!(interpretar("Volume: 1.00\n"), Some((100, false)));
        assert_eq!(interpretar("Volume: 0.00\n"), Some((0, false)));
    }

    #[test]
    fn reconoce_el_silencio() {
        assert_eq!(interpretar("Volume: 0.42 [MUTED]\n"), Some((42, true)));
    }

    /// PipeWire llega a 1.5 con la sobreamplificación y el icono no tiene un
    /// escalón por encima del máximo.
    #[test]
    fn la_sobreamplificacion_se_recorta() {
        assert_eq!(interpretar("Volume: 1.50\n"), Some((100, false)));
    }

    /// Sin PipeWire delante `wpctl` no imprime nada útil, y eso no puede
    /// acabar en un 0 % que parezca un volumen de verdad.
    #[test]
    fn una_salida_que_no_es_dice_que_no() {
        assert_eq!(interpretar(""), None);
        assert_eq!(interpretar("no hay servidor\n"), None);
        assert_eq!(interpretar("Volume: mucho\n"), None);
    }
}
