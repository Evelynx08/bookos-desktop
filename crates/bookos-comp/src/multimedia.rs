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

use std::process::{Command, Stdio};

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
    let (icono, nivel, texto) = match tecla {
        Tecla::Volumen(paso) => volumen(paso),
        Tecla::Silenciar => silenciar("@DEFAULT_AUDIO_SINK@", false),
        Tecla::SilenciarMicro => silenciar("@DEFAULT_AUDIO_SOURCE@", true),
        Tecla::Brillo(paso) => brillo(state, paso),
        Tecla::BrilloTeclado(paso) => brillo_teclado(paso),
        Tecla::Touchpad => touchpad(state),
    };
    // El chasquido al mover el volumen: es lo que dice que la tecla ha hecho
    // algo cuando no estás mirando la pantalla. Solo con el volumen del
    // altavoz —al bajar el brillo no suena nada en ningún escritorio— y no al
    // silenciar, que sonaría justo cuando has pedido silencio.
    if matches!(tecla, Tecla::Volumen(_)) {
        chasquido();
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.mostrar_osd(icono, nivel, texto);
    }
    state.needs_redraw = true;
    despertar_para_la_salida(state);
}

/// El sonido de "volumen cambiado" del tema de sonidos de freedesktop.
///
/// Se lanza con `canberra-gtk-play`, que es lo que usa Plasma: sabe resolver el
/// nombre del evento dentro del tema instalado, así que no hay que codificar
/// una ruta a un `.oga` que en otra distribución está en otro sitio. Va sin
/// esperar —igual que `wpctl`— porque son 30 ms de proceso y el compositor no
/// puede quedarse parado mientras suena.
///
/// Si el binario no está, no suena nada y ya: un escritorio sin sonidos de
/// sistema funciona, uno que se bloquea buscándolos no.
fn chasquido() {
    let _ = Command::new("canberra-gtk-play")
        .args(["-i", "audio-volume-change"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
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
    let result = state
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
fn volumen(paso: i32) -> (&'static str, Option<u8>, Option<String>) {
    let (actual, silenciado) = leer_volumen().unwrap_or((0, false));
    let nuevo = (actual as i32 + paso).clamp(0, 100) as u8;
    let _ = wpctl(&[
        "set-volume",
        "@DEFAULT_AUDIO_SINK@",
        &format!("{:.2}", nuevo as f32 / 100.0),
    ]);
    // Subir el volumen de algo silenciado lo devuelve a la vida: si no, la
    // barra sube y no se oye nada.
    if silenciado && paso > 0 {
        let _ = wpctl(&["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]);
    }
    (icono_volumen(nuevo, false), Some(nuevo), None)
}

fn silenciar(destino: &str, micro: bool) -> (&'static str, Option<u8>, Option<String>) {
    let (nivel, silenciado) = leer_volumen_de(destino).unwrap_or((0, false));
    let _ = wpctl(&["set-mute", destino, "toggle"]);
    let ahora = !silenciado;
    if micro {
        let icono = if ahora { "micro-silencio" } else { "micro" };
        let texto = if ahora { "Micrófono silenciado" } else { "Micrófono activo" };
        (icono, None, Some(texto.into()))
    } else if ahora {
        ("volumen-silencio", Some(0), None)
    } else {
        (icono_volumen(nivel, false), Some(nivel), None)
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

fn leer_volumen() -> Option<(u8, bool)> {
    leer_volumen_de("@DEFAULT_AUDIO_SINK@")
}

fn leer_volumen_de(destino: &str) -> Option<(u8, bool)> {
    let salida = Command::new("wpctl")
        .arg("get-volume")
        .arg(destino)
        .output()
        .ok()?;
    salida.status.success().then_some(())?;
    let texto = String::from_utf8_lossy(&salida.stdout);
    let resto = texto.split_once("Volume:")?.1;
    let valor: f32 = resto.split_whitespace().next()?.parse().ok()?;
    Some((
        (valor * 100.0).round().clamp(0.0, 100.0) as u8,
        texto.contains("[MUTED]"),
    ))
}

fn wpctl(args: &[&str]) -> Option<std::process::Child> {
    Command::new("wpctl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// Sube o baja el brillo de la pantalla por logind, que es quien tiene el
/// permiso: `/sys/class/backlight/*/brightness` es de root.
fn brillo(state: &mut BookosComp, paso: i32) -> (&'static str, Option<u8>, Option<String>) {
    let actual = bookos_shell::brillo_actual().unwrap_or(50);
    let nuevo = (actual as i32 + paso).clamp(5, 100) as u8;
    if let Some((dispositivo, maximo)) = bookos_shell::backlight() {
        let crudo = (maximo as f64 * nuevo as f64 / 100.0).round() as u32;
        let _ = Command::new("busctl")
            .args([
                "call",
                "org.freedesktop.login1",
                "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session",
                "SetBrightness",
                "ssu",
                "backlight",
                &dispositivo,
                &crudo.to_string(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    // El widget del panel lo relee en su próximo refresco; forzarlo aquí
    // costaría los 21 ms de `wpctl` que ya evitamos en otro sitio.
    let _ = state;
    ("brillo", Some(nuevo), None)
}

/// El teclado tiene su propia retroiluminación, con niveles enteros (0..max) y
/// no un tanto por ciento: en este portátil son cuatro pasos.
fn brillo_teclado(paso: i32) -> (&'static str, Option<u8>, Option<String>) {
    let Some((dispositivo, maximo)) = leds_teclado() else {
        return ("teclado", None, Some("Sin luz de teclado".into()));
    };
    let actual = std::fs::read_to_string(format!("/sys/class/leds/{dispositivo}/brightness"))
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .unwrap_or(0);
    let nuevo = (actual + paso).clamp(0, maximo as i32);
    let _ = Command::new("busctl")
        .args([
            "call",
            "org.freedesktop.login1",
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.login1.Session",
            "SetBrightness",
            "ssu",
            "leds",
            &dispositivo,
            &nuevo.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    // El nivel se enseña en tanto por ciento para que la barra diga algo: con
    // cuatro pasos, "1 de 3" no se lee de un vistazo.
    let porciento = (nuevo * 100 / maximo.max(1) as i32) as u8;
    ("teclado", Some(porciento), None)
}

fn leds_teclado() -> Option<(String, u32)> {
    let dir = std::fs::read_dir("/sys/class/leds").ok()?;
    for entrada in dir.filter_map(|e| e.ok()) {
        let nombre = entrada.file_name().to_string_lossy().to_string();
        if !nombre.contains("kbd_backlight") {
            continue;
        }
        let max = std::fs::read_to_string(entrada.path().join("max_brightness"))
            .ok()?
            .trim()
            .parse::<u32>()
            .ok()?;
        return Some((nombre, max));
    }
    None
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
