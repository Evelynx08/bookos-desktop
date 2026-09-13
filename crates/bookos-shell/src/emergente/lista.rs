//! Las piezas que comparten las tarjetas de conectividad: Wi-Fi y Bluetooth.
//!
//! Las dos son la misma tarjeta del diseño (`Applets` en Figma): título con un
//! interruptor a la derecha, una lista de filas con icono, nombre, estado y un
//! número a la derecha, y un pie con dos botones. Sin esto habría dos copias de
//! la misma geometría que se irían separando a la primera corrección.
//!
//! Las medidas son las del diseño escaladas al ancho que usa el resto del
//! shell: la maqueta está a 202 px con textos de 8, y aquí la tarjeta mide 335
//! como la de energía, así que todo va multiplicado por 1,66.

use iced_core::alignment::Vertical;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;

/// Ancho de la tarjeta, el mismo que el de energía.
pub const ANCHO: f32 = 335.0;
pub const MARGEN: f32 = 21.0;
/// Alto de una fila de la lista.
pub const FILA: f32 = 43.0;
/// Hueco entre filas.
pub const HUECO_FILA: f32 = 6.0;
/// Lado del icono de una fila.
pub const ICONO: f32 = 21.0;
/// Alto de la cabecera, con el título y el interruptor.
pub const CABECERA: f32 = 40.0;
/// Alto de los botones del pie.
///
/// 34 y no los 28 de antes: eran dos barras de 28 px de alto con texto de 12
/// dentro de una tarjeta de 335, y se leían como dos rayas. Un botón de la
/// tabla del sistema mide 34.
pub const PIE_BOTON: f32 = 34.0;
/// Lo que hay por encima de los botones: el divisor de 1 px y su aire.
pub const PIE_AIRE: f32 = 11.0;
/// Alto del bloque entero del pie.
pub const PIE: f32 = PIE_AIRE + PIE_BOTON;
/// Medidas del interruptor.
pub const INTERRUPTOR_ANCHO: f32 = 40.0;
pub const INTERRUPTOR_ALTO: f32 = 22.0;

/// Una entrada de la lista: una red o un dispositivo.
pub struct Entrada {
    pub nombre: String,
    /// Debajo del nombre, en pequeño: «Conectado», «Emparejado»…
    pub estado: String,
    /// El número de la derecha: la señal de una red o la batería de unos
    /// auriculares. Sin unidad, que la pone quien la dibuja.
    pub derecha: Option<String>,
    pub icono: Option<Icono>,
    /// Se dibuja con el fondo de acento.
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
    let bolita = container(Space::new())
        .width(Length::Fixed(INTERRUPTOR_ALTO - 4.0))
        .height(Length::Fixed(INTERRUPTOR_ALTO - 4.0))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(Color::WHITE.into()),
            border: Border {
                radius: ((INTERRUPTOR_ALTO - 4.0) / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
    // La bolita se coloca con un hueco a un lado o al otro en vez de con una
    // posición absoluta: es lo mismo que hace el deslizador del volumen y evita
    // una capa `stack` solo para mover 18 px. Animada, el hueco es el recorrido
    // entero por la fracción, y con el muelle se pasa un poco del extremo — por
    // eso se recorta a cero: un hueco negativo no lo acepta el layout.
    let recorrido = (INTERRUPTOR_ANCHO - INTERRUPTOR_ALTO) * encendido;
    let dentro: PanelElement<'a> = row![
        Space::new().width(Length::Fixed(recorrido.max(0.0))),
        bolita
    ]
    .into();
    container(dentro)
        .width(Length::Fixed(INTERRUPTOR_ANCHO))
        .height(Length::Fixed(INTERRUPTOR_ALTO))
        .padding(2)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            // El fondo cruza del gris al acento con la bolita. El recorte a
            // [0,1] es por el muelle otra vez: pasarse de 1 en un canal de
            // color no es un rebote, es un color inventado.
            background: Some(
                tema::mezclar(
                    tema::alfa(tema::tinta(), 0.20),
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
pub fn cabecera<'a>(titulo: &str, encendido: Option<f32>) -> PanelElement<'a> {
    let mut fila =
        row![text(titulo.to_string()).size(18.0).color(tema::texto())].align_y(Vertical::Center);
    if let Some(encendido) = encendido {
        fila = fila
            .push(Space::new().width(Length::Fill))
            .push(interruptor(encendido));
    }
    container(fila)
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(CABECERA))
        .center_y(Length::Fixed(CABECERA))
        .into()
}

/// Lo que le queda al nombre de una fila una vez descontados el icono, el aire
/// y el hueco del número de la derecha.
///
/// Sin esto un SSID largo —los de las operadoras traen doce dígitos— empuja al
/// número fuera de la tarjeta y la fila se queda sin señal ni batería. Medido
/// con «2qxm79NAQsjDVNVj68nTTLF2CsZ9UJ8m»: ocupaba los 293 px enteros.
const ANCHO_NOMBRE: f32 = ANCHO - MARGEN * 2.0 - 20.0 - ICONO - 12.0 - 44.0;

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
    // La conectada se marca con **borde** de acento y el texto en acento, no
    // con el fondo relleno: en el diseño el relleno se reserva para lo que se
    // está pulsando, y una fila rellena entera tapaba su propio icono.
    let (reposo, realzado, color, borde) = if entrada.activa {
        (
            tema::alfa(tema::acento(), 0.10),
            // La conectada ya lleva acento: su hover sube el mismo relleno en
            // vez de meter un gris que lo ensuciaría.
            tema::alfa(tema::acento(), 0.20),
            tema::acento(),
            tema::acento(),
        )
    } else {
        (
            tema::alfa(tema::tinta(), 0.04),
            tema::hover(),
            tema::texto(),
            Color::TRANSPARENT,
        )
    };
    let fondo = tema::mezclar(reposo, realzado, señalada);
    let estado_color = if entrada.activa {
        tema::alfa(tema::acento(), 0.75)
    } else {
        tema::TEXTO2
    };
    let mut textos = column![
        text(recortar(&entrada.nombre, ANCHO_NOMBRE, 14.0))
            .size(14.0)
            .color(color)
    ];
    if !entrada.estado.is_empty() {
        textos = textos.push(
            text(recortar(&entrada.estado, ANCHO_NOMBRE, 10.0))
                .size(10.0)
                .color(estado_color),
        );
    }
    let dibujo: PanelElement<'a> = match &entrada.icono {
        Some(ic) => icono::ver_teñido(ic, ICONO, Some(color)),
        None => Space::new().width(Length::Fixed(ICONO)).into(),
    };
    let mut contenido =
        row![dibujo, Space::new().width(Length::Fixed(12.0)), textos].align_y(Vertical::Center);
    if let Some(derecha) = &entrada.derecha {
        contenido = contenido
            .push(Space::new().width(Length::Fill))
            .push(text(derecha.as_str()).size(12.0).color(estado_color));
    }
    container(contenido)
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(FILA))
        .padding([0, 10])
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
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
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(FILA))
        .center_x(Length::Fixed(ANCHO - MARGEN * 2.0))
        .center_y(Length::Fixed(FILA))
        .into()
}

/// El pie con dos botones, como en el diseño. `realce` señala el 0 o el 1.
pub fn pie<'a>(izquierda: &str, derecha: &str, realce: &tema::Realce) -> PanelElement<'a> {
    let boton = |etiqueta: &str, realzado: f32| -> PanelElement<'a> {
        let ancho = (ANCHO - MARGEN * 2.0 - 8.0) / 2.0;
        container(
            text(etiqueta.to_string())
                .size(tema::T_CUERPO)
                .color(tema::mezclar(tema::TEXTO2, tema::texto(), realzado)),
        )
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(PIE_BOTON))
        .center_x(Length::Fixed(ancho))
        .center_y(Length::Fixed(PIE_BOTON))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(
                tema::mezclar(tema::alfa(tema::tinta(), 0.06), tema::hover(), realzado).into(),
            ),
            border: Border {
                radius: tema::R_BOTON.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    };
    column![
        container(Space::new().height(Length::Fixed(1.0)))
            .width(Length::Fixed(ANCHO - MARGEN * 2.0))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(tema::divisor().into()),
                ..Default::default()
            }),
        Space::new().height(Length::Fixed(PIE_AIRE - 1.0)),
        row![
            boton(izquierda, realce.intensidad(0)),
            Space::new().width(Length::Fixed(8.0)),
            boton(derecha, realce.intensidad(1)),
        ],
    ]
    .into()
}

/// Dónde cae un punto dentro de una lista que empieza en `y0`.
///
/// El hueco entre filas no es de nadie: pulsarlo no debe activar la de al lado.
pub fn fila_en(x: f32, y: f32, y0: f32, n: usize) -> Option<usize> {
    if x < MARGEN || x > ANCHO - MARGEN || y < y0 {
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
            fila_en(5.0, 110.0, 100.0, 2),
            None,
            "fuera por la izquierda"
        );
    }
}
