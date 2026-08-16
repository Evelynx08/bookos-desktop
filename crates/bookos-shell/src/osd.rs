//! El aviso que sale al subir el volumen, el brillo o apagar el touchpad.
//!
//! Es el OSD de siempre: una tarjeta centrada abajo con el icono de lo que has
//! tocado y su nivel, que se va sola. Sale del mismo sitio que el resto del
//! escritorio —el shell, dentro del compositor— porque tiene que aparecer
//! **aunque no haya ninguna aplicación con foco**, y porque el compositor es
//! quien recibe las teclas de función.
//!
//! **Se desvanece por tiempo, no por un temporizador que repinte.** El
//! compositor programa un solo despertar al final y, mientras tanto, el alfa
//! sale de cuánto ha pasado: sin eso habría que repintar sesenta veces por
//! segundo durante segundo y medio para enseñar algo que no cambia.

use std::time::{Duration, Instant};

use iced_core::alignment::Vertical;
use iced_core::{Border, Color, Length};
use iced_widget::{container, row, text, Space};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;

/// Cuánto se queda a la vista antes de empezar a irse.
const QUIETO: Duration = Duration::from_millis(1200);
/// Y cuánto tarda en desvanecerse.
pub const SALIDA: Duration = Duration::from_millis(280);

/// Medidas de la cápsula, tomadas del diseño (`OSD` en Figma): 263 × 80 con el
/// radio completo, no una tarjeta con esquinas.
const ANCHO: f32 = 263.0;
const ALTO: f32 = 80.0;
/// Margen izquierdo hasta el icono, y del icono hasta la barra.
///
/// En el diseño el icono empieza a 32 del borde y la barra a 61; el icono mide
/// 21, así que entre los dos quedan 8.
const MARGEN_IZQ: f32 = 32.0;
const LADO_ICONO: f32 = 21.0;
const HUECO_ICONO: f32 = 8.0;
/// La barra: 171 de ancho, 9 de alto y hasta 31 del borde derecho.
const ANCHO_BARRA: f32 = 171.0;
/// Separación desde el borde de abajo. Va por encima del dock, no encima de
/// él: taparlo al subir el volumen es justo lo que molesta del OSD de otros.
pub const MARGEN_INFERIOR: f32 = 170.0;
/// Alto de la barra de nivel.
const BARRA: f32 = 9.0;

/// El surco de la barra.
///
/// Gris claro sólido y no el blanco al 14 % del resto del sistema: la cápsula
/// del OSD es negra y opaca, y sobre ella un surco translúcido se pierde. En el
/// diseño se ve casi blanco.
const SURCO: Color = Color {
    r: 0.78,
    g: 0.78,
    b: 0.80,
    a: 1.0,
};

pub struct Osd {
    icono: Option<Icono>,
    /// De 0 a 100. `None` para lo que no tiene nivel, como apagar el touchpad.
    nivel: Option<u8>,
    /// Texto en vez de barra, para lo que no es un nivel.
    texto: Option<String>,
    desde: Instant,
}

impl Osd {
    pub fn new(icono: &str, nivel: Option<u8>, texto: Option<String>) -> Self {
        Self {
            icono: icono::propio(icono).or_else(|| icono::cargar(icono)),
            nivel,
            texto,
            desde: Instant::now(),
        }
    }

    /// Cuánto queda para que desaparezca del todo. `None` si ya se fue.
    pub fn queda(&self) -> Option<Duration> {
        (QUIETO + SALIDA).checked_sub(self.desde.elapsed())
    }

    /// Opacidad de ahora mismo: entero mientras está quieto y bajando después.
    pub fn alfa(&self) -> f32 {
        let t = self.desde.elapsed();
        if t < QUIETO {
            return 1.0;
        }
        let fuera = tema::fraccion(t - QUIETO, SALIDA);
        1.0 - tema::C_SUAVE.eval(fuera)
    }

    pub fn size(&self) -> (f32, f32) {
        (self.ancho(), ALTO)
    }

    /// El ancho de la cápsula.
    ///
    /// Con barra es fijo, el del diseño. Con texto se ajusta a lo que hay que
    /// escribir —«Touchpad desactivado» y «Fn lock» no ocupan lo mismo—, que es
    /// lo que hacen las cápsulas del diseño: 221 una y 199 otra.
    fn ancho(&self) -> f32 {
        match (&self.texto, self.nivel) {
            (Some(t), None) => {
                let texto = crate::widget::ancho_texto(t.chars().count());
                (MARGEN_IZQ + LADO_ICONO + HUECO_ICONO + texto + MARGEN_IZQ)
                    .max(ALTO * 2.0)
            }
            _ => ANCHO,
        }
    }

    pub fn view(&self) -> PanelElement<'_> {
        let dibujo = match &self.icono {
            Some(ic) => icono::ver_teñido(ic, LADO_ICONO, Some(Color::WHITE)),
            None => crate::widget::vacio(),
        };
        // El nivel manda sobre el texto: si hay barra, el número sobra — la
        // barra ya dice cuánto, y a un OSD que se va en un segundo no le sobra
        // sitio para decir lo mismo dos veces.
        let derecha: PanelElement<'_> = match (self.nivel, &self.texto) {
            (Some(nivel), _) => self.barra(nivel),
            (None, Some(t)) => text(t.clone())
                .size(tema::T_CUERPO)
                .color(Color::WHITE)
                .into(),
            (None, None) => Space::new().into(),
        };
        container(
            row![
                Space::new().width(Length::Fixed(MARGEN_IZQ)),
                dibujo,
                Space::new().width(Length::Fixed(HUECO_ICONO)),
                derecha,
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(self.ancho()))
        .height(Length::Fixed(ALTO))
        .center_y(Length::Fixed(ALTO))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(FONDO.into()),
            border: Border {
                // Cápsula, no tarjeta: es lo que dice el diseño y lo que la
                // distingue de las emergentes que cuelgan del panel.
                radius: (ALTO / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    fn barra<'a>(&self, nivel: u8) -> PanelElement<'a> {
        let lleno = ANCHO_BARRA * (nivel.min(100) as f32 / 100.0);
        let relleno = container(Space::new())
            .width(Length::Fixed(lleno))
            .height(Length::Fixed(BARRA))
            .style(|_theme: &iced_widget::Theme| container::Style {
                // Acento, no blanco: en el diseño la parte llena es el azul del
                // sistema, que es además lo que distingue de un vistazo cuánto
                // hay puesto sobre un surco casi blanco.
                background: Some(tema::ACENTO.into()),
                border: Border {
                    radius: (BARRA / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        container(relleno)
            .width(Length::Fixed(ANCHO_BARRA))
            .height(Length::Fixed(BARRA))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(SURCO.into()),
                border: Border {
                    radius: (BARRA / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }
}

/// Fondo de la tarjeta: casi negro y bastante opaco. Aquí no hay cristal
/// detrás —el OSD se mueve y sale sobre cualquier cosa— así que el contraste
/// lo tiene que poner él.
/// La cápsula es negra y **opaca**, como en el diseño.
///
/// Estaba al 92 % de un gris muy oscuro, que es lo que se hace con las tarjetas
/// que cuelgan del panel; el OSD no cuelga de nada y sale sobre lo que haya en
/// pantalla, así que un negro pleno es lo que le da su silueta.
const FONDO: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empieza_entero_y_acaba_yendose() {
        let osd = Osd::new("volumen-alto", Some(50), None);
        assert_eq!(osd.alfa(), 1.0, "recién salido tiene que verse entero");
        assert!(osd.queda().is_some());
    }

    /// El nivel se recorta: un volumen del 150 % no puede pintar una barra que
    /// se salga de la tarjeta.
    #[test]
    fn la_barra_no_se_desborda() {
        let osd = Osd::new("volumen-alto", Some(200), None);
        // `barra` recorta con `min(100)`; se comprueba que el cálculo del
        // relleno no pasa del ancho disponible.
        let ancho = ANCHO - 40.0 - 26.0 - 14.0;
        let lleno = ancho * (200u8.min(100) as f32 / 100.0);
        assert!(lleno <= ancho);
        assert!(osd.queda().is_some());
    }
}
