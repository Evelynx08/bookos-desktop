//! En qué escritorio estás: una fila de puntos.
//!
//! Con varios escritorios y sin esto, cambiar de escritorio es teletransportarse
//! a un sitio vacío sin saber a cuál has ido ni cuántos quedan a cada lado. El
//! indicador es lo que convierte «mis ventanas han desaparecido» en «estoy en el
//! tercero de cuatro».
//!
//! **El dato lo pone el compositor**, por `Widget::escritorios`. El shell no
//! conoce a Smithay a propósito —esa frontera es lo que permite probar el panel
//! sin arrancar un compositor—, así que el escritorio activo entra como
//! parámetro, igual que las ventanas abiertas del dock.
//!
//! Se dibuja con puntos y no con números porque a 32 px de alto un «3/4» pide
//! leerlo y un punto relleno entre cuatro se ve sin mirar. Los puntos son
//! pulsables: el que tocas es al que vas.

use std::time::Instant;

use iced_core::{Border, Color, Length};
use iced_widget::{Space, container, row, stack};

use crate::tema;
use crate::view::PanelElement;
use crate::widget::Widget;

/// Diámetro del punto de un escritorio.
const PUNTO: f32 = 7.0;
/// Aire entre dos puntos.
const HUECO: f32 = 6.0;

pub struct Escritorios {
    /// Cuántos hay. Cero mientras el compositor no lo diga, y entonces el widget
    /// no se dibuja: en el arranque no se sabe todavía, y un punto suelto
    /// mientras tanto sería peor que nada.
    cuantos: usize,
    activo: usize,
    /// De qué escritorio viene el punto de acento y desde cuándo se mueve.
    viaje: Option<(usize, Instant)>,
}

impl Escritorios {
    pub fn new() -> Self {
        Self {
            // La configuración BookOS trae dos escritorios; mostrarlos desde
            // el primer fotograma evita que aparezca un punto suelto mientras
            // llega la primera actualización del compositor.
            cuantos: 2,
            activo: 0,
            viaje: None,
        }
    }

    /// A qué escritorio corresponde una `x` dentro del widget.
    ///
    /// El hueco entre puntos **sí** cuenta, y va para el punto de su izquierda:
    /// apuntar a un círculo de 7 px con el ratón en movimiento falla más de lo
    /// que parece, y aquí fallar significa no ir a ninguna parte.
    pub fn en(&self, x: f32) -> Option<usize> {
        if self.cuantos == 0 || x < 0.0 {
            return None;
        }
        let i = (x / (PUNTO + HUECO)) as usize;
        (i < self.cuantos).then_some(i)
    }
}

impl Widget for Escritorios {
    fn nombre(&self) -> &'static str {
        "escritorios"
    }

    /// No lee nada: su dato entra por `Widget::escritorios`, no de sysfs.
    fn refrescar(&mut self) -> bool {
        false
    }

    fn pulsar(&self, x: f32) -> Option<crate::Accion> {
        self.en(x).map(|i| {
            if i == self.activo {
                crate::Accion::VistaEscritorios
            } else {
                crate::Accion::Escritorio(i)
            }
        })
    }

    fn escritorios(&mut self, activo: usize, cuantos: usize) -> bool {
        if (activo, cuantos) == (self.activo, self.cuantos) {
            return false;
        }
        // Solo se desliza al cambiar de escritorio con los mismos que había;
        // si aparece o desaparece uno, el punto va directo a su sitio.
        self.viaje =
            (cuantos == self.cuantos && activo != self.activo && !tema::efectos_reducidos())
                .then(|| (self.activo, Instant::now()));
        self.activo = activo;
        self.cuantos = cuantos;
        true
    }

    fn animando(&self) -> bool {
        self.viaje.is_some_and(|(_, t)| t.elapsed() < tema::D_MODAL)
    }

    fn ancho(&self) -> f32 {
        if self.cuantos == 0 {
            return 0.0;
        }
        self.cuantos as f32 * PUNTO + (self.cuantos - 1) as f32 * HUECO
    }

    fn ver(&self) -> PanelElement<'_> {
        if self.cuantos == 0 {
            return crate::widget::vacio();
        }
        let punto = |color: Color| {
            container(Space::new())
                .width(Length::Fixed(PUNTO))
                .height(Length::Fixed(PUNTO))
                .style(move |_theme: &iced_widget::Theme| container::Style {
                    background: Some(color.into()),
                    border: Border {
                        radius: (PUNTO / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
        };
        // Todos los puntos en blanco al 35 % y el acento encima, en su propia
        // capa: así puede ir de un escritorio a otro deslizándose en vez de
        // apagarse uno y encenderse otro. Es el mismo tratamiento que los
        // puntos de página del launchpad, para que un punto signifique lo
        // mismo en los dos sitios.
        let mut fila = row![];
        for i in 0..self.cuantos {
            if i > 0 {
                fila = fila.push(Space::new().width(Length::Fixed(HUECO)));
            }
            fila = fila.push(punto(Color {
                a: 0.35,
                ..tema::tinta()
            }));
        }
        let paso = PUNTO + HUECO;
        let destino = self.activo as f32 * paso;
        let x = match self.viaje {
            Some((origen, desde)) if desde.elapsed() < tema::D_MODAL => {
                let t = tema::C_MUELLE.eval(tema::fraccion(desde.elapsed(), tema::D_MODAL));
                let origen = origen as f32 * paso;
                // El muelle se pasa un poco del destino; se acota para que el
                // punto no se salga por los extremos de la fila.
                (origen + (destino - origen) * t).clamp(0.0, self.ancho() - PUNTO)
            }
            _ => destino,
        };
        let acento = container(punto(tema::acento())).padding(iced_core::Padding::ZERO.left(x));
        container(stack![fila, acento])
            .center_y(Length::Fill)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Con cero escritorios el widget no ocupa sitio: si ocupara, dejaría un
    /// hueco en el panel.
    ///
    /// Se fuerza el cero en vez de dar por hecho el del constructor: arranca en
    /// dos —los de la configuración de BookOS— para no enseñar un punto suelto
    /// mientras llega la primera actualización del compositor.
    #[test]
    fn sin_escritorios_no_se_dibuja() {
        let mut e = Escritorios::new();
        e.escritorios(0, 0);
        assert_eq!(e.ancho(), 0.0);
        assert_eq!(e.en(3.0), None);
    }

    /// El hueco entre puntos es del punto de su izquierda: un blanco de 7 px es
    /// demasiado poco para apuntar con el ratón en marcha.
    #[test]
    fn el_hueco_entre_puntos_es_pulsable() {
        let mut e = Escritorios::new();
        e.escritorios(0, 4);
        assert_eq!(e.en(0.0), Some(0));
        assert_eq!(
            e.en(PUNTO + 2.0),
            Some(0),
            "el hueco va con el de su izquierda"
        );
        assert_eq!(e.en(PUNTO + HUECO + 1.0), Some(1));
        assert_eq!(e.en(e.ancho() + 20.0), None, "pasado el último, nada");
    }

    /// El ancho crece con los escritorios: es lo que reparte las zonas de clic
    /// del panel, así que si miente, el widget de al lado se come los clics.
    #[test]
    fn el_ancho_cuenta_puntos_y_huecos() {
        let mut e = Escritorios::new();
        e.escritorios(0, 1);
        assert_eq!(e.ancho(), PUNTO, "uno solo no lleva huecos");
        e.escritorios(0, 4);
        assert_eq!(e.ancho(), 4.0 * PUNTO + 3.0 * HUECO);
    }

    /// Repetir el mismo dato no pide repintar.
    #[test]
    fn el_mismo_dato_no_repinta() {
        let mut e = Escritorios::new();
        assert!(e.escritorios(1, 4));
        assert!(!e.escritorios(1, 4));
        assert!(e.escritorios(2, 4));
    }
}
