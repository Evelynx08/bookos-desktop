//! El aviso que asoma arriba a la derecha cuando llega una notificación.
//!
//! Es la otra mitad de las notificaciones: la tarjeta del panel guarda lo que
//! ha pasado, y esto es lo que se ve **mientras** pasa. Sin él, `notify-send`
//! parece no hacer nada hasta que abres la campana.
//!
//! Comparte forma con [`crate::osd`] —una superficie temporal que entra, se
//! queda y se va sola, sin repintarse mientras está quieta— pero no comparte
//! código: el aviso de volumen es una cápsula centrada abajo con una barra, y
//! este es una tarjeta arriba a la derecha con tres renglones. Lo único que de
//! verdad tienen igual son las curvas, y esas ya están en [`crate::tema`].

use std::time::{Duration, Instant};

use iced_core::alignment::Vertical;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::notificaciones::Notificacion;
use crate::tema;
use crate::view::PanelElement;

/// Ancho de la tarjeta. Más estrecha que las del panel: es un aviso de paso, no
/// algo que se lea entero.
const ANCHO: f32 = 340.0;
const ALTO: f32 = 76.0;
const ALTO_ACCIONES: f32 = 112.0;
const MARGEN: f32 = 14.0;
/// Lado del icono de la aplicación.
const ICONO: f32 = 32.0;
/// Hueco para la sombra dentro del buffer, igual que en el aviso de volumen.
pub const MARGEN_SOMBRA: f32 = 18.0;
/// Separación desde el borde derecho y desde el panel.
pub const MARGEN_LATERAL: f32 = 12.0;
pub const MARGEN_SUPERIOR: f32 = 8.0;

/// Lo que se queda a la vista una normal, y una crítica.
///
/// Cinco segundos es lo que tarda en leerse un renglón y medio sin prisa; el
/// doble para las críticas, que son las que no conviene perderse. Una
/// aplicación puede pedir otra cosa con `expire_timeout`, y se le respeta
/// dentro de estos límites: hay programas que piden treinta segundos, y un
/// aviso que no se va en medio minuto deja de ser un aviso.
const QUIETO: Duration = Duration::from_secs(5);
const QUIETO_CRITICA: Duration = Duration::from_secs(10);
const MINIMO: Duration = Duration::from_secs(2);
const MAXIMO: Duration = Duration::from_secs(15);
/// Lo que tarda en entrar y en irse. Los mismos tokens que el resto.
const ENTRADA: Duration = tema::D_POPOVER;
pub const SALIDA: Duration = Duration::from_millis(280);

pub struct Toast {
    notificacion: Notificacion,
    /// Cuánto se queda quieto antes de empezar a irse.
    quieto: Duration,
    desde: Instant,
}

impl Toast {
    /// `caducidad` es el `expire_timeout` de D-Bus: negativo «lo que decida el
    /// servidor», cero «que no se vaya sola» y positivo, milisegundos.
    pub fn new(notificacion: Notificacion, caducidad: i32) -> Self {
        let por_defecto = if notificacion.critica {
            QUIETO_CRITICA
        } else {
            QUIETO
        };
        let quieto = match caducidad {
            // El «que no se vaya» se atiende como el máximo y no como para
            // siempre: un aviso clavado en la esquina que solo se quita
            // pulsándolo es lo que hace que la gente desactive las
            // notificaciones. Lo que pasó sigue en la tarjeta del panel.
            0 => MAXIMO,
            ms if ms > 0 => Duration::from_millis(ms as u64).clamp(MINIMO, MAXIMO),
            _ => por_defecto,
        };
        Self {
            notificacion,
            quieto,
            desde: Instant::now(),
        }
    }

    pub fn id(&self) -> u32 {
        self.notificacion.id
    }

    /// Cuánto queda para que desaparezca del todo. `None` si ya se fue.
    pub fn queda(&self) -> Option<Duration> {
        (self.quieto + SALIDA).checked_sub(self.desde.elapsed())
    }

    /// Lo empuja a irse ya: es lo que hace pulsarlo.
    pub fn descartar(&mut self) {
        // Se le deja la salida por delante en vez de quitarlo de golpe, para
        // que se vaya con la misma animación que si hubiera caducado.
        self.desde = Instant::now() - self.quieto;
    }

    pub fn alfa(&self) -> f32 {
        let t = self.desde.elapsed();
        if t < self.quieto {
            return tema::C_ENTRADA.eval(tema::fraccion(t, ENTRADA));
        }
        1.0 - tema::C_SUAVE.eval(tema::fraccion(t - self.quieto, SALIDA))
    }

    /// Entra creciendo un poco, como los popovers.
    pub fn escala(&self) -> f32 {
        let t = self.desde.elapsed();
        if t >= ENTRADA {
            return 1.0;
        }
        0.94 + 0.06 * tema::C_MUELLE_POPOVER.eval(tema::fraccion(t, ENTRADA))
    }

    /// Solo mientras entra: lo demás es alfa, y de eso se encarga el compositor
    /// con un único despertar programado.
    pub fn animando(&self) -> bool {
        self.desde.elapsed() < ENTRADA
    }

    pub fn size(&self) -> (f32, f32) {
        let alto = if self.notificacion.acciones.is_empty() {
            ALTO
        } else {
            ALTO_ACCIONES
        };
        (ANCHO + MARGEN_SOMBRA * 2.0, alto + MARGEN_SOMBRA * 2.0)
    }

    /// Devuelve la clave de la acción pulsada, en coordenadas relativas a la
    /// tarjeta (sin el margen reservado para la sombra).
    pub fn accion_en(&self, x: f32, y: f32) -> Option<String> {
        if self.notificacion.acciones.is_empty() || !(0.0..=ANCHO).contains(&x) {
            return None;
        }
        let y0 = ALTO_ACCIONES - 30.0;
        if !(y0..=ALTO_ACCIONES).contains(&y) {
            return None;
        }
        let hueco = 6.0;
        let total = self.notificacion.acciones.len() as f32;
        let ancho = (ANCHO - 2.0 * MARGEN - hueco * (total - 1.0)) / total;
        let i = ((x - MARGEN) / (ancho + hueco)) as usize;
        if i >= self.notificacion.acciones.len() {
            return None;
        }
        let local = x - MARGEN - i as f32 * (ancho + hueco);
        (0.0..=ancho)
            .contains(&local)
            .then(|| self.notificacion.acciones[i].clave.clone())
    }

    pub fn view(&self) -> PanelElement<'_> {
        let n = &self.notificacion;
        let dibujo: PanelElement<'_> = match &n.icono {
            Some(ic) => crate::icono::ver(ic, ICONO, ICONO),
            None => Space::new().width(Length::Fixed(ICONO)).into(),
        };
        let ancho_texto = ANCHO - MARGEN * 2.0 - ICONO - 12.0;
        let mut textos = column![
            // Quién avisa. En rojo si es crítica: es lo único que la distingue
            // de las demás sin meterle un fondo de color que taparía su icono.
            text(n.app.clone()).size(11.0).color(if n.critica {
                tema::rojo()
            } else {
                tema::TEXTO2
            }),
            text(crate::emergente::recortar_texto(
                &n.resumen,
                ancho_texto,
                14.0
            ))
            .size(14.0)
            .color(tema::texto()),
        ];
        if !n.cuerpo.is_empty() {
            textos = textos.push(
                text(crate::emergente::recortar_texto(
                    &n.cuerpo,
                    ancho_texto,
                    11.0,
                ))
                .size(11.0)
                .color(tema::TEXTO2),
            );
        }

        let cuerpo = row![
            dibujo,
            Space::new().width(Length::Fixed(12.0)),
            // Ancho fijo: sin él, iced parte el resumen en dos líneas y la
            // tarjeta se desborda por abajo.
            container(textos).width(Length::Fixed(ancho_texto)),
        ]
        .align_y(Vertical::Center);
        let mut contenido = column![cuerpo];
        if let Some(valor) = n.progreso {
            let lleno = (ancho_texto * valor as f32 / 100.0).max(2.0);
            contenido = contenido.push(
                container(
                    container(Space::new())
                        .width(Length::Fixed(lleno))
                        .height(Length::Fixed(4.0))
                        .style(|_| iced_widget::container::Style {
                            background: Some(tema::acento().into()),
                            ..Default::default()
                        }),
                )
                .width(Length::Fixed(ancho_texto))
                .height(Length::Fixed(4.0))
                .style(|_| container::Style {
                    background: Some(tema::surco().into()),
                    border: Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        if n.progreso_indeterminado && n.progreso.is_none() {
            contenido = contenido.push(text("En curso…").size(10.0).color(tema::TEXTO2));
        }
        if !n.acciones.is_empty() {
            let botones = n.acciones.iter().fold(row![], |fila, accion| {
                fila.push(
                    container(
                        text(accion.etiqueta.clone())
                            .size(11.0)
                            .color(tema::sobre_acento()),
                    )
                    .padding([4, 8])
                    .style(|_| container::Style {
                        background: Some(tema::acento().into()),
                        border: Border {
                            radius: tema::R_BOTON_PEQUENO.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                )
                .push(Space::new().width(Length::Fixed(6.0)))
            });
            contenido = contenido.push(botones);
        }
        let alto = if n.acciones.is_empty() {
            ALTO
        } else {
            ALTO_ACCIONES
        };
        let tarjeta = container(contenido)
            .width(Length::Fixed(ANCHO))
            .height(Length::Fixed(alto))
            .center_y(Length::Fixed(alto))
            .padding([0, MARGEN as u16])
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_POPOVER.into(),
                    width: 1.0,
                    color: tema::borde(),
                },
                // La misma sombra que el aviso de volumen: el toast también flota
                // sobre lo que haya, y sin ella se pega al fondo de pantalla.
                shadow: iced_core::Shadow {
                    color: Color {
                        a: 0.35,
                        ..Color::BLACK
                    },
                    offset: iced_core::Vector::new(0.0, 8.0),
                    blur_radius: 24.0,
                },
                ..Default::default()
            });
        container(tarjeta).padding(MARGEN_SOMBRA).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notif(critica: bool) -> Notificacion {
        Notificacion::nueva(
            1,
            "Prueba".into(),
            "Resumen".into(),
            String::new(),
            "",
            critica,
        )
    }

    /// La caducidad que pide la aplicación se respeta, pero acotada: hay
    /// programas que piden medio minuto, y eso deja de ser un aviso.
    #[test]
    fn la_caducidad_pedida_se_acota() {
        assert_eq!(
            Toast::new(notif(false), 3000).quieto,
            Duration::from_secs(3)
        );
        assert_eq!(Toast::new(notif(false), 60_000).quieto, MAXIMO);
        assert_eq!(Toast::new(notif(false), 500).quieto, MINIMO);
        // «Lo que decida el servidor» y «que no se vaya sola».
        assert_eq!(Toast::new(notif(false), -1).quieto, QUIETO);
        assert_eq!(Toast::new(notif(true), -1).quieto, QUIETO_CRITICA);
        assert_eq!(Toast::new(notif(false), 0).quieto, MAXIMO);
    }

    /// Entra, se queda y se va; y pulsarlo lo manda a irse sin quitarlo de
    /// golpe, para que la salida se vea.
    #[test]
    fn entra_y_al_descartarlo_se_va_con_su_animacion() {
        let mut t = Toast::new(notif(false), -1);
        assert!(
            t.alfa() < 1.0 && t.escala() < 1.0,
            "tiene que estar entrando"
        );
        assert!(t.animando());
        std::thread::sleep(ENTRADA);
        assert_eq!(t.alfa(), 1.0);
        assert_eq!(t.escala(), 1.0);
        assert!(!t.animando());

        t.descartar();
        let queda = t.queda().expect("todavía se está yendo");
        assert!(
            queda <= SALIDA,
            "la salida tiene que quedar por delante: {queda:?}"
        );
        // Justo al descartarlo el alfa sigue entero —la salida empieza ahí—; lo
        // que importa es que a partir de ese momento baje.
        assert_eq!(t.alfa(), 1.0);
        std::thread::sleep(SALIDA / 2);
        assert!(t.alfa() < 1.0, "no se está yendo");
    }

    #[test]
    fn las_acciones_amplian_el_toast_y_devuelven_su_clave() {
        let t = Toast::new(
            Notificacion::nueva_con_datos(
                7,
                "Prueba".into(),
                "Resumen".into(),
                String::new(),
                "",
                false,
                vec![crate::notificaciones::Accion {
                    clave: "abrir".into(),
                    etiqueta: "Abrir".into(),
                }],
                Some(42),
                false,
            ),
            -1,
        );
        assert!(t.size().1 > ALTO + MARGEN_SOMBRA * 2.0);
        assert_eq!(
            t.accion_en(80.0, ALTO_ACCIONES - 10.0).as_deref(),
            Some("abrir")
        );
    }
}
