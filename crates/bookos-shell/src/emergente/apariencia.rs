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
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::tema::{self, Acento, ModoTema, Realce};
use crate::view::PanelElement;

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
/// Alto de una miniatura de fondo, sin contar el aire de debajo.
///
/// El ancho sale de repartir la fila entre las familias que haya; el alto va en
/// proporción 16:10, que es la de los fondos que trae BookOS.
const FONDO_ALTO: f32 = 44.0;
/// Hueco entre dos miniaturas de fondo.
const FONDO_HUECO: f32 = 9.0;
/// Lo que se separa el anillo de elegida de la imagen. Sin este aire el anillo
/// cae encima de la foto y con el acento azul sobre la miniatura azul no se ve.
const HUECO_ANILLO: f32 = 4.0;

/// Los tres modos, en el orden en que se enseñan.
const MODOS: [ModoTema; 3] = [ModoTema::Claro, ModoTema::Oscuro, ModoTema::Automatico];

/// Ancho de una muestra de tema. Tres, con un hueco entre cada dos.
fn ancho_muestra() -> f32 {
    (ANCHO - MARGEN * 2.0 - MUESTRA_HUECO * (MODOS.len() - 1) as f32) / MODOS.len() as f32
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
    ///
    /// Es el **modo**, no el tema pintado: con «Automático» puesto, `actual()`
    /// dice claro u oscuro según la hora y marcar esa muestra convertiría el
    /// automático en fijo con solo abrir la tarjeta.
    modo: ModoTema,
    acento: Acento,
    /// Lo señalado por el puntero: las dos muestras de tema y la rejilla.
    hover_tema: Realce,
    hover_color: Realce,
    /// Lo elegido, animado: el anillo crece en el nuevo y se apaga en el
    /// anterior en vez de saltar de un sitio a otro.
    marca_tema: Realce,
    marca_color: Realce,
    /// Las parejas de fondos instaladas, leídas **al abrir** y no en cada
    /// `view`: recorrer tres directorios sesenta veces por segundo para una
    /// lista que no cambia mientras la tarjeta está abierta es trabajo por nada.
    fondos: Vec<crate::fondos::Familia>,
    hover_fondo: Realce,
    marca_fondo: Realce,
}

impl Apariencia {
    /// `modo` es lo que dice la configuración, que la tarjeta no puede deducir:
    /// `tema::actual()` solo sabe qué se está pintando.
    pub fn new(modo: ModoTema) -> Self {
        let acento = tema::acento_actual();
        let mut marca_tema = Realce::nuevo();
        let mut marca_color = Realce::nuevo();
        // Al abrirse ya está elegido: sin esto la tarjeta aparece con todo
        // apagado y las marcas entrando, como si nadie hubiera elegido nada.
        marca_tema.señalar(MODOS.iter().position(|m| *m == modo));
        marca_color.señalar(indice(acento));
        let mut fondos = crate::fondos::instaladas(tema::es_claro());
        ordenar_por_afinidad(&mut fondos, acento);
        // Cuál marcar. Se compara por **nombre de familia** y no por ruta: con
        // el tema oscuro puesto, el fichero cargado es el `_dark` de la pareja,
        // y comparar rutas no encontraría la miniatura de su propia familia.
        let puesta = crate::fondos::elegida();
        let elegido = puesta
            .as_deref()
            .and_then(|nombre| fondos.iter().position(|f| f.nombre == nombre));
        let mut marca_fondo = Realce::nuevo();
        marca_fondo.señalar(elegido);
        marca_tema.terminar();
        marca_color.terminar();
        marca_fondo.terminar();
        Self {
            modo,
            acento,
            hover_tema: Realce::nuevo(),
            hover_color: Realce::nuevo(),
            marca_tema,
            marca_color,
            fondos,
            hover_fondo: Realce::nuevo(),
            marca_fondo,
        }
    }

    /// Ancho de una miniatura: la fila se reparte entre las familias que haya.
    fn ancho_fondo(&self) -> f32 {
        let n = self.fondos.len().max(1) as f32;
        (ANCHO - MARGEN * 2.0 - FONDO_HUECO * (n - 1.0)) / n
    }

    pub fn size(&self) -> (f32, f32) {
        let mut alto =
            CABECERA + MUESTRA + SEPARACION + ROTULO + CASILLA * filas() as f32 + MARGEN * 2.0;
        // Sin fondos instalados la sección no se dibuja: un rótulo con una fila
        // vacía debajo no es «no hay ninguno», es «esto está roto».
        if !self.fondos.is_empty() {
            alto += SEPARACION + ROTULO + FONDO_ALTO;
        }
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
            || self.hover_fondo.animando()
            || self.marca_fondo.animando()
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
        let paso = ancho_muestra() + MUESTRA_HUECO;
        let i = (rel / paso) as usize;
        // El hueco entre dos muestras no es de nadie: pulsar en la calle no
        // puede elegir la de la izquierda.
        if rel - i as f32 * paso > ancho_muestra() {
            return None;
        }
        (i < MODOS.len()).then_some(i)
    }

    /// `y` donde empieza la fila de miniaturas de fondo.
    fn y_fondos(&self) -> f32 {
        self.y_rejilla() + CASILLA * filas() as f32 + SEPARACION + ROTULO
    }

    fn fondo_en(&self, x: f32, y: f32) -> Option<usize> {
        if self.fondos.is_empty() {
            return None;
        }
        let y0 = self.y_fondos();
        if y < y0 || y > y0 + FONDO_ALTO || x < MARGEN || x > ANCHO - MARGEN {
            return None;
        }
        let paso = self.ancho_fondo() + FONDO_HUECO;
        let rel = x - MARGEN;
        let i = (rel / paso) as usize;
        // La calle entre dos miniaturas no es de nadie.
        if rel - i as f32 * paso > self.ancho_fondo() {
            return None;
        }
        (i < self.fondos.len()).then_some(i)
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
        let (m, c, f) = match punto {
            Some((x, y)) => (
                self.muestra_en(x, y),
                self.color_en(x, y),
                self.fondo_en(x, y),
            ),
            None => (None, None, None),
        };
        // Los tres `señalar` **siempre**, sin cortocircuito: con `||` los de
        // después no se evalúan cuando el primero cambia y la rejilla se queda
        // con un color encendido al salir el ratón por arriba.
        let a = self.hover_tema.señalar(m);
        let b = self.hover_color.señalar(c);
        let d = self.hover_fondo.señalar(f);
        a || b || d
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(i) = self.muestra_en(x, y) {
            self.modo = MODOS[i];
            self.marca_tema.señalar(Some(i));
            return Some(self.accion());
        }
        if let Some(i) = self.fondo_en(x, y) {
            self.marca_fondo.señalar(Some(i));
            let familia = &self.fondos[i];
            // El fondo va en su propia acción y no en `Apariencia`: son dos
            // claves distintas del fichero y elegir un fondo no toca el tema.
            return Some(Accion::Fondo {
                claro: familia.claro.clone(),
                oscuro: familia.oscuro.clone(),
            });
        }
        let i = self.color_en(x, y)?;
        self.acento = Acento::TODOS[i];
        self.marca_color.señalar(Some(i));
        Some(self.accion())
    }

    fn accion(&self) -> Accion {
        Accion::Apariencia {
            modo: self.modo,
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
            MODOS.iter().enumerate().fold(row![], |fila, (i, modo)| {
                let fila = if i > 0 {
                    fila.push(Space::new().width(Length::Fixed(MUESTRA_HUECO)))
                } else {
                    fila
                };
                fila.push(muestra(
                    *modo,
                    self.marca_tema.intensidad(i),
                    self.hover_tema.intensidad(i),
                ))
            }),
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

        // La sección de fondos solo si hay alguno: un rótulo con una fila vacía
        // debajo no dice «no hay ninguno», dice «esto está roto».
        let contenido = if self.fondos.is_empty() {
            contenido
        } else {
            let fila = self.fondos.iter().enumerate().fold(row![], |fila, (i, f)| {
                let fila = if i > 0 {
                    fila.push(Space::new().width(Length::Fixed(FONDO_HUECO)))
                } else {
                    fila
                };
                fila.push(miniatura(
                    f,
                    self.ancho_fondo(),
                    self.marca_fondo.intensidad(i),
                    self.hover_fondo.intensidad(i),
                ))
            });
            contenido
                .push(separador())
                .push(
                    container(text("Fondo").size(tema::T_CUERPO).color(tema::TEXTO2))
                        .height(Length::Fixed(ROTULO))
                        .center_y(Length::Fixed(ROTULO)),
                )
                .push(fila)
        };

        super::control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

/// Pone delante el fondo que acompaña al acento elegido.
///
/// `fondos::instaladas` los devuelve por nombre —blue, ember, pine, purple— y
/// alfabético no le dice nada al ojo: con el acento en verde, el fondo que le
/// va queda el tercero de cuatro. Ordenando por cercanía de tono, la primera
/// miniatura es siempre la pareja natural.
///
/// **Ordena, no elige.** El fondo se sigue cambiando a mano y elegir un acento
/// no lo toca: son dos acciones distintas a propósito, y mover el fondo por
/// detrás pisaría el que el usuario hubiera puesto. Esto solo pone el candidato
/// donde se mira primero.
///
/// Grafito no tiene tono —es gris— y con él se deja el orden alfabético: no hay
/// ningún fondo que le pegue más que otro. Lo mismo con las familias sin SVG,
/// que van al final porque de ellas no se sabe el color sin decodificar el PNG.
fn ordenar_por_afinidad(fondos: &mut [crate::fondos::Familia], acento: Acento) {
    let Some(objetivo) = tono_de_acento(acento) else {
        return;
    };
    fondos.sort_by(|a, b| {
        let d = |f: &crate::fondos::Familia| {
            crate::fondos::tono(f)
                .map(|t| crate::fondos::distancia_de_tono(t, objetivo))
                // Sin vectorial no se sabe el tono: detrás de cualquiera que sí.
                .unwrap_or(f32::MAX)
        };
        d(a).total_cmp(&d(b)).then_with(|| a.nombre.cmp(&b.nombre))
    });
}

/// El tono del acento, o `None` si es gris y no tiene.
///
/// El umbral es de croma y no de cero exacto: el Grafito oscuro es `#a1a1a6`,
/// con cinco niveles de diferencia entre canales, y la cuenta le sacaría un
/// 240° azul perfectamente formal que dejaría el fondo azul el primero por un
/// gris. Por debajo de 0,08 no hay tono que signifique nada.
fn tono_de_acento(acento: Acento) -> Option<f32> {
    let c = acento.color();
    let (max, min) = (c.r.max(c.g).max(c.b), c.r.min(c.g).min(c.b));
    let croma = max - min;
    if croma < 0.08 {
        return None;
    }
    let h = if max == c.r {
        60.0 * (((c.g - c.b) / croma) % 6.0)
    } else if max == c.g {
        60.0 * ((c.b - c.r) / croma + 2.0)
    } else {
        60.0 * ((c.r - c.g) / croma + 4.0)
    };
    Some((h + 360.0) % 360.0)
}

fn indice(acento: Acento) -> Option<usize> {
    Acento::TODOS.iter().position(|a| *a == acento)
}

/// Los dos colores de un tema, escritos aquí a mano.
///
/// Son los mismos `bg` y `card` de la tabla de tokens, pero no se le pueden
/// preguntar a `tema::bg()`: la muestra enseña el tema que **no** está puesto.
fn colores(claro: bool) -> (Color, Color) {
    if claro {
        (Color::from_rgb(0.949, 0.949, 0.969), Color::WHITE)
    } else {
        (Color::BLACK, Color::from_rgb(0.110, 0.110, 0.118))
    }
}

/// Un trozo de escritorio en miniatura: la barra del panel arriba y una tarjeta
/// debajo, que es lo que distingue de un vistazo un tema del otro.
fn mini<'a>(
    claro: bool,
    ancho: f32,
    alto: f32,
    radio: iced_core::border::Radius,
) -> PanelElement<'a> {
    let (fondo, tarjeta) = colores(claro);
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
    // El radio lo lleva **este** contenedor porque es el que pinta el fondo: el
    // `clip` del lienzo de fuera recorta en rectángulo, no por la curva, así
    // que si el redondeo se dejara allí las muestras saldrían con las esquinas
    // en pico. En la muestra partida cada mitad redondea solo su lado, que es
    // lo que hace que las dos juntas se lean como una sola pastilla.
    container(dibujo)
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(alto))
        .clip(true)
        .style(move |_| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: radio,
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// La muestra de un modo.
///
/// «Automático» se dibuja partido: la mitad clara a la izquierda y la oscura a
/// la derecha. Es lo mismo que enseña macOS, y dice lo que hace sin necesitar
/// el rótulo. El corte es vertical y no en diagonal porque una diagonal pide un
/// lienzo con su propio dibujado y aquí basta con dos contenedores.
fn muestra<'a>(cual: ModoTema, elegida: f32, señalada: f32) -> PanelElement<'a> {
    let ancho = ancho_muestra();
    let alto_lienzo = MUESTRA - 22.0;

    use iced_core::border::Radius;
    let entero = Radius::from(tema::R_BOTON);
    let izquierda = Radius::default()
        .top_left(tema::R_BOTON)
        .bottom_left(tema::R_BOTON);
    let derecha = Radius::default()
        .top_right(tema::R_BOTON)
        .bottom_right(tema::R_BOTON);

    let dentro: PanelElement<'a> = match cual {
        ModoTema::Claro => mini(true, ancho, alto_lienzo, entero),
        ModoTema::Oscuro => mini(false, ancho, alto_lienzo, entero),
        // Cada mitad lleva su propio fondo, así que el lienzo de fuera no pinta
        // ninguno: si lo hiciera, asomaría por la junta al redondear.
        ModoTema::Automatico => row![
            mini(true, ancho / 2.0, alto_lienzo, izquierda),
            mini(false, ancho / 2.0, alto_lienzo, derecha),
        ]
        .into(),
    };

    let lienzo = container(dentro)
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(alto_lienzo))
        .clip(true)
        .style(move |_| container::Style {
            border: Border {
                radius: tema::R_BOTON.into(),
                // El marco de elegido crece con la animación en vez de
                // aparecer: 2 px que salen de golpe se leen como un parpadeo.
                width: 2.0 * elegida,
                color: tema::alfa(tema::acento(), elegida),
            },
            ..Default::default()
        });

    let rotulo = text(match cual {
        ModoTema::Claro => "Claro",
        ModoTema::Oscuro => "Oscuro",
        ModoTema::Automatico => "Automático",
    })
    .size(tema::T_PEQUENO)
    // El rótulo es de la tarjeta, no de la muestra: va con la tinta del tema
    // que está puesto. Señalado sube de secundario a normal; elegido, al acento.
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

/// La miniatura de una familia de fondos.
///
/// Se dibuja el **SVG** y no la foto: decodificar los cuatro PNG de 2880×1800
/// cuesta 152 ms medidos, y eso sería lo que tardaría la tarjeta en abrirse.
/// Los SVG de al lado son de cuatro kilobytes y los rasteriza resvg al tamaño
/// que se le pida, por el mismo camino que los iconos del dock. Una familia sin
/// vectorial se queda con un rectángulo liso: se puede elegir igual, que es lo
/// que importa.
fn miniatura<'a>(
    familia: &crate::fondos::Familia,
    ancho: f32,
    elegida: f32,
    señalada: f32,
) -> PanelElement<'a> {
    // La imagen va **dentro** de un margen y el anillo por fuera, igual que los
    // círculos de acento. Pegado a la imagen no se vería: el anillo es del color
    // de acento y con el acento azul sobre la miniatura azul son el mismo color.
    let (iw, ih) = (ancho - HUECO_ANILLO * 2.0, FONDO_ALTO - HUECO_ANILLO * 2.0);
    let dentro: PanelElement<'a> = match familia.vista.as_ref() {
        Some(ruta) => iced_widget::svg(iced_core::svg::Handle::from_path(ruta))
            .width(Length::Fixed(iw))
            .height(Length::Fixed(ih))
            .content_fit(iced_core::ContentFit::Cover)
            .into(),
        None => Space::new()
            .width(Length::Fixed(iw))
            .height(Length::Fixed(ih))
            .into(),
    };
    let imagen = container(dentro)
        .width(Length::Fixed(iw))
        .height(Length::Fixed(ih))
        .clip(true)
        .style(move |_| container::Style {
            // El liso de debajo se ve solo cuando no hay SVG; con él encima
            // queda tapado y no cuesta nada.
            background: Some(tema::hover().into()),
            border: Border {
                radius: tema::R_BOTON.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    container(imagen)
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(FONDO_ALTO))
        .center_x(Length::Fixed(ancho))
        .center_y(Length::Fixed(FONDO_ALTO))
        .style(move |_| container::Style {
            border: Border {
                radius: (tema::R_BOTON + HUECO_ANILLO).into(),
                // El anillo de elegida crece con la animación, igual que en las
                // muestras de tema; el de señalada es más flojo y no es acento,
                // que se reserva para lo que está puesto.
                width: 2.0 * elegida.max(señalada),
                color: tema::mezclar(
                    tema::alfa(tema::texto(), 0.35 * señalada),
                    tema::acento(),
                    elegida,
                ),
            },
            ..Default::default()
        })
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
        let a = Apariencia::new(ModoTema::Oscuro);
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
        let mut a = Apariencia::new(ModoTema::Oscuro);
        let y0 = a.y_rejilla();
        let accion = a.pulsar(MARGEN + paso() * 2.0 + 1.0, y0 + 1.0);
        assert_eq!(
            accion,
            Some(Accion::Apariencia {
                modo: ModoTema::Oscuro,
                acento: Acento::TODOS[2],
            })
        );
        assert_eq!(a.marca_color.actual(), Some(2));
    }

    /// El fondo que acompaña al acento sale el primero, y el gris no reordena.
    ///
    /// Se comprueba con los tonos, no con los nombres: si mañana entra un
    /// fondo nuevo, el test sigue diciendo lo que importa —que el primero es
    /// el más cercano— en vez de clavar una lista que habría que actualizar.
    #[test]
    fn el_fondo_afin_al_acento_sale_el_primero() {
        let mut fondos = crate::fondos::instaladas(false);
        if fondos.len() < 2 {
            return;
        }
        for acento in [Acento::Verde, Acento::Rojo, Acento::Morado, Acento::Azul] {
            let mut orden = fondos.clone();
            ordenar_por_afinidad(&mut orden, acento);
            let objetivo = tono_de_acento(acento).expect("estos cuatro tienen tono");
            let d = |f: &crate::fondos::Familia| {
                crate::fondos::tono(f).map(|t| crate::fondos::distancia_de_tono(t, objetivo))
            };
            let primera = d(&orden[0]).expect("los fondos de BookOS traen SVG");
            for otra in &orden[1..] {
                if let Some(x) = d(otra) {
                    assert!(
                        primera <= x,
                        "con {acento:?} sale {} ({primera:.0}°) por delante de {} ({x:.0}°)",
                        orden[0].nombre,
                        otra.nombre
                    );
                }
            }
        }

        // Grafito es gris: no hay fondo que le pegue más que otro, así que se
        // queda el orden alfabético que trae `instaladas`.
        let alfabetico: Vec<String> = fondos.iter().map(|f| f.nombre.clone()).collect();
        ordenar_por_afinidad(&mut fondos, Acento::Grafito);
        let despues: Vec<String> = fondos.iter().map(|f| f.nombre.clone()).collect();
        assert_eq!(alfabetico, despues, "el gris no debería reordenar nada");
    }

    #[test]
    fn la_fila_de_fondos_elige_familia() {
        let mut a = Apariencia::new(ModoTema::Oscuro);
        if a.fondos.is_empty() {
            assert_eq!(a.fondo_en(MARGEN + 1.0, a.y_fondos() + 1.0), None);
            return;
        }
        let y = a.y_fondos() + 1.0;
        let paso = a.ancho_fondo() + FONDO_HUECO;
        for i in 0..a.fondos.len() {
            let x = MARGEN + i as f32 * paso + 1.0;
            assert_eq!(a.fondo_en(x, y), Some(i), "la miniatura {i} no responde");
            // Pulsar pide la pareja **entera**, que es lo que permite que el
            // cambio de tema se lleve el fondo con él.
            let esperada = a.fondos[i].clone();
            assert_eq!(
                a.pulsar(x, y),
                Some(Accion::Fondo {
                    claro: esperada.claro,
                    oscuro: esperada.oscuro,
                })
            );
            assert_eq!(a.marca_fondo.actual(), Some(i));
        }
        // La calle entre dos no es de nadie, y por debajo de la fila tampoco.
        if a.fondos.len() > 1 {
            let calle = MARGEN + a.ancho_fondo() + FONDO_HUECO / 2.0;
            assert_eq!(a.fondo_en(calle, y), None);
        }
        assert_eq!(
            a.fondo_en(MARGEN + 1.0, a.y_fondos() + FONDO_ALTO + 5.0),
            None
        );
        // Y la fila entra en la tarjeta: con el alto mal contado, la última
        // quedaría cortada por abajo.
        assert!(a.y_fondos() + FONDO_ALTO <= a.size().1 - MARGEN + 0.01);
    }

    /// Las tres muestras eligen su modo, cada una en su sitio, y las calles
    /// que las separan no eligen nada.
    #[test]
    fn las_tres_muestras_eligen_modo() {
        let mut a = Apariencia::new(ModoTema::Oscuro);
        let y = a.y_muestras() + 4.0;
        let paso = ancho_muestra() + MUESTRA_HUECO;
        for (i, esperado) in MODOS.iter().enumerate() {
            let x = MARGEN + i as f32 * paso + 4.0;
            assert_eq!(
                a.pulsar(x, y),
                Some(Accion::Apariencia {
                    modo: *esperado,
                    acento: tema::acento_actual(),
                }),
                "la muestra {i} no eligió {esperado:?}"
            );
            // Y la calle que hay a su derecha no es de nadie: pulsar entre dos
            // muestras no puede elegir la de la izquierda.
            if i + 1 < MODOS.len() {
                let calle = MARGEN + i as f32 * paso + ancho_muestra() + MUESTRA_HUECO / 2.0;
                assert_eq!(a.muestra_en(calle, y), None, "la calle {i} eligió algo");
            }
        }
        // Y la última muestra llega hasta el borde del margen, sin sobrar ni
        // faltar: con el ancho mal repartido, la de la derecha quedaría cortada.
        assert_eq!(a.muestra_en(ANCHO - MARGEN - 1.0, y), Some(MODOS.len() - 1));
    }
}
