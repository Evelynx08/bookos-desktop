//! La vista del panel.
//!
//! Los colores no se eligen aquí: salen de [`crate::tema`], que es el sistema de
//! diseño que ya siguen los widgets de Plasma de BookOS. Antes esta paleta era
//! propia y aproximada —el acento era `#5A8FFA` en vez del `#0A84FF` del
//! sistema— y eso hacía que el panel y los plasmoides no pareciesen del mismo
//! escritorio.

use iced_core::theme::Palette;
use iced_core::{Element, Length, Theme};
use iced_tiny_skia::Renderer;
use iced_widget::{container, row, stack, Space};

use crate::tema;
use crate::widget::Panel;

// Los nombres viejos siguen valiendo dentro del crate: son los roles que usan
// los widgets, y apuntan a los tokens del sistema.
pub use tema::acento as ACENTO;
pub use tema::rojo as PELIGRO;
pub use tema::texto as TEXT;
pub use tema::verde as OK;

pub fn theme() -> Theme {
    Theme::custom(
        "BookOS",
        Palette {
            background: tema::panel(),
            text: tema::texto(),
            primary: tema::acento(),
            success: tema::verde(),
            warning: tema::amarillo(),
            danger: tema::rojo(),
        },
    )
}

pub type PanelElement<'a> = Element<'a, (), Theme, Renderer>;

/// Separación entre los estados de la derecha.
///
/// 16 deja respirar cada estado sin que la fila derecha parezca demasiado
/// dispersa. El hit-test reparte la mitad del hueco a cada vecino, así que
/// reducirlo no crea franjas muertas entre iconos.
pub const HUECO: f32 = 16.0;
/// Margen izquierdo y derecho del panel.
pub const MARGEN_PANEL: f32 = 12.0;
/// Lado del logo, en lógicos. 20 sobre un panel de 32 deja 6 de aire arriba y
/// abajo, que es lo que hace que no parezca metido con calzador.
pub const LADO_LOGO: f32 = 20.0;
/// Zona sensible de la marca, a la izquierda: pulsar ahí abre el menú.
///
/// Es un ancho fijo y no el del elemento porque el layout lo calcula iced y
/// aquí hace falta antes, para el hit-test. Cubre el margen de 12 y el logo de
/// 20, más un blanco de cortesía para no fallar el clic por dos píxeles: el
/// logo va solo, sin el nombre escrito al lado, y una zona de 90 se comía el
/// hueco de la izquierda sin nada que la justificara.
pub const ANCHO_LOGO: f32 = 44.0;

pub fn panel(widgets: &Panel) -> PanelElement<'_> {
    // Los widgets se montan en una lista y no en un `row!` fijo porque casi
    // todos pueden faltar: sin batería, sin red o sin retroiluminación no se
    // dibuja un hueco vacío, se dibuja una cosa menos.
    let mut derecha = row![].align_y(iced_core::alignment::Vertical::Center);
    for (i, estado) in widgets.derecha().into_iter().enumerate() {
        if i > 0 {
            derecha = derecha.push(Space::new().width(Length::Fixed(HUECO)));
        }
        derecha = derecha.push(estado);
    }

    let fila = row![
        Space::new().width(Length::Fixed(MARGEN_PANEL)),
        crate::marca::ver(LADO_LOGO),
        Space::new().width(crate::FILL),
        derecha,
        Space::new().width(Length::Fixed(MARGEN_PANEL)),
    ]
    .align_y(iced_core::alignment::Vertical::Center)
    .height(Length::Fill);

    // El widget del centro va en su propia capa, no dentro de la fila, para
    // que esté centrado en la **pantalla** y no entre sus vecinos. Metido en la
    // fila se desplazaba cada vez que la derecha cambiaba de ancho: bastaba con
    // que la batería pasara de "85%" a "85% 2:15" para que la hora diese un
    // salto.
    let contenido: PanelElement<'_> = match widgets.centro() {
        Some(centro) => stack![fila, centro].into(),
        None => fila.into(),
    };

    container(contenido)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(tema::panel().into()),
            ..Default::default()
        })
        .into()
}
