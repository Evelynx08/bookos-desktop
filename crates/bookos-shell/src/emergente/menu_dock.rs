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
use iced_widget::{column, container, Space};

use crate::tema;
use crate::view::PanelElement;
use crate::Accion;

use super::menu::{divisor, fila, DIVISOR, FILA};
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
}

pub struct MenuDock {
    entradas: Vec<Entrada>,
    señalada: tema::Realce,
    x: f32,
}

impl MenuDock {
    pub fn new(objetivo: Objetivo) -> Self {
        let mut entradas = vec![Entrada::Item {
            etiqueta: "Abrir una nueva ventana".into(),
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
            entradas,
            señalada: tema::Realce::nuevo(),
            x: objetivo.x,
        }
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
        Ancla::SobreElDock { x: self.x }
    }

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

    pub fn animando(&self) -> bool {
        self.señalada.animando()
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let i = self.entrada_en(x, y)?;
        match &self.entradas[i] {
            Entrada::Item { accion, .. } => Some(accion.clone()),
            Entrada::Divisor => None,
        }
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut col = column![Space::new().height(Length::Fixed(BORDE_SUP))];
        for (i, entrada) in self.entradas.iter().enumerate() {
            col = col.push(match entrada {
                Entrada::Item {
                    etiqueta, icono, ..
                } => fila(etiqueta, None, icono, self.señalada.intensidad(i)),
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// El hueco del divisor no activa nada.
    #[test]
    fn el_divisor_no_se_pulsa() {
        let mut m = MenuDock::new(objetivo(true, true));
        assert!(m.pulsar(20.0, BORDE_SUP + FILA * 2.0 + 1.0).is_none());
    }
}
