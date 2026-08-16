//! Wi-Fi o cable, y si hay enlace.

use crate::icono::{self, Icono};
use crate::state::{Link, Network};
use crate::tema;
use crate::view::PanelElement;
use crate::widget::Widget;

pub struct Red {
    dato: Option<Network>,
    icono: Option<Icono>,
}

impl Red {
    pub fn new() -> Self {
        let dato = Network::read();
        let icono = icono::cargar(nombre_icono(dato));
        Self { dato, icono }
    }
}

/// El icono de Breeze que toca. Wi-Fi caído sale con el icono de
/// desconectado en vez de con el de señal, que sería mentir.
fn nombre_icono(dato: Option<Network>) -> &'static str {
    match dato {
        Some(Network { kind: Link::Wifi, up: true }) => "network-wireless-connected-100",
        Some(Network { kind: Link::Cable, up: true }) => "network-wired-activated",
        Some(_) => "network-disconnect",
        None => "network-disconnect",
    }
}

impl Widget for Red {
    fn nombre(&self) -> &'static str {
        "red"
    }

    fn subsistemas(&self) -> &'static [&'static str] {
        &["net"]
    }

    fn refrescar(&mut self) -> bool {
        let fresco = Network::read();
        if fresco == self.dato {
            return false;
        }
        self.dato = fresco;
        self.icono = icono::cargar(nombre_icono(self.dato));
        true
    }

    /// Solo el icono, sin texto: "Wi-Fi" escrito al lado de un icono de Wi-Fi
    /// no añade nada y ocupa el ancho de tres estados más.
    fn ver(&self) -> PanelElement<'_> {
        if self.dato.is_none() {
            return crate::widget::vacio();
        }
        // Caída se atenúa, como el texto que había antes: no es un fallo, es
        // una cosa menos encendida.
        let color = match self.dato {
            Some(red) if red.up => tema::TEXTO,
            _ => tema::TEXTO2,
        };
        match &self.icono {
            Some(ic) => icono::ver_teñido(ic, tema::ICONO_PANEL, Some(color)),
            None => crate::widget::vacio(),
        }
    }
}
