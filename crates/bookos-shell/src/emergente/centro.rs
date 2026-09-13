//! El centro de control, con el aspecto del plasmoide `bookos-controlcenter`.
//!
//! Cinco bloques, de arriba abajo: quién ha iniciado sesión con tres botones al
//! lado, las dos tarjetas de conectividad, la rejilla de ocho conmutadores, los
//! deslizadores de volumen y brillo, y lo que esté sonando.
//!
//! **El acento significa «encendido» y nada más.** Un conmutador apagado va en
//! `--surface`, el velo neutro del sistema de diseño, con el glifo en texto
//! secundario. Antes iba en acento aclarado, y el resultado se ve en cuanto se
//! mira la rejilla entera: ocho círculos azules de los que ninguno dice si está
//! activo, porque el único color que quedaba libre para decirlo era ese mismo
//! azul. La regla es del sistema de diseño —el acento es lo único que significa
//! «esto está activo»— y aquí no se cumplía.
//!
//! **Cada baldosa lleva su nombre debajo.** Ocho círculos mudos obligan a
//! aprenderse cuál es el ojo y cuál el rombo.
//!
//! **Todo lo que se dibuja hace algo.** Los estados que se pueden leer se leen
//! desde la caché compartida para red, Bluetooth y audio; los que no tienen
//! forma de consultarse son acciones, no interruptores. Mantener la pantalla
//! encendida es la excepción: su estado **es** un proceso hijo vivo, y por eso
//! lo guarda el shell.

use std::process::{Child, Command, Stdio};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::bloqueo::Usuario;
use crate::icono;
use crate::medios::{self, Sonando};
use crate::tema;
use crate::view::PanelElement;

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
/// Alto de una celda de la rejilla: el círculo, un hueco y el rótulo.
const CELDA: f32 = BOTON + 4.0 + 14.0 + 6.0;
/// Alto de un deslizador. El icono y el porcentaje van **dentro**, así que
/// tiene que caber un glifo de 18 con aire.
const PILDORA: f32 = 42.0;
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
/// El círculo de un conmutador apagado: otro escalón de `--surface`.
///
/// Neutro a propósito. Pasó por el `#AECAFF` del plasmoide y luego por
/// `acento_suave()`, que al menos seguía al acento elegido, pero las dos
/// versiones tenían el mismo fallo de fondo: con siete baldosas apagadas en
/// acento claro y una encendida en acento sólido, lo que se ve es una rejilla
/// de un solo color. El estado tiene que leerse antes que la marca.
///
/// Es un velo sobre lo que hay detrás y no un color fijo porque el círculo va
/// **dentro** de una tarjeta que ya es `--surface`: los dos alfas se acumulan,
/// el círculo se separa de su tarjeta y la tarjeta del panel.
fn apagado() -> Color {
    tema::superficie()
}

/// La tinta del icono dentro de un círculo apagado: el texto secundario.
///
/// El mismo gris en los dos temas —lo fija así el sistema de diseño—, y por eso
/// no se calcula contra el fondo como cuando el círculo era de acento: sobre
/// `--surface`, que es casi el fondo, sale igual en claro y en oscuro.
fn tinta_apagado() -> Color {
    tema::TEXTO2
}

/// Fondo de las tarjetas interiores: el `--surface` del sistema de diseño.
///
/// Antes era `alfa(card(), 0.80)` en oscuro, o sea el mismo token que pinta el
/// panel que las contiene: `#1c1c1e` al 80 % **sobre** `#1c1c1e` da otra vez
/// `#1c1c1e`. Las cuatro subdivisiones existían en el árbol de widgets y no se
/// veía ninguna. En claro pasaba lo mismo de forma más suave: `bg` al 80 %
/// sobre la tarjeta blanca deja `#f5f5f9`, tres puntos por canal.
fn contenedor() -> Color {
    tema::superficie()
}

/// Un conmutador o una acción de la rejilla.
struct Baldosa {
    icono: &'static str,
    /// El rótulo de debajo. Es lo que hace, no cómo se llama el icono.
    nombre: &'static str,
    /// Encendida: círculo en acento sólido. Apagada: el acento aclarado.
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
    /// El servicio puede tardar en aparecer o estar reiniciándose. Se separa
    /// de `wifi` para no pintar «apagado» cuando todavía no hay estado.
    wifi_disponible: bool,
    bluetooth: bool,
    bluetooth_disponible: bool,
    /// La red a la que se está conectado, si hay alguna. La radio encendida no
    /// basta: `nmcli radio wifi` dice `enabled` con la antena buscando y sin
    /// haber llegado a ninguna parte.
    red: Option<String>,
    /// El dispositivo Bluetooth conectado, si hay alguno.
    dispositivo: Option<String>,
    volumen: u8,
    audio_disponible: bool,
    brillo: u8,
    agarre: Agarre,
    backlight: Option<(String, u32)>,
    sonando: Option<Sonando>,
    /// El `systemd-inhibit` que impide que la pantalla se apague mientras esté
    /// puesto. Matarlo es apagar el conmutador.
    inhibidor: Option<Child>,
}

impl Centro {
    pub fn refrescar(&mut self) -> bool {
        let state = bookos_system::snapshot();
        let wifi_disponible = !state.network.is_null();
        let bluetooth_disponible = !state.bluetooth.is_null();
        let audio_disponible = !state.audio.is_null();
        let wifi = wifi_disponible && state.network["enabled"] == true;
        let bluetooth = bluetooth_disponible && state.bluetooth["enabled"] == true;
        let red = wifi_disponible.then(conectado_a).flatten();
        let dispositivo = bluetooth_disponible.then(emparejado_con).flatten();
        let volumen = if self.agarre == Agarre::Volumen { self.volumen } else { bookos_system::volume(false).map(|v|v.0).unwrap_or(self.volumen) };
        let changed = self.wifi != wifi || self.wifi_disponible != wifi_disponible
            || self.bluetooth != bluetooth || self.bluetooth_disponible != bluetooth_disponible
            || self.audio_disponible != audio_disponible
            || self.red != red || self.dispositivo != dispositivo || self.volumen != volumen;
        self.wifi = wifi; self.wifi_disponible = wifi_disponible;
        self.bluetooth = bluetooth; self.bluetooth_disponible = bluetooth_disponible;
        self.audio_disponible = audio_disponible;
        self.red = red; self.dispositivo = dispositivo; self.volumen = volumen;
        let mut changed = changed;
        if self.agarre != Agarre::Brillo {
            let backlight = crate::backlight();
            let brillo = crate::brillo_actual().unwrap_or(0);
            changed |= self.backlight != backlight || self.brillo != brillo;
            self.backlight = backlight; self.brillo = brillo;
        }
        if let Some(tile) = self.baldosas.iter_mut().find(|b| b.icono == "avion") {
            let active = wifi_disponible && bluetooth_disponible
                && state.network["enabled"] == false && state.bluetooth["enabled"] == false;
            changed |= tile.activa != active; tile.activa = active;
        }
        changed
    }

    pub fn new() -> Self {
        let state = bookos_system::snapshot();
        let wifi_disponible = !state.network.is_null();
        let bluetooth_disponible = !state.bluetooth.is_null();
        let audio_disponible = !state.audio.is_null();
        let wifi = wifi_disponible && state.network["enabled"] == true;
        let bluetooth = bluetooth_disponible && state.bluetooth["enabled"] == true;
        let red = wifi_disponible.then(conectado_a).flatten();
        let dispositivo = bluetooth_disponible.then(emparejado_con).flatten();
        let mut c = Self {
            // El centro de control no ve la configuración: la foto de `panel.conf`
            // la resuelve el bloqueo, y aquí basta con las de siempre.
            usuario: Usuario::leer(None),
            baldosas: Vec::new(),
            señalada: tema::Realce::nuevo(),
            wifi,
            wifi_disponible,
            bluetooth,
            bluetooth_disponible,
            red,
            dispositivo,
            // El nivel de partida viene de la caché compartida.
            // medidos, pero esto ocurre al abrir la tarjeta y no en el refresco
            // del panel.
            volumen: crate::widgets::volumen::consultar("@DEFAULT_AUDIO_SINK@")
                .map(|(n, _)| n)
                .unwrap_or(0),
            audio_disponible,
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
        REJILLA_MARGEN * 2.0 + filas * CELDA
    }

    fn alto_deslizadores(&self) -> f32 {
        REJILLA_MARGEN * 2.0 + PILDORA * 2.0 + 10.0
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
        let alto_celda = CELDA;
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

    /// Las píldoras ocupan la tarjeta entera de lado a lado: sin el botón
    /// redondo de al lado, los 50 px que se comía vuelven al recorrido. Con el
    /// volumen al 3 % eso son 9 px de relleno en vez de 7, pero lo que importa
    /// es que **toda** la píldora es agarrable, así que un nivel bajo se sigue
    /// pudiendo coger.
    fn ancho_pildora(&self) -> f32 {
        ANCHO - (MARGEN + REJILLA_MARGEN) * 2.0
    }

    fn rect_pildora(&self, cual: Agarre) -> iced_core::Rectangle {
        let y0 = self.y_deslizadores() + REJILLA_MARGEN;
        let y = match cual {
            Agarre::Brillo => y0 + PILDORA + 10.0,
            _ => y0,
        };
        iced_core::Rectangle {
            x: MARGEN + REJILLA_MARGEN,
            y,
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
            let disponible = if bluetooth { self.bluetooth_disponible } else { self.wifi_disponible };
            if x < mitad + 8.0 + ICONO_CONEXION {
                if !disponible {
                    return None;
                }
                let encendido = if bluetooth { self.bluetooth } else { self.wifi };
                bookos_system::request(if bluetooth {
                    bookos_system::Operation::BluetoothPower { enabled: !encendido }
                } else { bookos_system::Operation::WifiPower { enabled: !encendido } });
                return None;
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
        if self.baldosas[i].icono == "avion" {
            bookos_system::request(bookos_system::Operation::Airplane { enabled: !self.baldosas[i].activa });
            return None;
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
        bookos_system::request(bookos_system::Operation::Volume { target: "output".into(), value: nivel as u32 });
        true
    }

    fn poner_brillo(&mut self, nivel: u8) -> bool {
        // Mismo suelo que el emergente del brillo: a cero no se ve ni el
        // deslizador con el que volver a subirlo.
        if self.backlight.is_none() { return false; }
        let nivel = nivel.clamp(5, 100);
        if nivel == self.brillo {
            return false;
        }
        self.brillo = nivel;
        if let Some((dispositivo, maximo)) = &self.backlight {
            let crudo = (*maximo as f64 * nivel as f64 / 100.0).round() as u32;
            crate::retroiluminacion::solicitar("backlight", dispositivo, crudo);
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
                redondo,
                Space::new().width(Length::Fixed(10.0)),
                column![
                    text(self.usuario.nombre.clone())
                        .size(15.0)
                        .color(tema::texto()),
                    text(format!("@{}", self.usuario.cuenta))
                        .size(11.0)
                        .color(tema::TEXTO2),
                ],
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

        // Apagar va el último y con el glifo en rojo. Es la única acción de la
        // tarjeta que se lleva la sesión por delante, y estaba en medio de las
        // otras dos pintada igual que ellas: la esquina y el color son lo que
        // impide darle queriendo abrir Preferencias.
        row![
            pildora,
            Space::new().width(Length::Fixed(8.0)),
            Self::circulo("editar", BOTON_CABECERA, contenedor(), tema::texto()),
            Space::new().width(Length::Fixed(8.0)),
            Self::circulo("preferencias", BOTON_CABECERA, contenedor(), tema::texto()),
            Space::new().width(Length::Fixed(8.0)),
            Self::circulo("apagar", BOTON_CABECERA, contenedor(), tema::rojo()),
        ]
        .align_y(Vertical::Center)
        .into()
    }

    /// Una de las dos tarjetas de conectividad.
    fn conexion(&self, bluetooth: bool) -> PanelElement<'_> {
        // El círculo va en acento cuando hay **conexión**, no cuando la radio
        // está encendida. Es la misma regla que la rejilla: el acento dice que
        // algo está funcionando. Con la antena encendida y sin red, la tarjeta
        // se queda apagada y el subtítulo dice «Sin conexión», que es lo que
        // hay que ver cuando algo no va.
        let disponible = if bluetooth { self.bluetooth_disponible } else { self.wifi_disponible };
        let (nombre, encendido, conectado, icono) = if bluetooth {
            (
                "Bluetooth",
                self.bluetooth,
                self.dispositivo.as_deref(),
                if self.bluetooth && disponible {
                    "bluetooth"
                } else {
                    "bluetooth-apagado"
                },
            )
        } else {
            (
                "Wi-Fi",
                self.wifi,
                self.red.as_deref(),
                if self.wifi && disponible { "wifi" } else { "sin-red" },
            )
        };
        // El subtítulo se corta a lo que cabe en una línea. La tarjeta mide
        // 154 de ancho y el círculo con su hueco se lleva 54, así que quedan
        // 84 px: «Wifi Recamales-5G» a 11 px ocupa 89 y bajaba a una segunda
        // línea que se salía de la tarjeta por abajo, porque el alto es fijo.
        let estado = if !disponible { "Servicio no disponible".to_owned() } else { match (encendido, conectado) {
            (_, Some(donde)) => recortar(donde, 15),
            (true, None) => "Sin conexión".to_owned(),
            (false, None) => "Desactivado".to_owned(),
        }};
        container(
            row![
                Self::circulo(
                    icono,
                    ICONO_CONEXION,
                    if conectado.is_some() {
                        acento_centro()
                    } else {
                        apagado()
                    },
                    if conectado.is_some() {
                        tema::sobre_acento()
                    } else {
                        tinta_apagado()
                    },
                ),
                Space::new().width(Length::Fixed(10.0)),
                column![
                    text(nombre)
                        .size(15.0)
                        .color(tema::texto())
                        .wrapping(iced_core::text::Wrapping::None),
                    // Sin envolver: el SSID recortado sigue teniendo espacios
                    // y iced parte por ellos antes que desbordar. «Wifi
                    // Recamales…» bajaba a dos líneas y la segunda se salía de
                    // la tarjeta, que tiene el alto fijo.
                    text(estado)
                        .size(11.0)
                        .color(tema::TEXTO2)
                        .wrapping(iced_core::text::Wrapping::None),
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
        // El rótulo de una baldosa encendida va en texto pleno y el de una
        // apagada en secundario: el estado se lee dos veces, en el círculo y
        // en la palabra, que es lo que necesita quien no distingue el acento
        // del gris de al lado.
        let rotulo = text(baldosa.nombre)
            .size(11.0)
            .color(if baldosa.activa {
                tema::texto()
            } else {
                tema::TEXTO2
            })
            .align_x(Horizontal::Center)
            .width(Length::Fixed(celda));
        container(
            column![
                Self::circulo(baldosa.icono, BOTON, fondo, tinta),
                Space::new().height(Length::Fixed(4.0)),
                rotulo,
            ]
            .align_x(Horizontal::Center),
        )
        .width(Length::Fixed(celda))
        .height(Length::Fixed(CELDA))
        .center_x(Length::Fixed(celda))
        .center_y(Length::Fixed(CELDA))
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
        let ancho = self.ancho_pildora();
        // El relleno nunca es más estrecho que alto. Con 6 px de ancho, iced
        // recorta su radio a 3 —min(w,h)/2— y sale una barrita casi recta,
        // mientras que el surco de detrás sigue curvándose con radio 21: el
        // relleno del volumen al 2 % asomaba por encima y por debajo del surco
        // en las dos esquinas. El radio del contenedor no recorta a los hijos,
        // así que no hay forma de meterlo dentro de la curva.
        //
        // El precio es que por debajo del 14 % —42 de 293— la barra enseña más
        // de lo que hay. Es lo que hacen los deslizadores de iOS y de One UI, y
        // se prefiere a un relleno roto: el número exacto está escrito al lado.
        let lleno = match nivel.min(100) {
            0 => 0.0,
            n => (ancho * n as f32 / 100.0).max(PILDORA),
        };

        // El icono y el porcentaje viran cuando el relleno los alcanza, y cada
        // uno decide por su cuenta: sobre el acento va la tinta que se lea
        // encima, sobre el surco va la del tema. Fijarlos a blanco no vale —el
        // surco en claro es negro al 12 %, o sea un gris de luminancia 0,83, y
        // un glifo blanco encima da 1,2:1 y desaparece. Se vio en el volcado de
        // `--example acento` con el volumen al 2 %.
        //
        // Los umbrales son dónde acaba cada elemento, no el centro de la
        // píldora: el icono ocupa de 14 a 32 y el porcentaje los 44 últimos.
        let tinta_de = |pisado: bool| {
            if pisado {
                tema::sobre_acento()
            } else {
                tema::texto()
            }
        };
        let dentro = row![
            match icono::propio(icono) {
                Some(ic) => icono::ver_teñido_propio(&ic, 18.0, tinta_de(lleno >= 32.0)),
                None => Space::new().width(Length::Fixed(18.0)).into(),
            },
            Space::new().width(Length::Fill),
            text(format!("{nivel}%"))
                .size(12.0)
                .color(tema::alfa(tinta_de(lleno >= ancho - 44.0), 0.75)),
        ]
        .align_y(Vertical::Center);

        let relleno = container(Space::new())
            .width(Length::Fixed(lleno))
            .height(Length::Fixed(PILDORA))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(acento_centro().into()),
                border: Border {
                    radius: (PILDORA / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        // El `Stack` mide por su **primer** hijo: con el relleno delante, la
        // capa del icono se recortaba a lo que midiera el relleno y el «50 %»
        // salía pegado al borde del azul en vez de al de la píldora. Por eso
        // el relleno va envuelto en una capa del ancho completo.
        container(iced_widget::stack![
            container(relleno)
                .width(Length::Fixed(ancho))
                .height(Length::Fixed(PILDORA)),
            container(dentro)
                .width(Length::Fixed(ancho))
                .height(Length::Fixed(PILDORA))
                .padding([0, 14])
                .center_y(Length::Fixed(PILDORA)),
        ])
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(PILDORA))
        .clip(true)
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(tema::surco().into()),
            border: Border {
                radius: (PILDORA / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
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
    let state = bookos_system::snapshot();
    let avion = !state.network.is_null() && !state.bluetooth.is_null()
        && state.network["enabled"] == false && state.bluetooth["enabled"] == false;
    let ahorro = std::fs::read_to_string("/sys/firmware/acpi/platform_profile").unwrap_or_default().contains("low-power");
    vec![
        Baldosa {
            icono: "avion",
            nombre: "Avión",
            activa: avion,
            accion: None,
            interna: None,
        },
        Baldosa {
            icono: "notificaciones",
            nombre: "Avisos",
            activa: false,
            accion: None,
            interna: None,
        },
        Baldosa {
            icono: "ahorro",
            nombre: "Ahorro",
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
            nombre: "Despierta",
            activa: false,
            accion: None,
            interna: Some(Interna::Mantener),
        },
        Baldosa {
            icono: "noche",
            nombre: "Noche",
            activa: false,
            // La luz nocturna la aplica el compositor cambiando la curva de
            // color de la salida, y eso todavía no existe: de momento abre
            // donde se configurará.
            accion: Some(Accion::Lanzar("bookos-settings --pantalla".into())),
            interna: None,
        },
        Baldosa {
            icono: "compartir",
            nombre: "Compartir",
            activa: false,
            accion: Some(Accion::Lanzar("bookos-share || kdeconnect-app".into())),
            interna: None,
        },
        Baldosa {
            icono: "captura",
            nombre: "Captura",
            activa: false,
            accion: Some(Accion::Lanzar("bookos-captura || spectacle".into())),
            interna: None,
        },
        Baldosa {
            icono: "bloquear",
            nombre: "Bloquear",
            activa: false,
            accion: Some(Accion::Lanzar(
                "loginctl lock-session || qdbus6 org.freedesktop.ScreenSaver /ScreenSaver Lock"
                    .into(),
            )),
            interna: None,
        },
    ]
}

/// Corta un nombre a `max` caracteres y le pone puntos suspensivos.
///
/// Por caracteres y no por píxeles: medir el texto pide el `Renderer`, que aquí
/// no está, y el error de contar caracteres solo se nota con un SSID lleno de
/// emes. El corte es por `char` y no por byte para no partir una letra acentuada
/// en dos.
fn recortar(texto: &str, max: usize) -> String {
    if texto.chars().count() <= max {
        return texto.to_owned();
    }
    texto.chars().take(max - 1).collect::<String>() + "…"
}

fn conectado_a() -> Option<String> {
    let s = bookos_system::snapshot();
    s.network["ssid"].as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}
fn emparejado_con() -> Option<String> {
    let s = bookos_system::snapshot();
    s.bluetooth["devices"].as_array()?.iter().find(|d| d["connected"] == true)?["name"].as_str().map(str::to_owned)
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
        assert_eq!(
            c.baldosa_en(x0 + 5.0, y0 + CELDA + 5.0),
            Some(4),
            "la segunda fila empieza tras la celda entera, rótulo incluido"
        );
        assert_eq!(
            c.baldosa_en(x0 + 5.0, y0 + BOTON + 12.0),
            Some(0),
            "el rótulo de la primera fila sigue siendo su baldosa"
        );
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
        assert_eq!(c.wifi, antes, "el estado solo cambia al confirmarlo el servicio");
        assert_eq!(accion, None);
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
