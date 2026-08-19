//! Apariencia: el tema claro u oscuro y el color de acento.
//!
//! Es la primera tarjeta del shell que **escribe** en la configuración algo que
//! no es una lista de aplicaciones: al elegir aquí, el compositor aplica el
//! cambio en caliente y lo guarda en `panel.conf`. El escritorio no se
//! reinicia y no hay que editar el fichero a mano, que era la única forma de
//! cambiar de tema hasta ahora.
//!
//! **Por qué vive en el shell y no en `bookos-settings`.** El acento es un
//! global del proceso del compositor (`tema::aplicar_acento`): quien lo cambie
//! desde fuera tendría que avisar por algún camino que hoy no existe —el shell
//! no tiene D-Bus ni vigila el fichero—, y el panel se quedaría con la paleta
//! anterior hasta reiniciar la sesión. Cuando `bookos-settings` tenga por dónde
//! decírselo, esta tarjeta sigue valiendo: es el mismo control, más a mano.

use iced_core::alignment::Vertical;
use iced_core::{Border, Color, Length};
use iced_widget::{column, container, row, text, Space};

use crate::tema::{self, Acento, Realce, Tema};
use crate::view::PanelElement;
use crate::Accion;

use super::{Ancla, Tecla};

/// El ancho de las demás tarjetas del shell.
const ANCHO: f32 = 335.0;
const MARGEN: f32 = 21.0;
const CABECERA: f32 = 40.0;
/// Alto de la muestra de un tema, con su rótulo debajo.
const MUESTRA: f32 = 78.0;
const MUESTRA_HUECO: f32 = 11.0;
/// Alto del renglón «Color de acento».
const ROTULO: f32 = 30.0;
/// Lado de la casilla de un color: el círculo de 30 más su anillo.
const CASILLA: f32 = 42.0;
const COLUMNAS: usize = 5;
const CIRCULO: f32 = 30.0;
/// El divisor y el aire que lo rodea.
const SEPARACION: f32 = 17.0;

/// Ancho de una muestra de tema. Dos, con un hueco en medio.
fn ancho_muestra() -> f32 {
    (ANCHO - MARGEN * 2.0 - MUESTRA_HUECO) / 2.0
}

/// Paso horizontal de la rejilla de colores: lo que sobra se reparte entre las
/// cuatro calles, no se acumula a la derecha.
fn paso() -> f32 {
    let libre = ANCHO - MARGEN * 2.0 - CASILLA * COLUMNAS as f32;
    CASILLA + libre / (COLUMNAS - 1) as f32
}

fn filas() -> usize {
    Acento::TODOS.len().div_ceil(COLUMNAS)
}

pub struct Apariencia {
    /// Lo elegido. No se lee de `tema::actual()` en cada `view` porque el
    /// cambio lo aplica el compositor y puede tardar un frame en volver: la
    /// tarjeta se marca a sí misma en cuanto se pulsa.
    tema: Tema,
    acento: Acento,
    /// Lo señalado por el puntero: las dos muestras de tema y la rejilla.
    hover_tema: Realce,
    hover_color: Realce,
    /// Lo elegido, animado: el anillo crece en el nuevo y se apaga en el
    /// anterior en vez de saltar de un sitio a otro.
    marca_tema: Realce,
    marca_color: Realce,
}

impl Apariencia {
    pub fn new() -> Self {
        let tema = tema::actual();
        let acento = tema::acento_actual();
        let mut marca_tema = Realce::nuevo();
        let mut marca_color = Realce::nuevo();
        // Al abrirse ya está elegido: sin esto la tarjeta aparece con todo
        // apagado y las marcas entrando, como si nadie hubiera elegido nada.
        marca_tema.señalar(Some(matches!(tema, Tema::Oscuro) as usize));
        marca_color.señalar(indice(acento));
        marca_tema.terminar();
        marca_color.terminar();
        Self {
            tema,
            acento,
            hover_tema: Realce::nuevo(),
            hover_color: Realce::nuevo(),
            marca_tema,
            marca_color,
        }
    }

    pub fn size(&self) -> (f32, f32) {
        let alto =
            CABECERA + MUESTRA + SEPARACION + ROTULO + CASILLA * filas() as f32 + MARGEN * 2.0;
        (ANCHO, alto)
    }

    pub fn ancla(&self) -> Ancla {
        // Cuelga del panel por la izquierda, como el menú del que se abre.
        Ancla::BajoElPanel { x: 8.0 }
    }

    /// ¿Se está moviendo algo dentro de la tarjeta?
    pub fn animando(&self) -> bool {
        self.hover_tema.animando()
            || self.hover_color.animando()
            || self.marca_tema.animando()
            || self.marca_color.animando()
    }

    /// `y` donde empieza la fila de muestras de tema.
    fn y_muestras(&self) -> f32 {
        MARGEN + CABECERA
    }

    fn y_rejilla(&self) -> f32 {
        self.y_muestras() + MUESTRA + SEPARACION + ROTULO
    }

    fn muestra_en(&self, x: f32, y: f32) -> Option<usize> {
        let y0 = self.y_muestras();
        if y < y0 || y > y0 + MUESTRA || x < MARGEN || x > ANCHO - MARGEN {
            return None;
        }
        let rel = x - MARGEN;
        let ancho = ancho_muestra();
        match rel {
            r if r <= ancho => Some(0),
            r if r >= ancho + MUESTRA_HUECO => Some(1),
            // El hueco entre las dos no es de nadie.
            _ => None,
        }
    }

    fn color_en(&self, x: f32, y: f32) -> Option<usize> {
        let y0 = self.y_rejilla();
        if y < y0 || x < MARGEN {
            return None;
        }
        let (col, fila) = (
            ((x - MARGEN) / paso()) as usize,
            ((y - y0) / CASILLA) as usize,
        );
        // Fuera de la casilla, en la calle que la separa de la siguiente: no
        // cuenta, o pulsar entre dos colores elegiría el de la izquierda.
        if x - MARGEN - col as f32 * paso() > CASILLA || col >= COLUMNAS {
            return None;
        }
        let i = fila * COLUMNAS + col;
        (i < Acento::TODOS.len()).then_some(i)
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let (m, c) = match punto {
            Some((x, y)) => (self.muestra_en(x, y), self.color_en(x, y)),
            None => (None, None),
        };
        // Los dos `señalar` **siempre**, sin cortocircuito: con `||` el
        // segundo no se evalúa cuando el primero cambia y la rejilla se queda
        // con un color encendido al salir el ratón por arriba.
        let a = self.hover_tema.señalar(m);
        let b = self.hover_color.señalar(c);
        a || b
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(i) = self.muestra_en(x, y) {
            self.tema = if i == 0 { Tema::Claro } else { Tema::Oscuro };
            self.marca_tema.señalar(Some(i));
            return Some(self.accion());
        }
        let i = self.color_en(x, y)?;
        self.acento = Acento::TODOS[i];
        self.marca_color.señalar(Some(i));
        Some(self.accion())
    }

    fn accion(&self) -> Accion {
        Accion::Apariencia {
            tema: self.tema,
            acento: self.acento,
        }
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut rejilla = column![];
        for f in 0..filas() {
            let mut fila = row![];
            for c in 0..COLUMNAS {
                let i = f * COLUMNAS + c;
                let Some(acento) = Acento::TODOS.get(i).copied() else {
                    break;
                };
                if c > 0 {
                    fila = fila.push(Space::new().width(Length::Fixed(paso() - CASILLA)));
                }
                fila = fila.push(casilla(
                    acento,
                    self.marca_color.intensidad(i),
                    self.hover_color.intensidad(i),
                ));
            }
            rejilla = rejilla.push(fila);
        }

        let contenido = column![
            container(text("Apariencia").size(tema::T_TITULO).color(tema::texto()))
                .height(Length::Fixed(CABECERA))
                .center_y(Length::Fixed(CABECERA)),
            row![
                muestra(
                    Tema::Claro,
                    self.marca_tema.intensidad(0),
                    self.hover_tema.intensidad(0)
                ),
                Space::new().width(Length::Fixed(MUESTRA_HUECO)),
                muestra(
                    Tema::Oscuro,
                    self.marca_tema.intensidad(1),
                    self.hover_tema.intensidad(1)
                ),
            ],
            separador(),
            container(
                text("Color de acento")
                    .size(tema::T_CUERPO)
                    .color(tema::TEXTO2)
            )
            .height(Length::Fixed(ROTULO))
            .center_y(Length::Fixed(ROTULO)),
            rejilla,
        ];

        super::control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

fn indice(acento: Acento) -> Option<usize> {
    Acento::TODOS.iter().position(|a| *a == acento)
}

/// La muestra de un tema: un trozo de escritorio en miniatura.
///
/// Se dibuja con los colores de ese tema y **no** con los del que está puesto:
/// la gracia de la muestra es enseñar cómo se vería lo otro.
fn muestra<'a>(cual: Tema, elegida: f32, señalada: f32) -> PanelElement<'a> {
    let claro = matches!(cual, Tema::Claro);
    // Los mismos `bg` y `card` de la tabla de tokens, pero escritos aquí: la
    // muestra enseña el tema que **no** está puesto, así que no puede
    // preguntárselos a `tema::bg()`.
    let (fondo, tarjeta) = if claro {
        (Color::from_rgb(0.949, 0.949, 0.969), Color::WHITE)
    } else {
        (Color::BLACK, Color::from_rgb(0.110, 0.110, 0.118))
    };
    let ancho = ancho_muestra();
    // La miniatura: la barra del panel arriba y una tarjeta debajo, que es lo
    // que distingue de un vistazo un tema del otro.
    let dibujo = column![
        // El `clip` del lienzo recorta en rectángulo, no por el radio, así que
        // la barra lleva sus propias esquinas: sin esto, la muestra clara
        // asomaba dos cuadraditos blancos por encima de la curva.
        container(Space::new())
            .width(Length::Fill)
            .height(Length::Fixed(7.0))
            .style(move |_| container::Style {
                background: Some(tarjeta.into()),
                border: Border {
                    radius: iced_core::border::Radius::default()
                        .top_left(tema::R_BOTON)
                        .top_right(tema::R_BOTON),
                    ..Default::default()
                },
                ..Default::default()
            }),
        container(
            container(Space::new())
                .width(Length::Fixed(ancho * 0.5))
                .height(Length::Fixed(14.0))
                .style(move |_| container::Style {
                    background: Some(tarjeta.into()),
                    border: Border {
                        radius: tema::R_CHIP.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
        )
        .padding(8)
    ];

    let alto_lienzo = MUESTRA - 22.0;
    let lienzo = container(dibujo)
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(alto_lienzo))
        .clip(true)
        .style(move |_| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_BOTON.into(),
                // El marco de elegido crece con la animación en vez de
                // aparecer: 2 px que salen de golpe se leen como un parpadeo.
                width: 2.0 * elegida,
                color: tema::alfa(tema::acento(), elegida),
            },
            ..Default::default()
        });

    let rotulo = text(if claro { "Claro" } else { "Oscuro" })
        .size(tema::T_PEQUENO)
        // El elegido pasa a acento; el señalado, a texto normal.
        // El rótulo es de la tarjeta, no de la muestra: va con la tinta del
        // tema que está puesto. Señalado sube de secundario a normal; elegido,
        // al acento.
        .color(tema::mezclar(
            tema::mezclar(tema::TEXTO2, tema::texto(), señalada),
            tema::acento(),
            elegida,
        ));

    container(
        column![lienzo, Space::new().height(Length::Fixed(4.0)), rotulo]
            .align_x(iced_core::alignment::Horizontal::Center),
    )
    .width(Length::Fixed(ancho))
    .height(Length::Fixed(MUESTRA))
    .into()
}

/// Una casilla de la rejilla: el círculo del color, con su anillo si está
/// elegido y su halo si está señalado.
fn casilla<'a>(acento: Acento, elegido: f32, señalado: f32) -> PanelElement<'a> {
    // El color de la casilla es el del acento **en el tema que está puesto**,
    // que es como se va a ver si se elige.
    let color = acento.color();
    // Señalado, el círculo crece 3 px sin mover a los vecinos: la casilla mide
    // 42 y el círculo 30, así que hay sitio de sobra dentro.
    let lado = CIRCULO + 3.0 * señalado;
    let punto = container(Space::new())
        .width(Length::Fixed(lado))
        .height(Length::Fixed(lado))
        .style(move |_| container::Style {
            background: Some(color.into()),
            border: Border {
                radius: tema::R_PILL.into(),
                ..Default::default()
            },
            ..Default::default()
        });
    container(punto)
        .width(Length::Fixed(CASILLA))
        .height(Length::Fixed(CASILLA))
        .center_x(Length::Fixed(CASILLA))
        .center_y(Length::Fixed(CASILLA))
        .style(move |_| container::Style {
            border: Border {
                radius: tema::R_PILL.into(),
                // El anillo va del color del propio acento y no del acento
                // puesto: el que se está eligiendo todavía no lo es.
                width: 2.0 * elegido,
                color: tema::alfa(color, elegido),
            },
            ..Default::default()
        })
        .into()
}

fn separador<'a>() -> PanelElement<'a> {
    container(
        container(Space::new().height(Length::Fixed(1.0)))
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(tema::divisor().into()),
                ..Default::default()
            }),
    )
    .height(Length::Fixed(SEPARACION))
    .center_y(Length::Fixed(SEPARACION))
    .align_y(Vertical::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Las diez casillas tienen que caber en la rejilla y responder cada una
    /// en su sitio: pulsar entre dos colores no elige ninguno.
    #[test]
    fn la_rejilla_reparte_los_diez_colores() {
        let a = Apariencia::new();
        let y0 = a.y_rejilla();
        assert_eq!(a.color_en(MARGEN + 1.0, y0 + 1.0), Some(0));
        assert_eq!(a.color_en(MARGEN + paso() + 1.0, y0 + 1.0), Some(1));
        assert_eq!(
            a.color_en(MARGEN + 1.0, y0 + CASILLA + 1.0),
            Some(COLUMNAS),
            "la segunda fila empieza en el sexto color"
        );
        assert_eq!(
            a.color_en(MARGEN + CASILLA + 3.0, y0 + 1.0),
            None,
            "la calle entre casillas no elige nada"
        );
        assert_eq!(a.color_en(MARGEN - 5.0, y0 + 1.0), None);
        // Y la última casilla existe: con COLUMNAS mal puesto, el décimo color
        // quedaría fuera de la tarjeta y sería imposible de elegir.
        let ultimo = Acento::TODOS.len() - 1;
        let (f, c) = (ultimo / COLUMNAS, ultimo % COLUMNAS);
        assert_eq!(
            a.color_en(
                MARGEN + c as f32 * paso() + 1.0,
                y0 + f as f32 * CASILLA + 1.0
            ),
            Some(ultimo)
        );
        assert!(a.y_rejilla() + CASILLA * filas() as f32 <= a.size().1);
    }

    #[test]
    fn elegir_un_color_lo_pide_y_lo_marca() {
        let mut a = Apariencia::new();
        let y0 = a.y_rejilla();
        let accion = a.pulsar(MARGEN + paso() * 2.0 + 1.0, y0 + 1.0);
        assert_eq!(
            accion,
            Some(Accion::Apariencia {
                tema: tema::actual(),
                acento: Acento::TODOS[2],
            })
        );
        assert_eq!(a.marca_color.actual(), Some(2));
    }

    #[test]
    fn las_dos_muestras_eligen_tema() {
        let mut a = Apariencia::new();
        let y = a.y_muestras() + 4.0;
        assert!(matches!(
            a.pulsar(MARGEN + 4.0, y),
            Some(Accion::Apariencia {
                tema: Tema::Claro,
                ..
            })
        ));
        assert!(matches!(
            a.pulsar(ANCHO - MARGEN - 4.0, y),
            Some(Accion::Apariencia {
                tema: Tema::Oscuro,
                ..
            })
        ));
        // El hueco de en medio no es de ninguna de las dos.
        assert_eq!(a.muestra_en(ANCHO / 2.0, y), None);
    }
}
