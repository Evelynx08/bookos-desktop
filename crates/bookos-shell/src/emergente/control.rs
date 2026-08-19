//! Los controles que comparten las superficies emergentes.
//!
//! La píldora y el botón redondo salen igual en Sonido, en Brillo y en la
//! Estación de Control, con las mismas medidas del sistema de diseño. Están
//! aquí para que una sola sea la buena: tres copias de un deslizador acaban
//! con tres radios distintos y tres zonas agarrables que no coinciden.

use iced_core::{Border, Length};
use iced_widget::{container, Space};

use crate::icono::Icono;
use crate::tema;
use crate::view::PanelElement;

/// Alto de la píldora. Es la medida del plasmoide, y lo que la hace agarrable
/// sin apuntar: un deslizador de línea fina obliga a acertarle.
pub const PILDORA: f32 = 26.0;
/// Lado del botón redondo que va a su derecha.
pub const BOTON: f32 = 38.0;
/// Separación entre la píldora y el botón.
pub const HUECO: f32 = 10.0;
/// Cuánto se agranda la zona agarrable por arriba y por abajo.
///
/// Apuntar a 26 px de alto con el ratón en movimiento falla más de lo que
/// parece, y fallar aquí significa cerrar la emergente en vez de mover el
/// deslizador.
pub const MARGEN_AGARRE: f32 = 6.0;

/// La píldora: el canal lleno hasta `nivel` (de 0 a 100) sobre el surco.
pub fn pildora<'a>(ancho: f32, nivel: u8, apagado: bool) -> PanelElement<'a> {
    let lleno = ancho * (nivel.min(100) as f32 / 100.0);
    let color = if apagado {
        tema::TEXTO2
    } else {
        tema::acento()
    };
    let barra = container(Space::new())
        .width(Length::Fixed(lleno))
        .height(Length::Fixed(PILDORA))
        .style(move |_| container::Style {
            background: Some(color.into()),
            border: Border {
                radius: (PILDORA / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
    container(barra)
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(PILDORA))
        .style(|_| container::Style {
            background: Some(tema::surco().into()),
            border: Border {
                radius: (PILDORA / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// El botón redondo con un icono dentro.
///
/// `apagado` lo deja en gris con el icono en texto secundario; encendido va en
/// el acento con el icono en la tinta que se lea encima. `señalado`, de 0 a 1,
/// es el puntero por encima: aclara el fondo sin cambiar de estado, que es lo
/// que distingue «se puede pulsar» de «está pulsado».
pub fn boton<'a>(icono: Option<&'a Icono>, apagado: bool, señalado: f32) -> PanelElement<'a> {
    let (fondo, tinta) = if apagado {
        (tema::hover(), tema::TEXTO2)
    } else {
        (tema::acento(), tema::sobre_acento())
    };
    // Hacia la tinta y no hacia el blanco: sobre el tema claro, aclarar con
    // blanco un botón que ya es casi blanco no se ve.
    let fondo = tema::mezclar(fondo, tema::alfa(tema::tinta(), 0.22), 0.5 * señalado);
    let dentro = match icono {
        Some(ic) => crate::icono::ver_teñido(ic, 19.0, Some(tinta)),
        None => crate::widget::vacio(),
    };
    container(dentro)
        .width(Length::Fixed(BOTON))
        .height(Length::Fixed(BOTON))
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: (BOTON / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// De una `x` dentro de una píldora a su nivel, recortado a [0, 100].
///
/// Arrastrar más allá del borde deja el nivel en el tope y no en un número
/// imposible, que es lo que espera cualquiera al pasarse de largo.
pub fn nivel_en(x: f32, x0: f32, ancho: f32) -> u8 {
    (((x - x0) / ancho).clamp(0.0, 1.0) * 100.0).round() as u8
}

/// La caja de una tarjeta emergente: fondo, radio y margen del sistema.
pub fn tarjeta<'a>(contenido: PanelElement<'a>, ancho: f32, margen: f32) -> PanelElement<'a> {
    container(contenido)
        .padding(margen)
        .width(Length::Fixed(ancho))
        .style(|_| container::Style {
            background: Some(tema::card().into()),
            border: Border {
                radius: tema::R_TARJETA.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::nivel_en;

    #[test]
    fn los_extremos_dan_cero_y_cien() {
        assert_eq!(nivel_en(20.0, 20.0, 200.0), 0);
        assert_eq!(nivel_en(220.0, 20.0, 200.0), 100);
        assert_eq!(nivel_en(120.0, 20.0, 200.0), 50);
    }

    #[test]
    fn pasarse_de_largo_no_desborda() {
        assert_eq!(nivel_en(-9000.0, 20.0, 200.0), 0);
        assert_eq!(nivel_en(9000.0, 20.0, 200.0), 100);
    }
}
