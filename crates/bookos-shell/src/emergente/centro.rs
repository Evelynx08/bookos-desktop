//! El centro de control, con el aspecto del plasmoide `bookos-controlcenter`.
//!
//! Cinco bloques, de arriba abajo: quién ha iniciado sesión con dos botones al
//! lado, las dos píldoras de conectividad, la rejilla de ocho conmutadores, los
//! deslizadores de volumen y brillo, y lo que esté sonando. Las medidas son las
//! del lienzo «Widgets de BookOS»: 352 de ancho, 16 de margen y 12 entre
//! bloques, con los tres de abajo en grupos grises.
//!
//! **El acento significa «encendido» y nada más.** Un conmutador apagado va en
//! el color de la tarjeta, que dentro de su grupo gris es lo que se despega, con
//! el glifo en el color del texto. Antes iba en acento aclarado, y el resultado se ve en cuanto se
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
use iced_core::font::Weight;
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

const ANCHO: f32 = 352.0;
const MARGEN: f32 = 16.0;
/// Hueco entre bloques.
const HUECO: f32 = 12.0;
/// Alto de la cabecera y de sus botones redondos.
const CABECERA: f32 = 52.0;
const BOTON_CABECERA: f32 = 40.0;
/// Lado del avatar.
const AVATAR: f32 = 40.0;
/// Alto de las dos píldoras de conectividad.
const CONEXION: f32 = 52.0;
/// Lado del círculo de una píldora de conectividad.
const ICONO_CONEXION: f32 = 40.0;
/// Relleno de la píldora hasta su círculo.
const RELLENO_CONEXION: f32 = 6.0;
/// Rejilla: cuatro columnas por dos filas de botones redondos.
const COLUMNAS: usize = 4;
const BOTON: f32 = 48.0;
/// Relleno del grupo de la rejilla.
const REJILLA_MARGEN: f32 = 14.0;
/// Aire entre las dos filas de la rejilla.
const ENTRE_FILAS: f32 = 14.0;
/// Alto de una celda de la rejilla: el círculo, un hueco y el rótulo.
const CELDA: f32 = BOTON + 6.0 + 14.0;
/// Relleno de los grupos de los deslizadores y de lo que suena.
const GRUPO_MARGEN: f32 = 12.0;
/// Aire entre los dos deslizadores.
const ENTRE_DESLIZADORES: f32 = 10.0;
/// Lado de la carátula de lo que suena.
const CARATULA: f32 = 48.0;
/// Alto de la barra de avance de la canción: la misma píldora que el volumen,
/// más estrecha, para que no se lea como otro volumen.
const AVANCE: f32 = 10.0;
/// Alto del renglón de los tiempos.
const TIEMPOS: f32 = 14.0;
/// Lado del botón de reproducir.
const REPRODUCIR: f32 = 44.0;
/// Hueco entre los botones de reproducción.
const ENTRE_MANDOS: f32 = 28.0;
/// Alto de lo que suena: carátula y textos, avance, tiempos y mandos, con el
/// relleno del grupo.
const MEDIOS: f32 =
    GRUPO_MARGEN * 2.0 + CARATULA + 10.0 + AVANCE + 6.0 + TIEMPOS + 10.0 + REPRODUCIR;

/// El encendido de un conmutador de la estación: el acento del sistema.
///
/// Era el `#4184FF` del plasmoide, escrito a pelo. Deja de valer desde que el
/// acento se elige: con la constante, poner el escritorio en verde dejaba esta
/// tarjeta —la más azul de todas— en azul, y el apagado `#AECAFF` de al lado,
/// que sí es un azul deliberado, se leía como el activo de otro tema.
fn acento_centro() -> Color {
    tema::acento()
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
    /// El altavoz silenciado. Lo dice el botón de al lado del volumen.
    silenciado: bool,
    brillo: u8,
    /// Si hay sensor de luz en esta máquina. Se mira una vez, como en el
    /// icono del panel: no aparece ni desaparece con la sesión abierta.
    sensor: bool,
    /// Si el brillo de pantalla sigue al sensor ahora mismo. Solo importa con
    /// sensor: sin él, el botón se queda apagado y no hace nada al pulsarlo.
    automatico: bool,
    agarre: Agarre,
    volumen_inicial: u8,
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
        let volumen = if self.agarre == Agarre::Volumen {
            self.volumen
        } else {
            bookos_system::volume(false)
                .map(|v| v.0)
                .unwrap_or(self.volumen)
        };
        let silenciado = bookos_system::volume(false).map_or(self.silenciado, |v| v.1);
        let changed = self.wifi != wifi
            || self.wifi_disponible != wifi_disponible
            || self.bluetooth != bluetooth
            || self.bluetooth_disponible != bluetooth_disponible
            || self.audio_disponible != audio_disponible
            || self.red != red
            || self.dispositivo != dispositivo
            || self.volumen != volumen
            || self.silenciado != silenciado;
        self.wifi = wifi;
        self.wifi_disponible = wifi_disponible;
        self.bluetooth = bluetooth;
        self.bluetooth_disponible = bluetooth_disponible;
        self.audio_disponible = audio_disponible;
        self.red = red;
        self.dispositivo = dispositivo;
        self.volumen = volumen;
        self.silenciado = silenciado;
        let mut changed = changed;
        if self.agarre != Agarre::Brillo {
            let backlight = crate::backlight();
            let brillo = crate::brillo_actual().unwrap_or(0);
            // El sensor no se vuelve a mirar: no aparece ni desaparece con la
            // sesión abierta, y es un recorrido de `/sys/bus/iio/devices` que
            // no hace falta pagar en cada refresco del panel.
            let automatico = self.sensor && crate::retroiluminacion::automatico().pantalla;
            changed |= self.backlight != backlight
                || self.brillo != brillo
                || self.automatico != automatico;
            self.backlight = backlight;
            self.brillo = brillo;
            self.automatico = automatico;
        }
        if let Some(tile) = self.baldosas.iter_mut().find(|b| b.icono == "avion") {
            let active = wifi_disponible
                && bluetooth_disponible
                && state.network["enabled"] == false
                && state.bluetooth["enabled"] == false;
            changed |= tile.activa != active;
            tile.activa = active;
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
        let sensor = crate::retroiluminacion::hay_sensor_luz();
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
            silenciado: bookos_system::volume(false).is_some_and(|v| v.1),
            brillo: crate::brillo_actual().unwrap_or(0),
            sensor,
            automatico: sensor && crate::retroiluminacion::automatico().pantalla,
            agarre: Agarre::Nada,
            volumen_inicial: 0,
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
        REJILLA_MARGEN * 2.0 + filas * CELDA + (filas - 1.0).max(0.0) * ENTRE_FILAS
    }

    fn alto_deslizadores(&self) -> f32 {
        GRUPO_MARGEN * 2.0 + control::FILA_PILDORA * 2.0 + ENTRE_DESLIZADORES
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
        if x < x0 || y < y0 {
            return None;
        }
        let columna = ((x - x0) / ancho_celda) as usize;
        // El aire entre filas va con la de arriba: es donde cae el rótulo al
        // apuntar con prisa, y no hay nada debajo que pudiera querer el clic.
        let fila = ((y - y0) / (CELDA + ENTRE_FILAS)) as usize;
        if columna >= COLUMNAS {
            return None;
        }
        let i = fila * COLUMNAS + columna;
        (i < self.baldosas.len()).then_some(i)
    }

    /// Cuál de las dos píldoras de conectividad cae en un punto: `false` la del
    /// Wi-Fi, `true` la del Bluetooth.
    fn conexion_en(&self, x: f32, y: f32) -> Option<bool> {
        let y0 = self.y_conexiones();
        if y < y0 || y > y0 + CONEXION || x < MARGEN || x > ANCHO - MARGEN {
            return None;
        }
        Some(x > ANCHO / 2.0)
    }

    /// Los dos botones redondos de la cabecera: `false` Preferencias, `true`
    /// apagar.
    fn cabecera_en(&self, x: f32, y: f32) -> Option<bool> {
        let y0 = MARGEN + (CABECERA - BOTON_CABECERA) / 2.0;
        if y < y0 || y > y0 + BOTON_CABECERA {
            return None;
        }
        let apagar = ANCHO - MARGEN - BOTON_CABECERA;
        let preferencias = apagar - 8.0 - BOTON_CABECERA;
        match x {
            x if x >= apagar && x <= apagar + BOTON_CABECERA => Some(true),
            x if x >= preferencias && x <= preferencias + BOTON_CABECERA => Some(false),
            _ => None,
        }
    }

    /// El ancho de la fila de un deslizador, dentro de su grupo.
    fn ancho_fila(&self) -> f32 {
        ANCHO - (MARGEN + GRUPO_MARGEN) * 2.0
    }

    fn ancho_pildora(&self) -> f32 {
        control::ancho_pildora(self.ancho_fila(), true)
    }

    fn rect_pildora(&self, cual: Agarre) -> iced_core::Rectangle {
        let y0 = self.y_deslizadores() + GRUPO_MARGEN;
        let y = match cual {
            Agarre::Brillo => y0 + control::FILA_PILDORA + ENTRE_DESLIZADORES,
            _ => y0,
        };
        iced_core::Rectangle {
            x: MARGEN + GRUPO_MARGEN + control::ICONO + control::HUECO,
            y: y + (control::FILA_PILDORA - control::PILDORA) / 2.0,
            width: self.ancho_pildora(),
            height: control::PILDORA,
        }
    }

    /// El botón redondo a la derecha de una píldora.
    fn rect_boton(&self, cual: Agarre) -> iced_core::Rectangle {
        let p = self.rect_pildora(cual);
        iced_core::Rectangle {
            x: p.x + p.width + control::HUECO,
            y: p.y - (control::FILA_PILDORA - control::PILDORA) / 2.0,
            width: control::BOTON,
            height: control::BOTON,
        }
    }

    /// Los tres mandos de lo que suena, si está.
    fn medio_en(&self, x: f32, y: f32) -> Option<medios::Orden> {
        self.sonando.as_ref()?;
        let y0 = self.y_medios() + MEDIOS - GRUPO_MARGEN - REPRODUCIR;
        if y < y0 || y > y0 + REPRODUCIR {
            return None;
        }
        let centro = ANCHO / 2.0;
        let mitad = REPRODUCIR / 2.0;
        let alcance = mitad + ENTRE_MANDOS + 22.0;
        Some(match x {
            x if x < centro - alcance || x > centro + alcance => return None,
            x if x < centro - mitad => medios::Orden::Anterior,
            x if x > centro + mitad => medios::Orden::Siguiente,
            _ => medios::Orden::Alternar,
        })
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        if let Some((x, _)) = punto {
            let r = self.rect_pildora(Agarre::Volumen);
            let nivel = control::nivel_en(x, r.x, r.width);
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
        let agarre = std::mem::replace(&mut self.agarre, Agarre::Nada);
        if agarre == Agarre::Volumen && self.volumen != self.volumen_inicial {
            bookos_system::request(bookos_system::Operation::Volume {
                target: "output".into(),
                value: self.volumen as u32,
            });
        }
        agarre != Agarre::Nada
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = iced_core::Point::new(x, y);
        let holgada = |mut r: iced_core::Rectangle| {
            r.y -= control::MARGEN_AGARRE;
            r.height += control::MARGEN_AGARRE * 2.0;
            r
        };
        let r = self.rect_pildora(Agarre::Volumen);
        let nivel = control::nivel_en(x, r.x, r.width);
        if self.rect_boton(Agarre::Volumen).contains(punto) {
            if self.audio_disponible {
                self.silenciado = !self.silenciado;
                bookos_system::request(bookos_system::Operation::Mute {
                    target: "output".into(),
                    muted: Some(self.silenciado),
                });
            }
            return None;
        }
        // El botón del brillo pone o quita el automático, igual que el del
        // volumen silencia: los dos actúan en el sitio, sin abrir nada más. El
        // teclado y la luz nocturna se quedan sin sitio aquí, pero siguen a un
        // clic del icono del panel, que abre la tarjeta entera.
        if self.sensor && self.rect_boton(Agarre::Brillo).contains(punto) {
            self.automatico = !self.automatico;
            let mut elegido = crate::retroiluminacion::automatico();
            elegido.pantalla = self.automatico;
            return Some(Accion::BrilloAutomatico(elegido));
        }
        if let Some(apagar) = self.cabecera_en(x, y) {
            return Some(if apagar {
                Accion::Emergente("apagar")
            } else {
                Accion::Lanzar("bookos-settings".into())
            });
        }
        if holgada(self.rect_pildora(Agarre::Volumen)).contains(punto) {
            self.volumen_inicial = self.volumen;
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
            let inicio = if bluetooth { ANCHO / 2.0 + 4.0 } else { MARGEN };
            let disponible = if bluetooth {
                self.bluetooth_disponible
            } else {
                self.wifi_disponible
            };
            if x < inicio + RELLENO_CONEXION + ICONO_CONEXION {
                if !disponible {
                    return None;
                }
                let encendido = if bluetooth { self.bluetooth } else { self.wifi };
                bookos_system::request(if bluetooth {
                    bookos_system::Operation::BluetoothPower {
                        enabled: !encendido,
                    }
                } else {
                    bookos_system::Operation::WifiPower {
                        enabled: !encendido,
                    }
                });
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
            bookos_system::request(bookos_system::Operation::Airplane {
                enabled: !self.baldosas[i].activa,
            });
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
        bookos_system::request(bookos_system::Operation::VolumePreview {
            target: "output".into(),
            value: nivel as u32,
        });
        true
    }

    fn poner_brillo(&mut self, nivel: u8) -> bool {
        // Mismo suelo que el emergente del brillo: a cero no se ve ni el
        // deslizador con el que volver a subirlo.
        if self.backlight.is_none() {
            return false;
        }
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
    fn circulo(
        nombre: &str,
        lado: f32,
        icono_px: f32,
        fondo: Color,
        tinta: Color,
    ) -> PanelElement<'static> {
        let dibujo: PanelElement<'static> = match icono::propio(nombre) {
            Some(ic) => icono::ver_teñido_propio(&ic, icono_px, tinta),
            None => Space::new().width(Length::Fixed(icono_px)).into(),
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
            .size(16.0)
            .font(control::peso(Weight::Semibold))
            .color(tema::sobre_acento())
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
                        .size(17.0)
                        .font(control::peso(Weight::Bold))
                        .color(tema::texto())
                        .wrapping(iced_core::text::Wrapping::None),
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
            ANCHO - MARGEN * 2.0 - (BOTON_CABECERA + 8.0) * 2.0,
        ))
        .height(Length::Fixed(CABECERA))
        .padding(iced_core::Padding {
            top: 6.0,
            right: 6.0,
            bottom: 6.0,
            left: 16.0,
        })
        .center_y(Length::Fixed(CABECERA))
        .clip(true)
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(contenedor().into()),
            border: Border {
                radius: (CABECERA / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });

        // Apagar va el último y con el glifo en rojo. Es la única acción de la
        // tarjeta que se lleva la sesión por delante: la esquina y el color son
        // lo que impide darle queriendo abrir Preferencias.
        row![
            pildora,
            Space::new().width(Length::Fixed(8.0)),
            Self::circulo(
                "preferencias",
                BOTON_CABECERA,
                18.0,
                contenedor(),
                tema::texto()
            ),
            Space::new().width(Length::Fixed(8.0)),
            Self::circulo("apagar", BOTON_CABECERA, 18.0, contenedor(), tema::rojo()),
        ]
        .align_y(Vertical::Center)
        .into()
    }

    /// Una de las dos píldoras de conectividad.
    fn conexion(&self, bluetooth: bool) -> PanelElement<'_> {
        // El círculo va en acento cuando hay **conexión**, no cuando la radio
        // está encendida. Es la misma regla que la rejilla: el acento dice que
        // algo está funcionando. Con la antena encendida y sin red, la píldora
        // se queda apagada y el subtítulo dice «Sin conexión», que es lo que
        // hay que ver cuando algo no va.
        let disponible = if bluetooth {
            self.bluetooth_disponible
        } else {
            self.wifi_disponible
        };
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
                if self.wifi && disponible {
                    "wifi"
                } else {
                    "sin-red"
                },
            )
        };
        // El subtítulo se corta a lo que cabe en una línea: la píldora tiene el
        // alto fijo y una segunda línea se saldría por abajo.
        let estado = if !disponible {
            "No disponible".to_owned()
        } else {
            match (encendido, conectado) {
                (_, Some(donde)) => recortar(donde, 14),
                (true, None) => "Sin conexión".to_owned(),
                (false, None) => "Desactivado".to_owned(),
            }
        };
        let (fondo, tinta) = if conectado.is_some() {
            (acento_centro(), tema::sobre_acento())
        } else {
            (tema::card(), tinta_apagado())
        };
        container(
            row![
                Self::circulo(icono, ICONO_CONEXION, 18.0, fondo, tinta),
                Space::new().width(Length::Fixed(10.0)),
                column![
                    text(nombre)
                        .size(14.0)
                        .font(control::peso(Weight::Medium))
                        .color(tema::texto())
                        .wrapping(iced_core::text::Wrapping::None),
                    // Sin envolver: el SSID recortado sigue teniendo espacios
                    // y iced parte por ellos antes que desbordar.
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
        .padding(iced_core::Padding {
            top: 0.0,
            right: 12.0,
            bottom: 0.0,
            left: RELLENO_CONEXION,
        })
        .center_y(Length::Fixed(CONEXION))
        .clip(true)
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(contenedor().into()),
            border: Border {
                radius: (CONEXION / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    fn baldosa(&self, i: usize) -> PanelElement<'_> {
        let baldosa = &self.baldosas[i];
        // Lo que cambia entre encendida y apagada es el círculo de detrás, no
        // el icono: acento contra el color de la tarjeta, que dentro del grupo
        // gris es lo que se despega. Señalada, se mezcla un poco hacia la
        // tinta, interpolado y no conmutado: la rejilla tiene ocho baldosas
        // juntas y un salto se lee como un parpadeo al cruzarla con el ratón.
        let (base, tinta) = if baldosa.activa {
            (acento_centro(), tema::sobre_acento())
        } else {
            (tema::card(), tema::texto())
        };
        let fondo = tema::mezclar(base, tema::tinta(), 0.08 * self.señalada.intensidad(i));
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
                Self::circulo(baldosa.icono, BOTON, 20.0, fondo, tinta),
                Space::new().height(Length::Fixed(6.0)),
                rotulo,
            ]
            .align_x(Horizontal::Center),
        )
        .width(Length::Fixed(celda))
        .height(Length::Fixed(CELDA))
        .center_x(Length::Fixed(celda))
        .into()
    }

    fn deslizador(&self, cual: Agarre) -> PanelElement<'_> {
        let (nivel, icono, boton, encendido, apagado) = match cual {
            Agarre::Brillo => (
                self.brillo,
                "brillo",
                // El icono del botón dice si el automático está puesto, igual
                // que el del panel: un sol con «A» en vez del sol pelado.
                if self.automatico {
                    "brillo-automatico"
                } else {
                    "brillo"
                },
                self.sensor && self.automatico,
                self.backlight.is_none(),
            ),
            _ => (
                self.volumen,
                "volumen-alto",
                if self.silenciado || self.volumen == 0 {
                    "volumen-silencio"
                } else {
                    match self.volumen {
                        1..=33 => "volumen-bajo",
                        34..=66 => "volumen-medio",
                        _ => "volumen-alto",
                    }
                },
                !self.silenciado && self.audio_disponible,
                self.silenciado,
            ),
        };
        let dibujo = |nombre: &str, px: f32, color: Color| -> PanelElement<'static> {
            match icono::propio(nombre) {
                Some(ic) => icono::ver_teñido_propio(&ic, px, color),
                None => Space::new().width(Length::Fixed(px)).into(),
            }
        };
        // El botón del volumen silencia y va en acento mientras suena; el del
        // brillo pone o quita el automático y va en acento mientras está
        // puesto. Los dos dicen «esto está activo» con el mismo lenguaje.
        let (fondo, tinta) = if encendido {
            (acento_centro(), tema::sobre_acento())
        } else {
            (tema::card(), tema::TEXTO2)
        };
        row![
            dibujo(icono, control::ICONO, tema::texto()),
            Space::new().width(Length::Fixed(control::HUECO)),
            control::pildora(
                self.ancho_pildora(),
                control::PILDORA,
                nivel,
                apagado,
                tema::card()
            ),
            Space::new().width(Length::Fixed(control::HUECO)),
            Self::circulo(boton, control::BOTON, 16.0, fondo, tinta),
        ]
        .align_y(Vertical::Center)
        .height(Length::Fixed(control::FILA_PILDORA))
        .into()
    }

    fn medios(&self, sonando: &Sonando) -> PanelElement<'_> {
        let ancho = ANCHO - (MARGEN + GRUPO_MARGEN) * 2.0;
        let avance = (sonando.avance().unwrap_or(0.0) * 100.0).round() as u8;
        let ancho_texto = ancho - CARATULA - 12.0;
        let caratula = container(match icono::propio("musica") {
            Some(ic) => icono::ver_teñido_propio(&ic, 22.0, tema::TEXTO2),
            None => Space::new().width(Length::Fixed(22.0)).into(),
        })
        .width(Length::Fixed(CARATULA))
        .height(Length::Fixed(CARATULA))
        .center_x(Length::Fixed(CARATULA))
        .center_y(Length::Fixed(CARATULA))
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(tema::card().into()),
            border: Border {
                radius: tema::R_BOTON_PEQUENO.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let textos = column![
            text(recortar_px(
                &format!("Reproduciendo en {}", sonando.aplicacion),
                ancho_texto,
                11.0
            ))
            .size(11.0)
            .color(tema::TEXTO2),
            text(recortar_px(&sonando.titulo, ancho_texto, 14.0))
                .size(14.0)
                .font(control::peso(Weight::Medium))
                .color(tema::texto()),
            text(recortar_px(&sonando.artista, ancho_texto, 11.0))
                .size(11.0)
                .color(tema::TEXTO2),
        ];
        let mando = |nombre: &str| -> PanelElement<'static> {
            match icono::propio(nombre) {
                Some(ic) => icono::ver_teñido_propio(&ic, 22.0, tema::texto()),
                None => Space::new().width(Length::Fixed(22.0)).into(),
            }
        };
        let contenido = column![
            row![caratula, Space::new().width(Length::Fixed(12.0)), textos]
                .align_y(Vertical::Center)
                .height(Length::Fixed(CARATULA)),
            Space::new().height(Length::Fixed(10.0)),
            control::pildora(ancho, AVANCE, avance, false, tema::card()),
            Space::new().height(Length::Fixed(6.0)),
            row![
                text(medios::reloj(sonando.posicion.unwrap_or(0)))
                    .size(11.0)
                    .color(tema::TEXTO2),
                Space::new().width(Length::Fill),
                text(
                    sonando
                        .duracion
                        .map(medios::reloj)
                        .unwrap_or_else(|| "--:--".into())
                )
                .size(11.0)
                .color(tema::TEXTO2),
            ]
            .height(Length::Fixed(TIEMPOS)),
            Space::new().height(Length::Fixed(10.0)),
            container(
                row![
                    mando("anterior"),
                    Space::new().width(Length::Fixed(ENTRE_MANDOS)),
                    Self::circulo(
                        if sonando.reproduciendo {
                            "pausa"
                        } else {
                            "reproducir"
                        },
                        REPRODUCIR,
                        20.0,
                        acento_centro(),
                        tema::sobre_acento(),
                    ),
                    Space::new().width(Length::Fixed(ENTRE_MANDOS)),
                    mando("siguiente"),
                ]
                .align_y(Vertical::Center)
            )
            .width(Length::Fill)
            .height(Length::Fixed(REPRODUCIR))
            .align_x(Horizontal::Center),
        ];
        Self::grupo(contenido.into(), MEDIOS, GRUPO_MARGEN)
    }

    /// Un grupo gris con su alto fijo: el alto es lo que usan las zonas de
    /// clic, así que se fija en vez de dejarlo al contenido.
    fn grupo<'a>(contenido: PanelElement<'a>, alto: f32, relleno: f32) -> PanelElement<'a> {
        container(control::grupo(contenido, ANCHO - MARGEN * 2.0, relleno))
            .height(Length::Fixed(alto))
            .into()
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut rejilla = column![].spacing(ENTRE_FILAS);
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
            Self::grupo(rejilla.into(), self.alto_rejilla(), REJILLA_MARGEN),
            Space::new().height(Length::Fixed(HUECO)),
            Self::grupo(
                column![
                    self.deslizador(Agarre::Volumen),
                    Space::new().height(Length::Fixed(ENTRE_DESLIZADORES)),
                    self.deslizador(Agarre::Brillo),
                ]
                .into(),
                self.alto_deslizadores(),
                GRUPO_MARGEN,
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
    let avion = !state.network.is_null()
        && !state.bluetooth.is_null()
        && state.network["enabled"] == false
        && state.bluetooth["enabled"] == false;
    let ahorro = std::fs::read_to_string("/sys/firmware/acpi/platform_profile")
        .unwrap_or_default()
        .contains("low-power");
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
            accion: Some(Accion::Lanzar("bookos-settings --page pantalla".into())),
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

/// Recorta al ancho en píxeles, con la misma medida que las listas.
fn recortar_px(texto: &str, ancho: f32, tamaño: f32) -> String {
    super::lista::recortar(texto, ancho, tamaño)
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
    s.network["ssid"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
fn emparejado_con() -> Option<String> {
    let s = bookos_system::snapshot();
    s.bluetooth["devices"]
        .as_array()?
        .iter()
        .find(|d| d["connected"] == true)?["name"]
        .as_str()
        .map(str::to_owned)
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
            c.baldosa_en(x0 + 5.0, y0 + CELDA + ENTRE_FILAS + 5.0),
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

    /// El botón del brillo pone y quita el automático en el sitio, igual que
    /// el del volumen silencia: sin abrir ninguna otra tarjeta.
    #[test]
    fn el_boton_del_brillo_alterna_el_automatico_en_el_sitio() {
        let mut c = Centro::new();
        c.sensor = true;
        c.automatico = false;
        let r = c.rect_boton(Agarre::Brillo);
        let accion = c.pulsar(r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert!(c.automatico, "el botón no puso el automático");
        match accion {
            Some(Accion::BrilloAutomatico(elegido)) => assert!(elegido.pantalla),
            otra => panic!("se esperaba BrilloAutomatico con pantalla=true, llegó {otra:?}"),
        }

        let accion = c.pulsar(r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert!(!c.automatico, "el segundo toque no lo quitó");
        match accion {
            Some(Accion::BrilloAutomatico(elegido)) => assert!(!elegido.pantalla),
            otra => panic!("se esperaba BrilloAutomatico con pantalla=false, llegó {otra:?}"),
        }
    }

    /// Sin sensor no hay automático que alternar, así que el botón no toca
    /// nada al pulsarlo.
    #[test]
    fn sin_sensor_el_boton_del_brillo_no_hace_nada() {
        let mut c = Centro::new();
        c.sensor = false;
        c.automatico = false;
        let r = c.rect_boton(Agarre::Brillo);
        c.pulsar(r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert!(!c.automatico, "sin sensor no debería haber cambiado nada");
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
        assert_eq!(
            c.wifi, antes,
            "el estado solo cambia al confirmarlo el servicio"
        );
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
