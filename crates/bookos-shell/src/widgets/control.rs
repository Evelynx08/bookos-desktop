//! El botón del centro de control.
//!
//! Es la entrada a la Estación de Control del diseño —wifi, bluetooth, brillo,
//! volumen y perfiles en un solo sitio—, que todavía no está: son doce mil
//! líneas de QML en el plasmoide original. El botón entra ya con su hueco y su
//! icono para que el panel tenga el orden definitivo desde el principio, y
//! porque el sitio de un icono en la barra es de las cosas que la mano aprende
//! y conviene no mover después.

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::Widget;

pub struct Control {
    icono: Option<Icono>,
}

impl Control {
    pub fn new() -> Self {
        Self {
            icono: icono::propio("control"),
        }
    }
}

impl Widget for Control {
    fn nombre(&self) -> &'static str {
        "control"
    }

    fn refrescar(&mut self) -> bool {
        false
    }

    fn ancho(&self) -> f32 {
        match self.icono {
            Some(_) => tema::ICONO_PANEL,
            None => 0.0,
        }
    }

    fn ver(&self) -> PanelElement<'_> {
        match &self.icono {
            Some(ic) => icono::ver_teñido(ic, tema::ICONO_PANEL, Some(TEXT())),
            None => crate::widget::vacio(),
        }
    }
}
