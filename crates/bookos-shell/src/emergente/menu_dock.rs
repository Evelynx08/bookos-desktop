//! El menú contextual de un icono del dock: el del clic derecho.
//!
//! Es la versión corta del menú del gestor de tareas de Plasma. De sus siete
//! entradas se quedan las tres que hacen algo aquí —abrir otra ventana, fijar o
//! soltar, cerrar—; las demás configuran el propio applet, que en este dock no
//! existe como applet configurable.
//!
//! Comparte las filas con el menú del logo: son el mismo control y el mismo
//! hover en acento sólido, y duplicarlos habría dejado dos menús que se parecen
//! pero no se comportan igual.

use iced_core::{Border, Length};
use iced_widget::{Space, column, container, text};

use crate::Accion;
use crate::tema;
use crate::view::PanelElement;

use super::menu::{DIVISOR, FILA, HUECO_ICONO, ICONO, MARGEN as MARGEN_FILA, divisor, fila};
use super::{Ancla, Tecla};

/// Más ancho que el del logo: "Quitar del dock" con su icono no cabía en 210.
const ANCHO: f32 = 230.0;
const BORDE_SUP: f32 = 4.0;
const BORDE_INF: f32 = 4.0;

/// Lo que sabe el menú del icono sobre el que se abrió.
pub struct Objetivo {
    pub app_id: String,
    pub exec: String,
    pub icono: String,
    pub etiqueta: String,
    pub anclada: bool,
    pub abierta: bool,
    /// Centro del icono en coordenadas lógicas **relativas al dock**, para
    /// sacar el menú justo encima de él.
    pub x: f32,
}

enum Entrada {
    Item {
        etiqueta: String,
        icono: &'static str,
        accion: Accion,
    },
    Divisor,
    /// Las muestras de color de carpeta, con la puesta ahora si la hay.
    Colores(Option<String>),
}

/// Lado de la casilla de cada muestra: el círculo y su anillo de elegida.
const MUESTRA: f32 = 20.0;
/// El círculo de color dentro de la casilla.
const CIRCULO: f32 = 14.0;
const HUECO_MUESTRA: f32 = 6.0;
/// Dónde empieza la primera muestra: en la columna de los iconos de las filas,
/// que es `MARGEN_FILA` más el relleno de 11 de `menu::fila`, menos el aire del
/// anillo para que el círculo —y no su casilla— quede alineado con ellos.
const X_MUESTRAS: f32 = MARGEN_FILA + 11.0 - (MUESTRA - CIRCULO) / 2.0;

pub struct MenuDock {
    ventana: bool,
    /// Ancho del menú: fijo en el de ventana y en el del dock, ajustado al
    /// texto más largo en el del escritorio.
    ancho: f32,
    entradas: Vec<Entrada>,
    señalada: tema::Realce,
    x: f32,
    /// Punto lógico donde anclar la esquina superior izquierda, si lo hay.
    /// `None` es el menú de ventana centrado de toda la vida.
    punto: Option<(f32, f32)>,
}

impl MenuDock {
    pub fn es_ventana(&self) -> bool {
        self.ventana
    }
    pub fn ventana(opciones: Vec<(String, Accion)>) -> Self {
        Self::construir(opciones, None)
    }

    /// El menú del clic derecho sobre el escritorio, anclado al punto.
    ///
    /// Con las filas compactas del dock y del logo, con icono, y no con las de
    /// 48 px del menú de ventana: aquel está pensado para leerse centrado en
    /// pantalla, y junto al puntero salía una losa de 310 px para una sola
    /// entrada. `None` en `opciones` es un divisor.
    ///
    /// `colores` añade al final las muestras de color de carpeta, con la que
    /// tenga ahora la selección: `Some(None)` es el automático.
    pub fn escritorio(
        opciones: Vec<Option<(String, &'static str, Accion)>>,
        colores: Option<Option<&str>>,
        punto: (f32, f32),
    ) -> Self {
        let texto = opciones
            .iter()
            .flatten()
            .map(|(etiqueta, _, _)| crate::widget::ancho_de(etiqueta, tema::T_CUERPO))
            .fold(0.0, f32::max);
        // Lo que rodea al texto en `menu::fila`: margen y relleno a cada lado,
        // icono y su hueco. Los 16 de propina son el aire que deja el logo a la
        // derecha de su entrada más larga; sin él el texto toca el borde.
        let ancho = (MARGEN_FILA * 2.0 + 22.0 + ICONO + HUECO_ICONO + texto + 16.0)
            .ceil()
            .max(160.0);
        let mut entradas: Vec<Entrada> = opciones
            .into_iter()
            .map(|opcion| match opcion {
                Some((etiqueta, icono, accion)) => Entrada::Item {
                    etiqueta,
                    icono,
                    accion,
                },
                None => Entrada::Divisor,
            })
            .collect();
        if let Some(actual) = colores {
            entradas.push(Entrada::Divisor);
            entradas.push(Entrada::Colores(actual.map(str::to_string)));
        }
        Self {
            ventana: false,
            ancho,
            entradas,
            señalada: tema::Realce::nuevo(),
            x: 0.0,
            punto: Some(punto),
        }
    }

    fn construir(opciones: Vec<(String, Accion)>, punto: Option<(f32, f32)>) -> Self {
        Self {
            ancho: 310.0,
            entradas: opciones
                .into_iter()
                .map(|(etiqueta, accion)| Entrada::Item {
                    etiqueta,
                    icono: "chevron-derecha",
                    accion,
                })
                .collect(),
            señalada: tema::Realce::nuevo(),
            x: 0.0,
            ventana: true,
            punto,
        }
    }
    pub fn new(objetivo: Objetivo) -> Self {
        // «Nueva ventana» solo tiene sentido si ya hay una: sobre una
        // aplicación cerrada lo que hace la entrada es abrirla, y así se llama.
        let mut entradas = vec![Entrada::Item {
            etiqueta: if objetivo.abierta {
                "Abrir una nueva ventana".into()
            } else {
                "Abrir".into()
            },
            icono: "ventana-nueva",
            accion: Accion::Lanzar(objetivo.exec.clone()),
        }];
        entradas.push(Entrada::Item {
            etiqueta: if objetivo.anclada {
                "Quitar del dock".into()
            } else {
                "Fijar en el dock".into()
            },
            icono: "fijar",
            accion: Accion::Anclar {
                app_id: objetivo.app_id.clone(),
                exec: objetivo.exec.clone(),
                icono: objetivo.icono.clone(),
                fijar: None,
            },
        });
        // Cerrar solo aparece si hay algo que cerrar: una entrada permanente
        // que la mitad de las veces no hace nada enseña a desconfiar del menú.
        if objetivo.abierta {
            entradas.push(Entrada::Divisor);
            entradas.push(Entrada::Item {
                etiqueta: "Cerrar".into(),
                icono: "cerrar",
                accion: Accion::Cerrar {
                    app_id: objetivo.app_id.clone(),
                },
            });
        }
        Self {
            ventana: false,
            ancho: ANCHO,
            entradas,
            señalada: tema::Realce::nuevo(),
            x: objetivo.x,
            punto: None,
        }
    }

    pub fn size(&self) -> (f32, f32) {
        let alto: f32 = self
            .entradas
            .iter()
            .map(|e| match e {
                Entrada::Item { .. } | Entrada::Colores(_) => self.alto_fila(),
                Entrada::Divisor => DIVISOR,
            })
            .sum();
        (self.ancho(), alto + BORDE_SUP + BORDE_INF)
    }

    fn ancho(&self) -> f32 {
        self.ancho
    }
    fn alto_fila(&self) -> f32 {
        if self.ventana { 48.0 } else { FILA }
    }

    pub fn ancla(&self) -> Ancla {
        if let Some((x, y)) = self.punto {
            return Ancla::Punto { x, y };
        }
        if self.ventana {
            return Ancla::Centrada;
        }
        Ancla::SobreElDock { x: self.x }
    }

    fn entrada_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x > self.ancho() {
            return None;
        }
        let mut cursor = BORDE_SUP;
        for (i, entrada) in self.entradas.iter().enumerate() {
            let alto = match entrada {
                Entrada::Item { .. } | Entrada::Colores(_) => self.alto_fila(),
                Entrada::Divisor => DIVISOR,
            };
            if y >= cursor && y < cursor + alto {
                return matches!(entrada, Entrada::Item { .. } | Entrada::Colores(_)).then_some(i);
            }
            cursor += alto;
        }
        None
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        // La fila de muestras no se realza entera: lo que se elige es un
        // círculo, y pintar la fila de acento taparía los colores.
        let fila = punto
            .and_then(|(x, y)| self.entrada_en(x, y))
            .filter(|&i| matches!(self.entradas[i], Entrada::Item { .. }));
        self.señalada.señalar(fila)
    }

    pub fn animando(&self) -> bool {
        self.señalada.animando()
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let i = self.entrada_en(x, y)?;
        match &self.entradas[i] {
            Entrada::Item { accion, .. } => Some(accion.clone()),
            Entrada::Divisor => None,
            Entrada::Colores(actual) => {
                let k = muestra_en(x)?;
                let muestra = crate::escritorio::MUESTRAS[k];
                // Pulsar otra vez la que ya está la quita, como en el
                // explorador: es la vuelta al color automático.
                let elegido = (actual.as_deref() != Some(muestra)).then(|| muestra.to_string());
                Some(Accion::EscritorioColor(elegido))
            }
        }
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            crate::TeclaPulsada::Arriba
            | crate::TeclaPulsada::Abajo
            | crate::TeclaPulsada::Tabulador => {
                let indices: Vec<_> = self
                    .entradas
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| matches!(e, Entrada::Item { .. }).then_some(i))
                    .collect();
                if indices.is_empty() {
                    return Tecla::Consumida;
                }
                let actual = self
                    .señalada
                    .actual()
                    .and_then(|a| indices.iter().position(|i| *i == a));
                let n = if tecla == crate::TeclaPulsada::Arriba {
                    actual.map_or(indices.len() - 1, |a| {
                        (a + indices.len() - 1) % indices.len()
                    })
                } else {
                    actual.map_or(0, |a| (a + 1) % indices.len())
                };
                self.señalada.señalar(Some(indices[n]));
                Tecla::Consumida
            }
            crate::TeclaPulsada::Intro => {
                if let Some(Entrada::Item { accion, .. }) =
                    self.señalada.actual().and_then(|i| self.entradas.get(i))
                {
                    Tecla::Hacer(accion.clone())
                } else {
                    Tecla::Consumida
                }
            }
            _ => Tecla::Ignorada,
        }
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut col = column![Space::new().height(Length::Fixed(BORDE_SUP))];
        for (i, entrada) in self.entradas.iter().enumerate() {
            col = col.push(match entrada {
                Entrada::Item {
                    etiqueta, icono, ..
                } => {
                    if self.ventana {
                        let realce = self.señalada.intensidad(i);
                        container(
                            text(super::recortar_texto(etiqueta, 270.0, 14.0))
                                .size(14)
                                .color(tema::texto()),
                        )
                        .padding([14, 20])
                        .width(self.ancho())
                        .height(self.alto_fila())
                        .style(move |_| container::Style {
                            background: Some(tema::alfa(tema::acento(), realce * 0.10).into()),
                            border: Border {
                                radius: 13.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                        .into()
                    } else {
                        fila(etiqueta, None, icono, self.señalada.intensidad(i))
                    }
                }
                Entrada::Divisor => divisor(),
                Entrada::Colores(actual) => muestras(actual.as_deref()),
            });
        }
        col = col.push(Space::new().height(Length::Fixed(BORDE_INF)));

        container(col)
            .width(Length::Fixed(self.ancho()))
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

/// Qué muestra hay en `x`, relativo al menú. Se cuenta la casilla entera y
/// no solo el círculo: con 14 px de diana se fallaba más que se acertaba.
fn muestra_en(x: f32) -> Option<usize> {
    let dentro = x - X_MUESTRAS;
    if dentro < 0.0 {
        return None;
    }
    let k = (dentro / (MUESTRA + HUECO_MUESTRA)) as usize;
    let en_casilla = dentro - k as f32 * (MUESTRA + HUECO_MUESTRA) < MUESTRA;
    (en_casilla && k < crate::escritorio::MUESTRAS.len()).then_some(k)
}

/// La fila de círculos. La elegida lleva un anillo del color del texto
/// alrededor, con aire de por medio, como las muestras de Ajustes.
fn muestras(actual: Option<&str>) -> PanelElement<'static> {
    let mut fila = iced_widget::row![].spacing(HUECO_MUESTRA);
    for muestra in crate::escritorio::MUESTRAS {
        let color = u32::from_str_radix(&muestra[1..], 16)
            .map(tema::hex)
            .expect("las muestras son #RRGGBB escritas a mano");
        let elegida = actual == Some(muestra);
        let circulo = container(Space::new())
            .width(Length::Fixed(CIRCULO))
            .height(Length::Fixed(CIRCULO))
            .style(move |_| container::Style {
                background: Some(color.into()),
                border: Border {
                    radius: (CIRCULO / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        fila = fila.push(
            container(circulo)
                .width(Length::Fixed(MUESTRA))
                .height(Length::Fixed(MUESTRA))
                .center_x(Length::Fixed(MUESTRA))
                .center_y(Length::Fixed(MUESTRA))
                .style(move |_| container::Style {
                    border: Border {
                        radius: (MUESTRA / 2.0).into(),
                        width: if elegida { 1.5 } else { 0.0 },
                        color: tema::texto(),
                    },
                    ..Default::default()
                }),
        );
    }
    container(fila)
        .padding(iced_core::Padding {
            left: X_MUESTRAS,
            ..Default::default()
        })
        .height(Length::Fixed(FILA))
        .center_y(Length::Fixed(FILA))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teclado_selecciona_el_destino_y_escape_cancela() {
        let mut menu = MenuDock::ventana(vec![
            ("Siempre encima".into(), Accion::VentanaEncima),
            ("Mover a Escritorio 2".into(), Accion::VentanaEscritorio(1)),
        ]);
        menu.tecla(crate::TeclaPulsada::Abajo);
        menu.tecla(crate::TeclaPulsada::Abajo);
        assert!(matches!(
            menu.tecla(crate::TeclaPulsada::Intro),
            Tecla::Hacer(Accion::VentanaEscritorio(1))
        ));
        assert!(matches!(
            menu.tecla(crate::TeclaPulsada::Escape),
            Tecla::Cerrar
        ));
    }

    fn objetivo(anclada: bool, abierta: bool) -> Objetivo {
        Objetivo {
            app_id: "konsole".into(),
            exec: "konsole".into(),
            icono: "utilities-terminal".into(),
            etiqueta: "Terminal".into(),
            anclada,
            abierta,
            x: 100.0,
        }
    }

    /// Sin ventanas no hay nada que cerrar, y la entrada no debe salir.
    #[test]
    fn cerrar_solo_con_ventana_abierta() {
        let cerrado = MenuDock::new(objetivo(true, false));
        assert_eq!(cerrado.entradas.len(), 2);
        let abierto = MenuDock::new(objetivo(true, true));
        assert_eq!(abierto.entradas.len(), 4, "faltan el divisor y Cerrar");
    }

    /// La segunda entrada dice lo contrario de lo que ya está: si está fijada,
    /// quitarla.
    #[test]
    fn la_entrada_de_fijar_cambia_con_el_estado() {
        let m = MenuDock::new(objetivo(true, false));
        let Entrada::Item { etiqueta, .. } = &m.entradas[1] else {
            panic!("la segunda entrada no es un item");
        };
        assert_eq!(etiqueta, "Quitar del dock");
        let m = MenuDock::new(objetivo(false, false));
        let Entrada::Item { etiqueta, .. } = &m.entradas[1] else {
            panic!("la segunda entrada no es un item");
        };
        assert_eq!(etiqueta, "Fijar en el dock");
    }

    /// Cada círculo elige su color, y el que ya está puesto vuelve al
    /// automático.
    #[test]
    fn las_muestras_eligen_y_la_puesta_se_quita() {
        let mut m = MenuDock::escritorio(vec![], Some(Some("#FF3B30")), (0.0, 0.0));
        let y = BORDE_SUP + DIVISOR + FILA / 2.0;
        let x = |k: usize| X_MUESTRAS + k as f32 * (MUESTRA + HUECO_MUESTRA) + MUESTRA / 2.0;
        assert_eq!(
            m.pulsar(x(0), y),
            Some(Accion::EscritorioColor(Some("#FFCC00".into())))
        );
        assert_eq!(m.pulsar(x(1), y), Some(Accion::EscritorioColor(None)));
        // El hueco entre dos casillas no es de ninguna.
        assert_eq!(m.pulsar(X_MUESTRAS + MUESTRA + 1.0, y), None);
    }

    /// El hueco del divisor no activa nada.
    #[test]
    fn el_divisor_no_se_pulsa() {
        let mut m = MenuDock::new(objetivo(true, true));
        assert!(m.pulsar(20.0, BORDE_SUP + FILA * 2.0 + 1.0).is_none());
    }
}
