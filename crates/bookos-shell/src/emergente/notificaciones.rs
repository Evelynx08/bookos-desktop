//! El emergente de las notificaciones.
//!
//! **La lista está vacía y no es un olvido.** Las notificaciones llegan por
//! D-Bus (`org.freedesktop.Notifications`) y este shell no habla con ningún
//! servicio a propósito: es lo que hace que el panel esté pintado en el primer
//! frame, sin esperar a que arranque nada. Implementar el servidor es una
//! decisión aparte —hay que registrar un nombre en el bus, atender `Notify` y
//! `CloseNotification`, y sostener una cola con caducidades—, no un rato de
//! trabajo.
//!
//! Lo que sí funciona es «No molestar», que es estado del propio shell: se
//! guarda aquí y el widget del panel lo enseña. Cuando haya servidor, será
//! además lo que decida si una notificación se enseña o se guarda.

use std::time::{Duration, Instant};

use iced_core::alignment::Vertical;
use iced_core::{Border, Color, Length};
use iced_widget::{column, container, row, text, Space};

use crate::tema;
use crate::view::PanelElement;
use crate::Accion;

use super::control;
use super::lista::{self, interruptor};
use super::{Ancla, Tecla};

/// Alto de la fila de «No molestar», con su explicación y su interruptor.
const BLOQUE: f32 = 56.0;
/// Alto de cada botón de duración.
const DURACION: f32 = 34.0;
/// Lado de la pastilla del icono, la misma que en la tarjeta de energía.
const PASTILLA: f32 = 32.0;

/// Cuánto dura el silencio, en la rejilla de dos por dos del diseño.
const DURACIONES: &[(&str, Option<Duration>)] = &[
    ("1 hora", Some(Duration::from_secs(3600))),
    ("4 horas", Some(Duration::from_secs(4 * 3600))),
    ("8 horas", Some(Duration::from_secs(8 * 3600))),
    ("Hasta desactivarlo", None),
];

pub struct Notificaciones {
    /// Cuándo se activó el silencio y hasta cuándo. `None` = no está puesto.
    silencio: Option<(Instant, Option<Duration>)>,
    /// Qué duración está elegida, para marcarla.
    duracion: usize,
    señalada: Option<usize>,
    /// El puntero está sobre el interruptor.
    sobre_interruptor: bool,
    /// La luna de «No molestar».
    icono: Option<crate::icono::Icono>,
}

impl Notificaciones {
    pub fn new() -> Self {
        Self {
            silencio: None,
            // «Hasta desactivarlo» por defecto: es lo que espera quien pulsa el
            // interruptor sin mirar las opciones.
            duracion: 3,
            señalada: None,
            sobre_interruptor: false,
            icono: crate::icono::propio("noche"),
        }
    }

    pub fn size(&self) -> (f32, f32) {
        // Las duraciones solo ocupan sitio cuando el silencio está puesto: es
        // lo que dice el diseño y evita cuatro botones que no hacen nada.
        // El aire tras la fila del silencio, la primera fila con su canal y la
        // segunda sin él: el canal de 8 px va contado dentro de `DURACION`.
        let duraciones = if self.silenciado() {
            8.0 + DURACION + (DURACION - 8.0)
        } else {
            0.0
        };
        (
            lista::ANCHO,
            lista::MARGEN * 2.0 + lista::CABECERA + lista::FILA + 12.0 + BLOQUE + duraciones,
        )
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("notificaciones")
    }

    /// ¿Está el silencio puesto ahora mismo?
    ///
    /// Se calcula en vez de guardarse porque caduca solo: un temporizador que
    /// despierte al shell a la hora en punto para apagar un booleano es
    /// exactamente el despertar de más que este compositor evita.
    pub fn silenciado(&self) -> bool {
        match self.silencio {
            None => false,
            Some((_, None)) => true,
            Some((desde, Some(cuanto))) => desde.elapsed() < cuanto,
        }
    }

    /// El rectángulo del interruptor.
    ///
    /// Va en la fila de «No molestar» y no en la cabecera, que es donde estaba:
    /// allí parecía apagar las notificaciones enteras —al lado de «Borrar
    /// todo»— cuando lo que hace es silenciarlas.
    fn rect_interruptor(&self) -> iced_core::Rectangle {
        iced_core::Rectangle {
            x: lista::ANCHO - lista::MARGEN - 10.0 - lista::INTERRUPTOR_ANCHO,
            y: self.y_bloque() + (BLOQUE - lista::INTERRUPTOR_ALTO) / 2.0,
            width: lista::INTERRUPTOR_ANCHO,
            height: lista::INTERRUPTOR_ALTO,
        }
    }

    /// La `y` donde empieza la fila de «No molestar».
    fn y_bloque(&self) -> f32 {
        lista::MARGEN + lista::CABECERA + lista::FILA + 12.0
    }

    /// La `y` donde empieza la rejilla de duraciones.
    fn y_duraciones(&self) -> f32 {
        self.y_bloque() + BLOQUE + 8.0
    }

    /// Qué botón de duración cae en un punto.
    fn duracion_en(&self, x: f32, y: f32) -> Option<usize> {
        if !self.silenciado() {
            return None;
        }
        let y0 = self.y_duraciones();
        if x < lista::MARGEN || x > lista::ANCHO - lista::MARGEN || y < y0 {
            return None;
        }
        let fila = ((y - y0) / DURACION) as usize;
        if fila > 1 {
            return None;
        }
        let columna = usize::from(x > lista::ANCHO / 2.0);
        Some(fila * 2 + columna)
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let señalada = punto.and_then(|(x, y)| self.duracion_en(x, y));
        let sobre = punto.is_some_and(|(x, y)| {
            self.rect_interruptor()
                .contains(iced_core::Point::new(x, y))
        });
        if señalada == self.señalada && sobre == self.sobre_interruptor {
            return false;
        }
        self.señalada = señalada;
        self.sobre_interruptor = sobre;
        true
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = iced_core::Point::new(x, y);
        if self.rect_interruptor().contains(punto) {
            self.silencio = if self.silenciado() {
                None
            } else {
                Some((Instant::now(), DURACIONES[self.duracion].1))
            };
            return None;
        }
        if let Some(i) = self.duracion_en(x, y) {
            self.duracion = i;
            // Elegir una duración enciende el silencio: nadie pulsa «4 horas»
            // para dejarlo apagado.
            self.silencio = Some((Instant::now(), DURACIONES[i].1));
        }
        None
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    fn boton_duracion(&self, i: usize) -> PanelElement<'_> {
        let (etiqueta, _) = DURACIONES[i];
        let elegido = self.duracion == i && self.silenciado();
        // Chips de verdad, con relleno: sin fondo no se veía que fueran cuatro
        // botones, solo cuatro renglones de texto gris debajo del interruptor.
        let (fondo, color) = match (elegido, self.señalada == Some(i)) {
            (true, _) => (tema::ACENTO, Color::WHITE),
            (false, true) => (tema::HOVER, tema::TEXTO),
            (false, false) => (Color { a: 0.04, ..Color::WHITE }, tema::TEXTO2),
        };
        // Cuatro celdas de dos en dos, con 8 px de canal en medio.
        let ancho = (lista::ANCHO - lista::MARGEN * 2.0 - 8.0) / 2.0;
        container(text(etiqueta).size(12.0).color(color))
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(DURACION - 8.0))
            .center_x(Length::Fixed(ancho))
            .center_y(Length::Fixed(DURACION - 8.0))
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_BOTON_PEQUENO.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// Una fila de chips con su canal en medio.
    fn fila_duraciones(&self, i: usize) -> PanelElement<'_> {
        row![
            self.boton_duracion(i),
            Space::new().width(Length::Fixed(8.0)),
            self.boton_duracion(i + 1),
        ]
        .into()
    }

    pub fn view(&self) -> PanelElement<'_> {
        // «Config» y «Borrar todo» van en texto plano y no en píldoras: con
        // relleno pesaban más que el propio título —una de ellas roja— y la
        // cabecera se leía como una barra de tres botones.
        let boton = |etiqueta: &'static str, rojo: bool| -> PanelElement<'static> {
            container(
                text(etiqueta)
                    .size(12.0)
                    .color(if rojo { tema::ROJO } else { tema::TEXTO2 }),
            )
            .height(Length::Fixed(24.0))
            .center_y(Length::Fixed(24.0))
            .padding([0, 6])
            .into()
        };
        let cabecera = container(
            row![
                text("Notificaciones")
                    .size(tema::T_TITULO)
                    .color(tema::TEXTO),
                Space::new().width(Length::Fill),
                boton("Config", false),
                boton("Borrar todo", true),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(lista::ANCHO - lista::MARGEN * 2.0))
        .height(Length::Fixed(lista::CABECERA))
        .center_y(Length::Fixed(lista::CABECERA));

        // «No molestar» es una fila-control con su interruptor dentro, con la
        // misma pastilla de icono que los perfiles de energía: es lo que hace
        // que las dos tarjetas se lean como del mismo sistema.
        let pastilla_fondo = if self.silenciado() {
            Color { a: 0.20, ..tema::ACENTO }
        } else {
            Color { a: 0.06, ..Color::WHITE }
        };
        let tinta = if self.silenciado() {
            tema::ACENTO
        } else {
            tema::TEXTO2
        };
        let pastilla = container(match &self.icono {
            Some(ic) => crate::icono::ver_teñido(ic, 18.0, Some(tinta)),
            None => Space::new().width(Length::Fixed(18.0)).into(),
        })
        .width(Length::Fixed(PASTILLA))
        .height(Length::Fixed(PASTILLA))
        .center_x(Length::Fixed(PASTILLA))
        .center_y(Length::Fixed(PASTILLA))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(pastilla_fondo.into()),
            border: Border {
                radius: (PASTILLA / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let no_molestar = container(
            row![
                pastilla,
                Space::new().width(Length::Fixed(12.0)),
                column![
                    text("No molestar").size(14.0).color(tema::TEXTO),
                    text("Las notificaciones solo se guardan aquí")
                        .size(11.0)
                        .color(tema::TEXTO2),
                ],
                Space::new().width(Length::Fill),
                interruptor(self.silenciado()),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(lista::ANCHO - lista::MARGEN * 2.0))
        .height(Length::Fixed(BLOQUE))
        .center_y(Length::Fixed(BLOQUE))
        .padding([0, 10])
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(Color { a: 0.04, ..Color::WHITE }.into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let mut contenido = column![
            cabecera,
            lista::vacia("No hay notificaciones"),
            Space::new().height(Length::Fixed(12.0)),
            no_molestar,
        ];
        if self.silenciado() {
            contenido = contenido
                .push(Space::new().height(Length::Fixed(8.0)))
                .push(self.fila_duraciones(0))
                .push(Space::new().height(Length::Fixed(8.0)))
                .push(self.fila_duraciones(2));
        }
        control::tarjeta(contenido.into(), lista::ANCHO, lista::MARGEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El interruptor enciende y apaga el silencio.
    #[test]
    fn el_interruptor_alterna_el_silencio() {
        let mut n = Notificaciones::new();
        assert!(!n.silenciado());
        let r = n.rect_interruptor();
        n.pulsar(r.x + 2.0, r.y + 2.0);
        assert!(n.silenciado());
        n.pulsar(r.x + 2.0, r.y + 2.0);
        assert!(!n.silenciado(), "el segundo toque lo apaga");
    }

    /// Las duraciones solo existen con el silencio puesto —es lo que dice el
    /// diseño—, y elegir una lo mantiene con esa cuenta.
    #[test]
    fn las_duraciones_solo_estan_con_el_silencio_puesto() {
        let mut n = Notificaciones::new();
        let y = n.y_duraciones() + 2.0;
        n.pulsar(30.0, y);
        assert!(!n.silenciado(), "sin silencio ahí no hay ningún botón");

        let r = n.rect_interruptor();
        n.pulsar(r.x + 2.0, r.y + 2.0);
        assert!(n.silenciado());
        n.pulsar(30.0, n.y_duraciones() + 2.0);
        assert_eq!(n.duracion, 0, "el primero es «1 hora»");
        assert!(n.silenciado());
    }

    /// Un silencio caducado no silencia. Se comprueba con una duración de cero,
    /// que es lo único que se puede medir sin esperar una hora.
    #[test]
    fn el_silencio_caduca_solo() {
        let mut n = Notificaciones::new();
        n.silencio = Some((Instant::now(), Some(Duration::ZERO)));
        assert!(!n.silenciado());
    }
}
