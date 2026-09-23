//! Las piezas que comparten las tarjetas de conectividad: Wi-Fi y Bluetooth.
//!
//! Las dos son la misma tarjeta del diseño (`Applets` en Figma): título con un
//! interruptor a la derecha y, debajo, un grupo gris con las filas en baldosas
//! —icono, nombre, estado y un número a la derecha— y un pie con dos botones.
//! Sin esto habría dos copias de la misma geometría que se irían separando a la
//! primera corrección.
//!
//! Las medidas son las del lienzo «Widgets de BookOS»: tarjeta de 336 con
//! margen de 16, cabecera de 32, grupo con 6 de relleno y baldosas de 48.

use iced_core::alignment::Vertical;
use iced_core::font::Weight;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;

use super::control::{self, GRUPO};

/// Ancho de la tarjeta.
pub const ANCHO: f32 = 336.0;
pub const MARGEN: f32 = 16.0;
/// Ancho de una baldosa: la tarjeta menos su margen y el relleno del grupo.
pub const BALDOSA: f32 = ANCHO - MARGEN * 2.0 - GRUPO * 2.0;
/// Alto de una fila de la lista.
pub const FILA: f32 = 48.0;
/// Hueco entre filas.
pub const HUECO_FILA: f32 = 6.0;
/// Lado del icono de una fila.
pub const ICONO: f32 = 22.0;
/// Alto de la cabecera, con el título y el interruptor.
pub const CABECERA: f32 = 32.0;
/// Aire entre la cabecera y el grupo.
pub const BAJO_CABECERA: f32 = 12.0;
/// Alto de los botones del pie.
pub const PIE_BOTON: f32 = 36.0;
/// Lo que hay entre la última fila y los botones: hueco, divisor con su aire de
/// 2 px por lado, y otro hueco.
pub const PIE_AIRE: f32 = HUECO_FILA + 2.0 + 0.5 + 2.0 + HUECO_FILA;
/// Alto del bloque entero del pie.
pub const PIE: f32 = PIE_AIRE + PIE_BOTON;
/// Medidas del interruptor: pista de 46×28 y bolita de 22 con 3 de margen,
/// las del sistema de diseño.
pub const INTERRUPTOR_ANCHO: f32 = 46.0;
pub const INTERRUPTOR_ALTO: f32 = 28.0;

/// Una entrada de la lista: una red o un dispositivo.
pub struct Entrada {
    pub nombre: String,
    /// Debajo del nombre, en pequeño: «Conectado», «Emparejado»…
    pub estado: String,
    /// El número de la derecha: la señal de una red o la batería de unos
    /// auriculares. Sin unidad, que la pone quien la dibuja.
    pub derecha: Option<String>,
    pub icono: Option<Icono>,
    /// Se dibuja como seleccionada.
    pub activa: bool,
}

/// El interruptor de la cabecera. `encendido` va de 0 a 1: es el recorrido de
/// la bolita, no un booleano.
///
/// El sistema de diseño le da 250 ms con muelle (§2.7, «conmutador»), que es la
/// duración de [`crate::tema::D_MODAL`]: el estado del interruptor **es** la
/// decisión que se acaba de tomar y se ve mejor que ninguna otra cosa de la
/// tarjeta. El que lo use guarda una [`crate::tema::Transicion`] y le pasa aquí
/// su valor.
pub fn interruptor<'a>(encendido: f32) -> PanelElement<'a> {
    let lado = INTERRUPTOR_ALTO - 6.0;
    let bolita = container(Space::new())
        .width(Length::Fixed(lado))
        .height(Length::Fixed(lado))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(Color::WHITE.into()),
            border: Border {
                radius: (lado / 2.0).into(),
                ..Default::default()
            },
            shadow: iced_core::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.2),
                offset: iced_core::Vector::new(0.0, 1.0),
                blur_radius: 3.0,
            },
            ..Default::default()
        });
    // La bolita se coloca con un hueco a un lado o al otro en vez de con una
    // posición absoluta: evita una capa `stack` solo para mover 18 px. Animada,
    // el hueco es el recorrido entero por la fracción, y con el muelle se pasa
    // un poco del extremo — por eso se recorta a cero: un hueco negativo no lo
    // acepta el layout.
    let recorrido = (INTERRUPTOR_ANCHO - INTERRUPTOR_ALTO) * encendido;
    let dentro: PanelElement<'a> = row![
        Space::new().width(Length::Fixed(recorrido.max(0.0))),
        bolita
    ]
    .into();
    container(dentro)
        .width(Length::Fixed(INTERRUPTOR_ANCHO))
        .height(Length::Fixed(INTERRUPTOR_ALTO))
        .padding(3)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            // Del `--toff` opaco al acento con la bolita. El recorte a [0,1] es
            // por el muelle otra vez: pasarse de 1 en un canal de color no es
            // un rebote, es un color inventado.
            background: Some(
                tema::mezclar(
                    tema::control_apagado(),
                    tema::acento(),
                    encendido.clamp(0.0, 1.0),
                )
                .into(),
            ),
            border: Border {
                radius: (INTERRUPTOR_ALTO / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// La cabecera: título a la izquierda, interruptor a la derecha.
///
/// Los 4 px de relleno lateral dejan el título a 20 del borde de la tarjeta,
/// que es el padding de una fila en el sistema de diseño, mientras el grupo de
/// debajo se queda a 16.
pub fn cabecera<'a>(titulo: &'a str, encendido: Option<f32>) -> PanelElement<'a> {
    let mut fila = row![control::titulo(titulo)].align_y(Vertical::Center);
    if let Some(encendido) = encendido {
        fila = fila
            .push(Space::new().width(Length::Fill))
            .push(interruptor(encendido));
    }
    container(fila)
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(CABECERA))
        .padding([0, 4])
        .center_y(Length::Fixed(CABECERA))
        .into()
}

/// El grupo gris que envuelve la lista y su pie.
pub fn grupo<'a>(contenido: PanelElement<'a>) -> PanelElement<'a> {
    control::grupo(contenido, ANCHO - MARGEN * 2.0, GRUPO)
}

/// Lo que le queda al nombre de una fila una vez descontados el relleno, el
/// icono, su hueco y el número de la derecha.
///
/// Sin esto un SSID largo —los de las operadoras traen doce dígitos— empuja al
/// número fuera de la tarjeta y la fila se queda sin señal ni batería. Medido
/// con «2qxm79NAQsjDVNVj68nTTLF2CsZ9UJ8m»: ocupaba los 293 px enteros.
const ANCHO_NOMBRE: f32 = BALDOSA - 24.0 - ICONO - 12.0 - 44.0;

/// Recorta un texto al ancho disponible y le pone puntos suspensivos.
///
/// La medida es la aproximación de [`crate::widget::ancho_texto`], que es un
/// ancho medio por carácter: el shell no tiene acceso al `Paragraph` de iced
/// antes de construirlo, y un carácter de más o de menos aquí no rompe nada —el
/// hueco reservado son 44 px.
pub fn recortar(texto: &str, ancho: f32, tamaño: f32) -> String {
    let por_letra = crate::widget::ancho_texto(1) * tamaño / crate::tema::T_CUERPO;
    let caben = (ancho / por_letra).floor() as usize;
    if texto.chars().count() <= caben {
        return texto.to_string();
    }
    // Uno menos para dejarle sitio a los puntos, que ocupan como una letra.
    texto
        .chars()
        .take(caben.saturating_sub(1))
        .collect::<String>()
        + "…"
}

/// Una fila de la lista. `señalada` va de 0 a 1: el hover entrando.
pub fn fila<'a>(entrada: &'a Entrada, señalada: f32) -> PanelElement<'a> {
    // La conectada va con el acento al 10 % y un filo de acento, con el texto
    // en acento: la referencia la marcaba con el filo, y el relleno tenue es lo
    // que el sistema de diseño pide para lo seleccionado en una lista.
    let (fondo, color, borde, estado_color) = if entrada.activa {
        (
            control::seleccionada(señalada),
            tema::acento(),
            tema::alfa(tema::acento(), 0.45),
            tema::acento(),
        )
    } else {
        (
            control::baldosa(señalada),
            tema::texto(),
            Color::TRANSPARENT,
            tema::TEXTO2,
        )
    };
    let mut textos = column![
        text(recortar(&entrada.nombre, ANCHO_NOMBRE, 14.0))
            .size(14.0)
            .font(control::peso(Weight::Medium))
            .color(color)
    ];
    if !entrada.estado.is_empty() {
        textos = textos.push(
            text(recortar(&entrada.estado, ANCHO_NOMBRE, 11.0))
                .size(11.0)
                .color(estado_color),
        );
    }
    let dibujo: PanelElement<'a> = match &entrada.icono {
        Some(ic) => icono::ver_teñido(
            ic,
            ICONO,
            Some(if entrada.activa {
                tema::acento()
            } else {
                tema::TEXTO2
            }),
        ),
        None => Space::new().width(Length::Fixed(ICONO)).into(),
    };
    let mut contenido =
        row![dibujo, Space::new().width(Length::Fixed(12.0)), textos].align_y(Vertical::Center);
    if let Some(derecha) = &entrada.derecha {
        contenido = contenido
            .push(Space::new().width(Length::Fill))
            .push(text(derecha.as_str()).size(13.0).color(estado_color));
    }
    container(contenido)
        .width(Length::Fixed(BALDOSA))
        .height(Length::Fixed(FILA))
        .center_y(Length::Fixed(FILA))
        .padding([0, 12])
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
                width: 1.0,
                color: borde,
            },
            ..Default::default()
        })
        .into()
}

/// Un renglón cuando no hay nada que listar.
pub fn vacia<'a>(texto_: &str) -> PanelElement<'a> {
    container(text(texto_.to_string()).size(13.0).color(tema::TEXTO2))
        .width(Length::Fixed(BALDOSA))
        .height(Length::Fixed(FILA))
        .center_x(Length::Fixed(BALDOSA))
        .center_y(Length::Fixed(FILA))
        .into()
}

/// El pie con dos botones, dentro del grupo. `realce` señala el 0 o el 1.
pub fn pie<'a>(izquierda: &'a str, derecha: &'a str, realce: &tema::Realce) -> PanelElement<'a> {
    let ancho = (BALDOSA - HUECO_FILA) / 2.0;
    let boton = |etiqueta: &'a str, realzado: f32| -> PanelElement<'a> {
        container(
            text(etiqueta)
                .size(tema::T_CUERPO)
                .font(control::peso(Weight::Semibold))
                .color(tema::texto()),
        )
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(PIE_BOTON))
        .center_x(Length::Fixed(ancho))
        .center_y(Length::Fixed(PIE_BOTON))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(control::baldosa(realzado).into()),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    };
    column![
        Space::new().height(Length::Fixed(HUECO_FILA + 2.0)),
        container(control::divisor(BALDOSA - 12.0)).padding([0, 6]),
        Space::new().height(Length::Fixed(2.0 + HUECO_FILA)),
        row![
            boton(izquierda, realce.intensidad(0)),
            Space::new().width(Length::Fixed(HUECO_FILA)),
            boton(derecha, realce.intensidad(1)),
        ],
    ]
    .into()
}

/// Sobre qué botón del pie cae un punto, si el pie empieza en `y0`: `false` el
/// izquierdo y `true` el derecho.
pub fn pie_en(x: f32, y: f32, y0: f32) -> Option<bool> {
    if y < y0 || y > y0 + PIE_BOTON {
        return None;
    }
    (x > MARGEN + GRUPO && x < ANCHO - MARGEN - GRUPO).then(|| x > ANCHO / 2.0)
}

/// Dónde cae un punto dentro de una lista que empieza en `y0`.
///
/// El hueco entre filas no es de nadie: pulsarlo no debe activar la de al lado.
pub fn fila_en(x: f32, y: f32, y0: f32, n: usize) -> Option<usize> {
    if x < MARGEN + GRUPO || x > ANCHO - MARGEN - GRUPO || y < y0 {
        return None;
    }
    let rel = y - y0;
    let i = (rel / (FILA + HUECO_FILA)) as usize;
    if rel - i as f32 * (FILA + HUECO_FILA) > FILA {
        return None;
    }
    (i < n).then_some(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un SSID largo se recorta con puntos en vez de empujar al número de la
    /// derecha fuera de la tarjeta.
    #[test]
    fn un_nombre_largo_se_recorta() {
        let largo = "2qxm79NAQsjDVNVj68nTTLF2CsZ9UJ8m";
        let corto = recortar(largo, ANCHO_NOMBRE, 14.0);
        assert!(corto.ends_with('…'), "{corto}");
        assert!(corto.chars().count() < largo.chars().count());
        assert_eq!(
            recortar("Wifi", ANCHO_NOMBRE, 14.0),
            "Wifi",
            "lo que cabe no se toca"
        );
    }

    #[test]
    fn el_hueco_entre_filas_no_es_de_nadie() {
        assert_eq!(fila_en(50.0, 100.0, 100.0, 2), Some(0));
        assert_eq!(fila_en(50.0, 100.0 + FILA + 2.0, 100.0, 2), None);
        assert_eq!(
            fila_en(50.0, 100.0 + FILA + HUECO_FILA + 1.0, 100.0, 2),
            Some(1)
        );
        assert_eq!(fila_en(50.0, 99.0, 100.0, 2), None, "por encima");
        assert_eq!(
            fila_en(MARGEN, 110.0, 100.0, 2),
            None,
            "fuera por la izquierda"
        );
    }
}
