//! El brillo de la pantalla, en tanto por ciento.

use crate::icono::{self, Icono};
use crate::state::read_brightness;
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::Widget;

pub struct Brillo {
    dato: Option<u8>,
    icono: Option<Icono>,
}

impl Brillo {
    pub fn new() -> Self {
        Self {
            dato: read_brightness(),
            icono: icono::propio("brillo"),
        }
    }
}

impl Widget for Brillo {
    fn nombre(&self) -> &'static str {
        "brillo"
    }

    fn subsistemas(&self) -> &'static [&'static str] {
        &["backlight"]
    }

    fn refrescar(&mut self) -> bool {
        let fresco = read_brightness();
        if fresco == self.dato {
            return false;
        }
        self.dato = fresco;
        true
    }

    /// Solo el icono: el porcentaje ya se ve en la tarjeta al pulsarlo. Además
    /// así el panel no cambia de anchura al pasar de 9 a 100.
    fn ancho(&self) -> f32 {
        tema::ICONO_PANEL
    }

    fn ver(&self) -> PanelElement<'_> {
        self.icono
            .as_ref()
            .map(|ic| icono::ver_teñido(ic, tema::ICONO_PANEL, Some(if self.dato.is_some() { TEXT() } else { tema::TEXTO2 })))
            .unwrap_or_else(crate::widget::vacio)
    }
}
