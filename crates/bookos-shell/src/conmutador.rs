//! Los dos conmutadores de BookOS.
//!
//! Toma la forma del de macOS: una fila de iconos centrada en la pantalla, con
//! la elegida realzada y su nombre debajo. Se mantiene mientras el modificador
//! siga pulsado y se resuelve al soltarlo.
//!
//! Los dos enseñan **una celda por ventana**; lo que cambia es qué se dibuja en
//! ella. `Alt+Tab` pone el icono de la aplicación y el título debajo. `Meta+Tab`
//! deja un hueco transparente: el compositor coloca ahí la superficie viva del
//! cliente, escalada por la GPU.
//!
//! El orden es de **uso reciente**: la primera es la que acabas de dejar, así
//! que un Alt+Tab suelto alterna entre las dos últimas, que es el 90 % de los
//! usos. Ese orden lo lleva el compositor, que es quien ve el foco; aquí solo se
//! dibuja lo que llega.
//!
//! El shell **no sabe a qué ventana lleva cada celda**: devuelve el índice de la
//! elegida y el compositor lo traduce. Antes devolvía el `app_id`, y eso era lo
//! que ataba una celda a una aplicación entera.

use iced_core::alignment::Horizontal;
use iced_core::{Border, Color, Length, Rectangle};
use iced_widget::{Column, Row, Space, column, container, row, text};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;

/// Lado del icono de cada aplicación.
///
/// 64 es el tamaño al que los temas traen dibujo de verdad —por debajo se
/// rasteriza un SVG y por encima se interpola— y el mismo que usa el launchpad.
const ICONO: f32 = 64.0;
/// Aire alrededor del icono dentro de su celda.
const AIRE: f32 = 10.0;
/// Lado de la celda, que es lo que se realza al seleccionar.
const CELDA: f32 = ICONO + AIRE * 2.0;
/// Margen de la tarjeta.
const MARGEN: f32 = 14.0;
/// Alto del renglón del nombre, debajo de la fila.
const NOMBRE: f32 = 26.0;

/// Cuántas aplicaciones caben antes de dejar de crecer.
///
/// Doce celdas son 1.008 px, que a 1.280 lógicos ya roza los bordes. Pasado ese
/// número el conmutador dejaría de caber en pantalla, y una fila que se sale es
/// peor que una lista recortada: al menos así se ve que hay más.
pub const MAXIMO: usize = 12;

/// Qué presenta el conmutador y, por tanto, qué significa cada celda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    Aplicaciones,
    Ventanas,
}

const PREVIA_ANCHO_MAX: f32 = 300.0;
const PREVIA_ALTO_MAX: f32 = 250.0;
const PREVIA_HUECO: f32 = 14.0;
const PREVIA_MARGEN_PANTALLA: f32 = 48.0;
/// Ancho de celda por debajo del cual la fila única se parte en varias.
///
/// Meta+Tab enseña ventanas, y una miniatura de menos de 150 px no se distingue
/// de otra: a partir de ahí compensa perder la fila y ganar tamaño.
const PREVIA_UNA_FILA_MIN: f32 = 150.0;
/// Aire entre el borde de la celda y la ventana viva que se compone dentro.
///
/// 8 y no 4: la ventana se dibuja **por delante** de la tarjeta y con esquinas
/// rectas, así que con menos aire tapaba el radio de la celda y el marco se
/// veía cortado en las cuatro esquinas.
const PREVIA_BORDE: f32 = 8.0;

/// Una celda de la fila: una ventana.
pub struct Entrada {
    /// Lo que se enseña debajo cuando está elegida: el título de la ventana.
    pub nombre: String,
    /// El de su aplicación. Dos ventanas del mismo programa repiten icono, y por
    /// eso el que las distingue es el nombre.
    pub icono: Option<Icono>,
}

pub struct Conmutador {
    modo: Modo,
    entradas: Vec<Entrada>,
    elegida: usize,
    /// El recuadro de la elegida, moviéndose de una celda a otra.
    ///
    /// Va aparte de `elegida` porque quien decide qué está elegido es el
    /// teclado y tiene que saberlo al instante —soltar Alt activa esa ventana—
    /// mientras que el recuadro puede ir un fotograma por detrás.
    marca: tema::Realce,
    rejilla: Option<Rejilla>,
}

#[derive(Debug, Clone, Copy)]
struct Rejilla {
    columnas: usize,
    filas: usize,
    celda_ancho: f32,
    celda_alto: f32,
}

impl Conmutador {
    pub fn new(modo: Modo, entradas: Vec<Entrada>, pantalla: (f32, f32)) -> Self {
        // La primera es la que está enfocada. El compositor aplica después el
        // primer paso (+1 o -1), igual que los pasos repetidos; así Mayús no
        // necesita una excepción distinta al abrir.
        let elegida = 0;
        let rejilla = (modo == Modo::Ventanas).then(|| Rejilla::new(entradas.len(), pantalla));
        let mut marca = tema::Realce::nuevo();
        // Ya puesta en la primera al abrir: la tarjeta aparece con su recuadro,
        // no dibujándolo.
        marca.señalar(Some(elegida));
        marca.terminar();
        Self {
            modo,
            entradas,
            elegida,
            marca,
            rejilla,
        }
    }

    pub fn vacio(&self) -> bool {
        self.entradas.is_empty()
    }

    /// El índice de la elegida, que es lo que el compositor traduce a ventana.
    pub fn elegida(&self) -> Option<usize> {
        (self.elegida < self.entradas.len()).then_some(self.elegida)
    }

    /// Avanza la selección. `1` hacia delante, `-1` hacia atrás con Mayús.
    ///
    /// Aquí **sí** se da la vuelta, al contrario que en los escritorios: la fila
    /// se ve entera, así que pasar del último al primero no desorienta —lo
    /// tienes delante— y recorrer ocho aplicaciones sin poder volver por el otro
    /// lado sería absurdo.
    pub fn mover(&mut self, pasos: i32) {
        if self.entradas.is_empty() {
            return;
        }
        let n = self.entradas.len() as i32;
        self.elegida = (((self.elegida as i32 + pasos) % n + n) % n) as usize;
        self.marca.señalar(Some(self.elegida));
    }

    /// ¿Se está moviendo el recuadro? Mientras sí, hay que repintar.
    pub fn animando(&self) -> bool {
        self.marca.animando()
    }

    /// Qué celda cae en un punto, para poder elegir con el ratón.
    pub fn en(&self, x: f32, y: f32) -> Option<usize> {
        match self.rejilla {
            None => {
                if x < MARGEN || y < MARGEN || y >= MARGEN + CELDA {
                    return None;
                }
                let i = ((x - MARGEN) / CELDA) as usize;
                (i < self.entradas.len()).then_some(i)
            }
            Some(r) => r.en(x, y, self.entradas.len()),
        }
    }

    pub fn elegir(&mut self, i: usize) -> bool {
        if i < self.entradas.len() && i != self.elegida {
            self.elegida = i;
            self.marca.señalar(Some(i));
            true
        } else {
            false
        }
    }

    pub fn size(&self) -> (f32, f32) {
        match self.rejilla {
            // La tarjeta de ventanas se mide como la de aplicaciones: celdas,
            // renglón del nombre y margen alrededor.
            Some(r) => {
                let (w, h) = r.size();
                (w + MARGEN * 2.0, h + NOMBRE + MARGEN * 2.0)
            }
            None => {
                let n = self.entradas.len().max(1) as f32;
                (n * CELDA + MARGEN * 2.0, CELDA + NOMBRE + MARGEN * 2.0)
            }
        }
    }

    /// Huecos transparentes en los que el compositor coloca cada ventana.
    pub fn miniaturas(&self) -> Vec<Rectangle> {
        let Some(r) = self.rejilla else {
            return Vec::new();
        };
        (0..self.entradas.len()).map(|i| r.miniatura(i)).collect()
    }

    pub fn view(&self) -> PanelElement<'_> {
        if self.modo == Modo::Ventanas {
            return self.view_ventanas();
        }

        let mut fila = row![];
        for (i, entrada) in self.entradas.iter().enumerate() {
            let elegida = self.marca.intensidad(i);
            let dibujo: PanelElement<'_> = match &entrada.icono {
                // Sin teñir: son iconos de aplicación, y convertir el zorro de
                // Firefox en una silueta blanca no ayuda a reconocerlo.
                Some(ic) => icono::ver(ic, ICONO, ICONO),
                None => Space::new().width(Length::Fixed(ICONO)).into(),
            };
            fila = fila.push(
                container(dibujo)
                    .width(Length::Fixed(CELDA))
                    .height(Length::Fixed(CELDA))
                    .center_x(Length::Fixed(CELDA))
                    .center_y(Length::Fixed(CELDA))
                    .style(move |_theme: &iced_widget::Theme| container::Style {
                        // La elegida lleva un recuadro por detrás, como en
                        // macOS. Nada de un borde fino a secas: sobre iconos de
                        // colores un contorno se pierde y el relleno se ve de
                        // refilón. El relleno es el acento apagado —el azul es
                        // lo que significa «esto está elegido» en el resto del
                        // sistema— y encima va un contorno del acento entero,
                        // que es lo que lo separa de un icono azul cualquiera.
                        // El recuadro no salta de una celda a otra: se apaga en
                        // la que deja y se enciende en la que toma, con el
                        // token de hover. Alt+Tab repetido recorre la fila y el
                        // salto seco se leía como un parpadeo por icono.
                        background: Some(tema::alfa(tema::acento(), 0.22 * elegida).into()),
                        border: Border {
                            radius: tema::R_CONTROL.into(),
                            width: 1.5 * elegida,
                            color: tema::alfa(tema::acento(), elegida),
                        },
                        ..Default::default()
                    }),
            );
        }

        let nombre = self
            .entradas
            .get(self.elegida)
            .map(|e| e.nombre.clone())
            .unwrap_or_default();
        let contenido = column![
            fila,
            container(text(nombre).size(tema::T_CUERPO).color(tema::texto()))
                .width(Length::Fill)
                .height(Length::Fixed(NOMBRE))
                .align_x(Horizontal::Center)
                .center_y(Length::Fixed(NOMBRE)),
        ]
        .align_x(Horizontal::Center);

        container(contenido)
            .padding(MARGEN)
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(
                    Color {
                        a: 0.86,
                        ..tema::card()
                    }
                    .into(),
                ),
                border: Border {
                    radius: tema::R_TARJETA.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// Meta+Tab: la misma tarjeta que Alt+Tab, con miniaturas en vez de iconos.
    ///
    /// El nombre va **una sola vez** debajo de la fila, como en el conmutador de
    /// macOS y como en el modo de aplicaciones de aquí al lado. Antes cada celda
    /// llevaba el suyo: con cuatro terminales abiertas eran cuatro títulos
    /// largos compitiendo, y el que importa es el de la que vas a elegir.
    fn view_ventanas(&self) -> PanelElement<'_> {
        let r = self
            .rejilla
            .expect("el modo ventanas siempre tiene rejilla");
        let mut rejilla = Column::new().spacing(PREVIA_HUECO);
        for fila_i in 0..r.filas {
            let mut fila = Row::new().spacing(PREVIA_HUECO);
            for columna in 0..r.columnas {
                let i = fila_i * r.columnas + columna;
                if self.entradas.get(i).is_some() {
                    let elegida = self.marca.intensidad(i);
                    // La celda va vacía a propósito: lo que se ve dentro es la
                    // ventana del cliente, que el compositor compone encima. Un
                    // icono aquí quedaría medio tapado por ella —se probó, se ve
                    // asomar una esquina— y por eso el icono va con el nombre.
                    fila = fila.push(
                        container(Space::new())
                            .width(Length::Fixed(r.celda_ancho))
                            .height(Length::Fixed(r.celda_alto))
                            .style(move |_theme: &iced_widget::Theme| container::Style {
                                // La celda tiene fondo: la ventana viva se compone
                                // **encima**, así que el relleno solo se ve en el
                                // marco, y sin él la celda era un marco flotando.
                                background: Some(
                                    tema::mezclar(
                                        tema::alfa(tema::tinta(), 0.10),
                                        tema::alfa(tema::acento(), 0.22),
                                        elegida,
                                    )
                                    .into(),
                                ),
                                border: Border {
                                    radius: tema::R_CONTROL.into(),
                                    // El marco de la elegida es más grueso: crece
                                    // con ella en vez de aparecer de golpe.
                                    width: 1.0 + 2.0 * elegida,
                                    color: tema::mezclar(tema::borde(), tema::acento(), elegida),
                                },
                                ..Default::default()
                            }),
                    );
                } else {
                    fila = fila.push(
                        Space::new()
                            .width(Length::Fixed(r.celda_ancho))
                            .height(Length::Fixed(r.celda_alto)),
                    );
                }
            }
            rejilla = rejilla.push(fila);
        }

        // El rótulo de la elegida: su icono y su título. El icono aquí y no en
        // la celda porque dos ventanas de la misma terminal son idénticas a
        // este tamaño, y hace falta ver de qué programa es la que vas a sacar.
        let elegida = self.entradas.get(self.elegida);
        let mut rotulo = Row::new().spacing(8).align_y(iced_core::Alignment::Center);
        if let Some(ic) = elegida.and_then(|e| e.icono.as_ref()) {
            rotulo = rotulo.push(icono::ver(ic, 18.0, 18.0));
        }
        rotulo = rotulo.push(
            text(elegida.map(|e| e.nombre.clone()).unwrap_or_default())
                .size(tema::T_CUERPO)
                .color(tema::texto()),
        );
        let contenido = column![
            rejilla,
            container(rotulo)
                .width(Length::Fill)
                .height(Length::Fixed(NOMBRE))
                .center_x(Length::Fill)
                .center_y(Length::Fixed(NOMBRE)),
        ]
        .align_x(Horizontal::Center);

        container(contenido)
            .padding(MARGEN)
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(
                    Color {
                        a: 0.86,
                        ..tema::card()
                    }
                    .into(),
                ),
                border: Border {
                    radius: tema::R_TARJETA.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }
}

impl Rejilla {
    /// Una fila mientras las miniaturas salgan de un tamaño reconocible, y
    /// varias solo cuando no queda sitio.
    ///
    /// La fila única es la forma del conmutador de macOS y es la que se lee de
    /// un vistazo: el recorrido con Tab es horizontal, y una rejilla obliga a
    /// buscar dónde saltó la selección al cambiar de renglón. Con doce ventanas
    /// en una pantalla pequeña deja de caber, y entonces sí se parte.
    fn new(n: usize, pantalla: (f32, f32)) -> Self {
        let n = n.max(1);
        let ancho = (pantalla.0 - (PREVIA_MARGEN_PANTALLA + MARGEN) * 2.0).max(160.0);
        let alto = (pantalla.1 - (PREVIA_MARGEN_PANTALLA + MARGEN) * 2.0 - NOMBRE).max(120.0);
        // Las miniaturas llevan la proporción de la pantalla: lo que se compone
        // dentro es una ventana de ese escritorio, y otra proporción la dejaría
        // con franjas a los lados.
        let proporcion = (pantalla.1 / pantalla.0.max(1.0)).clamp(0.4, 1.4);

        let de_una_fila =
            ((ancho - PREVIA_HUECO * (n - 1) as f32) / n as f32).min(PREVIA_ANCHO_MAX);
        let (columnas, filas) = if de_una_fila >= PREVIA_UNA_FILA_MIN {
            (n, 1)
        } else {
            let aspecto = (ancho / alto).clamp(0.5, 3.0);
            let columnas = ((n as f32 * aspecto).sqrt().ceil() as usize).clamp(1, n);
            (columnas, n.div_ceil(columnas))
        };

        let celda_ancho = ((ancho - PREVIA_HUECO * (columnas.saturating_sub(1)) as f32)
            / columnas as f32)
            .min(PREVIA_ANCHO_MAX)
            .max(36.0);
        let alto_por_proporcion = celda_ancho * proporcion;
        let cabe_de_alto = (alto - PREVIA_HUECO * (filas.saturating_sub(1)) as f32) / filas as f32;
        let celda_alto = alto_por_proporcion
            .min(cabe_de_alto)
            .min(PREVIA_ALTO_MAX)
            .max(28.0);
        // Y si el alto es el que manda, el ancho se recorta con él para que la
        // celda no acabe siendo un rectángulo apaisado con la ventana flotando.
        let celda_ancho = celda_ancho.min(celda_alto / proporcion.max(0.01));
        Self {
            columnas,
            filas,
            celda_ancho,
            celda_alto,
        }
    }

    /// Lo que ocupan las celdas, sin el margen de la tarjeta ni el rótulo.
    fn size(self) -> (f32, f32) {
        (
            self.columnas as f32 * self.celda_ancho
                + self.columnas.saturating_sub(1) as f32 * PREVIA_HUECO,
            self.filas as f32 * self.celda_alto
                + self.filas.saturating_sub(1) as f32 * PREVIA_HUECO,
        )
    }

    fn miniatura(self, i: usize) -> Rectangle {
        let columna = i % self.columnas;
        let fila = i / self.columnas;
        Rectangle {
            x: MARGEN + columna as f32 * (self.celda_ancho + PREVIA_HUECO) + PREVIA_BORDE,
            y: MARGEN + fila as f32 * (self.celda_alto + PREVIA_HUECO) + PREVIA_BORDE,
            width: (self.celda_ancho - PREVIA_BORDE * 2.0).max(1.0),
            height: (self.celda_alto - PREVIA_BORDE * 2.0).max(1.0),
        }
    }

    fn en(self, x: f32, y: f32, n: usize) -> Option<usize> {
        let (x, y) = (x - MARGEN, y - MARGEN);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let paso_x = self.celda_ancho + PREVIA_HUECO;
        let paso_y = self.celda_alto + PREVIA_HUECO;
        let columna = (x / paso_x) as usize;
        let fila = (y / paso_y) as usize;
        if columna >= self.columnas || fila >= self.filas {
            return None;
        }
        let dentro_x = x - columna as f32 * paso_x < self.celda_ancho;
        let dentro_y = y - fila as f32 * paso_y < self.celda_alto;
        let i = fila * self.columnas + columna;
        (dentro_x && dentro_y && i < n).then_some(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entradas(n: usize) -> Vec<Entrada> {
        (0..n)
            .map(|i| Entrada {
                nombre: format!("Ventana {i}"),
                icono: None,
            })
            .collect()
    }

    /// Con varias ventanas, el conmutador arranca en la **segunda**: quien pulsa
    /// Alt+Tab quiere la de antes, no la que ya tiene delante.
    #[test]
    fn arranca_en_la_actual_y_el_primer_paso_va_a_la_anterior() {
        let c = Conmutador::new(Modo::Aplicaciones, entradas(3), (1280.0, 720.0));
        assert_eq!(c.elegida(), Some(0));
        let mut c = c;
        c.mover(1);
        assert_eq!(c.elegida(), Some(1));
    }

    /// Con una sola no hay a dónde ir.
    #[test]
    fn con_una_sola_se_queda_donde_esta() {
        let c = Conmutador::new(Modo::Aplicaciones, entradas(1), (1280.0, 720.0));
        assert_eq!(c.elegida(), Some(0));
    }

    /// La fila **sí** da la vuelta, al revés que los escritorios: se ve entera,
    /// así que pasar del último al primero no desorienta.
    #[test]
    fn la_seleccion_da_la_vuelta() {
        let mut c = Conmutador::new(Modo::Aplicaciones, entradas(3), (1280.0, 720.0));
        c.mover(1);
        assert_eq!(c.elegida(), Some(1));
        c.mover(1);
        assert_eq!(c.elegida(), Some(2));
        c.mover(1);
        assert_eq!(c.elegida(), Some(0), "del último al primero");
        c.mover(-1);
        assert_eq!(c.elegida(), Some(2), "y hacia atrás con Mayús");
    }

    /// El ancho crece con las aplicaciones: es lo que usa el compositor para
    /// centrar la tarjeta, así que si miente, sale descentrada.
    #[test]
    fn el_ancho_cuenta_las_celdas() {
        assert_eq!(
            Conmutador::new(Modo::Aplicaciones, entradas(3), (1280.0, 720.0))
                .size()
                .0,
            3.0 * CELDA + MARGEN * 2.0
        );
    }

    /// Elegir con el ratón acierta la celda de debajo del cursor.
    #[test]
    fn el_raton_elige_su_celda() {
        let c = Conmutador::new(Modo::Aplicaciones, entradas(4), (1280.0, 720.0));
        assert_eq!(c.en(MARGEN + 1.0, 40.0), Some(0));
        assert_eq!(c.en(MARGEN + CELDA * 2.5, 40.0), Some(2));
        assert_eq!(c.en(MARGEN + CELDA * 9.0, 40.0), None, "fuera de la fila");
    }

    #[test]
    fn la_rejilla_da_un_hueco_y_hit_test_por_ventana() {
        let c = Conmutador::new(Modo::Ventanas, entradas(5), (1280.0, 720.0));
        let miniaturas = c.miniaturas();
        assert_eq!(miniaturas.len(), 5);
        let primera = miniaturas[0];
        assert_eq!(
            c.en(
                primera.x + primera.width / 2.0,
                primera.y + primera.height / 2.0
            ),
            Some(0)
        );
        let (w, h) = c.size();
        assert!(w <= 1280.0 && h <= 720.0);
    }

    #[test]
    fn muchas_ventanas_siguen_dentro_de_la_pantalla() {
        let c = Conmutador::new(Modo::Ventanas, entradas(48), (1280.0, 720.0));
        assert_eq!(c.miniaturas().len(), 48);
        let (w, h) = c.size();
        assert!(w <= 1280.0 - PREVIA_MARGEN_PANTALLA * 2.0 + 0.1);
        assert!(h <= 720.0 - PREVIA_MARGEN_PANTALLA * 2.0 + 0.1);
    }
}
