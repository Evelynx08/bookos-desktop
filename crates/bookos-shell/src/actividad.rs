//! La isla de actividades vivas de las aplicaciones de sistema.
//!
//! No es una notificación: permanece mientras la tarea siga viva, tiene
//! controles y solo recibe estado del contrato privado de BookOS. La forma se
//! pinta en CPU una vez; la entrada y el cambio de estado los compone la GPU.
//!
//! ## Cómo está construida la cara
//!
//! - **Negro, no gris.** La isla usa el fondo de pantalla completa
//!   ([`tema::bg`]) y no el de tarjeta: colgando del borde superior tiene que
//!   leerse como una prolongación del marco de la pantalla, no como una
//!   ventanita. El temporizador es la excepción y va **relleno de color**,
//!   porque es el único que reclama atención por sí mismo.
//! - **La geometría vive en constantes, no en el árbol de widgets.** Las zonas
//!   sensibles al clic se calculan a mano en [`Actividad::pulsar`] —aquí no hay
//!   eventos de iced, el compositor entrega un punto— así que un `padding` que
//!   se cambia en la vista y no en la hitbox deja el botón donde no se ve. Con
//!   las medidas en un solo sitio, mover un control mueve las dos cosas.
//! - **Los pictogramas son SVG, no glifos.** Estaban escritos como texto
//!   («‹‹», «Ⅱ», «♥»), y eso depende de que la fuente del sistema los tenga:
//!   medido, el corazón de «favorita» no se dibujaba **nada** en esta máquina y
//!   la fila de la cola salía vacía en ese hueco.
//! - **La portada manda en el color.** De ella sale el color de los controles,
//!   así que la isla de cada disco se ve distinta sin que la aplicación mande
//!   ningún color. Se calcula una vez al recibir el estado, no por fotograma.

use std::time::Instant;

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::font::{Font, Weight};
use iced_core::gradient::Linear;
use iced_core::{Background, Border, Color, Gradient, Length, Radians, Shadow, Vector};
use iced_widget::{column, container, image, row, stack, text, Space};

use crate::icono;
use crate::tema;
use crate::view::PanelElement;

/// Aire alrededor de la tarjeta para que quepa la sombra. El compositor lo
/// resta al colocar la superficie, así que subirlo no mueve la isla.
pub const MARGEN_SOMBRA: f32 = 26.0;
/// Aire entre el final del panel y la actividad. La posición absoluta la
/// calcula el compositor con la altura real del panel, también en HiDPI.
pub const SEPARACION_PANEL: f32 = 10.0;

// ── Geometría ────────────────────────────────────────────────────────────
// Todas las medidas del interior de la tarjeta. Ver la nota del módulo: estas
// constantes las comparten la vista y las zonas de clic.

const COMPACTO_W: f32 = 380.0;
const COMPACTO_H: f32 = 72.0;
/// El estado cerrado es una tarjeta redondeada, no una píldora. Mantener el
/// radio por debajo de la mitad de su alto deja laterales verticales visibles.
const RADIO_COMPACTO: f32 = 22.0;
/// La píldora del temporizador es más corta: solo lleva el reloj y la cuenta.
/// Tiene que estar en `size` y no solo en la vista, porque el compositor centra
/// **el buffer** en la pantalla y una tarjeta más estrecha dentro de un buffer
/// ancho se vería descentrada.
const TIMER_COMPACTO_W: f32 = 236.0;
const ABIERTO_W: f32 = 440.0;
/// Margen interior de las vistas abiertas.
const PAD: f32 = 22.0;

/// Reproductor abierto, de arriba abajo.
const ARTE: f32 = 66.0;
const CAB_H: f32 = ARTE;
const CTRL_Y: f32 = PAD + CAB_H + 26.0;
const CTRL_H: f32 = 56.0;
/// Los cinco controles: aleatorio, anterior, reproducir, siguiente, repetir.
const CTRL_PLAY: f32 = 56.0;
const CTRL_LADO: f32 = 44.0;
const CTRL_PLANO: f32 = 26.0;
const CTRL_SEP: f32 = 12.0;
/// Fila del progreso: tiempo, barra, tiempo, cola y volumen.
const BARRA_Y: f32 = CTRL_Y + CTRL_H + 22.0;
const BARRA_H: f32 = 26.0;
const BARRA_GRUESO: f32 = 8.0;
/// Ancho reservado a cada rótulo de tiempo dentro de esa fila.
const TIEMPO_W: f32 = 42.0;
const ICONO_FILA: f32 = 24.0;
const PLAYER_H: f32 = BARRA_Y + BARRA_H + PAD;

/// La cola cuelga por debajo del reproductor, dentro de la misma tarjeta.
const COLA_SEP: f32 = 10.0;
const COLA_CAB_H: f32 = 30.0;
const COLA_FILA_H: f32 = 60.0;
const COLA_PAD: f32 = 14.0;
const COLA_MAX: usize = 4;

/// Temporizador abierto.
const TIMER_CAB_H: f32 = 52.0;
const TIMER_AVISO_H: f32 = 20.0;
const TIMER_BOTON_Y: f32 = PAD + TIMER_CAB_H + 10.0 + TIMER_AVISO_H + 16.0;
const BOTON_H: f32 = 48.0;
const TIMER_H: f32 = TIMER_BOTON_Y + BOTON_H + PAD;

/// Grabadora abierta.
const REC_CAB_H: f32 = 28.0;
const REC_ONDAS_H: f32 = 46.0;
const REC_BOTON_Y: f32 = PAD + REC_CAB_H + 16.0 + REC_ONDAS_H + 20.0;
const REC_BOTON: f32 = 56.0;
const RECORDER_H: f32 = REC_BOTON_Y + REC_BOTON + PAD;

/// Radio de las tarjetas abiertas. Más generoso que el de una tarjeta normal:
/// la referencia del sistema de diseño para la isla es casi una píldora.
const RADIO: f32 = 30.0;

/// Las ondas son un indicador ambiental, no una visualización de audio de
/// precisión. Redibujar y subir el bitmap completo de la tarjeta a 120 Hz
/// hacía competir a iced con el cursor. A 12,5 Hz conservan movimiento en la
/// píldora compacta sin monopolizar el hilo de entrada en builds de desarrollo.
const FRAME_ONDAS: std::time::Duration = std::time::Duration::from_millis(80);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clase {
    Player,
    Timer,
    Recorder,
}

#[derive(Debug, Clone)]
pub struct Portada {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct ItemCola {
    pub id: String,
    pub titulo: String,
    pub artista: String,
    /// Duración de la pista. `0` si la aplicación no la manda: entonces no se
    /// enseña, en vez de escribir un «00:00» que no significa nada.
    pub duracion_ms: i64,
    pub favorita: bool,
    pub actual: bool,
}

#[derive(Debug, Clone)]
pub struct Estado {
    pub app_id: String,
    pub clase: Clase,
    pub activo: bool,
    pub pausado: bool,
    pub titulo: String,
    pub subtitulo: String,
    pub posicion_ms: i64,
    pub duracion_ms: i64,
    pub restante_ms: i64,
    pub volumen: u8,
    pub nivel: f32,
    /// Reproducción aleatoria y repetición, para poder pintar encendidos sus
    /// dos botones. Son estado de la aplicación: el compositor no los decide.
    pub aleatorio: bool,
    pub repetir: bool,
    pub portada: Option<Portada>,
    pub cola: Vec<ItemCola>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accion {
    pub app_id: String,
    pub nombre: String,
    pub valor: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Vista {
    Compacta,
    Abierta,
    Cola,
}

pub struct Actividad {
    estado: Estado,
    vista: Vista,
    volumen_visible: bool,
    animaciones: bool,
    desde: Instant,
    ultimo_frame_ondas: Instant,
    /// La portada con las esquinas ya recortadas y el color que la resume.
    ///
    /// Las dos cosas se calculan al recibir el estado y no al dibujar: recortar
    /// son 90 000 píxeles por portada, y hacerlo en cada fotograma de una
    /// animación de 220 ms es pagarlo trece veces para ver lo mismo.
    arte: Option<Portada>,
    tinte: Option<Color>,
}

impl Actividad {
    pub fn nueva(estado: Estado, animaciones: bool) -> Self {
        let (arte, tinte) = preparar_portada(estado.portada.as_ref());
        Self {
            estado,
            vista: Vista::Compacta,
            volumen_visible: false,
            animaciones,
            desde: Instant::now(),
            ultimo_frame_ondas: Instant::now() - FRAME_ONDAS,
            arte,
            tinte,
        }
    }

    pub fn actualizar(&mut self, estado: Estado) {
        // La portada solo se vuelve a preparar si de verdad cambió: un
        // reproductor manda estado cada segundo para mover la barra de
        // progreso, y con la misma carátula.
        let cambio = match (&self.estado.portada, &estado.portada) {
            (Some(a), Some(b)) => a.width != b.width || a.height != b.height || a.rgba != b.rgba,
            (None, None) => false,
            _ => true,
        };
        if cambio {
            let (arte, tinte) = preparar_portada(estado.portada.as_ref());
            self.arte = arte;
            self.tinte = tinte;
        }
        self.estado = estado;
    }

    pub fn app_id(&self) -> &str { &self.estado.app_id }
    pub fn clase(&self) -> Clase { self.estado.clase }
    pub fn set_animaciones(&mut self, animaciones: bool) {
        self.animaciones = animaciones;
    }

    /// Abre la tarjeta sin fingir una pulsación. Solo lo usa el contrato de
    /// previsualización del compositor en builds de desarrollo.
    pub fn abrir_previsualizacion(&mut self) -> bool {
        if self.vista != Vista::Compacta { return false; }
        self.vista = Vista::Abierta;
        self.desde = Instant::now();
        true
    }

    fn filas_cola(&self) -> usize {
        self.estado.cola.len().min(COLA_MAX)
    }

    /// Alto del panel de la cola colgando del reproductor, incluida su
    /// separación. Cero si no hay cola que enseñar.
    fn alto_cola(&self) -> f32 {
        let n = self.filas_cola();
        if n == 0 { return 0.0; }
        COLA_SEP + COLA_PAD * 2.0 + COLA_CAB_H + COLA_FILA_H * n as f32
    }

    pub fn size(&self) -> (f32, f32) {
        let (w, h) = match self.vista {
            Vista::Compacta => (
                if self.estado.clase == Clase::Timer { TIMER_COMPACTO_W } else { COMPACTO_W },
                COMPACTO_H,
            ),
            Vista::Abierta | Vista::Cola => (
                ABIERTO_W,
                match self.estado.clase {
                    Clase::Player if self.vista == Vista::Cola => PLAYER_H + self.alto_cola(),
                    Clase::Player => PLAYER_H,
                    Clase::Timer => TIMER_H,
                    Clase::Recorder => RECORDER_H,
                },
            ),
        };
        (w + MARGEN_SOMBRA * 2.0, h + MARGEN_SOMBRA * 2.0)
    }

    pub fn animando(&self) -> bool {
        self.animaciones && (self.desde.elapsed() < tema::D_TARJETA
            || (self.vista == Vista::Compacta
                && self.sonando()
                && matches!(self.estado.clase, Clase::Player | Clase::Recorder)))
    }

    /// Si toca reconstruir el bitmap por el siguiente paso del vúmetro.
    /// La animación de entrada no entra aquí: alfa, escala y desplazamiento se
    /// aplican a la textura ya pintada desde el compositor/GPU.
    pub fn ondas_pendientes(&self) -> bool {
        self.animaciones
            && self.vista == Vista::Compacta
            && self.sonando()
            && matches!(self.estado.clase, Clase::Player | Clase::Recorder)
            && self.ultimo_frame_ondas.elapsed() >= FRAME_ONDAS
    }

    pub fn marcar_ondas_pintadas(&mut self) {
        self.ultimo_frame_ondas = Instant::now();
    }

    pub fn entrada(&self) -> (f32, f32, f32) {
        if !self.animaciones { return (1.0, 1.0, 0.0); }
        let p = tema::C_MUELLE_POPOVER.eval(tema::fraccion(
            self.desde.elapsed(),
            tema::D_TARJETA,
        ));
        let alfa = tema::C_ENTRADA.eval(tema::fraccion(
            self.desde.elapsed(),
            tema::D_TARJETA,
        ));
        (alfa, 0.94 + 0.06 * p, -18.0 * (1.0 - p))
    }

    /// La columna del volumen se despliega al pasar por encima de su botón, en
    /// la esquina inferior derecha.
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let antes = self.volumen_visible;
        self.volumen_visible = self.vista != Vista::Compacta
            && self.estado.clase == Clase::Player
            && punto.is_some_and(|(x, y)| {
                let (x, y) = (x - MARGEN_SOMBRA, y - MARGEN_SOMBRA);
                let x_min = ABIERTO_W - PAD - ICONO_FILA - 10.0;
                if antes {
                    // Una vez abierto admite algo de desviación lateral y por
                    // encima. Sin esta histéresis, alcanzar el 100 % exigía
                    // no sobrepasar ni una fracción de píxel el extremo.
                    x > x_min - VOL_TOLERANCIA_X
                        && x < ABIERTO_W - PAD + VOL_TOLERANCIA_X
                        && y > VOL_BASE_Y - VOL_ALTO - VOL_TOLERANCIA_Y
                        && y < BARRA_Y + BARRA_H + VOL_TOLERANCIA_Y
                } else {
                    // Cerrado solo responde al icono; así pasar por la zona
                    // vacía de encima no hace aparecer controles por sorpresa.
                    x > x_min
                        && y > BARRA_Y - 4.0
                        && y < BARRA_Y + BARRA_H + 4.0
                }
            });
        antes != self.volumen_visible
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let x = x - MARGEN_SOMBRA;
        let y = y - MARGEN_SOMBRA;
        if x < 0.0 || y < 0.0 { return None; }
        if self.vista == Vista::Compacta {
            self.vista = Vista::Abierta;
            self.desde = Instant::now();
            return None;
        }
        match self.estado.clase {
            Clase::Player => self.pulsar_player(x, y),
            Clase::Timer => {
                // La cabecera entera cierra: con una tarjeta sin chevron, es lo
                // que queda por descubrir y no hay nada más ahí que pulsar.
                if y < TIMER_BOTON_Y {
                    self.colapsar();
                    return None;
                }
                let nombre = if x < ABIERTO_W / 2.0 {
                    if self.estado.restante_ms < 0 { "snooze" }
                    else if self.estado.pausado { "resume" } else { "pause" }
                } else { "stop" };
                Some(self.accion(nombre, String::new()))
            }
            Clase::Recorder => {
                if y < REC_BOTON_Y {
                    self.colapsar();
                    return None;
                }
                let nombre = if x < ABIERTO_W / 2.0 { "stop" }
                    else { if self.estado.pausado { "resume" } else { "pause" } };
                Some(self.accion(nombre, String::new()))
            }
        }
    }

    fn colapsar(&mut self) {
        self.vista = Vista::Compacta;
        self.volumen_visible = false;
        self.desde = Instant::now();
    }

    fn pulsar_player(&mut self, x: f32, y: f32) -> Option<Accion> {
        // Volumen desplegado: la columna se lee de abajo arriba y se queda con
        // el clic antes que nada de lo que tenga debajo.
        if self.volumen_visible
            && x > ABIERTO_W - PAD - ICONO_FILA - 10.0 - VOL_TOLERANCIA_X
            && y > VOL_BASE_Y - VOL_ALTO - VOL_TOLERANCIA_Y
            && y < VOL_BASE_Y
        {
            let value = (((VOL_BASE_Y - y) / VOL_ALTO) * 100.0).round().clamp(0.0, 100.0);
            return Some(self.accion("volume", format!("{value:.0}")));
        }
        if y < CTRL_Y {
            self.colapsar();
            return None;
        }
        if y < CTRL_Y + CTRL_H {
            // Los cinco controles, medidos desde el centro hacia fuera con los
            // mismos anchos y separaciones con que se dibujan.
            let centro = ABIERTO_W / 2.0;
            let d = (x - centro).abs();
            let mitad_play = CTRL_PLAY / 2.0;
            let fin_lado = mitad_play + CTRL_SEP + CTRL_LADO;
            let fin_plano = fin_lado + CTRL_SEP + CTRL_PLANO;
            let nombre = if d <= mitad_play { "play-pause" }
                else if d <= fin_lado { if x < centro { "previous" } else { "next" } }
                else if d <= fin_plano { if x < centro { "shuffle" } else { "repeat" } }
                else { return None };
            return Some(self.accion(nombre, String::new()));
        }
        if y < BARRA_Y + BARRA_H {
            let cola_x = ABIERTO_W - PAD - ICONO_FILA * 2.0 - 12.0;
            if x >= ABIERTO_W - PAD - ICONO_FILA - 10.0 { return None; }
            if x >= cola_x {
                self.vista = if self.vista == Vista::Cola { Vista::Abierta } else { Vista::Cola };
                self.desde = Instant::now();
                return None;
            }
            let ini = PAD + TIEMPO_W + 10.0;
            let fin = cola_x - TIEMPO_W - 14.0;
            let value = ((x - ini) / (fin - ini)).clamp(0.0, 1.0);
            return Some(self.accion("seek", format!("{value:.4}")));
        }
        // Por debajo del reproductor está el panel de la cola.
        let y = y - (PLAYER_H + COLA_SEP + COLA_PAD + COLA_CAB_H);
        if y < 0.0 { return None; }
        let i = (y / COLA_FILA_H) as usize;
        let item = self.estado.cola.get(i)?;
        let id = item.id.clone();
        let nombre = if x > ABIERTO_W - COLA_PAD - 40.0 { "favorite" } else { "queue-goto" };
        Some(self.accion(nombre, id))
    }

    fn accion(&self, raw: &str, fallback: String) -> Accion {
        let (nombre, valor) = raw.split_once('\u{1f}')
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .unwrap_or_else(|| (raw.to_string(), fallback));
        Accion { app_id: self.estado.app_id.clone(), nombre, valor }
    }

    // ── Vista ────────────────────────────────────────────────────────────

    pub fn view(&self) -> PanelElement<'_> {
        let (w, h) = self.size();
        let tarjeta = match self.vista {
            Vista::Compacta => self.compacta(),
            _ => self.abierta(),
        };
        container(tarjeta)
            .width(Length::Fixed(w))
            .height(Length::Fixed(h))
            .padding(MARGEN_SOMBRA)
            .into()
    }

    /// El color de los controles. La portada manda; sin portada, el acento del
    /// sistema. La grabadora es siempre roja: es lo que significa grabar.
    fn color(&self) -> Color {
        match self.estado.clase {
            Clase::Recorder => tema::acento(),
            Clase::Timer => tema::acento(),
            Clase::Player => self.tinte.unwrap_or_else(tema::acento),
        }
    }

    fn sonando(&self) -> bool {
        self.estado.activo && !self.estado.pausado
    }

    fn compacta(&self) -> PanelElement<'_> {
        let color = self.color();
        match self.estado.clase {
            Clase::Player => tarjeta(
                self.cabecera_player(48.0),
                COMPACTO_W, COMPACTO_H, RADIO_COMPACTO, Relleno::Negro, [12, 18],
            ),
            // El temporizador va relleno de color y sin más adornos: un reloj
            // blanco y la cuenta atrás, que es lo único que importa de él.
            Clase::Timer => {
                tarjeta(
                    row![
                        reloj_blanco(40.0, color),
                        text(formato_tiempo(self.estado.restante_ms))
                            .size(30).font(gorda()).color(Color::WHITE),
                        Space::new().width(Length::Fill),
                    ].spacing(14).align_y(Vertical::Center).into(),
                    TIMER_COMPACTO_W, COMPACTO_H, RADIO_COMPACTO,
                    Relleno::Color(color),
                    [12, 16],
                )
            }
            Clase::Recorder => tarjeta(
                row![
                    punto(self.sonando(), tema::rojo(), 14.0),
                    text(if self.estado.pausado { "En pausa" } else { "Grabando" })
                        .size(16).font(negrita()),
                    Space::new().width(Length::Fill),
                    ondas(self.estado.nivel, self.sonando(), tema::rojo(), 30.0, 17, 5.0),
                ].spacing(14).align_y(Vertical::Center).into(),
                COMPACTO_W, COMPACTO_H, RADIO_COMPACTO, Relleno::Negro, [12, 20],
            ),
        }
    }

    fn abierta(&self) -> PanelElement<'_> {
        let color = self.color();
        let (w, h) = self.size();
        let (w, h) = (w - MARGEN_SOMBRA * 2.0, h - MARGEN_SOMBRA * 2.0);
        match self.estado.clase {
            Clase::Player => {
                let mut cuerpo = column![
                    container(self.cabecera_player(ARTE)).height(Length::Fixed(CAB_H)),
                    Space::new().height(Length::Fixed(CTRL_Y - PAD - CAB_H)),
                    self.controles(color),
                    Space::new().height(Length::Fixed(BARRA_Y - CTRL_Y - CTRL_H)),
                    self.fila_progreso(color),
                ];
                if self.vista == Vista::Cola && self.filas_cola() > 0 {
                    cuerpo = cuerpo
                        .push(Space::new().height(Length::Fixed(COLA_SEP + PAD)))
                        .push(self.panel_cola(color));
                }
                // El panel de la cola llega hasta los bordes de la tarjeta, así
                // que el margen inferior lo pone él y no el contenedor.
                let abajo = if self.vista == Vista::Cola && self.filas_cola() > 0 { 0.0 } else { PAD };
                // El volumen es una capa flotante. Dentro de `fila_progreso`
                // quedaba limitado a los 26 px de alto de la fila y el stack
                // lo recortaba, aunque el estado de hover sí cambiase.
                let cuerpo = stack![
                    cuerpo,
                    container(self.volumen_vertical(color))
                        .width(Length::Fill)
                        // Acaba al comenzar la fila: queda encima del icono,
                        // no montado sobre él.
                        .height(Length::Fixed(BARRA_Y - PAD))
                        .align_x(Horizontal::Right)
                        .align_y(Vertical::Bottom),
                ];
                tarjeta_v(cuerpo.into(), w, h, RADIO, Relleno::Negro, [PAD, PAD, abajo, PAD])
            }
            Clase::Timer => {
                let vencido = self.estado.restante_ms < 0;
                let fondo = color;
                let cuerpo = column![
                    container(row![
                        reloj_blanco(48.0, fondo),
                        text(formato_tiempo(self.estado.restante_ms))
                            .size(40).font(gorda()).color(Color::WHITE),
                        Space::new().width(Length::Fill),
                    ].spacing(16).align_y(Vertical::Center))
                        .height(Length::Fixed(TIMER_CAB_H)),
                    Space::new().height(Length::Fixed(10.0)),
                    container(
                        text(if vencido { "¡El tiempo se ha acabado!" }
                             else if self.estado.pausado { "Temporizador en pausa" }
                             else { "Temporizador en marcha" })
                            .size(13).color(Color { a: 0.85, ..Color::WHITE }),
                    ).width(Length::Fill).height(Length::Fixed(TIMER_AVISO_H))
                        .align_x(Horizontal::Center),
                    Space::new().height(Length::Fixed(16.0)),
                    row![
                        boton_sobre_color(
                            if vencido { "Aplazar" } else if self.estado.pausado { "Reanudar" } else { "Pausar" },
                            fondo, false,
                        ),
                        boton_sobre_color("Parar", fondo, true),
                    ].spacing(12),
                ];
                tarjeta(cuerpo.into(), w, h, RADIO, Relleno::Color(fondo), [PAD as u16, PAD as u16])
            }
            Clase::Recorder => {
                let cuerpo = column![
                    container(row![
                        punto(self.sonando(), tema::rojo(), 14.0),
                        text(if self.estado.pausado { "En pausa" } else { "Grabando" })
                            .size(17).font(negrita()),
                        Space::new().width(Length::Fill),
                        text(formato_tiempo(self.estado.posicion_ms)).size(17).font(negrita()),
                    ].spacing(13).align_y(Vertical::Center))
                        .height(Length::Fixed(REC_CAB_H)),
                    Space::new().height(Length::Fixed(16.0)),
                    container(ondas(self.estado.nivel, self.sonando(), tema::rojo(), REC_ONDAS_H, 29, 5.0))
                        .width(Length::Fill).height(Length::Fixed(REC_ONDAS_H))
                        .align_x(Horizontal::Center),
                    Space::new().height(Length::Fixed(20.0)),
                    container(row![
                        redondo(icono_inline(STOP), REC_BOTON, Fondo::Solido, color),
                        redondo(
                            icono_propio(if self.estado.pausado { "reproducir" } else { "pausa" }),
                            REC_BOTON, Fondo::Solido, color,
                        ),
                    ].spacing(18)).width(Length::Fill).align_x(Horizontal::Center),
                ];
                tarjeta(cuerpo.into(), w, h, RADIO, Relleno::Negro, [PAD as u16, PAD as u16])
            }
        }
    }

    fn cabecera_player(&self, lado: f32) -> PanelElement<'_> {
        let color = self.color();
        let grande = lado > 50.0;
        row![
            caratula(self.arte.as_ref(), lado, color),
            column![
                text(self.estado.titulo.clone())
                    .size(if grande { 17 } else { 15 }).font(negrita()),
                text(self.estado.subtitulo.clone())
                    .size(if grande { 14 } else { 13 }).color(tema::TEXTO2),
            ].spacing(4).width(Length::Fill),
            // El ecualizador: cinco barras gruesas, no un vúmetro fino. Es el
            // adorno que dice «esto está sonando» sin ocupar sitio.
            ondas(self.estado.nivel, self.sonando(), color, if grande { 30.0 } else { 26.0 }, 5, 6.0),
        ].spacing(16).align_y(Vertical::Center).into()
    }

    fn controles(&self, color: Color) -> PanelElement<'_> {
        let central = if self.estado.pausado { "reproducir" } else { "pausa" };
        let plano = |activo: bool| if activo { color } else { apagado(0.8) };
        container(row![
            icono_teñido(icono_inline(ALEATORIO), 21.0, plano(self.estado.aleatorio)),
            redondo(icono_propio("anterior"), CTRL_LADO, Fondo::Tenue, color),
            redondo(icono_propio(central), CTRL_PLAY, Fondo::Solido, color),
            redondo(icono_propio("siguiente"), CTRL_LADO, Fondo::Tenue, color),
            icono_teñido(icono_inline(REPETIR), 21.0, plano(self.estado.repetir)),
        ].spacing(CTRL_SEP).align_y(Vertical::Center))
            .width(Length::Fill)
            .height(Length::Fixed(CTRL_H))
            .align_x(Horizontal::Center)
            .into()
    }

    /// Tiempo, barra, tiempo, cola y volumen: todo en una línea, como el
    /// reproductor de la referencia.
    fn fila_progreso(&self, color: Color) -> PanelElement<'_> {
        let (pos, total) = (self.estado.posicion_ms, self.estado.duracion_ms);
        let f = if total > 0 { (pos.max(0) as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
        let lleno = (f * 1000.0) as u16;
        let vacio = 1000 - lleno.min(1000);
        // El resto de la barra va en el mismo color rebajado y no en gris: es
        // lo que hace que se lea como «cuánto queda de esto» y no como un
        // carril vacío.
        let barra = stack![
            container(Space::new())
                .width(Length::Fill).height(Length::Fixed(BARRA_GRUESO))
                .style(move |_| pill(tema::mezclar(color, Color::WHITE, 0.45))),
            row![
                container(Space::new())
                    .width(Length::FillPortion(lleno.max(1)))
                    .height(Length::Fixed(BARRA_GRUESO))
                    .style(move |_| pill(color)),
                Space::new().width(Length::FillPortion(vacio.max(1))),
            ],
        ];
        let fila = row![
            container(text(formato_corto(pos)).size(12).font(negrita()))
                .width(Length::Fixed(TIEMPO_W)),
            container(barra).width(Length::Fill).height(Length::Fixed(BARRA_GRUESO)),
            container(text(formato_corto(total)).size(12).font(negrita()))
                .width(Length::Fixed(TIEMPO_W))
                .align_x(Horizontal::Right),
            icono_teñido(
                icono_propio("cola-lista"),
                18.0,
                if self.vista == Vista::Cola { color } else { apagado(0.82) },
            ),
            icono_teñido(icono_propio("volumen-alto"), 20.0, apagado(0.82)),
        ].spacing(10).align_y(Vertical::Center);
        container(fila).height(Length::Fixed(BARRA_H)).align_y(Vertical::Center).into()
    }

    fn panel_cola(&self, color: Color) -> PanelElement<'_> {
        let mut filas = column![
            container(text("A CONTINUACIÓN").size(11).font(gorda()).color(tema::TEXTO2))
                .height(Length::Fixed(COLA_CAB_H))
                .align_y(Vertical::Center),
        ];
        for item in self.estado.cola.iter().take(COLA_MAX) {
            let actual = item.actual;
            filas = filas.push(
                container(row![
                    // La miniatura es del color del disco mientras la
                    // aplicación no mande una por pista: un cuadrado gris en
                    // cada fila se ve como un hueco sin cargar.
                    if actual {
                        caratula(self.arte.as_ref(), 42.0, color)
                    } else {
                        caratula_pequena(color)
                    },
                    column![
                        text(item.titulo.clone()).size(14).font(negrita())
                            .color(if actual { color } else { tema::texto() }),
                        text(item.artista.clone()).size(12).color(tema::TEXTO2),
                    ].spacing(3).width(Length::Fill),
                    text(if item.duracion_ms > 0 { formato_corto(item.duracion_ms) } else { String::new() })
                        .size(12).color(tema::TEXTO2),
                    icono_teñido(
                        icono_inline(if item.favorita { CORAZON_LLENO } else { CORAZON }),
                        18.0,
                        if item.favorita { color } else { apagado(0.45) },
                    ),
                ].spacing(13).align_y(Vertical::Center))
                    .height(Length::Fixed(COLA_FILA_H)),
            );
        }
        container(filas)
            .width(Length::Fill)
            .padding(iced_core::Padding {
                top: COLA_PAD,
                right: COLA_PAD,
                bottom: COLA_PAD,
                left: COLA_PAD,
            })
            .style(|_| container::Style {
                background: Some(tema::alfa(tema::texto(), 0.07).into()),
                border: Border { radius: RADIO.into(), ..Default::default() },
                ..Default::default()
            })
            .into()
    }

    fn volumen_vertical(&self, color: Color) -> PanelElement<'_> {
        if !self.volumen_visible { return Space::new().into(); }
        let lleno = VOL_ALTO * self.estado.volumen.min(100) as f32 / 100.0;
        let pista = container(Space::new())
            .width(Length::Fixed(7.0))
            .height(Length::Fixed(VOL_ALTO))
            .style(move |_| pill(tema::mezclar(color, Color::WHITE, 0.45)));
        let progreso = container(
            container(Space::new())
                .width(Length::Fixed(7.0))
                .height(Length::Fixed(lleno.max(3.0)))
                .style(move |_| pill(color)),
        )
        .width(Length::Fixed(7.0))
        .height(Length::Fixed(VOL_ALTO))
        .align_y(Vertical::Bottom);
        container(stack![pista, progreso])
        // Veinte píxeles centran la pista con el pictograma inferior. El
        // contenedor anterior medía 28 y, al alinearse a la derecha, dejaba
        // la pista cuatro píxeles desplazada hacia la izquierda.
        .width(Length::Fixed(20.0))
        .height(Length::Fixed(VOL_ALTO))
        .align_x(Horizontal::Center)
        .into()
    }
}

/// Alto útil de la columna de volumen.
const VOL_ALTO: f32 = 64.0;
/// Base de la parte rellenable. Se deja un pequeño hueco sobre el icono para
/// que ambos controles se lean como piezas distintas.
const VOL_BASE_Y: f32 = BARRA_Y;
const VOL_TOLERANCIA_X: f32 = 14.0;
const VOL_TOLERANCIA_Y: f32 = 14.0;

// ── Piezas ───────────────────────────────────────────────────────────────

enum Relleno {
    /// El fondo de pantalla completa: negro en oscuro, blanco en claro.
    Negro,
    /// Relleno del color, para el temporizador.
    Color(Color),
}

fn tarjeta<'a>(
    contenido: PanelElement<'a>,
    w: f32,
    h: f32,
    radio: f32,
    relleno: Relleno,
    padding: [u16; 2],
) -> PanelElement<'a> {
    let (v, hh) = (padding[0] as f32, padding[1] as f32);
    tarjeta_v(contenido, w, h, radio, relleno, [v, hh, v, hh])
}

/// La tarjeta, con el padding por lados.
///
/// El fondo es un degradado de muy poco recorrido —un 4 % de luminosidad entre
/// arriba y abajo— para que el canto superior se lea como iluminado y la isla
/// no parezca un rectángulo recortado. La sombra va **también en oscuro**: la
/// isla cuelga sobre el escritorio, y sin ella se confunde con una ventana
/// negra pegada al borde.
fn tarjeta_v<'a>(
    contenido: PanelElement<'a>,
    w: f32,
    h: f32,
    radio: f32,
    relleno: Relleno,
    padding: [f32; 4],
) -> PanelElement<'a> {
    let (arriba, abajo, borde) = match relleno {
        Relleno::Negro => {
            let base = tema::bg();
            let claro = tema::es_claro();
            (
                if claro { Color::WHITE } else { tema::mezclar(base, Color::WHITE, 0.06) },
                if claro { tema::mezclar(base, Color::WHITE, 0.6) } else { base },
                tema::alfa(tema::texto(), if claro { 0.10 } else { 0.12 }),
            )
        }
        Relleno::Color(c) => (
            tema::mezclar(c, Color::WHITE, 0.10),
            tema::mezclar(c, Color::BLACK, 0.12),
            tema::alfa(Color::WHITE, 0.16),
        ),
    };
    container(contenido)
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        // `Padding` no se construye desde un array de cuatro; se arma por
        // lados, que además dice cuál es cuál.
        .padding(iced_core::Padding {
            top: padding[0],
            right: padding[1],
            bottom: padding[2],
            left: padding[3],
        })
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(
                Linear::new(Radians(std::f32::consts::PI))
                    .add_stop(0.0, arriba)
                    .add_stop(1.0, abajo),
            ))),
            border: Border { radius: radio.into(), width: 1.0, color: borde },
            shadow: Shadow {
                color: Color { a: if tema::es_claro() { 0.20 } else { 0.5 }, ..Color::BLACK },
                offset: Vector::new(0.0, 10.0),
                blur_radius: 30.0,
            },
            ..Default::default()
        })
        .into()
}

/// La carátula, o un cuadrado del color con una nota cuando no hay ninguna.
fn caratula(arte: Option<&Portada>, lado: f32, color: Color) -> PanelElement<'static> {
    match arte {
        Some(p) => container(
            image(image::Handle::from_rgba(p.width, p.height, p.rgba.clone()))
                .width(lado).height(lado),
        )
        .style(move |_| container::Style {
            shadow: Shadow { color: tema::alfa(color, 0.45), offset: Vector::new(0.0, 4.0), blur_radius: 14.0 },
            ..Default::default()
        })
        .into(),
        None => container(icono_teñido(icono_propio("musica"), lado * 0.46, tema::tinta_sobre(color)))
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .style(move |_| container::Style {
                background: Some(degradado(color, 1.0, 0.68)),
                border: Border { radius: (lado * 0.22).into(), ..Default::default() },
                shadow: Shadow { color: tema::alfa(color, 0.45), offset: Vector::new(0.0, 4.0), blur_radius: 14.0 },
                ..Default::default()
            })
            .into(),
    }
}

fn caratula_pequena(color: Color) -> PanelElement<'static> {
    let c = tema::mezclar(color, tema::bg(), 0.45);
    container(icono_teñido(icono_propio("musica"), 18.0, tema::tinta_sobre(color)))
        .width(Length::Fixed(42.0))
        .height(Length::Fixed(42.0))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(degradado(c, 1.0, 0.68)),
            border: Border { radius: 11.0.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

/// El reloj del temporizador: círculo blanco con las agujas en el color de la
/// tarjeta, que es como se lee sobre un fondo lleno de color.
fn reloj_blanco(lado: f32, color: Color) -> PanelElement<'static> {
    container(icono_teñido(icono_inline(AGUJAS), lado * 0.62, color))
        .width(Length::Fixed(lado))
        .height(Length::Fixed(lado))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(|_| container::Style {
            background: Some(Color::WHITE.into()),
            border: Border { radius: tema::R_PILL.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

#[derive(Clone, Copy, PartialEq)]
enum Fondo {
    /// Relleno con el color de la isla: el control principal.
    Solido,
    /// El mismo color muy rebajado: los de alrededor.
    Tenue,
}

fn redondo(ic: Option<icono::Icono>, lado: f32, fondo: Fondo, color: Color) -> PanelElement<'static> {
    let solido = fondo == Fondo::Solido;
    let px = lado * 0.44;
    let tinta = if solido { tema::tinta_sobre(color) } else { Color::WHITE };
    let tinta = if solido || !tema::es_claro() { tinta } else { tema::texto() };
    container(icono_teñido(ic, px, tinta))
        .width(Length::Fixed(lado))
        .height(Length::Fixed(lado))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(if solido {
                degradado(color, 1.0, 0.8)
            } else {
                Background::Color(tema::alfa(color, 0.5))
            }),
            border: Border { radius: (lado / 2.0).into(), ..Default::default() },
            shadow: if solido {
                Shadow { color: tema::alfa(color, 0.45), offset: Vector::new(0.0, 4.0), blur_radius: 12.0 }
            } else {
                Shadow::default()
            },
            ..Default::default()
        })
        .into()
}

/// Botón ancho sobre una tarjeta llena de color: el fondo no puede ser un gris
/// del tema, tiene que salir del propio color o desaparece.
fn boton_sobre_color(rotulo: &'static str, fondo: Color, principal: bool) -> PanelElement<'static> {
    let relleno = if principal {
        tema::mezclar(fondo, Color::WHITE, 0.22)
    } else {
        tema::mezclar(fondo, Color::BLACK, 0.35)
    };
    container(text(rotulo).size(14).font(negrita()).color(Color::WHITE))
        .width(Length::Fill)
        .height(Length::Fixed(BOTON_H))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(relleno.into()),
            border: Border { radius: 16.0.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

/// El punto de la grabadora.
fn punto(activa: bool, color: Color, lado: f32) -> PanelElement<'static> {
    let c = if activa { color } else { tema::TEXTO2 };
    container(Space::new())
        .width(Length::Fixed(lado))
        .height(Length::Fixed(lado))
        .style(move |_| container::Style {
            background: Some(c.into()),
            border: Border { radius: tema::R_PILL.into(), ..Default::default() },
            shadow: Shadow { color: tema::alfa(c, 0.6), offset: Vector::new(0.0, 0.0), blur_radius: 10.0 },
            ..Default::default()
        })
        .into()
}

/// El ecualizador. Barras con perfil propio y desfase entre ellas: en fase se
/// ven como un acordeón, no como sonido.
fn ondas(nivel: f32, activa: bool, color: Color, alto: f32, barras: usize, ancho: f32) -> PanelElement<'static> {
    let fase = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f32() * 6.5);
    let mut r = row![].spacing((ancho * 0.7).max(3.0)).align_y(Vertical::Center);
    for i in 0..barras {
        // El seno de un irracional por el índice da un perfil que no se repite
        // dentro de la fila, que es lo que distingue un vúmetro de una valla.
        // Con pocas barras el perfil tiene que ser suave: con el del vúmetro
        // largo, cinco barras salían dos altas y tres puntos.
        let pocas = barras <= 8;
        let variacion = if pocas { 0.45 } else { 0.55 };
        let b = (1.0 - variacion) + variacion * ((i as f32 * 2.399).sin() * 0.5 + 0.5);
        // El ecualizador de la cabecera es un adorno de «esto suena», no un
        // vúmetro: si le hiciera caso al nivel, una canción bajita lo dejaría
        // en cinco puntos.
        let n = if pocas { nivel.clamp(0.8, 1.0) } else { nivel.clamp(0.6, 1.0) };
        let h = if activa {
            let onda = ((fase + i as f32 * 0.8).sin() * 0.5 + 0.5) * 0.4 + 0.6;
            (alto * b * n * onda).max(ancho * 1.6)
        } else {
            ancho * 1.6
        };
        r = r.push(
            container(Space::new())
                .width(Length::Fixed(ancho))
                .height(Length::Fixed(h))
                .style(move |_| pill(color)),
        );
    }
    container(r).height(Length::Fixed(alto)).align_y(Vertical::Center).into()
}

/// El color del texto rebajado hasta `f`, mezclando con el fondo en vez de con
/// el canal alfa: el teñido de un SVG **descarta el alfa**, así que un icono
/// «al 35 %» se dibujaba blanco puro.
fn apagado(f: f32) -> Color {
    tema::mezclar(tema::bg(), tema::texto(), f)
}

fn pill(color: Color) -> container::Style {
    container::Style {
        background: Some(color.into()),
        border: Border { radius: tema::R_PILL.into(), ..Default::default() },
        ..Default::default()
    }
}

/// Un relleno con el color en dos intensidades. Un botón de un solo color plano
/// se ve pegado al fondo; con medio punto de degradado parece una superficie.
fn degradado(color: Color, arriba: f32, abajo: f32) -> Background {
    Background::Gradient(Gradient::Linear(
        Linear::new(Radians(std::f32::consts::PI))
            .add_stop(0.0, tema::mezclar(Color::BLACK, color, arriba))
            .add_stop(1.0, tema::mezclar(Color::BLACK, color, abajo)),
    ))
}

fn negrita() -> Font { Font { weight: Weight::Semibold, ..Font::DEFAULT } }
fn gorda() -> Font { Font { weight: Weight::Bold, ..Font::DEFAULT } }

fn icono_propio(nombre: &str) -> Option<icono::Icono> {
    match nombre {
        // No hay fichero para estos: son cuatro trazos, y un SVG en memoria
        // ahorra un asset que solo usa esta tarjeta.
        "cola-lista" => icono_inline(COLA_LISTA),
        otro => icono::propio(otro),
    }
}

fn icono_inline(svg: &str) -> Option<icono::Icono> {
    Some(icono::desde_svg(svg))
}

fn icono_teñido(ic: Option<icono::Icono>, px: f32, color: Color) -> PanelElement<'static> {
    match ic {
        Some(ic) => icono::ver_teñido_propio(&ic, px, color),
        // Un nombre mal escrito deja el hueco, no descuadra la fila.
        None => Space::new().width(Length::Fixed(px)).height(Length::Fixed(px)).into(),
    }
}

// Pictogramas que no están en la biblioteca del panel. Trazos sólidos y no
// `stroke`: el teñido de iced sustituye el color de relleno, y un icono
// perfilado saldría hueco.
const COLA_LISTA: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><rect x="2" y="3.4" width="12" height="1.8" rx=".9"/><rect x="2" y="7.1" width="12" height="1.8" rx=".9"/><rect x="2" y="10.8" width="12" height="1.8" rx=".9"/></svg>"##;
const CORAZON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M8 14.1 2.9 9.2A3.4 3.4 0 0 1 8 4.7a3.4 3.4 0 0 1 5.1 4.5zm0-1.9 3.8-3.7a2 2 0 0 0-3-2.6L8 6.9 7.2 5.9a2 2 0 0 0-3 2.6z"/></svg>"##;
const CORAZON_LLENO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M8 14.1 2.9 9.2A3.4 3.4 0 0 1 8 4.7a3.4 3.4 0 0 1 5.1 4.5z"/></svg>"##;
const STOP: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><rect x="3.6" y="3.6" width="8.8" height="8.8" rx="2.2"/></svg>"##;
const ALEATORIO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M10.6 2.5a.8.8 0 0 1 1.1 0l1.8 1.8a.8.8 0 0 1 0 1.1l-1.8 1.8a.8.8 0 1 1-1.1-1.1l.4-.4h-.7c-.6 0-1 .2-1.4.7L8 7.2 7.1 6l.3-.4c.7-.9 1.6-1.4 2.7-1.4h.7l-.4-.5a.8.8 0 0 1 0-1.1zM2.3 4.2h1.9c1.1 0 2 .5 2.7 1.4l3 3.9c.4.5.8.7 1.4.7h.7l-.4-.4a.8.8 0 1 1 1.1-1.1l1.8 1.8a.8.8 0 0 1 0 1.1l-1.8 1.8a.8.8 0 1 1-1.1-1.1l.4-.4h-.7c-1.1 0-2-.5-2.7-1.4l-3-3.9c-.4-.5-.8-.7-1.4-.7H2.3a.8.8 0 0 1 0-1.6zm0 6.1h1.9c.5 0 1-.2 1.3-.6l.9 1.2c-.6.7-1.4 1-2.2 1H2.3a.8.8 0 1 1 0-1.6z"/></svg>"##;
const REPETIR: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M4.8 2.6a.8.8 0 0 1 0 1.2l-.3.3h6a3 3 0 0 1 3 3v1a.8.8 0 1 1-1.6 0v-1c0-.8-.6-1.4-1.4-1.4h-6l.3.3a.8.8 0 1 1-1.2 1.1L1.9 5.4a.8.8 0 0 1 0-1.1l1.7-1.7a.8.8 0 0 1 1.2 0zm6.4 10.8a.8.8 0 0 1 0-1.2l.3-.3h-6a3 3 0 0 1-3-3v-1a.8.8 0 1 1 1.6 0v1c0 .8.6 1.4 1.4 1.4h6l-.3-.3a.8.8 0 1 1 1.2-1.1l1.7 1.7a.8.8 0 0 1 0 1.1l-1.7 1.7a.8.8 0 0 1-1.2 0z"/></svg>"##;
/// Solo las agujas: van sobre el disco blanco que dibuja `reloj_blanco`.
const AGUJAS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="#fff"><path d="M8.95 3.7v3.75l2.4 1.45a.95.95 0 0 1-.98 1.63L7.5 8.85a.95.95 0 0 1-.46-.82V3.7a.95.95 0 0 1 1.9 0"/></svg>"##;

// ── Portada ──────────────────────────────────────────────────────────────

/// Recorta las esquinas de la carátula y saca su color.
///
/// El recorte se hace sobre los píxeles y no con un `border-radius` del
/// contenedor: iced dibuja la imagen **encima** del fondo del contenedor, así
/// que el radio del borde no la recorta y la carátula salía con las cuatro
/// esquinas cuadradas dentro de una tarjeta redondeada.
fn preparar_portada(portada: Option<&Portada>) -> (Option<Portada>, Option<Color>) {
    let Some(p) = portada else { return (None, None) };
    if p.width == 0 || p.height == 0 || p.rgba.len() < (p.width * p.height * 4) as usize {
        return (None, None);
    }
    (Some(redondear(p)), Some(color_dominante(p)))
}

fn redondear(p: &Portada) -> Portada {
    let (w, h) = (p.width as i32, p.height as i32);
    // El mismo 22 % de radio que usa la carátula sin portada, para que las dos
    // tengan la misma curva a cualquier tamaño.
    let r = (w.min(h) as f32 * 0.22).round() as i32;
    let mut rgba = p.rgba.clone();
    for y in 0..h {
        for x in 0..w {
            // Distancia al centro de la esquina más cercana; fuera del cuarto
            // de círculo, transparente. Un píxel de antialias en el borde
            // evita el escalón.
            let dx = if x < r { r - x } else if x >= w - r { x - (w - r - 1) } else { 0 };
            let dy = if y < r { r - y } else if y >= h - r { y - (h - r - 1) } else { 0 };
            if dx == 0 || dy == 0 { continue; }
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            let alfa = ((r as f32 + 0.5 - d).clamp(0.0, 1.0) * 255.0) as u8;
            if alfa < 255 {
                let i = ((y * w + x) * 4 + 3) as usize;
                rgba[i] = (rgba[i] as u16 * alfa as u16 / 255) as u8;
            }
        }
    }
    Portada { rgba, width: p.width, height: p.height }
}

/// El color que resume la carátula.
///
/// Media de una rejilla de muestras, no de todos los píxeles: 400 muestras
/// dicen lo mismo que 90 000 y cuestan nada. Después se sube la saturación y se
/// lleva a una luminosidad de trabajo, porque la media de una foto tiende al
/// gris y un gris no sirve para teñir nada.
fn color_dominante(p: &Portada) -> Color {
    let (w, h) = (p.width, p.height);
    let paso_x = (w / 20).max(1);
    let paso_y = (h / 20).max(1);
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for y in (0..h).step_by(paso_y as usize) {
        for x in (0..w).step_by(paso_x as usize) {
            let i = ((y * w + x) * 4) as usize;
            if p.rgba[i + 3] < 128 { continue; }
            r += p.rgba[i] as u64;
            g += p.rgba[i + 1] as u64;
            b += p.rgba[i + 2] as u64;
            n += 1;
        }
    }
    if n == 0 { return tema::acento(); }
    let (r, g, b) = (r as f32 / n as f32 / 255.0, g as f32 / n as f32 / 255.0, b as f32 / n as f32 / 255.0);
    let media = (r + g + b) / 3.0;
    let saturar = |c: f32| (media + (c - media) * 2.1).clamp(0.0, 1.0);
    let (r, g, b) = (saturar(r), saturar(g), saturar(b));
    let l = 0.299 * r + 0.587 * g + 0.114 * b;
    let objetivo = if tema::es_claro() { 0.48 } else { 0.58 };
    let c = Color::from_rgb(r, g, b);
    if l < objetivo {
        tema::mezclar(c, Color::WHITE, (objetivo - l) / (1.0 - l).max(0.01))
    } else {
        tema::mezclar(c, Color::BLACK, (l - objetivo) / l.max(0.01))
    }
}

/// `01:18`, y con horas cuando las hay.
fn formato_tiempo(ms: i64) -> String {
    let signo = if ms < 0 { "−" } else { "" };
    let s = ms.unsigned_abs() / 1000;
    if s >= 3600 {
        format!("{signo}{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{signo}{:02}:{:02}", s / 60, s % 60)
    }
}

/// `1:18`, sin el cero de delante: es como se escriben las duraciones de las
/// canciones y así los dos rótulos de la barra ocupan lo mismo que el hueco.
fn formato_corto(ms: i64) -> String {
    let signo = if ms < 0 { "−" } else { "" };
    let s = ms.unsigned_abs() / 1000;
    if s >= 3600 {
        format!("{signo}{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{signo}{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn estado(clase: Clase) -> Estado {
        Estado {
            app_id: "com.bookos.test".into(), clase, activo: true,
            pausado: false, titulo: "Título".into(), subtitulo: "Autor".into(),
            posicion_ms: 30_000, duracion_ms: 120_000, restante_ms: 30_000,
            volumen: 70, nivel: 0.5, aleatorio: false, repetir: false,
            portada: None, cola: Vec::new(),
        }
    }

    fn cola(n: usize) -> Vec<ItemCola> {
        (0..n)
            .map(|i| ItemCola {
                id: format!("id{i}"), titulo: "t".into(), artista: "a".into(),
                duracion_ms: 200_000, favorita: false, actual: i == 0,
            })
            .collect()
    }

    #[test]
    fn click_compacto_abre_sin_ordenar_nada_a_la_app() {
        let mut a = Actividad::nueva(estado(Clase::Player), true);
        assert!(a.pulsar(MARGEN_SOMBRA + 20.0, MARGEN_SOMBRA + 20.0).is_none());
        assert_eq!(a.vista, Vista::Abierta);
        assert_eq!(a.size().1, PLAYER_H + MARGEN_SOMBRA * 2.0);
    }

    #[test]
    fn timer_vencido_ofrece_aplazar_y_parar() {
        let mut e = estado(Clase::Timer);
        e.restante_ms = -4_000;
        let mut a = Actividad::nueva(e, false);
        a.vista = Vista::Abierta;
        let y = MARGEN_SOMBRA + TIMER_BOTON_Y + BOTON_H / 2.0;
        let aplazar = a.pulsar(MARGEN_SOMBRA + 70.0, y).unwrap();
        let parar = a.pulsar(MARGEN_SOMBRA + 300.0, y).unwrap();
        assert_eq!(aplazar.nombre, "snooze");
        assert_eq!(parar.nombre, "stop");
    }

    #[test]
    fn movimiento_reducido_no_mantiene_frames_vivos() {
        let a = Actividad::nueva(estado(Clase::Recorder), false);
        assert!(!a.animando());
        assert_eq!(a.entrada(), (1.0, 1.0, 0.0));
    }

    #[test]
    fn la_vista_abierta_no_repinta_el_bitmap_por_las_ondas() {
        let mut a = Actividad::nueva(estado(Clase::Player), true);
        a.desde = Instant::now() - tema::D_TARJETA;
        a.ultimo_frame_ondas = Instant::now() - FRAME_ONDAS;
        assert!(a.animando());
        assert!(a.ondas_pendientes());

        a.vista = Vista::Abierta;
        assert!(!a.animando());
        assert!(!a.ondas_pendientes());

        a.vista = Vista::Compacta;
        a.estado.pausado = true;
        assert!(!a.animando());
        assert!(!a.ondas_pendientes());
    }

    #[test]
    fn hover_sobre_volumen_despliega_el_slider() {
        let mut a = Actividad::nueva(estado(Clase::Player), false);
        a.vista = Vista::Abierta;
        let x = MARGEN_SOMBRA + ABIERTO_W - PAD - ICONO_FILA / 2.0;
        let y = MARGEN_SOMBRA + BARRA_Y + BARRA_H / 2.0;
        assert!(a.puntero(Some((x, y))), "el estado de hover no cambió");
        assert!(a.volumen_visible);
        // Se puede alcanzar el extremo superior y desviarse un poco hacia la
        // izquierda sin que el control desaparezca bajo el cursor.
        assert!(!a.puntero(Some((
            x - VOL_TOLERANCIA_X,
            MARGEN_SOMBRA + VOL_BASE_Y - VOL_ALTO - VOL_TOLERANCIA_Y / 2.0,
        ))));
        assert!(a.volumen_visible);
        assert!(a.puntero(None), "salir del icono no ocultó el slider");
        assert!(!a.volumen_visible);
    }

    /// Los cinco controles caen donde se dibujan. Es la prueba que faltaba: las
    /// zonas se calculan a mano y nada las ata a la vista más que las
    /// constantes de geometría.
    #[test]
    fn los_cinco_controles_caen_donde_se_ven() {
        let mut a = Actividad::nueva(estado(Clase::Player), false);
        a.vista = Vista::Abierta;
        let centro = ABIERTO_W / 2.0;
        let mut en = |x: f32| {
            a.pulsar(MARGEN_SOMBRA + x, MARGEN_SOMBRA + CTRL_Y + CTRL_H / 2.0)
                .map(|ac| ac.nombre)
        };
        assert_eq!(en(centro), Some("play-pause".into()));
        assert_eq!(en(centro - CTRL_PLAY / 2.0 - CTRL_SEP - CTRL_LADO / 2.0), Some("previous".into()));
        assert_eq!(en(centro + CTRL_PLAY / 2.0 + CTRL_SEP + CTRL_LADO / 2.0), Some("next".into()));
        assert_eq!(en(centro - CTRL_PLAY / 2.0 - CTRL_SEP * 2.0 - CTRL_LADO - CTRL_PLANO / 2.0), Some("shuffle".into()));
        assert_eq!(en(centro + CTRL_PLAY / 2.0 + CTRL_SEP * 2.0 + CTRL_LADO + CTRL_PLANO / 2.0), Some("repeat".into()));
    }

    #[test]
    fn la_barra_de_progreso_manda_la_fraccion_pulsada() {
        let mut a = Actividad::nueva(estado(Clase::Player), false);
        a.vista = Vista::Abierta;
        let y = MARGEN_SOMBRA + BARRA_Y + BARRA_H / 2.0;
        let ini = PAD + TIEMPO_W + 10.0;
        let fin = ABIERTO_W - PAD - ICONO_FILA * 2.0 - 12.0 - TIEMPO_W - 14.0;
        let medio = a.pulsar(MARGEN_SOMBRA + (ini + fin) / 2.0, y).unwrap();
        assert_eq!(medio.nombre, "seek");
        let f: f32 = medio.valor.parse().unwrap();
        assert!((f - 0.5).abs() < 0.02, "fracción {f}");
        // Y en los extremos no se sale de [0,1].
        let izq: f32 = a.pulsar(MARGEN_SOMBRA, y).unwrap().valor.parse().unwrap();
        let der: f32 = a.pulsar(MARGEN_SOMBRA + fin, y).unwrap().valor.parse().unwrap();
        assert_eq!((izq, der), (0.0, 1.0));
    }

    /// El botón de la cola es un interruptor: la abre y la vuelve a cerrar.
    #[test]
    fn el_boton_de_la_cola_la_abre_y_la_cierra() {
        let mut e = estado(Clase::Player);
        e.cola = cola(3);
        let mut a = Actividad::nueva(e, false);
        a.vista = Vista::Abierta;
        let alto_sin = a.size().1;
        let x = MARGEN_SOMBRA + ABIERTO_W - PAD - ICONO_FILA * 2.0 - 6.0;
        let y = MARGEN_SOMBRA + BARRA_Y + BARRA_H / 2.0;
        assert!(a.pulsar(x, y).is_none());
        assert_eq!(a.vista, Vista::Cola);
        assert!(a.size().1 > alto_sin, "la tarjeta crece con la cola");
        assert!(a.pulsar(x, y).is_none());
        assert_eq!(a.vista, Vista::Abierta);
        assert_eq!(a.size().1, alto_sin);
    }

    /// La cola crece con las canciones que tiene, hasta el tope.
    #[test]
    fn la_cola_ocupa_lo_que_tiene_que_ocupar() {
        let mut dos = estado(Clase::Player);
        dos.cola = cola(2);
        let mut a = Actividad::nueva(dos, false);
        a.vista = Vista::Cola;

        let mut muchas = estado(Clase::Player);
        muchas.cola = cola(9);
        let mut b = Actividad::nueva(muchas, false);
        b.vista = Vista::Cola;
        assert!(b.size().1 > a.size().1);

        let mut tope = estado(Clase::Player);
        tope.cola = cola(COLA_MAX);
        let mut c = Actividad::nueva(tope, false);
        c.vista = Vista::Cola;
        assert_eq!(b.size().1, c.size().1, "a partir del tope no sigue creciendo");
    }

    #[test]
    fn la_tercera_cancion_de_la_cola_es_la_tercera_fila() {
        let mut e = estado(Clase::Player);
        e.cola = cola(4);
        let mut a = Actividad::nueva(e, false);
        a.vista = Vista::Cola;
        let y = PLAYER_H + COLA_SEP + COLA_PAD + COLA_CAB_H + COLA_FILA_H * 2.5;
        let accion = a.pulsar(MARGEN_SOMBRA + ABIERTO_W / 2.0, MARGEN_SOMBRA + y).unwrap();
        assert_eq!(accion.nombre, "queue-goto");
        assert_eq!(accion.valor, "id2");
        // Y el corazón de esa misma fila es «favorita», no «ir a».
        let corazon = a
            .pulsar(MARGEN_SOMBRA + ABIERTO_W - COLA_PAD - 20.0, MARGEN_SOMBRA + y)
            .unwrap();
        assert_eq!(corazon.nombre, "favorite");
        assert_eq!(corazon.valor, "id2");
    }

    /// Pulsar en la cabecera cierra la tarjeta: es lo que queda por descubrir
    /// cuando no hay ningún botón de cerrar dibujado.
    #[test]
    fn la_cabecera_colapsa_la_tarjeta() {
        for clase in [Clase::Player, Clase::Timer, Clase::Recorder] {
            let mut a = Actividad::nueva(estado(clase), false);
            a.vista = Vista::Abierta;
            assert!(a.pulsar(MARGEN_SOMBRA + 40.0, MARGEN_SOMBRA + PAD + 10.0).is_none());
            assert_eq!(a.vista, Vista::Compacta, "{clase:?}");
        }
    }

    /// Una hora larga de grabación no puede leerse como «01:05».
    #[test]
    fn a_partir_de_una_hora_se_enseñan_las_horas() {
        assert_eq!(formato_tiempo(65_000), "01:05");
        assert_eq!(formato_tiempo(3_725_000), "1:02:05");
        assert_eq!(formato_tiempo(-7_000), "−00:07");
        assert_eq!(formato_corto(65_000), "1:05");
        assert_eq!(formato_corto(235_000), "3:55");
    }

    /// El color de la carátula tiene que servir de fondo con texto encima, así
    /// que ni puede salir negro ni puede salir blanco.
    #[test]
    fn el_color_de_la_portada_es_utilizable() {
        for (r, g, b) in [(10u8, 10u8, 12u8), (250, 250, 250), (120, 30, 200)] {
            let p = Portada {
                rgba: std::iter::repeat([r, g, b, 255]).take(64 * 64).flatten().collect(),
                width: 64,
                height: 64,
            };
            let c = color_dominante(&p);
            let l = 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
            assert!(l > 0.12 && l < 0.92, "luminosidad {l} para {r},{g},{b}");
        }
    }

    /// Recortar las esquinas no puede tocar el centro de la imagen.
    #[test]
    fn el_recorte_solo_se_come_las_esquinas() {
        let p = Portada {
            rgba: std::iter::repeat([200u8, 100, 50, 255]).take(100 * 100).flatten().collect(),
            width: 100,
            height: 100,
        };
        let r = redondear(&p);
        let alfa = |x: usize, y: usize| r.rgba[(y * 100 + x) * 4 + 3];
        assert_eq!(alfa(50, 50), 255, "el centro se mantiene opaco");
        assert_eq!(alfa(0, 0), 0, "la esquina se recorta");
        assert_eq!(alfa(99, 99), 0, "y la de enfrente también");
    }
}
