//! El centro de control, con el aspecto del plasmoide `bookos-controlcenter`.
//!
//! Cinco bloques, de arriba abajo: quién ha iniciado sesión con tres botones al
//! lado, las dos tarjetas de conectividad, la rejilla de ocho conmutadores, los
//! deslizadores de volumen y brillo, y lo que esté sonando.
//!
//! **Los colores son los del plasmoide**, no los del resto del shell: un
//! conmutador apagado va en azul claro (`#AECAFF`) y no en gris. Es lo que hace
//! que la rejilla se lea como una fila de botones apagados en vez de como
//! botones deshabilitados, y era la diferencia que se veía de un vistazo entre
//! esto y el diseño.
//!
//! **Todo lo que se dibuja hace algo.** Los estados que se pueden leer se leen
//! al abrir la tarjeta —`nmcli`, `bluetoothctl`, `rfkill`— y los que no tienen
//! forma de consultarse son acciones, no interruptores. Mantener la pantalla
//! encendida es la excepción: su estado **es** un proceso hijo vivo, y por eso
//! lo guarda el shell.

use std::process::{Child, Command, Stdio};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length};
use iced_widget::{column, container, row, text, Space};

use crate::bloqueo::Usuario;
use crate::icono;
use crate::medios::{self, Sonando};
use crate::tema;
use crate::view::PanelElement;
use crate::Accion;

use super::control;
use super::{Ancla, Tecla};

const ANCHO: f32 = 341.0;
const MARGEN: f32 = 12.0;
/// Hueco entre bloques.
const HUECO: f32 = 10.0;
/// Alto de la cabecera y de sus botones redondos.
const CABECERA: f32 = 56.0;
const BOTON_CABECERA: f32 = 44.0;
/// Lado del avatar.
const AVATAR: f32 = 40.0;
/// Alto de las dos tarjetas de conectividad.
const CONEXION: f32 = 64.0;
/// Lado del icono redondo de una tarjeta de conectividad.
const ICONO_CONEXION: f32 = 44.0;
/// Rejilla: cuatro columnas por dos filas de botones redondos.
const COLUMNAS: usize = 4;
const BOTON: f32 = 52.0;
const REJILLA_MARGEN: f32 = 12.0;
/// Alto de un deslizador.
const PILDORA: f32 = 34.0;
/// Lado del botón redondo que va a la derecha de cada deslizador.
const BOTON_PILDORA: f32 = 40.0;
/// Alto de la tarjeta de medios.
const MEDIOS: f32 = 132.0;

/// El encendido de un conmutador de la estación: el acento del sistema.
///
/// Era el `#4184FF` del plasmoide, escrito a pelo. Deja de valer desde que el
/// acento se elige: con la constante, poner el escritorio en verde dejaba esta
/// tarjeta —la más azul de todas— en azul, y el apagado `#AECAFF` de al lado,
/// que sí es un azul deliberado, se leía como el activo de otro tema.
fn acento_centro() -> Color {
    tema::acento()
}
const APAGADO_OSCURO: Color = Color {
    r: 0.682,
    g: 0.792,
    b: 1.0,
    a: 1.0,
};
/// El círculo de un conmutador apagado.
///
/// En oscuro es el azul claro del plasmoide. En claro ese azul sobre una
/// tarjeta blanca deja de significar «apagado» —se parece demasiado al
/// encendido— y pasa a ser el gris `#e5e5ea` del sistema, que es lo que usan
/// los controles apagados de la paleta clara.
fn apagado() -> Color {
    if tema::es_claro() {
        Color {
            r: 0.898,
            g: 0.898,
            b: 0.918,
            a: 1.0,
        }
    } else {
        APAGADO_OSCURO
    }
}

/// La tinta del icono dentro de un círculo apagado.
///
/// Sobre el azul claro va **blanca**, encendida o apagada: con tinta oscura el
/// botón se leía como deshabilitado, que es justo lo contrario de «apagado pero
/// disponible». Sobre el gris del tema claro un icono blanco no se vería, y ahí
/// sí es oscura.
fn tinta_apagado() -> Color {
    if tema::es_claro() {
        tema::texto()
    } else {
        Color::WHITE
    }
}

/// Fondo de las tarjetas interiores: la superficie del tema al 80 %.
fn contenedor() -> Color {
    if tema::es_claro() {
        Color {
            r: 0.949,
            g: 0.949,
            b: 0.968,
            a: 0.80,
        }
    } else {
        Color {
            r: 0.16,
            g: 0.16,
            b: 0.17,
            a: 0.80,
        }
    }
}

/// Un conmutador o una acción de la rejilla.
struct Baldosa {
    icono: &'static str,
    /// Encendida: círculo en acento sólido. Apagada: azul claro.
    activa: bool,
    /// Qué ejecuta. `None` = lo resuelve el propio shell.
    accion: Option<Accion>,
    /// Es un conmutador del shell y no una orden suelta.
    interna: Option<Interna>,
}

/// Los conmutadores cuyo estado vive aquí dentro.
#[derive(Clone, Copy, PartialEq)]
enum Interna {
    /// Mantener la pantalla encendida.
    Mantener,
}

/// Qué está agarrado con el ratón.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Agarre {
    Nada,
    Volumen,
    Brillo,
}

pub struct Centro {
    usuario: Usuario,
    baldosas: Vec<Baldosa>,
    señalada: tema::Realce,
    wifi: bool,
    bluetooth: bool,
    volumen: u8,
    brillo: u8,
    agarre: Agarre,
    backlight: Option<(String, u32)>,
    sonando: Option<Sonando>,
    /// El `systemd-inhibit` que impide que la pantalla se apague mientras esté
    /// puesto. Matarlo es apagar el conmutador.
    inhibidor: Option<Child>,
}

impl Centro {
    pub fn new() -> Self {
        let wifi = orden("nmcli radio wifi").contains("enabled");
        let bluetooth = orden("bluetoothctl show").contains("Powered: yes");
        let mut c = Self {
            // El centro de control no ve la configuración: la foto de `panel.conf`
            // la resuelve el bloqueo, y aquí basta con las de siempre.
            usuario: Usuario::leer(None),
            baldosas: Vec::new(),
            señalada: tema::Realce::nuevo(),
            wifi,
            bluetooth,
            // El nivel de partida se lee de golpe: `wpctl` cuesta 21 ms
            // medidos, pero esto ocurre al abrir la tarjeta y no en el refresco
            // del panel.
            volumen: crate::widgets::volumen::consultar("@DEFAULT_AUDIO_SINK@")
                .map(|(n, _)| n)
                .unwrap_or(0),
            brillo: crate::brillo_actual().unwrap_or(0),
            agarre: Agarre::Nada,
            backlight: crate::backlight(),
            sonando: Sonando::leer(),
            inhibidor: None,
        };
        c.baldosas = baldosas();
        c
    }

    pub fn size(&self) -> (f32, f32) {
        let mut alto = MARGEN * 2.0
            + CABECERA
            + HUECO
            + CONEXION
            + HUECO
            + self.alto_rejilla()
            + HUECO
            + self.alto_deslizadores();
        if self.sonando.is_some() {
            alto += HUECO + MEDIOS;
        }
        (ANCHO, alto)
    }

    fn alto_rejilla(&self) -> f32 {
        let filas = self.baldosas.len().div_ceil(COLUMNAS) as f32;
        REJILLA_MARGEN * 2.0 + filas * (BOTON + 8.0)
    }

    fn alto_deslizadores(&self) -> f32 {
        REJILLA_MARGEN * 2.0 + BOTON_PILDORA * 2.0 + 10.0
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("control")
    }

    fn y_conexiones(&self) -> f32 {
        MARGEN + CABECERA + HUECO
    }

    fn y_rejilla(&self) -> f32 {
        self.y_conexiones() + CONEXION + HUECO
    }

    fn y_deslizadores(&self) -> f32 {
        self.y_rejilla() + self.alto_rejilla() + HUECO
    }

    fn y_medios(&self) -> f32 {
        self.y_deslizadores() + self.alto_deslizadores() + HUECO
    }

    /// Qué baldosa cae en un punto.
    fn baldosa_en(&self, x: f32, y: f32) -> Option<usize> {
        let x0 = MARGEN + REJILLA_MARGEN;
        let y0 = self.y_rejilla() + REJILLA_MARGEN;
        let ancho_celda = (ANCHO - x0 * 2.0) / COLUMNAS as f32;
        let alto_celda = BOTON + 8.0;
        if x < x0 || y < y0 {
            return None;
        }
        let columna = ((x - x0) / ancho_celda) as usize;
        let fila = ((y - y0) / alto_celda) as usize;
        if columna >= COLUMNAS {
            return None;
        }
        let i = fila * COLUMNAS + columna;
        (i < self.baldosas.len()).then_some(i)
    }

    /// Cuál de las dos tarjetas de conectividad cae en un punto: `false` la del
    /// Wi-Fi, `true` la del Bluetooth.
    fn conexion_en(&self, x: f32, y: f32) -> Option<bool> {
        let y0 = self.y_conexiones();
        if y < y0 || y > y0 + CONEXION || x < MARGEN || x > ANCHO - MARGEN {
            return None;
        }
        Some(x > ANCHO / 2.0)
    }

    fn ancho_pildora(&self) -> f32 {
        ANCHO - (MARGEN + REJILLA_MARGEN) * 2.0 - BOTON_PILDORA - 10.0
    }

    fn rect_pildora(&self, cual: Agarre) -> iced_core::Rectangle {
        let y0 = self.y_deslizadores() + REJILLA_MARGEN;
        let y = match cual {
            Agarre::Brillo => y0 + BOTON_PILDORA + 10.0,
            _ => y0,
        };
        iced_core::Rectangle {
            x: MARGEN + REJILLA_MARGEN,
            y: y + (BOTON_PILDORA - PILDORA) / 2.0,
            width: self.ancho_pildora(),
            height: PILDORA,
        }
    }

    /// Los tres botones de la tarjeta de medios, si está.
    fn medio_en(&self, x: f32, y: f32) -> Option<medios::Orden> {
        self.sonando.as_ref()?;
        let y0 = self.y_medios() + MEDIOS - 44.0;
        if y < y0 || y > y0 + 36.0 {
            return None;
        }
        let centro = ANCHO / 2.0;
        Some(match x {
            x if x < centro - 20.0 => medios::Orden::Anterior,
            x if x > centro + 20.0 => medios::Orden::Siguiente,
            _ => medios::Orden::Alternar,
        })
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        if let Some((x, _)) = punto {
            let nivel = control::nivel_en(x, MARGEN + REJILLA_MARGEN, self.ancho_pildora());
            match self.agarre {
                Agarre::Volumen => return self.poner_volumen(nivel),
                Agarre::Brillo => return self.poner_brillo(nivel),
                Agarre::Nada => {}
            }
        }
        let señalada = punto.and_then(|(x, y)| self.baldosa_en(x, y));
        self.señalada.señalar(señalada)
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    pub fn animando(&self) -> bool {
        self.señalada.animando()
    }

    pub fn agarrado(&self) -> bool {
        self.agarre != Agarre::Nada
    }

    pub fn soltar(&mut self) -> bool {
        std::mem::replace(&mut self.agarre, Agarre::Nada) != Agarre::Nada
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = iced_core::Point::new(x, y);
        let holgada = |mut r: iced_core::Rectangle| {
            r.y -= control::MARGEN_AGARRE;
            r.height += control::MARGEN_AGARRE * 2.0;
            r
        };
        let nivel = control::nivel_en(x, MARGEN + REJILLA_MARGEN, self.ancho_pildora());
        if holgada(self.rect_pildora(Agarre::Volumen)).contains(punto) {
            self.agarre = Agarre::Volumen;
            self.poner_volumen(nivel);
            return None;
        }
        if holgada(self.rect_pildora(Agarre::Brillo)).contains(punto) {
            self.agarre = Agarre::Brillo;
            self.poner_brillo(nivel);
            return None;
        }
        if let Some(orden) = self.medio_en(x, y) {
            return self
                .sonando
                .as_ref()
                .map(|s| Accion::Lanzar(s.orden(orden)));
        }
        if let Some(bluetooth) = self.conexion_en(x, y) {
            // El círculo enciende y apaga la radio; el resto de la tarjeta abre
            // la lista. Es lo que hace el plasmoide, y resuelve las dos cosas
            // que se piden de un vistazo: apagar el Wi-Fi rápido, o elegir a
            // qué red conectarse.
            let mitad = if bluetooth { ANCHO / 2.0 } else { MARGEN };
            if x < mitad + 8.0 + ICONO_CONEXION {
                let encendido = if bluetooth { self.bluetooth } else { self.wifi };
                if bluetooth {
                    self.bluetooth = !encendido;
                } else {
                    self.wifi = !encendido;
                }
                return Some(Accion::Lanzar(if bluetooth {
                    format!("bluetoothctl power {}", si_no(!encendido))
                } else {
                    format!("nmcli radio wifi {}", si_no(!encendido))
                }));
            }
            return Some(Accion::Emergente(if bluetooth {
                "bluetooth"
            } else {
                "red"
            }));
        }
        let i = self.baldosa_en(x, y)?;
        if let Some(interna) = self.baldosas[i].interna {
            self.alternar(interna, i);
            return None;
        }
        if self.baldosas[i].icono == "notificaciones" {
            // «No molestar» vive en su propia tarjeta, que es donde están las
            // duraciones; desde aquí solo se abre.
            return Some(Accion::Emergente("notificaciones"));
        }
        // El estado se da por cambiado sin releerlo: `rfkill` y compañía tardan
        // en aplicarlo y la baldosa se quedaría como estaba hasta la siguiente
        // apertura.
        self.baldosas[i].activa = !self.baldosas[i].activa;
        self.baldosas[i].accion.clone()
    }

    /// Enciende o apaga un conmutador cuyo estado vive en el shell.
    fn alternar(&mut self, cual: Interna, i: usize) {
        match cual {
            Interna::Mantener => {
                if let Some(mut hijo) = self.inhibidor.take() {
                    // Matar el `systemd-inhibit` suelta el bloqueo: es lo que
                    // hace el plasmoide, y garantiza que no quede un inhibidor
                    // colgado si el shell se cae.
                    let _ = hijo.kill();
                    let _ = hijo.wait();
                    self.baldosas[i].activa = false;
                    return;
                }
                self.inhibidor = Command::new("systemd-inhibit")
                    .args([
                        "--what=idle:sleep",
                        "--who=BookOS",
                        "--why=Mantener la pantalla encendida",
                        "sleep",
                        "infinity",
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .ok();
                self.baldosas[i].activa = self.inhibidor.is_some();
            }
        }
    }

    fn poner_volumen(&mut self, nivel: u8) -> bool {
        if nivel == self.volumen {
            return false;
        }
        self.volumen = nivel;
        let _ = Command::new("wpctl")
            .args([
                "set-volume",
                "@DEFAULT_AUDIO_SINK@",
                &format!("{:.2}", nivel as f32 / 100.0),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        true
    }

    fn poner_brillo(&mut self, nivel: u8) -> bool {
        // Mismo suelo que el emergente del brillo: a cero no se ve ni el
        // deslizador con el que volver a subirlo.
        let nivel = nivel.clamp(5, 100);
        if nivel == self.brillo {
            return false;
        }
        self.brillo = nivel;
        if let Some((dispositivo, maximo)) = &self.backlight {
            let crudo = (*maximo as f64 * nivel as f64 / 100.0).round() as u32;
            let _ = Command::new("busctl")
                .args([
                    "call",
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1/session/auto",
                    "org.freedesktop.login1.Session",
                    "SetBrightness",
                    "ssu",
                    "backlight",
                    dispositivo,
                    &crudo.to_string(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
        true
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    // --- Dibujo ------------------------------------------------------------

    /// Un círculo con un icono dentro, que es la forma de casi todo aquí.
    fn circulo(nombre: &str, lado: f32, fondo: Color, tinta: Color) -> PanelElement<'static> {
        let dibujo: PanelElement<'static> = match icono::propio(nombre) {
            Some(ic) => icono::ver_teñido_propio(&ic, lado * 0.44, tinta),
            None => Space::new().width(Length::Fixed(lado * 0.44)).into(),
        };
        container(dibujo)
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado))
            .center_x(Length::Fixed(lado))
            .center_y(Length::Fixed(lado))
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: (lado / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    fn cabecera(&self) -> PanelElement<'_> {
        let avatar: PanelElement<'_> = match &self.usuario.foto {
            Some(handle) => iced_widget::image(handle.clone())
                .width(Length::Fixed(AVATAR))
                .height(Length::Fixed(AVATAR))
                .content_fit(iced_core::ContentFit::Cover)
                .into(),
            None => text(
                self.usuario
                    .nombre
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default(),
            )
            .size(18.0)
            .color(Color::WHITE)
            .into(),
        };
        let redondo = container(avatar)
            .width(Length::Fixed(AVATAR))
            .height(Length::Fixed(AVATAR))
            .center_x(Length::Fixed(AVATAR))
            .center_y(Length::Fixed(AVATAR))
            .clip(true)
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(acento_centro().into()),
                border: Border {
                    radius: (AVATAR / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let pildora = container(
            row![
                column![
                    text(self.usuario.nombre.clone())
                        .size(15.0)
                        .color(tema::texto()),
                    text(format!("@{}", self.usuario.cuenta))
                        .size(11.0)
                        .color(tema::TEXTO2),
                ],
                Space::new().width(Length::Fill),
                redondo,
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(
            ANCHO - MARGEN * 2.0 - (BOTON_CABECERA + 8.0) * 3.0,
        ))
        .height(Length::Fixed(CABECERA))
        .padding([0, 8])
        .center_y(Length::Fixed(CABECERA))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(contenedor().into()),
            border: Border {
                radius: (CABECERA / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });

        row![
            pildora,
            Space::new().width(Length::Fixed(8.0)),
            Self::circulo("editar", BOTON_CABECERA, contenedor(), tema::texto()),
            Space::new().width(Length::Fixed(8.0)),
            Self::circulo("apagar", BOTON_CABECERA, contenedor(), tema::texto()),
            Space::new().width(Length::Fixed(8.0)),
            Self::circulo("preferencias", BOTON_CABECERA, contenedor(), tema::texto()),
        ]
        .align_y(Vertical::Center)
        .into()
    }

    /// Una de las dos tarjetas de conectividad.
    fn conexion(&self, bluetooth: bool) -> PanelElement<'_> {
        let (nombre, encendido, icono) = if bluetooth {
            (
                "Bluetooth",
                self.bluetooth,
                if self.bluetooth {
                    "bluetooth"
                } else {
                    "bluetooth-apagado"
                },
            )
        } else {
            (
                "Wi-Fi",
                self.wifi,
                if self.wifi { "wifi" } else { "sin-red" },
            )
        };
        container(
            row![
                Self::circulo(
                    icono,
                    ICONO_CONEXION,
                    if encendido {
                        acento_centro()
                    } else {
                        apagado()
                    },
                    if encendido {
                        tema::sobre_acento()
                    } else {
                        tinta_apagado()
                    },
                ),
                Space::new().width(Length::Fixed(10.0)),
                column![
                    text(nombre).size(15.0).color(tema::texto()),
                    text(if encendido { "Activado" } else { "Desactivado" })
                        .size(11.0)
                        .color(tema::TEXTO2),
                ],
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed((ANCHO - MARGEN * 2.0 - 8.0) / 2.0))
        .height(Length::Fixed(CONEXION))
        .padding([0, 8])
        .center_y(Length::Fixed(CONEXION))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(contenedor().into()),
            border: Border {
                radius: tema::R_TARJETA.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    fn baldosa(&self, i: usize) -> PanelElement<'_> {
        let baldosa = &self.baldosas[i];
        // Lo que cambia entre encendida y apagada es el círculo de detrás, no
        // el icono: acento contra el apagado del tema. La tinta la decide
        // [`tinta_apagado`], que es donde está escrito por qué sobre el azul
        // claro sigue siendo blanca.
        // Señalada, el círculo se aclara hasta el 85 % de alfa. Interpolado y
        // no conmutado: la rejilla tiene ocho baldosas juntas y el salto de
        // opacidad se lee como un parpadeo al cruzarla con el ratón.
        let base = if baldosa.activa {
            acento_centro()
        } else {
            apagado()
        };
        let fondo = tema::alfa(base, base.a - 0.15 * base.a * self.señalada.intensidad(i));
        let tinta = if baldosa.activa {
            tema::sobre_acento()
        } else {
            tinta_apagado()
        };
        let celda = (ANCHO - (MARGEN + REJILLA_MARGEN) * 2.0) / COLUMNAS as f32;
        container(Self::circulo(baldosa.icono, BOTON, fondo, tinta))
            .width(Length::Fixed(celda))
            .height(Length::Fixed(BOTON + 8.0))
            .center_x(Length::Fixed(celda))
            .center_y(Length::Fixed(BOTON + 8.0))
            .into()
    }

    fn deslizador(&self, cual: Agarre) -> PanelElement<'_> {
        let (nivel, icono) = match cual {
            Agarre::Brillo => (self.brillo, "brillo"),
            _ => (
                self.volumen,
                match self.volumen {
                    0 => "volumen-silencio",
                    1..=33 => "volumen-bajo",
                    34..=66 => "volumen-medio",
                    _ => "volumen-alto",
                },
            ),
        };
        row![
            control::pildora(self.ancho_pildora(), nivel, false),
            Space::new().width(Length::Fixed(10.0)),
            Self::circulo(icono, BOTON_PILDORA, apagado(), tinta_apagado()),
        ]
        .align_y(Vertical::Center)
        .into()
    }

    fn medios(&self, sonando: &Sonando) -> PanelElement<'_> {
        let ancho_barra = ANCHO - MARGEN * 2.0 - 24.0;
        let avance = sonando.avance().unwrap_or(0.0);
        let barra = container(
            container(Space::new())
                .width(Length::Fixed(ancho_barra * avance))
                .height(Length::Fixed(4.0))
                .style(|_theme: &iced_widget::Theme| container::Style {
                    background: Some(tema::texto().into()),
                    border: Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .width(Length::Fixed(ancho_barra))
        .height(Length::Fixed(4.0))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(
                Color {
                    a: 0.20,
                    ..tema::tinta()
                }
                .into(),
            ),
            border: Border {
                radius: 2.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let boton = |nombre: &str| -> PanelElement<'static> {
            match icono::propio(nombre) {
                Some(ic) => icono::ver_teñido_propio(&ic, 24.0, tema::texto()),
                None => Space::new().width(Length::Fixed(24.0)).into(),
            }
        };
        let cabecera = row![
            match icono::propio("musica") {
                Some(ic) => icono::ver_teñido_propio(&ic, 13.0, tema::TEXTO2),
                None => Space::new().width(Length::Fixed(13.0)).into(),
            },
            Space::new().width(Length::Fixed(6.0)),
            text(sonando.aplicacion.clone())
                .size(11.0)
                .color(tema::TEXTO2),
        ]
        .align_y(Vertical::Center);

        let contenido = column![
            cabecera,
            Space::new().height(Length::Fixed(4.0)),
            text(sonando.titulo.clone()).size(15.0).color(tema::texto()),
            text(sonando.artista.clone()).size(11.0).color(tema::TEXTO2),
            Space::new().height(Length::Fixed(6.0)),
            barra,
            row![
                text(medios::reloj(sonando.posicion.unwrap_or(0)))
                    .size(10.0)
                    .color(tema::TEXTO2),
                Space::new().width(Length::Fill),
                text(
                    sonando
                        .duracion
                        .map(medios::reloj)
                        .unwrap_or_else(|| "--:--".into())
                )
                .size(10.0)
                .color(tema::TEXTO2),
            ],
            container(
                row![
                    boton("anterior"),
                    Space::new().width(Length::Fixed(24.0)),
                    boton(if sonando.reproduciendo {
                        "pausa"
                    } else {
                        "reproducir"
                    }),
                    Space::new().width(Length::Fixed(24.0)),
                    boton("siguiente"),
                ]
                .align_y(Vertical::Center)
            )
            .width(Length::Fill)
            .align_x(Horizontal::Center),
        ];
        container(contenido)
            .width(Length::Fixed(ANCHO - MARGEN * 2.0))
            .height(Length::Fixed(MEDIOS))
            .padding([8, 12])
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(contenedor().into()),
                border: Border {
                    radius: tema::R_TARJETA.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// Una tarjeta interior con su fondo.
    fn tarjeta<'a>(contenido: PanelElement<'a>, alto: f32) -> PanelElement<'a> {
        container(contenido)
            .width(Length::Fixed(ANCHO - MARGEN * 2.0))
            .height(Length::Fixed(alto))
            .padding([0, REJILLA_MARGEN as u16])
            .center_y(Length::Fixed(alto))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(contenedor().into()),
                border: Border {
                    radius: tema::R_TARJETA.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut rejilla = column![];
        for fila in 0..self.baldosas.len().div_ceil(COLUMNAS) {
            let mut linea = row![];
            for columna in 0..COLUMNAS {
                let i = fila * COLUMNAS + columna;
                if i < self.baldosas.len() {
                    linea = linea.push(self.baldosa(i));
                }
            }
            rejilla = rejilla.push(linea);
        }

        let mut contenido = column![
            self.cabecera(),
            Space::new().height(Length::Fixed(HUECO)),
            row![
                self.conexion(false),
                Space::new().width(Length::Fixed(8.0)),
                self.conexion(true),
            ],
            Space::new().height(Length::Fixed(HUECO)),
            Self::tarjeta(rejilla.into(), self.alto_rejilla()),
            Space::new().height(Length::Fixed(HUECO)),
            Self::tarjeta(
                column![
                    self.deslizador(Agarre::Volumen),
                    Space::new().height(Length::Fixed(10.0)),
                    self.deslizador(Agarre::Brillo),
                ]
                .into(),
                self.alto_deslizadores(),
            ),
        ];
        if let Some(sonando) = &self.sonando {
            contenido = contenido
                .push(Space::new().height(Length::Fixed(HUECO)))
                .push(self.medios(sonando));
        }
        control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

/// Las ocho baldosas de la rejilla, en el orden del plasmoide.
fn baldosas() -> Vec<Baldosa> {
    // `rfkill list` dice si hay algo bloqueado por software; el modo avión es
    // exactamente eso.
    let avion = orden("rfkill list").contains("Soft blocked: yes");
    let ahorro = orden("cat /sys/firmware/acpi/platform_profile").contains("low-power");
    vec![
        Baldosa {
            icono: "avion",
            activa: avion,
            accion: Some(Accion::Lanzar(format!(
                "rfkill {} all",
                if avion { "unblock" } else { "block" }
            ))),
            interna: None,
        },
        Baldosa {
            icono: "notificaciones",
            activa: false,
            accion: None,
            interna: None,
        },
        Baldosa {
            icono: "ahorro",
            activa: ahorro,
            accion: Some(Accion::Lanzar(format!(
                "busctl --system set-property net.hadess.PowerProfiles \
                 /net/hadess/PowerProfiles net.hadess.PowerProfiles ActiveProfile s {}",
                if ahorro { "balanced" } else { "power-saver" }
            ))),
            interna: None,
        },
        Baldosa {
            icono: "mantener",
            activa: false,
            accion: None,
            interna: Some(Interna::Mantener),
        },
        Baldosa {
            icono: "noche",
            activa: false,
            // La luz nocturna la aplica el compositor cambiando la curva de
            // color de la salida, y eso todavía no existe: de momento abre
            // donde se configurará.
            accion: Some(Accion::Lanzar("bookos-settings --pantalla".into())),
            interna: None,
        },
        Baldosa {
            icono: "compartir",
            activa: false,
            accion: Some(Accion::Lanzar("bookos-share || kdeconnect-app".into())),
            interna: None,
        },
        Baldosa {
            icono: "captura",
            activa: false,
            accion: Some(Accion::Lanzar("bookos-captura || spectacle".into())),
            interna: None,
        },
        Baldosa {
            icono: "bloquear",
            activa: false,
            accion: Some(Accion::Lanzar(
                "loginctl lock-session || qdbus6 org.freedesktop.ScreenSaver /ScreenSaver Lock"
                    .into(),
            )),
            interna: None,
        },
    ]
}

/// `on`/`off`, que es lo que entienden `nmcli` y `bluetoothctl`.
fn si_no(encendido: bool) -> &'static str {
    if encendido {
        "on"
    } else {
        "off"
    }
}

/// Ejecuta una orden corta y devuelve su salida. Vacío si falla.
fn orden(cmd: &str) -> String {
    Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stderr(Stdio::null())
        .output()
        .map(|s| String::from_utf8_lossy(&s.stdout).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La rejilla reparte los clics por celdas, cuatro por dos.
    #[test]
    fn cada_celda_de_la_rejilla_es_de_su_baldosa() {
        let c = Centro::new();
        let x0 = MARGEN + REJILLA_MARGEN;
        let y0 = c.y_rejilla() + REJILLA_MARGEN;
        let ancho_celda = (ANCHO - x0 * 2.0) / COLUMNAS as f32;
        assert_eq!(c.baldosa_en(x0 + 5.0, y0 + 5.0), Some(0));
        assert_eq!(
            c.baldosa_en(x0 + ancho_celda * 3.0 + 5.0, y0 + 5.0),
            Some(3)
        );
        assert_eq!(c.baldosa_en(x0 + 5.0, y0 + BOTON + 12.0), Some(4));
        assert_eq!(
            c.baldosa_en(2.0, y0 + 5.0),
            None,
            "el margen no es de nadie"
        );
    }

    /// Los dos deslizadores no se pisan: agarrar el de abajo no mueve el de
    /// arriba, que es el fallo que ya tuvo el emergente del brillo.
    #[test]
    fn los_deslizadores_no_se_pisan() {
        let mut c = Centro::new();
        let r = c.rect_pildora(Agarre::Brillo);
        c.pulsar(r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert_eq!(c.agarre, Agarre::Brillo);
        c.soltar();
        let r = c.rect_pildora(Agarre::Volumen);
        c.pulsar(r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert_eq!(c.agarre, Agarre::Volumen);
    }

    /// Las dos tarjetas de conectividad abren su lista, cada una la suya.
    #[test]
    fn las_tarjetas_de_conexion_abren_su_lista() {
        let mut c = Centro::new();
        let y = c.y_conexiones() + CONEXION / 2.0;
        // Fuera del círculo: la mitad derecha de cada tarjeta.
        assert_eq!(c.pulsar(140.0, y), Some(Accion::Emergente("red")));
        assert_eq!(
            c.pulsar(ANCHO - 40.0, y),
            Some(Accion::Emergente("bluetooth"))
        );
    }

    /// Y el círculo de esas tarjetas enciende y apaga la radio, sin abrir nada.
    #[test]
    fn el_circulo_de_la_tarjeta_alterna_la_radio() {
        let mut c = Centro::new();
        let y = c.y_conexiones() + CONEXION / 2.0;
        let antes = c.wifi;
        let accion = c.pulsar(MARGEN + ICONO_CONEXION / 2.0, y);
        assert_ne!(c.wifi, antes, "el estado no cambió");
        let esperado = if antes { "off" } else { "on" };
        assert_eq!(
            accion,
            Some(Accion::Lanzar(format!("nmcli radio wifi {esperado}")))
        );
    }

    /// «Mantener la pantalla encendida» es un proceso vivo: encenderlo lo
    /// arranca y apagarlo lo mata.
    #[test]
    fn mantener_la_pantalla_es_un_proceso() {
        let mut c = Centro::new();
        let Some(i) = c
            .baldosas
            .iter()
            .position(|b| b.interna == Some(Interna::Mantener))
        else {
            panic!("no está la baldosa de mantener la pantalla");
        };
        c.alternar(Interna::Mantener, i);
        // Sin `systemd-inhibit` en el sistema no hay nada más que comprobar.
        if c.inhibidor.is_none() {
            return;
        }
        assert!(c.baldosas[i].activa);
        c.alternar(Interna::Mantener, i);
        assert!(c.inhibidor.is_none(), "el inhibidor sigue vivo");
        assert!(!c.baldosas[i].activa);
    }
}
