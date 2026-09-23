//! Brillo automático: la pantalla y la luz del teclado siguen al sensor de luz
//! ambiente.
//!
//! Los lux salen de iio-sensor-proxy y no de sysfs. El sensor HID del ISH
//! empuja lecturas por su buffer cuando la luz cambia, pero ese buffer
//! (`/dev/iio:device0`) es de root y ya lo tiene abierto iio-sensor-proxy. Leer
//! `in_illuminance_raw` desde aquí obligaría a sondear, y cada lectura es una
//! petición HID al ISH. El proxy manda `PropertiesChanged` solo cuando la luz
//! cambia: entre cambio y cambio no se despierta nada.
//!
//! Como las notificaciones, el bus vive en su propio hilo y los lux llegan al
//! bucle por un canal de calloop. El sensor solo se reclama (`ClaimLight`)
//! mientras haya algo en automático; sin reclamar, el proxy lo deja parado.
//!
//! ## Cambios a mano
//!
//! No se comprueba de dónde viene un cambio (la tecla Fn, la tarjeta, `busctl`
//! o el firmware): antes de ajustar se relee sysfs y se compara con lo último
//! que puso el automático.
//! - **Pantalla**: la diferencia se queda como desvío sobre la curva. Quien la
//!   sube un 20 % en un cuarto oscuro la tiene un 20 % más alta también con
//!   luz, en vez de verla volver a la curva con el siguiente cambio de luz.
//! - **Teclado**: solo se toca al cruzar de oscuro a claro o al revés. Lo que
//!   se ponga a mano dura hasta el siguiente cruce, y el último nivel encendido
//!   a mano es el que se usa al volver a encender.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{LoopHandle, RegistrationToken};

use crate::state::BookosComp;

/// Cuánto tiene que durar una bajada de luz antes de oscurecer. Una mano
/// delante del sensor o una sombra al pasar no deben apagar la pantalla a
/// medias. Subir no espera: salir a la luz con la pantalla oscura no se lee.
const ESPERA_AL_BAJAR: Duration = Duration::from_secs(3);

/// Tras escribir un brillo, sysfs tarda en reflejarlo: la escritura va por
/// `busctl` en el hilo de `retroiluminacion`. Durante este margen, una lectura
/// distinta se achaca a la escritura en curso y no a la mano del usuario.
const MARGEN_ESCRITURA: Duration = Duration::from_secs(2);

/// Diferencia mínima, en puntos, para mover la pantalla. El sensor avisa con
/// cambios del 1 % (`in_illuminance_hysteresis_relative` = 0.01 en este
/// portátil), y seguirlos todos sería ver la pantalla temblar.
const UMBRAL_PANTALLA: u8 = 4;

/// Lecturas en lux para encender y apagar el teclado. Separados para que una
/// luz justo en el límite no lo encienda y lo apague sin parar. No salen de
/// ninguna medición salvo una: el cuarto de pruebas con luz tenue daba 18 lux.
const TECLADO_ENCENDER: f64 = 15.0;
const TECLADO_APAGAR: f64 = 50.0;

/// La rampa: cada paso sube o baja una fracción de lo que queda.
const PASO_RAMPA: Duration = Duration::from_millis(40);

/// Lo que se pide al hilo del bus.
enum Orden {
    Reclamar,
    Soltar,
}

pub struct Estado {
    elegido: bookos_shell::BrilloAutomatico,
    ordenes: mpsc::Sender<Orden>,
    reclamado: bool,
    lux: Option<f64>,
    /// Último nivel de pantalla que puso el automático.
    pantalla_puesta: Option<u8>,
    /// Lo que el usuario movió la pantalla respecto a la curva, en puntos.
    desvio: i32,
    /// `Some(true)` con el teclado en la zona oscura, `None` antes de decidir.
    teclado_oscuro: Option<bool>,
    /// Nivel al que se enciende el teclado. Empieza en 1 y se actualiza con el
    /// que el usuario tenga encendido.
    teclado_encendido: u32,
    escrito: Option<Instant>,
    espera: Option<RegistrationToken>,
    rampa: Option<RegistrationToken>,
}

impl Estado {
    pub fn arrancar(
        loop_handle: &LoopHandle<'static, BookosComp>,
        elegido: bookos_shell::BrilloAutomatico,
    ) -> anyhow::Result<Self> {
        let (lux_tx, lux_rx) = smithay::reexports::calloop::channel::channel();
        loop_handle
            .insert_source(lux_rx, |evento, _, state| {
                if let smithay::reexports::calloop::channel::Event::Msg(lux) = evento {
                    recibir(state, lux);
                }
            })
            .map_err(|err| anyhow::anyhow!("insert_source(luz): {err}"))?;
        let (ordenes, ordenes_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("bookos-luz".into())
            .spawn(move || escuchar(lux_tx, ordenes_rx))?;
        bookos_shell::retroiluminacion::aplicar_automatico(elegido);
        let mut estado = Self {
            elegido,
            ordenes,
            reclamado: false,
            lux: None,
            pantalla_puesta: None,
            desvio: 0,
            teclado_oscuro: None,
            teclado_encendido: 1,
            escrito: None,
            espera: None,
            rampa: None,
        };
        estado.reclamar_si_hace_falta();
        Ok(estado)
    }

    fn reclamar_si_hace_falta(&mut self) {
        let quiere = self.elegido.pantalla || self.elegido.teclado;
        if quiere == self.reclamado {
            return;
        }
        self.reclamado = quiere;
        let orden = if quiere {
            Orden::Reclamar
        } else {
            Orden::Soltar
        };
        let _ = self.ordenes.send(orden);
    }
}

/// El hilo del bus: reclama o suelta el sensor según las órdenes y reenvía cada
/// lectura en lux.
fn escuchar(
    lux: smithay::reexports::calloop::channel::Sender<f64>,
    ordenes: mpsc::Receiver<Orden>,
) {
    let proxy = zbus::blocking::Connection::system().and_then(|conexion| {
        zbus::blocking::Proxy::new(
            &conexion,
            "net.hadess.SensorProxy",
            "/net/hadess/SensorProxy",
            "net.hadess.SensorProxy",
        )
    });
    let proxy = match proxy {
        Ok(proxy) => proxy,
        Err(err) => {
            tracing::warn!("sin iio-sensor-proxy, no habrá brillo automático: {err}");
            return;
        }
    };
    let escucha = proxy.clone();
    let lanzado = std::thread::Builder::new()
        .name("bookos-luz-lux".into())
        .spawn(move || {
            for cambio in escucha.receive_property_changed::<f64>("LightLevel") {
                // Con otra unidad («vendor») los números no son lux y la curva
                // no significa nada: mejor no mover el brillo que moverlo mal.
                let unidad = escucha.cached_property::<String>("LightLevelUnit");
                if !matches!(unidad, Ok(Some(ref u)) if u == "lux") {
                    continue;
                }
                if let Ok(valor) = cambio.get()
                    && lux.send(valor).is_err()
                {
                    return;
                }
            }
        });
    if let Err(err) = lanzado {
        tracing::warn!("no se pudo escuchar el sensor de luz: {err}");
        return;
    }
    for orden in ordenes {
        let (metodo, texto) = match orden {
            Orden::Reclamar => ("ClaimLight", "reclamar"),
            Orden::Soltar => ("ReleaseLight", "soltar"),
        };
        if let Err(err) = proxy.call_method(metodo, &()) {
            tracing::warn!("no se pudo {texto} el sensor de luz: {err}");
        }
    }
}

/// Lo que eligió el usuario desde la tarjeta o desde Settings.
pub fn elegir(state: &mut BookosComp, elegido: bookos_shell::BrilloAutomatico) {
    let e = &mut state.brillo_auto;
    bookos_shell::retroiluminacion::aplicar_automatico(elegido);
    // Al encender uno, se empieza de cero con él: un desvío o una zona de hace
    // una hora no dicen nada de lo que el usuario quiere ahora.
    if elegido.pantalla && !e.elegido.pantalla {
        e.pantalla_puesta = None;
        e.desvio = 0;
    }
    if elegido.teclado && !e.elegido.teclado {
        e.teclado_oscuro = None;
    }
    e.elegido = elegido;
    e.reclamar_si_hace_falta();
    if !elegido.pantalla {
        bookos_shell::retroiluminacion::poner_objetivo_sensor(None);
    }
    if !elegido.pantalla
        && let Some(token) = e.rampa.take()
    {
        state.loop_handle.remove(token);
    }
    evaluar(state);
    // El sol del panel lleva la «A» mientras la pantalla va sola: cambiar la
    // preferencia no toca el brillo, así que ningún evento de udev lo avisaría.
    if let Some(shell) = state.shell.as_mut() {
        shell.refresh();
    }
}

fn recibir(state: &mut BookosComp, lux: f64) {
    let e = &mut state.brillo_auto;
    let bajando = e.lux.is_some_and(|previo| lux < previo);
    e.lux = Some(lux);
    if !bajando {
        if let Some(token) = e.espera.take() {
            state.loop_handle.remove(token);
        }
        evaluar(state);
        return;
    }
    // Una espera ya en marcha evaluará con la lectura más reciente, que es esta.
    if e.espera.is_some() {
        return;
    }
    let token =
        state
            .loop_handle
            .insert_source(Timer::from_duration(ESPERA_AL_BAJAR), |_, _, state| {
                state.brillo_auto.espera = None;
                evaluar(state);
                TimeoutAction::Drop
            });
    state.brillo_auto.espera = token.ok();
}

/// Ajusta lo que esté en automático a la última lectura.
///
/// Se llama también al volver de DPMS: con la pantalla apagada no se ajusta
/// nada, y la luz pudo cambiar entretanto.
pub fn evaluar(state: &mut BookosComp) {
    let Some(lux) = state.brillo_auto.lux else {
        return;
    };
    if !state.dpms_encendido {
        return;
    }
    if state.brillo_auto.elegido.pantalla {
        pantalla(state, lux);
    }
    if state.brillo_auto.elegido.teclado {
        teclado(&mut state.brillo_auto, lux);
    }
}

/// El brillo que toca con esa luz, en tanto por ciento.
///
/// Logarítmica porque así se percibe la luz: de 1 a 10 lux se nota tanto como
/// de 100 a 1000. Da 12 % a oscuras, 40 % a 18 lux, 78 % a 1000 y 100 % desde
/// unos 10 000, que es luz de día cerca de una ventana.
fn curva(lux: f64) -> i32 {
    (12.0 + 22.0 * (1.0 + lux.max(0.0)).log10()).round() as i32
}

fn reciente(e: &Estado) -> bool {
    e.rampa.is_some() || e.escrito.is_some_and(|t| t.elapsed() < MARGEN_ESCRITURA)
}

fn pantalla(state: &mut BookosComp, lux: f64) {
    bookos_shell::retroiluminacion::poner_objetivo_sensor(Some(curva(lux).clamp(5, 100) as u8));
    let Some(actual) = bookos_shell::brillo_actual() else {
        return;
    };
    let e = &mut state.brillo_auto;
    if let Some(puesta) = e.pantalla_puesta
        && !reciente(e)
        && actual.abs_diff(puesta) > 2
    {
        e.desvio = (e.desvio + i32::from(actual) - i32::from(puesta)).clamp(-60, 60);
        tracing::debug!(desvio = e.desvio, "brillo movido a mano");
    }
    let objetivo = (curva(lux) + e.desvio).clamp(5, 100) as u8;
    if objetivo.abs_diff(actual) < UMBRAL_PANTALLA {
        if !reciente(e) {
            e.pantalla_puesta = Some(actual);
        }
        return;
    }
    e.pantalla_puesta = Some(objetivo);
    if let Some(token) = e.rampa.take() {
        state.loop_handle.remove(token);
    }
    let mut nivel = actual;
    let token = state
        .loop_handle
        .insert_source(Timer::immediate(), move |_, _, state| {
            let Some((dispositivo, maximo)) = bookos_shell::backlight() else {
                state.brillo_auto.rampa = None;
                return TimeoutAction::Drop;
            };
            // Un octavo de lo que queda, y al menos un punto: rápido al
            // principio y suave al llegar, en unos 600 ms para un salto grande.
            let queda = i32::from(objetivo) - i32::from(nivel);
            let paso = (queda / 8).abs().max(1) * queda.signum();
            nivel = (i32::from(nivel) + paso) as u8;
            let crudo = (f64::from(maximo) * f64::from(nivel) / 100.0).round() as u32;
            bookos_shell::retroiluminacion::solicitar("backlight", &dispositivo, crudo);
            state.brillo_auto.escrito = Some(Instant::now());
            if nivel == objetivo {
                state.brillo_auto.rampa = None;
                TimeoutAction::Drop
            } else {
                TimeoutAction::ToDuration(PASO_RAMPA)
            }
        });
    e.rampa = token.ok();
}

fn teclado(e: &mut Estado, lux: f64) {
    let Some((dispositivo, actual, maximo)) = bookos_shell::brillo_teclado_actual() else {
        return;
    };
    if actual > 0 && !reciente(e) {
        e.teclado_encendido = actual;
    }
    let oscuro = if lux < TECLADO_ENCENDER {
        true
    } else if lux > TECLADO_APAGAR {
        false
    } else {
        // Entre los dos umbrales manda la zona de antes; sin zona todavía, lo
        // que haya puesto se respeta.
        match e.teclado_oscuro {
            Some(zona) => zona,
            None => return,
        }
    };
    if e.teclado_oscuro == Some(oscuro) {
        return;
    }
    e.teclado_oscuro = Some(oscuro);
    let nivel = if oscuro {
        e.teclado_encendido.min(maximo)
    } else {
        0
    };
    if nivel != actual {
        bookos_shell::retroiluminacion::solicitar("leds", &dispositivo, nivel);
        e.escrito = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::curva;

    #[test]
    fn la_curva_sube_con_la_luz_y_no_se_sale() {
        assert_eq!(curva(0.0), 12);
        assert_eq!(curva(18.0), 40);
        assert!(curva(1000.0) < curva(10_000.0));
        assert!(curva(100_000.0) > 100, "la acota quien la usa, no la curva");
        assert_eq!(
            curva(-3.0),
            12,
            "una lectura negativa cuenta como oscuridad"
        );
    }
}
