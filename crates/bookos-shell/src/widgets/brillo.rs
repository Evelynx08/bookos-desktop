//! El brillo de la pantalla, en tanto por ciento.

use iced_widget::{row, text};

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
            icono: icono::cargar("brightness-high"),
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

    /// El icono es un SVG del tema, **no un glifo**: el rasterizador de iced se
    /// queda con la familia por defecto y no cae a otra fuente, así que un "☀"
    /// desaparecía sin avisar y se veía "60%" a secas, indistinguible del
    /// porcentaje de la batería de al lado.
    /// Icono, hueco y el porcentaje: "50%" o "100%".
    fn ancho(&self) -> f32 {
        let Some(brillo) = self.dato else {
            return 0.0;
        };
        tema::ICONO_PANEL + 4.0 + crate::widget::ancho_de(&format!("{brillo}%"), tema::T_CUERPO)
    }

    fn ver(&self) -> PanelElement<'_> {
        let Some(brillo) = self.dato else {
            return crate::widget::vacio();
        };
        let mut fila = row![].spacing(4).align_y(iced_core::alignment::Vertical::Center);
        if let Some(ic) = &self.icono {
            fila = fila.push(icono::ver_teñido(ic, tema::ICONO_PANEL, Some(TEXT)));
        }
        fila.push(text(format!("{brillo}%")).size(tema::T_CUERPO).color(TEXT))
            .into()
    }
}
