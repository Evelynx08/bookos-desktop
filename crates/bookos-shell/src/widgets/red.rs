//! Wi-Fi o cable, y si hay enlace.

use crate::icono::{self, Icono};
use crate::state::{Link, Network};
use crate::tema;
use crate::view::PanelElement;
use crate::widget::{Cruce, Widget};

pub struct Red {
    dato: Option<Network>,
    icono: Option<Icono>,
    cruce: Cruce,
}

impl Red {
    pub fn new() -> Self {
        let dato = estado_red();
        let icono = icono::propio(nombre_icono(dato));
        Self {
            dato,
            icono,
            cruce: Cruce::default(),
        }
    }

    /// Caída se atenúa, como el texto que había antes: no es un fallo, es una
    /// cosa menos encendida.
    fn color(&self) -> iced_core::Color {
        match self.dato {
            Some(red) if red.up => tema::texto(),
            _ => tema::TEXTO2,
        }
    }
}

/// El icono de BookOS que toca. Wi-Fi caído sale con el icono de
/// desconectado en vez de con el de señal, que sería mentir.
fn nombre_icono(dato: Option<Network>) -> &'static str {
    match dato {
        Some(Network {
            kind: Link::Wifi,
            up: true,
        }) => "wifi",
        Some(Network {
            kind: Link::Cable,
            up: true,
        }) => "cable",
        Some(_) => "sin-red",
        None => "sin-red",
    }
}

impl Widget for Red {
    fn nombre(&self) -> &'static str {
        "red"
    }

    fn ancho(&self) -> f32 {
        if self.dato.is_some() && self.icono.is_some() {
            tema::ICONO_PANEL
        } else {
            0.0
        }
    }

    fn subsistemas(&self) -> &'static [&'static str] {
        &["net"]
    }

    fn refrescar(&mut self) -> bool {
        let fresco = estado_red();
        if fresco == self.dato {
            return false;
        }
        let color_previo = self.color();
        self.dato = fresco;
        let previo = std::mem::replace(&mut self.icono, icono::propio(nombre_icono(self.dato)));
        self.cruce.empezar(previo, color_previo);
        true
    }

    fn animando(&self) -> bool {
        self.cruce.animando()
    }

    /// Solo el icono, sin texto: "Wi-Fi" escrito al lado de un icono de Wi-Fi
    /// no añade nada y ocupa el ancho de tres estados más.
    fn ver(&self) -> PanelElement<'_> {
        if self.dato.is_none() {
            return crate::widget::vacio();
        }
        match &self.icono {
            Some(ic) => self.cruce.ver(ic, self.color()),
            None => crate::widget::vacio(),
        }
    }
}

fn estado_red() -> Option<Network> {
    let s = bookos_system::snapshot().network;
    if s.is_null() {
        return None;
    }
    if s["ethernet"]["connected"] == true {
        return Some(Network {
            kind: Link::Cable,
            up: true,
        });
    }
    Some(Network {
        kind: Link::Wifi,
        up: s["ssid"].as_str().is_some_and(|s| !s.is_empty()),
    })
}
