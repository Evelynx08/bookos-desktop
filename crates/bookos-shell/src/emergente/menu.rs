//! El menú de BookOS: el que cuelga del logo, arriba a la izquierda.
//!
//! Es la versión propia del plasmoide `bookos-menu`, con sus mismas medidas
//! —210 px de ancho, filas de 26 px, radio 6, hover en acento sólido con el
//! texto en blanco— y sus mismas acciones. Lo que allí ejecuta un
//! `Plasma5Support.DataSource` de tipo "executable", aquí sale como una
//! [`Accion`] que ejecuta el compositor.
//!
//! Las acciones de energía pasan por logind y respetan sus inhibidores: una
//! aplicación que está guardando datos o presentando contenido puede retrasar
//! la transición de forma segura.

use iced_core::{Border, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::tema;
use crate::view::PanelElement;

use super::{Ancla, Tecla};

/// Ancho fijo, como el plasmoide.
const ANCHO: f32 = 236.0;
pub(super) const FILA: f32 = 26.0;
/// Alto del bloque separador, con su línea de 1 px en medio.
pub(super) const DIVISOR: f32 = 7.0;
/// Margen lateral de las filas dentro del menú.
const MARGEN: f32 = 6.0;
/// Márgenes de arriba y de abajo del menú entero.
const BORDE_SUP: f32 = 2.0;
const BORDE_INF: f32 = 4.0;

/// Alto del icono de una fila, y hueco hasta el texto.
///
/// 16 y no los 18 del panel: la fila mide 26 px y un icono de 18 la llena de
/// borde a borde, que es lo que hacía que el menú se leyese como una lista de
/// botones en vez de como un menú.
pub(super) const ICONO: f32 = 16.0;
pub(super) const HUECO_ICONO: f32 = 10.0;

/// Una entrada del menú, o un separador.
enum Entrada {
    Item {
        etiqueta: &'static str,
        atajo: Option<&'static str>,
        /// Nombre del icono propio que la acompaña.
        icono: &'static str,
        accion: fn() -> Accion,
    },
    Divisor,
}

/// El menú, en el mismo orden que el plasmoide.
fn entradas() -> Vec<Entrada> {
    use Entrada::{Divisor, Item};
    vec![
        Item {
            etiqueta: "Acerca de este PC",
            icono: "acerca",
            atajo: None,
            // La ventana es del propio shell, no un programa que lanzar: no
            // hay `bookos-about` en el sistema, y `kinfocenter` —lo que se
            // abría antes— es el panel de otro escritorio y enseña las
            // versiones de Plasma y de Qt en vez de las de BookOS.
            accion: || Accion::Acerca,
        },
        Divisor,
        Item {
            // Antes de «Preferencias del sistema…», que abre otro programa:
            // esta es del propio escritorio y se aplica al pulsarla.
            etiqueta: "Apariencia…",
            icono: "apariencia",
            atajo: None,
            accion: || Accion::Emergente("apariencia"),
        },
        Item {
            etiqueta: "Preferencias del sistema…",
            icono: "preferencias",
            atajo: None,
            accion: || Accion::Lanzar("bookos-settings".into()),
        },
        Item {
            etiqueta: "BookOS Store…",
            icono: "tienda",
            atajo: None,
            accion: || Accion::Lanzar("bookos-store".into()),
        },
        Divisor,
        Item {
            etiqueta: "Dormir",
            icono: "dormir",
            atajo: None,
            accion: || Accion::Lanzar("systemctl suspend".into()),
        },
        Item {
            etiqueta: "Reiniciar…",
            icono: "reiniciar",
            atajo: None,
            accion: || Accion::Lanzar("systemctl reboot".into()),
        },
        Item {
            etiqueta: "Apagar…",
            icono: "apagar",
            atajo: None,
            accion: || Accion::Lanzar("systemctl poweroff".into()),
        },
        Divisor,
        Item {
            etiqueta: "Pantalla de bloqueo",
            icono: "bloquear",
            atajo: Some("Meta+L"),
            accion: || Accion::Bloquear,
        },
        Item {
            etiqueta: "Cerrar sesión…",
            icono: "salir",
            atajo: Some("Ctrl+Alt+Supr"),
            accion: || Accion::CerrarSesion,
        },
    ]
}

pub struct Menu {
    entradas: Vec<Entrada>,
    /// La entrada bajo el puntero —o la elegida con el teclado— y cuánto le
    /// queda a la anterior para apagarse.
    señalada: tema::Realce,
}

impl Menu {
    pub fn new() -> Self {
        Self {
            entradas: entradas(),
            señalada: tema::Realce::nuevo(),
        }
    }

    /// El realce entrando o saliendo. Mientras dure, el compositor repinta.
    pub fn animando(&self) -> bool {
        self.señalada.animando()
    }

    pub fn size(&self) -> (f32, f32) {
        let alto: f32 = self
            .entradas
            .iter()
            .map(|e| match e {
                Entrada::Item { .. } => FILA,
                Entrada::Divisor => DIVISOR,
            })
            .sum();
        (ANCHO, alto + BORDE_SUP + BORDE_INF)
    }

    pub fn ancla(&self) -> Ancla {
        // Pegado al borde izquierdo, bajo el logo del panel.
        Ancla::BajoElPanel { x: 8.0 }
    }

    /// Qué entrada cae en una `y` lógica. Las filas no son todas del mismo alto
    /// —los divisores miden menos— así que se recorre en vez de dividir.
    fn entrada_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x > ANCHO {
            return None;
        }
        let mut cursor = BORDE_SUP;
        for (i, entrada) in self.entradas.iter().enumerate() {
            let alto = match entrada {
                Entrada::Item { .. } => FILA,
                Entrada::Divisor => DIVISOR,
            };
            if y >= cursor && y < cursor + alto {
                // Sobre un divisor no hay nada que señalar.
                return matches!(entrada, Entrada::Item { .. }).then_some(i);
            }
            cursor += alto;
        }
        None
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        self.señalada
            .señalar(punto.and_then(|(x, y)| self.entrada_en(x, y)))
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let i = self.entrada_en(x, y)?;
        match &self.entradas[i] {
            Entrada::Item { accion, .. } => Some(accion()),
            Entrada::Divisor => None,
        }
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Abajo => self.mover(1),
            T::Arriba => self.mover(-1),
            // Sin selección, Intro no hace nada: mejor eso que activar la
            // primera entrada, que aquí sería "Acerca de este PC" y en otro
            // menú podría ser "Apagar".
            T::Intro => Tecla::Ignorada,
            _ => Tecla::Ignorada,
        }
    }

    /// Mueve la selección saltándose los divisores.
    fn mover(&mut self, paso: isize) -> Tecla {
        let n = self.entradas.len();
        let mut i = match self.señalada.actual() {
            Some(i) => i as isize,
            // Al entrar por teclado se empieza por el extremo que toque.
            None if paso > 0 => -1,
            None => n as isize,
        };
        for _ in 0..n {
            i += paso;
            if i < 0 || i >= n as isize {
                return Tecla::Consumida;
            }
            if matches!(self.entradas[i as usize], Entrada::Item { .. }) {
                self.señalada.señalar(Some(i as usize));
                return Tecla::Consumida;
            }
        }
        Tecla::Consumida
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut col = column![Space::new().height(Length::Fixed(BORDE_SUP))];
        for (i, entrada) in self.entradas.iter().enumerate() {
            col = col.push(match entrada {
                Entrada::Item {
                    etiqueta,
                    atajo,
                    icono,
                    ..
                } => fila(etiqueta, *atajo, icono, self.señalada.intensidad(i)),
                Entrada::Divisor => divisor(),
            });
        }
        col = col.push(Space::new().height(Length::Fixed(BORDE_INF)));

        container(col)
            .width(Length::Fixed(ANCHO))
            .style(|_theme| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_POPOVER.into(),
                    width: 1.0,
                    color: tema::borde(),
                },
                ..Default::default()
            })
            .into()
    }
}

/// Una fila del menú. `señalada` va de 0 a 1: es el realce entrando.
///
/// El color se **interpola** en vez de conmutar. Con el hover de acento sólido
/// del plasmoide, encender y apagar de golpe se lee como un parpadeo al pasar
/// el ratón por el menú de arriba abajo; los 120 ms del token de hover lo
/// convierten en un rastro.
pub(super) fn fila(
    etiqueta: &str,
    atajo: Option<&str>,
    icono: &str,
    señalada: f32,
) -> PanelElement<'static> {
    // El hover del plasmoide es acento **sólido** con el texto en blanco, no un
    // gris sutil: es lo que hace que se lea como un menú de sistema. El fondo
    // entra por alfa y no mezclándose con la tarjeta porque debajo hay un
    // popover translúcido: mezclar con `card()` dejaría un rectángulo opaco.
    let fondo = tema::alfa(tema::acento(), señalada);
    let color = tema::mezclar(tema::texto(), tema::sobre_acento(), señalada);

    // El icono se tiñe del color de la fila para que siga al texto al pasar a
    // fondo de acento; si no, quedaría blanco sobre azul con el mismo tono y
    // desaparecería justo en la fila que se está mirando.
    let dibujo: PanelElement<'static> = match crate::icono::propio(icono) {
        Some(ic) => crate::icono::ver_teñido_propio(&ic, ICONO, color),
        // Un icono que falta no puede descolocar el texto de su fila respecto
        // a las demás: se deja el hueco.
        None => Space::new().width(Length::Fixed(ICONO)).into(),
    };
    let mut contenido = row![
        dibujo,
        Space::new().width(Length::Fixed(HUECO_ICONO)),
        text(etiqueta.to_string()).size(tema::T_CUERPO).color(color)
    ]
    .align_y(iced_core::alignment::Vertical::Center);
    contenido = contenido.push(Space::new().width(Length::Fill));
    if let Some(atajo) = atajo {
        let color_atajo = tema::mezclar(tema::TEXTO2, tema::sobre_acento(), señalada);
        contenido = contenido.push(
            text(atajo.to_string())
                .size(tema::T_PEQUENO - 1.0)
                .color(color_atajo),
        );
    }

    container(
        container(contenido)
            .padding([0, 11])
            .height(Length::Fill)
            .center_y(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_CHIP.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
    )
    .padding([0, MARGEN as u16])
    .height(Length::Fixed(FILA))
    .into()
}

pub(super) fn divisor() -> PanelElement<'static> {
    container(
        container(Space::new().height(Length::Fixed(1.0)))
            .width(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(tema::divisor().into()),
                ..Default::default()
            }),
    )
    .padding([3, 11])
    .height(Length::Fixed(DIVISOR))
    .into()
}
