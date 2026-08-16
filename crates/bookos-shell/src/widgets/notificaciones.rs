//! La campana de notificaciones.
//!
//! **Hoy solo es la campana.** Las notificaciones llegan por D-Bus
//! (`org.freedesktop.Notifications`) y este shell no habla con ningún servicio
//! a propósito: es lo que hace que el panel esté pintado en el primer frame.
//! Meter D-Bus es una decisión aparte, no un rato de trabajo, así que el widget
//! entra con su sitio hecho y el contador preparado.
//!
//! El icono cambia con el estado, que es lo que pide el diseño: campana lisa
//! sin nada pendiente y campana con aviso cuando hay algo, en el color de
//! acento para que se vea de refilón.

use iced_core::{Border, Length};
use iced_widget::{container, stack, text};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::Widget;

/// Diámetro de la chapa del contador. 14 px es lo que necesita un número de dos
/// cifras a 9 px sin que el círculo se convierta en una cápsula.
const CHAPA: f32 = 14.0;

pub struct Notificaciones {
    /// Cuántas hay sin leer. Mientras no haya D-Bus se queda en cero.
    pendientes: u32,
    icono: Option<Icono>,
    icono_nombre: &'static str,
}

impl Notificaciones {
    pub fn new() -> Self {
        let mut n = Self {
            // Escotilla para poder mirar la chapa mientras no haya D-Bus: sin
            // esto, el único estado que se puede dibujar es "ninguna", y el
            // contador se daría por bueno sin haberlo visto nunca.
            pendientes: std::env::var("BOOKOS_NOTIF_TEST")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            icono: None,
            icono_nombre: "",
        };
        n.actualizar_icono();
        n
    }

    fn actualizar_icono(&mut self) {
        let nombre = if self.pendientes > 0 {
            "notificaciones-aviso"
        } else {
            "notificaciones"
        };
        if nombre == self.icono_nombre {
            return;
        }
        self.icono = icono::propio(nombre);
        self.icono_nombre = nombre;
    }
}

impl Widget for Notificaciones {
    fn nombre(&self) -> &'static str {
        "notificaciones"
    }

    fn refrescar(&mut self) -> bool {
        false
    }

    /// La chapa se sale por la derecha del icono, así que el widget ocupa un
    /// poco más cuando hay algo pendiente: si no, se comería el hueco del
    /// vecino y el reparto de clics del panel dejaría de cuadrar.
    fn ancho(&self) -> f32 {
        match (&self.icono, self.pendientes) {
            (None, _) => 0.0,
            (Some(_), 0) => tema::ICONO_PANEL,
            (Some(_), _) => tema::ICONO_PANEL + CHAPA / 2.0,
        }
    }

    fn ver(&self) -> PanelElement<'_> {
        let Some(ic) = &self.icono else {
            return crate::widget::vacio();
        };
        // Con algo pendiente, el acento; sin nada, el color del texto.
        let color = if self.pendientes > 0 {
            tema::ACENTO
        } else {
            TEXT
        };
        let campana = icono::ver_teñido(ic, tema::ICONO_PANEL, Some(color));
        if self.pendientes == 0 {
            return campana;
        }
        // El número se corta en 9+: "12" dentro de un círculo de 14 px no se
        // lee, y lo que hace falta saber es que hay varias, no cuántas.
        let cuenta = if self.pendientes > 9 {
            "9+".to_string()
        } else {
            self.pendientes.to_string()
        };
        let chapa: PanelElement<'_> = container(
            container(
                text(cuenta)
                    .size(9.0)
                    .color(iced_core::Color::WHITE)
                    .align_x(iced_core::alignment::Horizontal::Center),
            )
            .width(Length::Fixed(CHAPA))
            .height(Length::Fixed(CHAPA))
            .center_y(Length::Fixed(CHAPA))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(tema::ROJO.into()),
                border: Border {
                    radius: (CHAPA / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        )
        // Arriba a la derecha de la campana, mordiéndola: pegada al borde
        // parecería un icono aparte.
        .padding(iced_core::Padding::ZERO.left(tema::ICONO_PANEL - CHAPA / 2.0))
        .into();

        stack![campana, chapa].into()
    }
}

