//! El emergente del brillo, colgado de su icono.
//!
//! Pantalla y teclado se detectan de nuevo al refrescar. Las escrituras
//! se agrupan en un hilo dedicado para no interrumpir el renderizado.

use iced_core::alignment::Vertical;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::icono::Icono;
use crate::tema;
use crate::view::PanelElement;

use super::control::{self, BOTON, HUECO, MARGEN_AGARRE, PILDORA};
use super::{Ancla, Tecla};

const ANCHO: f32 = 300.0;
const MARGEN: f32 = 16.0;
/// Ancho de la píldora dentro de la tarjeta.
const BARRA: f32 = ANCHO - MARGEN * 2.0 - BOTON - HUECO;

/// Hueco entre la fila de la pantalla y la del teclado.
const SEPARACION_FILAS: f32 = 18.0;
/// Alto de la tarjeta de la luz nocturna, al final.
const NOCTURNA: f32 = 52.0;

/// Cuánto sube o baja una muesca de rueda o una flecha.
const PASO: i32 = 5;

/// Por debajo de esto la pantalla se queda negra y el portátil parece apagado.
///
/// El plasmoide usa el mismo suelo. No es un capricho de diseño: con el brillo
/// a cero no se ve ni el propio deslizador para volver a subirlo.
const MINIMO: u8 = 5;

/// La retroiluminación del teclado: dispositivo en `/sys/class/leds`, valor
/// actual y máximo. Va con niveles enteros —en este portátil, cuatro pasos— y
/// no en tanto por ciento, así que se guarda crudo y se enseña convertido.
#[derive(PartialEq, Eq)]
struct Teclado {
    dispositivo: String,
    nivel: u32,
    maximo: u32,
}

/// Cuál es la luz del teclado, si la hay. Mismo criterio que el compositor al
/// atender la tecla XF86: el primer `led` cuyo nombre lleve `kbd_backlight`.
fn leds_teclado() -> Option<Teclado> {
    let (dispositivo, nivel, maximo) = crate::brillo_teclado_actual()?;
    Some(Teclado { dispositivo, nivel, maximo })
}

/// Qué píldora se está arrastrando. Sin esto, agarrar la del teclado movía la
/// de la pantalla, porque `agarrada` era un booleano suelto.
#[derive(PartialEq, Clone, Copy)]
enum Agarre {
    Nada,
    Pantalla,
    Teclado,
}

pub struct Brillo {
    nivel: u8,
    agarrada: Agarre,
    icono: Option<Icono>,
    /// Qué dispositivo de retroiluminación se maneja: `intel_backlight`,
    /// `amdgpu_bl0`… El nombre hace falta para la llamada a logind.
    dispositivo: Option<String>,
    /// Valor crudo máximo del dispositivo, para pasar de tanto por ciento.
    maximo: u32,
    /// La luz del teclado. `None` en un equipo que no la tenga: entonces la
    /// segunda fila ni se dibuja, en vez de enseñar un control muerto.
    teclado: Option<Teclado>,
    icono_teclado: Option<Icono>,
}

impl Brillo {
    pub fn new() -> Self {
        let dispositivo = crate::state::backlight_device();
        let maximo = dispositivo
            .as_deref()
            .and_then(crate::state::backlight_max)
            .unwrap_or(0);
        Self {
            nivel: crate::state::read_brightness().unwrap_or(0),
            agarrada: Agarre::Nada,
            icono: crate::icono::propio("brillo"),
            dispositivo,
            maximo,
            teclado: leds_teclado(),
            icono_teclado: crate::icono::propio("teclado"),
        }
    }

    pub fn refrescar(&mut self) -> bool {
        if self.agarrado() { return false; }
        let device = crate::backlight();
        let (dispositivo, maximo) = device.map(|(d, m)| (Some(d), m)).unwrap_or((None, 0));
        let nivel = crate::brillo_actual().unwrap_or(0);
        let teclado = leds_teclado();
        let changed = self.dispositivo != dispositivo || self.maximo != maximo
            || self.nivel != nivel || self.teclado != teclado;
        self.dispositivo = dispositivo; self.maximo = maximo;
        self.nivel = nivel; self.teclado = teclado;
        changed
    }

    pub fn size(&self) -> (f32, f32) {
        let fila = 18.0 + 8.0 + BOTON;
        let teclado = if self.teclado.is_some() {
            SEPARACION_FILAS + fila
        } else {
            0.0
        };
        (
            ANCHO,
            MARGEN * 2.0 + 22.0 + 14.0 + fila + teclado + SEPARACION_FILAS + NOCTURNA,
        )
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("brillo")
    }

    /// El rectángulo de la píldora, relativo a la emergente.
    fn rect_pildora(&self) -> iced_core::Rectangle {
        iced_core::Rectangle {
            x: MARGEN,
            y: MARGEN + 22.0 + 14.0 + 18.0 + 8.0 + (BOTON - PILDORA) / 2.0,
            width: BARRA,
            height: PILDORA,
        }
    }

    /// El rectángulo de la píldora del teclado, relativo a la emergente.
    fn rect_teclado(&self) -> Option<iced_core::Rectangle> {
        self.teclado.as_ref()?;
        let mut r = self.rect_pildora();
        r.y += SEPARACION_FILAS + 18.0 + 8.0 + BOTON;
        Some(r)
    }

    pub fn agarrado(&self) -> bool {
        self.agarrada != Agarre::Nada
    }

    /// Manda el nivel crudo del teclado a logind. `true` si cambió.
    ///
    /// El nivel llega en tanto por ciento porque es lo que da la píldora, pero
    /// el dispositivo tiene pasos enteros: aquí son cuatro, así que se redondea
    /// al paso más cercano y la píldora salta de 0 a 33, 66 y 100.
    fn poner_teclado(&mut self, porciento: u8) -> bool {
        let Some(teclado) = self.teclado.as_mut() else {
            return false;
        };
        let crudo = (teclado.maximo as f64 * porciento as f64 / 100.0).round() as u32;
        let crudo = crudo.min(teclado.maximo);
        if crudo == teclado.nivel {
            return false;
        }
        teclado.nivel = crudo;
        crate::retroiluminacion::solicitar("leds", &teclado.dispositivo, crudo);
        true
    }

    /// El nivel del teclado en tanto por ciento, para la píldora.
    fn porciento_teclado(&self) -> u8 {
        self.teclado
            .as_ref()
            .map(|t| (u64::from(t.nivel) * 100 / u64::from(t.maximo.max(1))) as u8)
            .unwrap_or(0)
    }

    /// Escribe el nivel y lo manda a logind. `true` si cambió.
    fn poner(&mut self, nivel: u8) -> bool {
        if self.dispositivo.is_none() || self.maximo == 0 { return false; }
        let nivel = nivel.clamp(MINIMO, 100);
        if nivel == self.nivel {
            return false;
        }
        self.nivel = nivel;
        let (Some(dispositivo), true) = (self.dispositivo.as_deref(), self.maximo > 0) else {
            return true;
        };
        // De tanto por ciento al valor crudo del dispositivo, que en este panel
        // llega a 400. Comprobado de punta a punta: pedir el 60 % deja
        // `actual_brightness` en 240, que es justo 400 × 0,6.
        let crudo = (self.maximo as f64 * nivel as f64 / 100.0).round() as u32;
        crate::retroiluminacion::solicitar("backlight", dispositivo, crudo);
        true
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let Some((x, _)) = punto else {
            return false;
        };
        let nivel = control::nivel_en(x, MARGEN, BARRA);
        match self.agarrada {
            Agarre::Nada => false,
            Agarre::Pantalla => self.poner(nivel),
            Agarre::Teclado => self.poner_teclado(nivel),
        }
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = iced_core::Point::new(x, y);
        let holgada = |mut zona: iced_core::Rectangle| {
            zona.y -= MARGEN_AGARRE;
            zona.height += MARGEN_AGARRE * 2.0;
            zona
        };
        let nivel = control::nivel_en(x, MARGEN, BARRA);
        if holgada(self.rect_pildora()).contains(punto) {
            self.agarrada = Agarre::Pantalla;
            self.poner(nivel);
        } else if self
            .rect_teclado()
            .is_some_and(|r| holgada(r).contains(punto))
        {
            self.agarrada = Agarre::Teclado;
            self.poner_teclado(nivel);
        }
        None
    }

    pub fn soltar(&mut self) -> bool {
        std::mem::replace(&mut self.agarrada, Agarre::Nada) != Agarre::Nada
    }

    pub fn desplazar(&mut self, _dx: f32, dy: f32) -> bool {
        if dy == 0.0 {
            return false;
        }
        let paso = if dy > 0.0 { PASO } else { -PASO };
        let nivel = (self.nivel as i32 + paso).clamp(0, 100) as u8;
        self.poner(nivel)
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Izquierda | T::Abajo => {
                self.poner((self.nivel as i32 - PASO).clamp(0, 100) as u8);
                Tecla::Consumida
            }
            T::Derecha | T::Arriba => {
                self.poner((self.nivel as i32 + PASO).clamp(0, 100) as u8);
                Tecla::Consumida
            }
            _ => Tecla::Ignorada,
        }
    }

    pub fn view(&self) -> PanelElement<'_> {
        let cabecera = row![
            text("Pantalla").size(tema::T_PEQUENO).color(tema::TEXTO2),
            Space::new().width(crate::FILL),
            text(if self.dispositivo.is_some() { format!("{}%", self.nivel) } else { "No disponible".into() })
                .size(tema::T_CUERPO)
                .color(tema::texto()),
        ];
        let controles = row![
            control::pildora(BARRA, self.nivel, self.dispositivo.is_none()),
            Space::new().width(Length::Fixed(HUECO)),
            // Sin realce de puntero: este botón no se pulsa —el brillo se
            // cambia con la píldora— y encenderlo al pasar por encima
            // prometería una acción que no existe.
            control::boton(self.icono.as_ref(), self.dispositivo.is_none(), 0.0),
        ]
        .align_y(Vertical::Center);
        let mut contenido = column![
            row![
                text("Brillo").size(tema::T_TITULO).color(tema::texto()),
                Space::new().width(crate::FILL),
            ]
            .align_y(Vertical::Center),
            Space::new().height(Length::Fixed(14.0)),
            cabecera,
            Space::new().height(Length::Fixed(8.0)),
            controles,
        ];
        if self.teclado.is_some() {
            let porciento = self.porciento_teclado();
            contenido = contenido
                .push(Space::new().height(Length::Fixed(SEPARACION_FILAS)))
                .push(row![
                    text("Teclado").size(tema::T_PEQUENO).color(tema::TEXTO2),
                    Space::new().width(crate::FILL),
                    text(format!("{porciento}%"))
                        .size(tema::T_CUERPO)
                        .color(tema::texto()),
                ])
                .push(Space::new().height(Length::Fixed(8.0)))
                .push(
                    row![
                        control::pildora(BARRA, porciento, false),
                        Space::new().width(Length::Fixed(HUECO)),
                        control::boton(self.icono_teclado.as_ref(), porciento == 0, 0.0),
                    ]
                    .align_y(Vertical::Center),
                );
        }
        // Estado informativo hasta que haya un control de luz nocturna.
        let nocturna = container(
            row![
                column![
                    text("Luz nocturna").size(14.0).color(tema::texto()),
                    text("No disponible").size(tema::T_PEQUENO).color(tema::TEXTO2),
                ],
                Space::new().width(crate::FILL),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(NOCTURNA))
        .padding([0, 10])
        .center_y(Length::Fixed(NOCTURNA))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(
                Color {
                    a: 0.05,
                    ..tema::tinta()
                }
                .into(),
            ),
            border: Border {
                radius: tema::R_CONTROL.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        contenido = contenido
            .push(Space::new().height(Length::Fixed(SEPARACION_FILAS)))
            .push(nocturna);

        control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El brillo no baja de un suelo: a cero no se ve ni el deslizador con el
    /// que volver a subirlo.
    #[test]
    fn no_se_puede_apagar_del_todo() {
        let mut b = Brillo::new();
        b.nivel = 50;
        b.poner(0);
        assert_eq!(b.nivel, MINIMO);
    }

    /// Y no se pasa de cien por arriba.
    #[test]
    fn no_se_pasa_de_cien() {
        let mut b = Brillo::new();
        b.nivel = 50;
        b.poner(200);
        assert_eq!(b.nivel, 100);
    }

    #[test]
    fn soltar_sin_agarrar_no_hace_nada() {
        let mut b = Brillo::new();
        assert!(!b.soltar());
    }
}
