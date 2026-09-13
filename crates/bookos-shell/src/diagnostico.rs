//! El panel de diagnóstico: qué cuesta cada fotograma y quién se lo lleva.
//!
//! Existe porque «va a tirones» no es un dato. Las decisiones que quedan por
//! delante —bajar el desenfoque cuando la GPU no da más, separar el ritmo de
//! las animaciones del refresco del monitor, apagar efectos con la batería
//! baja— necesitan un número antes de escribirse, y hasta ahora el único que
//! había era una línea de traza cada cinco segundos, agregada de todas las
//! salidas y sin desglosar en qué se iba el tiempo.
//!
//! Se dibuja como el OSD: dentro del compositor, sobre una superficie propia,
//! sin cliente ni D-Bus por medio. Así se puede mirar mientras se arrastra una
//! ventana o se abre el launchpad, que es justo cuando se nota el tirón.
//!
//! **Lo que enseña son medias de la última ventana de un segundo**, no del
//! último fotograma: un contador que cambia sesenta veces por segundo no se
//! lee, y además obligaría a repintar el propio panel a esa velocidad, con lo
//! que el instrumento falsearía la medida. Un segundo es también lo que hace
//! que `fps` signifique algo sin promediar tanto que un tirón de 300 ms se
//! diluya — para eso está `peor`, que sí es el peor fotograma de la ventana.
//!
//! Los datos los rellena el compositor (`bookos-comp/src/metricas.rs`); aquí
//! solo se dibujan. Este módulo no sabe medir nada.

use iced_core::{Border, Color, Font, Length};
use iced_widget::{Space, column, container, row, text};

use crate::tema;
use crate::view::PanelElement;

/// Lo medido de una salida durante la última ventana.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Salida {
    pub nombre: String,
    pub ancho: i32,
    pub alto: i32,
    pub escala: f64,
    /// Refresco del modo, en hercios. Sale de `vrefresh` del kernel.
    pub hz: f32,
    pub vrr: bool,
    /// Fotogramas que llegaron a la pantalla por segundo. Es lo que **de
    /// verdad** vio el ojo: se cuenta en el vblank, no al encolar.
    pub fps: f32,
    /// Coste medio de un fotograma: componer la escena, dibujarla y encolarla.
    pub ms_total: f32,
    pub ms_escena: f32,
    pub ms_render: f32,
    /// El peor fotograma de la ventana. La media sola engaña: 4 ms de media con
    /// un pico de 40 se ve peor que 8 ms constantes.
    pub ms_peor: f32,
    /// Se evaluó el damage y no había nada que cambiar. En un escritorio quieto
    /// esto es lo normal y lo bueno.
    pub saltados: u32,
    /// Había damage y la salida tenía un fotograma en el aire, así que se dejó
    /// para el vblank siguiente. Un goteo es normal; muchos por segundo
    /// significan que el compositor no llega al ritmo del monitor.
    pub aplazados: u32,
    /// Fotogramas cuyo coste superó el periodo del modo: el vblank ya había
    /// pasado cuando estuvieron listos. Estos sí son fotogramas perdidos, no
    /// una estimación.
    pub perdidos: u32,
}

/// Todo lo que enseña el panel. Lo llena el compositor una vez por ventana.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Datos {
    /// Porcentaje de un núcleo que consume el **proceso del compositor**, no la
    /// máquina: es lo que se puede achacar a este código.
    pub cpu: f32,
    /// Ocupación de la GPU, cuando el driver la publica. `amdgpu` e `i915`
    /// reciente traen `gpu_busy_percent`; otros no, y entonces no se inventa.
    pub gpu: Option<f32>,
    pub salidas: Vec<Salida>,
}

/// Medidas de la tarjeta. El ancho es fijo para que los números no bailen de
/// una ventana a la siguiente al cambiar de tres a cuatro cifras.
const ANCHO: f32 = 430.0;
const MARGEN: f32 = 14.0;
/// Alto de una línea de texto pequeño, con su interlineado.
const LINEA: f32 = 17.0;
/// Líneas que ocupa cada salida: título, ritmo y desglose.
const LINEAS_SALIDA: f32 = 3.0;
/// Aire entre salidas.
const HUECO: f32 = 8.0;

pub struct Hud {
    pub datos: Datos,
}

impl Hud {
    pub fn new() -> Self {
        Self {
            datos: Datos::default(),
        }
    }

    /// Tamaño lógico de la tarjeta. Depende de cuántos monitores haya, así que
    /// el buffer se rehace cuando cambia — de eso se encarga quien la pinta.
    pub fn size(&self) -> (f32, f32) {
        let salidas = self.datos.salidas.len().max(1) as f32;
        let alto = MARGEN * 2.0 + LINEA + salidas * (LINEAS_SALIDA * LINEA + HUECO);
        (ANCHO, alto)
    }

    pub fn view<'a>(&self) -> PanelElement<'a> {
        let gpu = match self.datos.gpu {
            Some(v) => format!("GPU {v:.0} %"),
            // Sin `gpu_busy_percent` no hay forma barata de saberlo, y poner un
            // cero sería mentir.
            None => "GPU —".to_string(),
        };
        let cabecera = row![
            etiqueta(format!("CPU {:.1} %", self.datos.cpu), tema::tinta()),
            Space::new().width(Length::Fill),
            etiqueta(gpu, tema::tinta()),
        ];

        let mut cuerpo = column![cabecera].spacing(HUECO);
        if self.datos.salidas.is_empty() {
            cuerpo = cuerpo.push(etiqueta("sin salidas activas".to_string(), tema::TEXTO2));
        }
        for salida in &self.datos.salidas {
            cuerpo = cuerpo.push(self.bloque(salida));
        }

        container(cuerpo)
            .width(Length::Fixed(ANCHO))
            .padding(MARGEN)
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(fondo().into()),
                border: Border {
                    radius: tema::R_POPOVER.into(),
                    width: 1.0,
                    color: tema::alfa(tema::tinta(), 0.12),
                },
                ..Default::default()
            })
            .into()
    }

    fn bloque<'a>(&self, s: &Salida) -> PanelElement<'a> {
        let vrr = if s.vrr { " · VRR" } else { "" };
        let titulo = format!(
            "{} · {}×{} @{:.2} · {:.0} Hz{}",
            s.nombre, s.ancho, s.alto, s.escala, s.hz, vrr
        );
        // El fps se colorea contra el refresco del **modo**, no contra 60: en un
        // monitor de 144 Hz, 60 fps es la mitad de lo que puede dar.
        let color = ritmo(s);
        column![
            etiqueta(titulo, tema::TEXTO2),
            etiqueta(
                format!(
                    "{:.1} fps · {:.2} ms · peor {:.2} ms",
                    s.fps, s.ms_total, s.ms_peor
                ),
                color,
            ),
            etiqueta(
                format!(
                    "escena {:.2} · render {:.2} · salt {} · apl {} · perd {}",
                    s.ms_escena, s.ms_render, s.saltados, s.aplazados, s.perdidos
                ),
                tema::TEXTO2,
            ),
        ]
        .into()
    }
}

impl Default for Hud {
    fn default() -> Self {
        Self::new()
    }
}

/// Verde mientras se llega al refresco, ámbar por debajo, rojo si además se
/// pierden fotogramas. Sin fotogramas presentados no se pinta de rojo: un
/// escritorio quieto tiene 0 fps y está perfecto.
fn ritmo(s: &Salida) -> Color {
    if s.perdidos > 0 {
        return tema::rojo();
    }
    if s.fps <= 0.0 || s.hz <= 0.0 || s.fps >= s.hz * 0.9 {
        return tema::tinta();
    }
    tema::amarillo()
}

/// Monoespaciada a propósito: con proporcional, los dígitos cambian de ancho al
/// actualizarse y la columna entera se mueve cada segundo.
fn etiqueta<'a>(txt: String, color: Color) -> PanelElement<'a> {
    text(txt)
        .font(Font::MONOSPACE)
        .size(tema::T_PEQUENO)
        .color(color)
        .into()
}

/// Opaco, como el OSD y por lo mismo: sale sobre cualquier cosa y el contraste
/// lo tiene que poner él. Además aquí hay letra pequeña, que es lo primero que
/// se pierde sobre un fondo de pantalla con detalle.
fn fondo() -> Color {
    if tema::es_claro() {
        Color::WHITE
    } else {
        Color::BLACK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La tarjeta crece con los monitores: si no, el segundo se saldría del
    /// buffer y se vería cortado.
    #[test]
    fn el_alto_depende_de_cuantas_salidas_haya() {
        let mut hud = Hud::new();
        hud.datos.salidas = vec![Salida::default()];
        let (_, uno) = hud.size();
        hud.datos.salidas.push(Salida::default());
        let (_, dos) = hud.size();
        assert!(dos > uno, "con dos salidas tiene que ser más alto");
        assert_eq!(dos - uno, LINEAS_SALIDA * LINEA + HUECO);
    }

    /// Un escritorio quieto no está fallando: 0 fps sin fotogramas perdidos se
    /// pinta con la tinta normal, no en ámbar.
    #[test]
    fn quieto_no_es_lento() {
        let parado = Salida {
            hz: 60.0,
            ..Default::default()
        };
        assert_eq!(ritmo(&parado), tema::tinta());
        let lento = Salida {
            hz: 60.0,
            fps: 31.0,
            ..Default::default()
        };
        assert_eq!(ritmo(&lento), tema::amarillo());
        let perdiendo = Salida {
            hz: 60.0,
            fps: 59.0,
            perdidos: 4,
            ..Default::default()
        };
        assert_eq!(ritmo(&perdiendo), tema::rojo());
    }
}
