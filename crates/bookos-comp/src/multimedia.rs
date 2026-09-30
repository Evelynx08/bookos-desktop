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
pub const XF86_TECLADO_ALTERNAR: u32 = 0x1008FF04;
pub const XF86_TOUCHPAD: u32 = 0x1008FFA9;
pub const XF86_SILENCIAR_MICRO: u32 = 0x1008FFB2;
pub const XF86_WEBCAM: u32 = 0x1008FF8F;
pub const XF86_FN_ESC: u32 = 0x100811D1;
pub const XF86_CAMARA_ENABLE: u32 = 0x1008124B;
pub const XF86_CAMARA_DISABLE: u32 = 0x1008124C;
pub const XF86_CAMARA_TOGGLE: u32 = 0x1008124D;

/// Lo que hace cada tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tecla {
    Volumen(i32),
    Silenciar,
    SilenciarMicro,
    Brillo(i32),
    BrilloTeclado(i32),
    AlternarBrilloTeclado,
    Touchpad,
    Camara(Option<bool>),
    FnLock,
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
        XF86_WEBCAM | XF86_CAMARA_TOGGLE => Tecla::Camara(None),
        XF86_CAMARA_ENABLE => Tecla::Camara(Some(true)),
        XF86_CAMARA_DISABLE => Tecla::Camara(Some(false)),
        XF86_FN_ESC => Tecla::FnLock,
        XF86_BRILLO_MAS => Tecla::Brillo(PASO),
        XF86_BRILLO_MENOS => Tecla::Brillo(-PASO),
        XF86_TECLADO_MAS => Tecla::BrilloTeclado(1),
        XF86_TECLADO_MENOS => Tecla::BrilloTeclado(-1),
        XF86_TECLADO_ALTERNAR => Tecla::AlternarBrilloTeclado,
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
        Tecla::BrilloTeclado(paso) => Some(brillo_teclado(state, paso)),
        Tecla::AlternarBrilloTeclado => Some(alternar_brillo_teclado(state)),
        Tecla::Touchpad => Some(touchpad(state)),
        Tecla::Camara(estado) => Some(camara(estado)),
        Tecla::FnLock => Some(("teclado", None, Some("Fn lock".into()))),
    };
    if let Some(osd) = osd {
        avisar(state, osd);
    }
    state.needs_redraw = true;
}

/// Enseña el aviso y programa su salida.
fn avisar(
    state: &mut BookosComp,
    (icono, nivel, texto): (&'static str, Option<u8>, Option<String>),
) {
    if let Some(shell) = state.shell.as_mut() {
        shell.mostrar_osd(icono, nivel, texto);
        despertar_para_la_salida(state);
    }
    state.needs_redraw = true;
}

/// El nombre con que el driver `samsung-galaxybook` registra la tapa de la
/// cámara como dispositivo de entrada.
const TAPA_CAMARA: &str = "Samsung Galaxy Book Camera Lens Cover";

/// Enseña el aviso de la cámara cuando se bloquea o desbloquea con su tecla.
///
/// En el Galaxy Book esa tecla (Fn+F11 en el Book5 Pro) no llega como tecla:
/// el driver la atiende y manda un **interruptor**, `SW_CAMERA_LENS_COVER`
/// —medido con `hexdump`: tipo 5, código 9, valor 1 al bloquearla y 0 al
/// soltarla—. libinput solo entiende los interruptores de tapa y de modo
/// tableta, así que se lee el dispositivo directamente.
///
/// Se abre por su ruta y **no** por libseat: libinput ya lo tiene abierto por
/// ese camino y logind no entrega dos veces el mismo dispositivo. El permiso lo
/// da la regla de udev `session/70-bookos-camara.rules`; sin ella se avisa en
/// el registro y la cámara sigue funcionando, solo que sin aviso.
pub fn vigilar_tapa_camara(state: &mut BookosComp) {
    use smithay::reexports::calloop::generic::Generic;
    use smithay::reexports::calloop::{Interest, Mode, PostAction};
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;

    let Some(nodo) = buscar_evento(TAPA_CAMARA) else {
        return;
    };
    let fichero = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(smithay::reexports::rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(&nodo)
    {
        Ok(f) => f,
        Err(err) => {
            tracing::warn!(
                ?nodo,
                "sin aviso de la cámara (¿falta la regla de udev?): {err}"
            );
            return;
        }
    };
    let registrado = state.loop_handle.insert_source(
        Generic::new(fichero, Interest::READ, Mode::Level),
        |_, fichero, state| {
            let mut buf = [0u8; EVENTO * 16];
            loop {
                let n = match (&**fichero).read(&mut buf) {
                    Ok(0) => return Ok(PostAction::Remove),
                    Ok(n) => n,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        return Ok(PostAction::Continue);
                    }
                    // Desenchufado o revocado: se deja de escuchar, sin tumbar
                    // el bucle.
                    Err(err) => {
                        tracing::warn!("se dejó de oír la tapa de la cámara: {err}");
                        return Ok(PostAction::Remove);
                    }
                };
                for evento in buf[..n].as_chunks::<EVENTO>().0 {
                    if let Some(activa) = camara_activa(evento) {
                        avisar(state, camara(Some(activa)));
                    }
                }
            }
        },
    );
    if let Err(err) = registrado {
        tracing::warn!("sin aviso de la cámara: {err}");
    }
}

/// Lo que mide un `struct input_event` en 64 bits: 16 bytes de tiempo, y
/// después tipo, código y valor.
const EVENTO: usize = 24;

/// Si el evento es el interruptor de la tapa, dice si la cámara queda activa.
fn camara_activa(evento: &[u8; EVENTO]) -> Option<bool> {
    const EV_SW: u16 = 5;
    const SW_CAMERA_LENS_COVER: u16 = 9;
    let tipo = u16::from_ne_bytes([evento[16], evento[17]]);
    let codigo = u16::from_ne_bytes([evento[18], evento[19]]);
    let valor = i32::from_ne_bytes([evento[20], evento[21], evento[22], evento[23]]);
    // Tapa puesta (1) es cámara desactivada.
    (tipo == EV_SW && codigo == SW_CAMERA_LENS_COVER).then_some(valor == 0)
}

/// El `/dev/input/eventN` del dispositivo de entrada con ese nombre.
fn buscar_evento(nombre: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir("/sys/class/input")
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
        .find(|e| {
            std::fs::read_to_string(e.path().join("device/name")).is_ok_and(|n| n.trim() == nombre)
        })
        .map(|e| std::path::Path::new("/dev/input").join(e.file_name()))
}

/// Enseña el aviso de la luz del teclado cuando la cambia el **firmware**.
///
/// En el Galaxy Book, Fn+F9 no llega como tecla: lo atiende el driver
/// `samsung-galaxybook`, cambia el LED él solo y avisa por
/// `brightness_hw_changed`, que da «No hay datos» hasta el primer cambio. Sin
/// esto la luz cambiaba sin ningún aviso en pantalla.
///
/// Un hilo que duerme en `poll(POLLPRI)`, que es como sysfs notifica un
/// atributo, y no un temporizador que relea el fichero: no despierta la CPU
/// hasta que alguien pulsa la tecla.
pub fn vigilar_luz_teclado(state: &mut BookosComp) {
    use smithay::reexports::calloop::channel;
    let Some((dispositivo, _, maximo)) = bookos_shell::brillo_teclado_actual() else {
        return;
    };
    let ruta = std::path::Path::new("/sys/class/leds")
        .join(&dispositivo)
        .join("brightness_hw_changed");
    // Solo lo tienen los LED que el hardware puede cambiar por su cuenta.
    let Ok(fichero) = std::fs::File::open(&ruta) else {
        return;
    };
    let (emisor, receptor) = channel::channel::<u32>();
    let registrado = state
        .loop_handle
        .insert_source(receptor, move |evento, _, state| {
            let channel::Event::Msg(nivel) = evento else {
                return;
            };
            // Para el brillo automático cuenta como un cambio a mano, igual que la
            // tecla: si no, lo deshacería en la siguiente lectura del sensor.
            crate::brillo_auto::tecla_teclado(state, &dispositivo, nivel);
            let porciento = (nivel * 100 / maximo.max(1)).min(100) as u8;
            avisar(state, ("teclado", Some(porciento), None));
        });
    if let Err(err) = registrado {
        tracing::warn!("sin aviso de la luz del teclado: {err}");
        return;
    }
    let lanzado = std::thread::Builder::new()
        .name("bookos-luz-teclado".into())
        .spawn(move || {
            use smithay::reexports::rustix::event::{PollFd, PollFlags, poll};
            let leer = || {
                let mut buf = [0u8; 16];
                let n = smithay::reexports::rustix::io::pread(&fichero, &mut buf, 0).ok()?;
                std::str::from_utf8(&buf[..n])
                    .ok()?
                    .trim()
                    .parse::<u32>()
                    .ok()
            };
            // sysfs exige una lectura antes del primer `poll`, o vuelve en
            // seguida; antes del primer cambio falla con ENODATA y da igual.
            let _ = leer();
            loop {
                let mut fds = [PollFd::new(&fichero, PollFlags::PRI | PollFlags::ERR)];
                if poll(&mut fds, None).is_err() {
                    return;
                }
                if let Some(nivel) = leer()
                    && emisor.send(nivel).is_err()
                {
                    // El compositor se ha ido: no hay a quién avisar.
                    return;
                }
            }
        });
    if let Err(err) = lanzado {
        tracing::warn!("sin aviso de la luz del teclado: {err}");
    }
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
        if result.is_ok() && matches!(operation, Operation::VolumeStep { .. }) {
            chasquido(state);
        }
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

/// El sonido de KDE al cambiar el volumen: se oye a qué nivel ha quedado.
///
/// Suena aquí y no en `bookos-system` porque es el compositor quien lee
/// `panel.conf`. Suena tras aplicar el paso, no al pulsar: antes sonaría al
/// volumen viejo. Los pasos seguidos llegan ya sumados en uno.
fn chasquido(state: &mut BookosComp) {
    if !state.sonido_volumen {
        return;
    }
    sonar(state, "audio-volume-change");
}

/// Toca un sonido del tema freedesktop sin esperar a que acabe.
fn sonar(state: &mut BookosComp, nombre: &str) {
    state
        .hijos
        .retain_mut(|hijo| !matches!(hijo.try_wait(), Ok(Some(_))));
    match std::process::Command::new("pw-play")
        .arg(format!("/usr/share/sounds/freedesktop/stereo/{nombre}.oga"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(hijo) => state.hijos.push(hijo),
        Err(err) => tracing::warn!(nombre, "sin sonido: {err}"),
    }
}

/// Suena y avisa al conectar o desconectar el cargador.
///
/// Se llama con cada aviso de udev de `power_supply`, que llegan en ráfagas
/// (el adaptador y la batería por separado, y luego cada cambio de
/// porcentaje): por eso se compara con lo último visto y no se avisa por
/// evento. Se mira el estado de la batería y no solo el del adaptador porque
/// es lo que ya usa el widget del panel, y así los dos nunca se contradicen.
pub fn cargador(state: &mut BookosComp) {
    let Some((enchufado, porciento)) = bookos_shell::cargador() else {
        return;
    };
    if state.enchufado.replace(enchufado) == Some(enchufado) {
        return;
    }
    let (resumen, cuerpo, sonido) = if enchufado {
        (
            "Cargador conectado",
            format!("{porciento} % · cargando"),
            "power-plug",
        )
    } else {
        (
            "Cargador desconectado",
            format!("{porciento} % de batería"),
            "power-unplug",
        )
    };
    tracing::info!(enchufado, porciento, "cambio de alimentación");
    sonar(state, sonido);
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    let mut notificacion = bookos_shell::notificaciones::Notificacion::nueva(
        0,
        "BookOS".into(),
        resumen.into(),
        cuerpo,
        "",
        false,
    );
    // El del panel y no uno del tema, para que el aviso y la barra se vean
    // iguales. Si la batería desapareció entre las dos lecturas se queda el
    // genérico de notificaciones que ya puso `nueva`.
    if let Some(icono) = bookos_shell::icono_cargador() {
        notificacion.icono = Some(icono);
    }
    shell.notificar(notificacion, 3000);
    state.needs_redraw = true;
}

/// Recoge cada segundo lo que el hilo del centro de control ha leído de MPRIS
/// y repinta si ha cambiado: canción, carátula, avance, pausa.
///
/// Se mira en cada fotograma si hace falta, y no al abrir la tarjeta, porque
/// hay muchos caminos para abrirla (clic, atajo, cambio desde otra tarjeta) y
/// todos acaban pintando. El temporizador se suelta solo al cerrarla: con el
/// centro cerrado no hay nada despertando cada segundo.
pub fn vigilar_centro(state: &mut BookosComp) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    const CADA: std::time::Duration = std::time::Duration::from_secs(1);
    let abierto = |state: &BookosComp| {
        state.shell.as_ref().and_then(|s| s.emergente_nombre()) == Some("centro")
    };
    if state.tick_centro.is_some() || !abierto(state) {
        return;
    }
    let token = state
        .loop_handle
        .insert_source(Timer::from_duration(CADA), move |_, _, state| {
            if !abierto(state) {
                state.tick_centro = None;
                return TimeoutAction::Drop;
            }
            if state.shell.as_mut().is_some_and(|s| s.refresh_emergente()) {
                state.needs_redraw = true;
            }
            TimeoutAction::ToDuration(CADA)
        });
    match token {
        Ok(token) => state.tick_centro = Some(token),
        Err(err) => tracing::warn!("sin segundero para el centro de control: {err}"),
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
    let Some(actual) = crate::brillo_auto::partida_tecla(state) else {
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
    crate::brillo_auto::tecla(state, nuevo);
    ("brillo", Some(nuevo), None)
}

/// El teclado tiene su propia retroiluminación, con niveles enteros (0..max) y
/// no un tanto por ciento: en este portátil son cuatro pasos.
fn brillo_teclado(state: &mut BookosComp, paso: i32) -> (&'static str, Option<u8>, Option<String>) {
    let Some((dispositivo, actual, maximo)) = bookos_shell::brillo_teclado_actual() else {
        return ("teclado", None, Some("Sin luz de teclado".into()));
    };
    let actual = crate::brillo_auto::partida_tecla_teclado(state, &dispositivo, actual);
    let nuevo = (i64::from(actual) + i64::from(paso)).clamp(0, i64::from(maximo)) as u32;
    bookos_shell::retroiluminacion::solicitar("leds", &dispositivo, nuevo);
    crate::brillo_auto::tecla_teclado(state, &dispositivo, nuevo);
    // El nivel se enseña en tanto por ciento para que la barra diga algo: con
    // cuatro pasos, "1 de 3" no se lee de un vistazo.
    let porciento = (u64::from(nuevo) * 100 / u64::from(maximo.max(1))) as u8;
    ("teclado", Some(porciento), None)
}

fn alternar_brillo_teclado(state: &mut BookosComp) -> (&'static str, Option<u8>, Option<String>) {
    let Some((dispositivo, actual, maximo)) = bookos_shell::brillo_teclado_actual() else {
        return ("teclado", None, Some("Sin luz de teclado".into()));
    };
    let actual = crate::brillo_auto::partida_tecla_teclado(state, &dispositivo, actual);
    let nuevo = if actual == 0 { 1.min(maximo) } else { 0 };
    bookos_shell::retroiluminacion::solicitar("leds", &dispositivo, nuevo);
    crate::brillo_auto::tecla_teclado(state, &dispositivo, nuevo);
    let porciento = (u64::from(nuevo) * 100 / u64::from(maximo.max(1))) as u8;
    ("teclado", Some(porciento), None)
}

fn camara(estado: Option<bool>) -> (&'static str, Option<u8>, Option<String>) {
    let (icono, texto) = match estado {
        Some(true) => ("camara", "Cámara activada"),
        Some(false) => ("camara-desactivado", "Cámara desactivada"),
        None => ("camara", "Cámara"),
    };
    // Icono propio: `camera-web` es del tema del sistema y en esta máquina no
    // resolvía a nada, así que el aviso salía sin dibujo.
    (icono, None, Some(texto.into()))
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
    let (icono, texto) = if state.touchpad_activo {
        ("touchpad", "Touchpad activado")
    } else {
        ("touchpad-desactivado", "Touchpad desactivado")
    };
    (icono, None, Some(texto.into()))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Los bytes tal cual los dio `hexdump -C /dev/input/event3` en el Book5 Pro
    /// al pulsar Fn+F11 dos veces: bloquear, un `SYN_REPORT` y desbloquear.
    #[test]
    fn la_tapa_de_la_camara_se_lee_como_la_manda_el_driver() {
        let volcado: [u8; 96] = [
            0xcd, 0xb2, 0xbb, 0x6a, 0, 0, 0, 0, 0x74, 0xed, 0x0b, 0, 0, 0, 0, 0, //
            0x05, 0, 0x09, 0, 0x01, 0, 0, 0, 0xcd, 0xb2, 0xbb, 0x6a, 0, 0, 0, 0, //
            0x74, 0xed, 0x0b, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
            0xce, 0xb2, 0xbb, 0x6a, 0, 0, 0, 0, 0xbd, 0x42, 0x0c, 0, 0, 0, 0, 0, //
            0x05, 0, 0x09, 0, 0, 0, 0, 0, 0xce, 0xb2, 0xbb, 0x6a, 0, 0, 0, 0, //
            0xbd, 0x42, 0x0c, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let lecturas: Vec<_> = volcado
            .as_chunks::<EVENTO>()
            .0
            .iter()
            .map(camara_activa)
            .collect();
        assert_eq!(lecturas, [Some(false), None, Some(true), None]);
    }
}
