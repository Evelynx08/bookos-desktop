//! La campana de notificaciones.
//!
//! El contador viene de fuera: quien atiende `org.freedesktop.Notifications` es
//! el compositor —el shell no habla D-Bus, para poder estar pintado en el
//! primer frame— y le pasa aquí cuántas hay sin leer.
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
    /// Cuántas hay sin leer, tal como las cuenta el compositor.
    pendientes: u32,
    icono: Option<Icono>,
    icono_nombre: &'static str,
}

impl Notificaciones {
    pub fn new() -> Self {
        let mut n = Self {
            // Escotilla para poder mirar la chapa sin provocar notificaciones
            // de verdad. Sigue valiendo con el servidor puesto: el panel se
            // pinta antes de que llegue ninguna.
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

    /// El contador lo pone el compositor, que es quien recibe las
    /// notificaciones. Cambiarlo puede cambiar también el icono y el ancho del
    /// widget —la chapa se sale por la derecha—, y por eso repinta el panel
    /// entero y no solo su hueco.
    fn notificaciones(&mut self, cuantas: u32) -> bool {
        if self.pendientes == cuantas {
            return false;
        }
        self.pendientes = cuantas;
        self.actualizar_icono();
        true
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
            tema::acento()
        } else {
            TEXT()
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
                background: Some(tema::rojo().into()),
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
