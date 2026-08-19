//! El bluetooth: el icono del panel, encendido, apagado o conectado.
//!
//! **De dónde sale el estado.** BlueZ no publica nada en sysfs que sirva: hay
//! un `/sys/class/bluetooth/hci0` pero no dice si el adaptador está encendido
//! ni qué hay conectado, que es justo lo que el icono tiene que enseñar. Eso
//! vive en D-Bus, y aquí se pregunta con `bluetoothctl`, que ya viene con
//! BlueZ.
//!
//! **Sin temporizador, como el volumen.** La consulta se lanza y se recoge en
//! la pasada siguiente: `bluetoothctl` tarda lo suyo en contestar y esperarlo
//! dentro del refresco bloquearía el compositor. El precio es el mismo: si
//! emparejas unos auriculares desde otro sitio, el icono tarda en enterarse.

use std::process::{Child, Command, Stdio};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::Widget;

/// En qué estado está el adaptador.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    /// No hay adaptador: el icono no se dibuja. Un portátil sin bluetooth no
    /// tiene por qué enseñar un icono tachado para siempre.
    SinAdaptador,
    Apagado,
    Encendido,
    /// Encendido y con algo conectado. Es el único caso que merece el acento:
    /// lo que importa de un vistazo es si tus auriculares están puestos.
    Conectado,
}

pub struct Bluetooth {
    estado: Estado,
    icono: Option<Icono>,
    icono_nombre: &'static str,
    pendiente: Option<Child>,
}

impl Bluetooth {
    pub fn new() -> Self {
        let mut b = Self {
            estado: Estado::SinAdaptador,
            icono: None,
            icono_nombre: "",
            pendiente: None,
        };
        // Lanzada y sin esperar: la recoge `esperar_arranque`, ya con la del
        // volumen corriendo a la vez. Ver [`crate::widget::Widget::esperar_arranque`].
        b.pendiente = lanzar();
        b
    }

    /// Recoge la consulta del arranque, esperándola si hace falta.
    fn recoger_arranque(&mut self) {
        let Some(hijo) = self.pendiente.take() else {
            return;
        };
        if let Some(estado) = hijo
            .wait_with_output()
            .ok()
            .filter(|s| s.status.success())
            .map(|s| interpretar(&String::from_utf8_lossy(&s.stdout)))
        {
            self.aplicar(estado);
        }
    }

    fn aplicar(&mut self, estado: Estado) -> bool {
        if estado == self.estado {
            return false;
        }
        self.estado = estado;
        let nombre = match estado {
            Estado::SinAdaptador => "",
            Estado::Apagado => "bluetooth-apagado",
            Estado::Encendido | Estado::Conectado => "bluetooth",
        };
        if nombre != self.icono_nombre {
            self.icono = (!nombre.is_empty())
                .then(|| icono::propio(nombre))
                .flatten();
            self.icono_nombre = nombre;
        }
        true
    }
}

impl Widget for Bluetooth {
    fn nombre(&self) -> &'static str {
        "bluetooth"
    }

    fn esperar_arranque(&mut self) {
        self.recoger_arranque();
    }

    fn refrescar(&mut self) -> bool {
        let mut cambio = false;
        if let Some(hijo) = &mut self.pendiente {
            match hijo.try_wait() {
                Ok(Some(_)) => {
                    let hijo = self.pendiente.take().expect("acabamos de verlo dentro");
                    if let Some(estado) = hijo
                        .wait_with_output()
                        .ok()
                        .filter(|s| s.status.success())
                        .map(|s| interpretar(&String::from_utf8_lossy(&s.stdout)))
                    {
                        cambio = self.aplicar(estado);
                    }
                }
                Ok(None) => return false,
                Err(_) => self.pendiente = None,
            }
        }
        self.pendiente = lanzar();
        cambio
    }

    /// Sale del icono y no del estado: `ver` no dibuja nada sin icono, y si
    /// `ancho` dijera otra cosa quedaría una zona que se come los clics del
    /// vecino sin enseñar nada. Que las dos respuestas salgan del mismo dato es
    /// lo que mantiene el invariante "ocupa sitio si y solo si se ve".
    fn ancho(&self) -> f32 {
        match self.icono {
            Some(_) => tema::ICONO_PANEL,
            None => 0.0,
        }
    }

    fn ver(&self) -> PanelElement<'_> {
        let Some(ic) = &self.icono else {
            return crate::widget::vacio();
        };
        // Encendido va en acento, como en el diseño y como en Plasma: el azul
        // **es** el bluetooth, no una marca de "hay algo conectado". Apagado se
        // queda en gris, que es lo que distingue de un vistazo.
        let color = match self.estado {
            Estado::Apagado => tema::TEXTO2,
            Estado::Encendido | Estado::Conectado => tema::acento(),
            Estado::SinAdaptador => TEXT(),
        };
        icono::ver_teñido(ic, tema::ICONO_PANEL, Some(color))
    }
}

/// Lo que se le pregunta a BlueZ. Van las dos en un `sh` para pagar un solo
/// proceso por refresco en vez de dos.
const ORDEN: &str = "bluetoothctl show; bluetoothctl devices Connected";

fn lanzar() -> Option<Child> {
    Command::new("sh")
        .arg("-c")
        .arg(ORDEN)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// Saca el estado de lo que escupen `bluetoothctl show` y `devices Connected`.
///
/// `show` sin adaptador dice "No default controller available"; con él,
/// `Powered: yes` o `no`. Cada línea `Device …` de la segunda orden es algo
/// conectado.
fn interpretar(salida: &str) -> Estado {
    if !salida.contains("Controller ") {
        return Estado::SinAdaptador;
    }
    if !salida.contains("Powered: yes") {
        return Estado::Apagado;
    }
    if salida
        .lines()
        .any(|l| l.trim_start().starts_with("Device "))
    {
        Estado::Conectado
    } else {
        Estado::Encendido
    }
}

#[cfg(test)]
mod tests {
    use super::{interpretar, Estado};

    #[test]
    fn sin_adaptador_no_se_dibuja_nada() {
        assert_eq!(
            interpretar("No default controller available\n"),
            Estado::SinAdaptador
        );
        assert_eq!(interpretar(""), Estado::SinAdaptador);
    }

    #[test]
    fn distingue_apagado_de_encendido() {
        let base = "Controller 40:C7:3C:E2:53:D2 (public)\n\tName: book5-pro\n";
        assert_eq!(
            interpretar(&format!("{base}\tPowered: no\n")),
            Estado::Apagado
        );
        assert_eq!(
            interpretar(&format!("{base}\tPowered: yes\n")),
            Estado::Encendido
        );
    }

    /// Lo que de verdad importa del icono: si hay algo puesto.
    #[test]
    fn con_algo_conectado_lo_dice() {
        let salida = "Controller 40:C7:3C:E2:53:D2 (public)\n\tPowered: yes\n\
                      Device E8:C9:13:E0:E6:DA Buds3 Pro de Evelyn\n";
        assert_eq!(interpretar(salida), Estado::Conectado);
    }
}
