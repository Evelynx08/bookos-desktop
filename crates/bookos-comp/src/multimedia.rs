//! Las teclas de función del portátil: volumen, brillo y el touchpad.
//!
//! Todas comparten forma: el compositor las intercepta antes que nadie —han de
//! funcionar aunque el foco lo tenga una aplicación a pantalla completa—, hace
//! el cambio y enseña el aviso con el nivel que ha quedado.
//!
//! **Por qué los keysyms van a mano.** Son los `XF86` de siempre y sus valores
//! están fijados desde hace treinta años; escribirlos aquí es más estable que
//! depender de cómo los llame la versión de turno de la biblioteca de teclado,
//! que ya ha cambiado de nombre entre versiones.
//!
//! **Qué no está.** Duplicar la pantalla necesita más de una salida, y el
//! compositor todavía usa solo la primera: mientras eso no exista, la tecla no
//! tendría a qué aplicarse.

use crate::state::BookosComp;

pub const XF86_BAJAR_VOLUMEN: u32 = 0x1008FF11;
pub const XF86_SILENCIAR: u32 = 0x1008FF12;
pub const XF86_SUBIR_VOLUMEN: u32 = 0x1008FF13;
pub const XF86_BRILLO_MAS: u32 = 0x1008FF02;
pub const XF86_BRILLO_MENOS: u32 = 0x1008FF03;
pub const XF86_TECLADO_MAS: u32 = 0x1008FF05;
pub const XF86_TECLADO_MENOS: u32 = 0x1008FF06;
pub const XF86_TOUCHPAD: u32 = 0x1008FFA9;
pub const XF86_SILENCIAR_MICRO: u32 = 0x1008FFB2;

/// Lo que hace cada tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tecla {
    Volumen(i32),
    Silenciar,
    SilenciarMicro,
    Brillo(i32),
    BrilloTeclado(i32),
    Touchpad,
}

/// Cuánto mueve una pulsación. 5 % es el paso de KDE: con 10 se pasa de largo
/// y con 2 hay que machacar la tecla.
const PASO: i32 = 5;

pub fn resolver(sym: u32) -> Option<Tecla> {
    Some(match sym {
        XF86_SUBIR_VOLUMEN => Tecla::Volumen(PASO),
        XF86_BAJAR_VOLUMEN => Tecla::Volumen(-PASO),
        XF86_SILENCIAR => Tecla::Silenciar,
        XF86_SILENCIAR_MICRO => Tecla::SilenciarMicro,
        XF86_BRILLO_MAS => Tecla::Brillo(PASO),
        XF86_BRILLO_MENOS => Tecla::Brillo(-PASO),
        XF86_TECLADO_MAS => Tecla::BrilloTeclado(1),
        XF86_TECLADO_MENOS => Tecla::BrilloTeclado(-1),
        XF86_TOUCHPAD => Tecla::Touchpad,
        _ => return None,
    })
}

pub fn ejecutar(state: &mut BookosComp, tecla: Tecla) {
    let osd = match tecla {
        Tecla::Volumen(paso) => {
            volumen(paso);
            None
        }
        Tecla::Silenciar => {
            silenciar("@DEFAULT_AUDIO_SINK@");
            None
        }
        Tecla::SilenciarMicro => {
            silenciar("@DEFAULT_AUDIO_SOURCE@");
            None
        }
        Tecla::Brillo(paso) => Some(brillo(state, paso)),
        Tecla::BrilloTeclado(paso) => Some(brillo_teclado(paso)),
        Tecla::Touchpad => Some(touchpad(state)),
    };
    if let (Some(shell), Some((icono, nivel, texto))) = (state.shell.as_mut(), osd) {
        shell.mostrar_osd(icono, nivel, texto);
        despertar_para_la_salida(state);
    }
    state.needs_redraw = true;
}

/// Programa **un** despertar para cuando el aviso empiece a irse.
///
/// Sin esto, el compositor se duerme con el OSD en pantalla y no vuelve a
/// dibujar: el aviso se quedaría puesto hasta que otra cosa provocara un
/// frame. Con esto es un solo despertar, no un temporizador que repita.
fn despertar_para_la_salida(state: &mut BookosComp) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    let Some(queda) = state.shell.as_ref().and_then(|s| s.osd_queda()) else {
        return;
    };
    let hasta_la_salida = queda.saturating_sub(bookos_shell::osd::SALIDA);
    let result =
        state
            .loop_handle
            .insert_source(Timer::from_duration(hasta_la_salida), |_, _, state| {
                state.needs_redraw = true;
                TimeoutAction::Drop
            });
    if let Err(err) = result {
        tracing::error!("no se pudo programar la salida del aviso: {err}");
    }
}

/// Sube o baja el volumen y devuelve cómo ha quedado.
///
/// Se lee **antes** de escribir en vez de llevar la cuenta aquí: el volumen lo
/// puede haber cambiado otro programa, y sumarle el paso a un número viejo da
/// saltos raros.
fn volumen(paso: i32) {
    bookos_system::request(bookos_system::Operation::VolumeStep { step: paso });
}

fn silenciar(destino: &str) {
    bookos_system::request(bookos_system::Operation::Mute {
        target: destino.into(),
        muted: None,
    });
}

pub fn recibir_sistema(state: &mut BookosComp) {
    use bookos_system::Operation;
    let mut mostro_osd = false;
    while let Some((operation, result)) = bookos_system::take_feedback() {
        let osd = match result {
            Err(e) => Some(("sin-red", None, Some(e))),
            Ok(()) => match operation {
                Operation::VolumeStep { .. }
                | Operation::Volume { .. }
                | Operation::Mute { .. } => {
                    let input = matches!(&operation,Operation::Mute{target,..}|Operation::Volume{target,..} if target=="input"||target=="@DEFAULT_AUDIO_SOURCE@");
                    bookos_system::volume(input).map(|(level, muted)| {
                        (
                            if input {
                                if muted { "micro-silencio" } else { "micro" }
                            } else {
                                icono_volumen(level, muted)
                            },
                            Some(if muted { 0 } else { level }),
                            None,
                        )
                    })
                }
                _ => None,
            },
        };
        if let (Some(shell), Some((icon, level, text))) = (state.shell.as_mut(), osd) {
            shell.mostrar_osd(icon, level, text);
            mostro_osd = true;
        }
    }
    if mostro_osd {
        despertar_para_la_salida(state);
    }
}

fn icono_volumen(nivel: u8, silenciado: bool) -> &'static str {
    match (silenciado, nivel) {
        (true, _) | (_, 0) => "volumen-silencio",
        (_, n) if n < 40 => "volumen-bajo",
        (_, n) if n < 75 => "volumen-medio",
        _ => "volumen-alto",
    }
}

/// Sube o baja el brillo de la pantalla por logind, que es quien tiene el
/// permiso: `/sys/class/backlight/*/brightness` es de root.
fn brillo(state: &mut BookosComp, paso: i32) -> (&'static str, Option<u8>, Option<String>) {
    let Some(actual) = bookos_shell::brillo_actual() else {
        return (
            "brillo",
            None,
            Some("Brillo de pantalla no disponible".into()),
        );
    };
    let nuevo = (actual as i32 + paso).clamp(5, 100) as u8;
    if let Some((dispositivo, maximo)) = bookos_shell::backlight() {
        let crudo = (maximo as f64 * nuevo as f64 / 100.0).round() as u32;
        bookos_shell::retroiluminacion::solicitar("backlight", &dispositivo, crudo);
    }
    // El widget del panel lo relee en su próximo refresco; forzarlo aquí
    // costaría los 21 ms de `wpctl` que ya evitamos en otro sitio.
    let _ = state;
    ("brillo", Some(nuevo), None)
}

/// El teclado tiene su propia retroiluminación, con niveles enteros (0..max) y
/// no un tanto por ciento: en este portátil son cuatro pasos.
fn brillo_teclado(paso: i32) -> (&'static str, Option<u8>, Option<String>) {
    let Some((dispositivo, actual, maximo)) = bookos_shell::brillo_teclado_actual() else {
        return ("teclado", None, Some("Sin luz de teclado".into()));
    };
    let nuevo = (i64::from(actual) + i64::from(paso)).clamp(0, i64::from(maximo));
    bookos_shell::retroiluminacion::solicitar("leds", &dispositivo, nuevo as u32);
    // El nivel se enseña en tanto por ciento para que la barra diga algo: con
    // cuatro pasos, "1 de 3" no se lee de un vistazo.
    let porciento = (nuevo * 100 / i64::from(maximo.max(1))) as u8;
    ("teclado", Some(porciento), None)
}

/// Enciende y apaga el touchpad. El cambio se guarda en el estado para que los
/// dispositivos que aparezcan después —tras un cambio de TTY— nazcan igual.
fn touchpad(state: &mut BookosComp) -> (&'static str, Option<u8>, Option<String>) {
    state.touchpad_activo = !state.touchpad_activo;
    // A los que ya están abiertos hay que decírselo ahora: `DeviceAdded` solo
    // llega con los que aparecen, y el touchpad del portátil apareció al
    // arrancar. El cierre lo pone el backend de sesión real; anidado no hay
    // libinput y no hay nada que aplicar.
    if let Some(aplicar) = state.aplicar_touchpad.as_ref() {
        aplicar(state.touchpad_activo);
    }
    let texto = if state.touchpad_activo {
        "Touchpad activado"
    } else {
        "Touchpad desactivado"
    };
    ("touchpad", None, Some(texto.into()))
}
