//! El emergente de Sonido, colgado del icono del volumen.
//!
//! Es el `fullRepresentation` del plasmoide `bookos-volume`, con sus mismas
//! medidas: tarjeta de 300 de ancho, márgenes de 16, la píldora gorda de 26 de
//! alto con el radio a la mitad de su altura, y el botón redondo de 38 al lado.
//! Dos secciones, Altavoces y Micrófono, cada una con su etiqueta, su
//! porcentaje a la derecha y su fila de control.
//!
//! **El deslizador es lo primero del shell que se arrastra.** Hasta ahora las
//! emergentes solo respondían a pulsaciones sueltas: el menú y el calendario se
//! recorren y se pulsan, y con eso basta. Una píldora necesita saber que sigue
//! agarrada mientras mueves el ratón, incluso si te sales de la tarjeta — que
//! es lo que hace cualquiera al llevar el volumen al máximo de un tirón.
//!
//! **Qué se escribe y qué no.** El nivel se manda a PipeWire con `wpctl` en
//! cuanto se mueve, sin esperar a soltar: el sonido tiene que seguir al dedo o
//! no se puede ajustar de oído. El proceso se lanza sin esperarlo, por lo mismo
//! que el widget del panel no espera la lectura: `wpctl` tarda 21 ms medidos y
//! eso es más de un frame.

use std::process::{Command, Stdio};

use iced_core::alignment::Vertical;
use iced_core::Length;
use iced_widget::{column, row, text, Space};

use crate::tema;
use crate::view::PanelElement;
use crate::Accion;

use super::control::{self, BOTON, HUECO, MARGEN_AGARRE, PILDORA};
use super::{Ancla, Tecla};

const ANCHO: f32 = 300.0;
const MARGEN: f32 = 16.0;
/// Separación entre las dos secciones.
const ENTRE_SECCIONES: f32 = 16.0;

/// Cuánto sube o baja el volumen una muesca de rueda.
const PASO_RUEDA: i32 = 5;

/// Qué fila se está tocando.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fila {
    Salida,
    Entrada,
}

pub struct Sonido {
    salida: Canal,
    entrada: Canal,
    /// Qué píldora está agarrada ahora mismo, si alguna.
    agarrada: Option<Fila>,
}

/// Un destino de PipeWire tal y como lo enseña el emergente.
struct Canal {
    nivel: u8,
    silenciado: bool,
    /// El nombre del destino para `wpctl`.
    destino: &'static str,
    /// Si es el micrófono. Decide qué familia de iconos usa.
    micro: bool,
    /// El icono del botón, ya cargado. Vive aquí y no se rasteriza en `view`
    /// porque el elemento que devuelve `ver_teñido` presta del `Icono`: si se
    /// carga dentro de la vista, muere antes de dibujarse.
    icono: Option<crate::icono::Icono>,
}

impl Canal {
    fn nuevo(destino: &'static str, micro: bool) -> Self {
        let (nivel, silenciado) = crate::widgets::volumen::consultar(destino).unwrap_or((0, false));
        let mut canal = Self {
            nivel,
            silenciado,
            destino,
            micro,
            icono: None,
        };
        canal.actualizar_icono();
        canal
    }

    /// El nombre del icono que le toca al estado actual.
    fn nombre_icono(&self) -> &'static str {
        match (self.micro, self.silenciado, self.nivel) {
            (true, true, _) => "micro-silencio",
            (true, false, _) => "micro",
            (_, true, _) | (_, _, 0) => "volumen-silencio",
            (_, _, n) if n < 40 => "volumen-bajo",
            (_, _, n) if n < 75 => "volumen-medio",
            _ => "volumen-alto",
        }
    }

    fn actualizar_icono(&mut self) {
        self.icono = crate::icono::propio(self.nombre_icono());
    }

    /// Manda el nivel a PipeWire. No espera: ver la cabecera del módulo.
    fn poner(&mut self, nivel: u8) {
        self.nivel = nivel.min(100);
        // Mover el deslizador de un canal silenciado lo devuelve a la vida: es
        // lo que hace el plasmoide, y lo que espera cualquiera que empuje la
        // barra para volver a oír algo.
        if self.silenciado && self.nivel > 0 {
            self.silenciado = false;
            let _ = lanzar(&["set-mute", self.destino, "0"]);
        }
        let _ = lanzar(&["set-volume", self.destino, &format!("{:.2}", self.nivel as f32 / 100.0)]);
        self.actualizar_icono();
    }

    fn alternar_silencio(&mut self) {
        self.silenciado = !self.silenciado;
        let _ = lanzar(&["set-mute", self.destino, "toggle"]);
        self.actualizar_icono();
    }

    fn etiqueta(&self) -> String {
        if self.silenciado {
            "Silenciado".into()
        } else {
            format!("{}%", self.nivel)
        }
    }
}

impl Sonido {
    pub fn new() -> Self {
        Self {
            salida: Canal::nuevo("@DEFAULT_AUDIO_SINK@", false),
            entrada: Canal::nuevo("@DEFAULT_AUDIO_SOURCE@", true),
            agarrada: None,
        }
    }

    pub fn size(&self) -> (f32, f32) {
        // Cabecera + dos secciones. Cada sección es su etiqueta (18) y su fila
        // de controles, que la marca el botón por ser lo más alto.
        let seccion = 18.0 + 8.0 + BOTON;
        (ANCHO, MARGEN * 2.0 + 22.0 + 14.0 + seccion * 2.0 + ENTRE_SECCIONES)
    }

    /// Cuelga del icono del volumen, como el calendario cuelga del reloj.
    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("volumen")
    }

    /// El rectángulo de la píldora de una fila, relativo a la emergente.
    fn rect_pildora(&self, fila: Fila) -> iced_core::Rectangle {
        let (_, alto) = self.size();
        let seccion = 18.0 + 8.0 + BOTON;
        let y0 = MARGEN + 22.0 + 14.0;
        let y = match fila {
            Fila::Salida => y0,
            Fila::Entrada => y0 + seccion + ENTRE_SECCIONES,
        } + 18.0
            + 8.0;
        debug_assert!(y + BOTON <= alto, "la fila se sale de la tarjeta");
        iced_core::Rectangle {
            x: MARGEN,
            y: y + (BOTON - PILDORA) / 2.0,
            width: ANCHO - MARGEN * 2.0 - BOTON - HUECO,
            height: PILDORA,
        }
    }

    /// El rectángulo del botón de silencio de una fila.
    fn rect_boton(&self, fila: Fila) -> iced_core::Rectangle {
        let p = self.rect_pildora(fila);
        iced_core::Rectangle {
            x: p.x + p.width + HUECO,
            y: p.y - (BOTON - PILDORA) / 2.0,
            width: BOTON,
            height: BOTON,
        }
    }

    fn canal(&mut self, fila: Fila) -> &mut Canal {
        match fila {
            Fila::Salida => &mut self.salida,
            Fila::Entrada => &mut self.entrada,
        }
    }

    /// De una `x` dentro de la píldora al nivel que le toca.
    fn nivel_en(&self, fila: Fila, x: f32) -> u8 {
        let r = self.rect_pildora(fila);
        control::nivel_en(x, r.x, r.width)
    }

    /// ¿Hay una píldora agarrada? Mientras la haya, el puntero le llega aunque
    /// se salga de la tarjeta.
    pub fn agarrado(&self) -> bool {
        self.agarrada.is_some()
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let (Some(fila), Some((x, _))) = (self.agarrada, punto) else {
            return false;
        };
        let nivel = self.nivel_en(fila, x);
        if self.canal(fila).nivel == nivel {
            return false;
        }
        self.canal(fila).poner(nivel);
        true
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = iced_core::Point::new(x, y);
        for fila in [Fila::Salida, Fila::Entrada] {
            if self.rect_boton(fila).contains(punto) {
                self.canal(fila).alternar_silencio();
                return None;
            }
            // La zona agarrable es más alta que la píldora: apuntar a 26 px de
            // alto con el ratón en movimiento falla más de lo que parece.
            let mut zona = self.rect_pildora(fila);
            zona.y -= MARGEN_AGARRE;
            zona.height += MARGEN_AGARRE * 2.0;
            if zona.contains(punto) {
                self.agarrada = Some(fila);
                let nivel = self.nivel_en(fila, x);
                self.canal(fila).poner(nivel);
                return None;
            }
        }
        None
    }

    /// Soltar el botón deja de arrastrar. Devuelve si hay que repintar.
    pub fn soltar(&mut self) -> bool {
        self.agarrada.take().is_some()
    }

    /// La rueda sobre una fila sube y baja su nivel, como en el plasmoide.
    pub fn desplazar(&mut self, _dx: f32, dy: f32) -> bool {
        if dy == 0.0 {
            return false;
        }
        let paso = if dy > 0.0 { PASO_RUEDA } else { -PASO_RUEDA };
        let nivel = (self.salida.nivel as i32 + paso).clamp(0, 100) as u8;
        if nivel == self.salida.nivel {
            return false;
        }
        self.salida.poner(nivel);
        true
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Izquierda | T::Abajo => {
                let n = (self.salida.nivel as i32 - PASO_RUEDA).clamp(0, 100) as u8;
                self.salida.poner(n);
                Tecla::Consumida
            }
            T::Derecha | T::Arriba => {
                let n = (self.salida.nivel as i32 + PASO_RUEDA).clamp(0, 100) as u8;
                self.salida.poner(n);
                Tecla::Consumida
            }
            _ => Tecla::Ignorada,
        }
    }

    pub fn view(&self) -> PanelElement<'_> {
        let contenido = column![
            text("Sonido")
                .size(tema::T_TITULO)
                .color(tema::TEXTO),
            Space::new().height(Length::Fixed(14.0)),
            self.seccion("Altavoces", &self.salida),
            Space::new().height(Length::Fixed(ENTRE_SECCIONES)),
            self.seccion("Micrófono", &self.entrada),
        ];
        control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }

    /// Una sección: etiqueta y porcentaje arriba, píldora y botón abajo.
    fn seccion<'a>(&'a self, titulo: &'a str, canal: &'a Canal) -> PanelElement<'a> {
        let color = if canal.silenciado {
            tema::TEXTO2
        } else {
            tema::TEXTO
        };
        let cabecera = row![
            text(titulo).size(tema::T_PEQUENO).color(tema::TEXTO2),
            Space::new().width(crate::FILL),
            text(canal.etiqueta()).size(tema::T_CUERPO).color(color),
        ];
        let ancho = ANCHO - MARGEN * 2.0 - BOTON - HUECO;
        let controles = row![
            control::pildora(ancho, canal.nivel, canal.silenciado),
            Space::new().width(Length::Fixed(HUECO)),
            control::boton(canal.icono.as_ref(), canal.silenciado),
        ]
        .align_y(Vertical::Center);
        column![cabecera, Space::new().height(Length::Fixed(8.0)), controles].into()
    }

}

/// Lanza `wpctl` sin esperarlo.
fn lanzar(args: &[&str]) -> Option<std::process::Child> {
    Command::new("wpctl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El nivel sale de dónde se pulsa dentro de la píldora, y los extremos
    /// tienen que dar 0 y 100 exactos: si el máximo se queda en 98, no hay
    /// forma de poner el volumen al tope arrastrando.
    #[test]
    fn la_pildora_reparte_el_nivel_de_extremo_a_extremo() {
        let s = Sonido::new();
        let r = s.rect_pildora(Fila::Salida);
        assert_eq!(s.nivel_en(Fila::Salida, r.x), 0);
        assert_eq!(s.nivel_en(Fila::Salida, r.x + r.width), 100);
        assert_eq!(s.nivel_en(Fila::Salida, r.x + r.width / 2.0), 50);
        // Y fuera de la barra no se desborda: arrastrar más allá del borde deja
        // el nivel en el tope, no en un número imposible.
        assert_eq!(s.nivel_en(Fila::Salida, r.x - 500.0), 0);
        assert_eq!(s.nivel_en(Fila::Salida, r.x + r.width + 500.0), 100);
    }

    /// Las dos filas no se solapan y caben en la tarjeta.
    #[test]
    fn las_dos_filas_caben_y_no_se_pisan() {
        let s = Sonido::new();
        let (ancho, alto) = s.size();
        let a = s.rect_boton(Fila::Salida);
        let b = s.rect_boton(Fila::Entrada);
        assert!(a.y + a.height <= b.y, "las filas se solapan");
        assert!(b.y + b.height <= alto, "la segunda fila se sale por abajo");
        assert!(a.x + a.width <= ancho, "el botón se sale por la derecha");
    }

    /// Soltar sin haber agarrado nada no obliga a repintar.
    #[test]
    fn soltar_sin_agarrar_no_hace_nada() {
        let mut s = Sonido::new();
        assert!(!s.soltar());
        assert!(!s.agarrado());
    }
}
