//! La fecha y la hora, a la derecha del panel.
//!
//! Va con la fecha delante —`15/8/26 1:54`— como el plasmoide `bookos-clock` y
//! como la referencia del panel: en un portátil la fecha se mira tanto como la
//! hora, y ponerla aquí ahorra abrir el calendario para saber en qué día vives.

use std::time::Duration;

use iced_widget::text;

use crate::state::{local_hhmm, Fecha};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::Widget;

pub struct Reloj {
    texto: String,
}

impl Reloj {
    pub fn new() -> Self {
        Self { texto: ahora() }
    }
}

/// `d/M/yy H:mm`, el formato del plasmoide.
fn ahora() -> String {
    let f = Fecha::hoy();
    format!("{}/{}/{:02} {}", f.dia, f.mes, f.anio % 100, local_hhmm())
}

impl Widget for Reloj {
    fn nombre(&self) -> &'static str {
        "reloj"
    }

    fn refrescar(&mut self) -> bool {
        let texto = ahora();
        if texto == self.texto {
            return false;
        }
        self.texto = texto;
        true
    }

    /// Despierta en el cambio de minuto, no una vez por segundo.
    ///
    /// Son 59 despertares menos por minuto. Cada uno saca a la CPU de su
    /// C-state, y ese goteo es justo lo que arruina la autonomía en reposo.
    fn proxima_alarma(&self) -> Option<Duration> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(Duration::from_secs(60 - (secs % 60)))
    }

    /// Lo que ocupa su texto: "15/8/26 19:23" son trece caracteres.
    fn ancho(&self) -> f32 {
        crate::widget::ancho_de(&self.texto, tema::T_CUERPO)
    }

    fn ver(&self) -> PanelElement<'_> {
        text(self.texto.clone()).size(tema::T_CUERPO).color(TEXT).into()
    }
}
