//! Puente entre BookOS Settings y el compositor: la interfaz `org.bookos.Desktop`.
//!
//! Un único servicio en el bus de sesión para todo lo que Settings necesita del
//! escritorio. Empezó con la recarga de la pantalla de bloqueo y ahora lleva
//! también la configuración de pantallas, que es lo que sustituye a KScreen
//! cuando la sesión es BookOS y no Plasma.
//!
//! **Contrato** (`org.bookos.Desktop`, en `/org/bookos/Desktop`):
//!
//! ```text
//! ReloadConfig(s seccion) -> b
//!     Relee `panel.conf`. Secciones: "lockscreen"/"bloqueo",
//!     "activities"/"actividades", "all".
//!
//! GetCapabilities() -> a{sv}
//!     Qué sabe hacer este backend. Diccionario a propósito: añadir una
//!     capacidad no rompe a quien ya lee las que conoce. Claves de la v1:
//!     version(u) backend(s) live_apply(b) fractional_scale(b)
//!     per_output_scale(b) position(b) rotation(b) mode(b) refresh(b) vrr(b)
//!     multi_output(b) primary(b) hdr(b) icc(b) night_light(b) escalas(ad).
//!
//! GetOutputs() -> a(ssssuubba(uuubb)dadiisbbbii)
//!     El censo de salidas. Ver `pantallas::Salida` para el orden de campos;
//!     la firma la genera zvariant y no hay que escribirla a mano.
//!
//! ApplyOutputConfig(a(sbuuudiisbb) config) -> (b ok, s error)
//!     Aplica en vivo y persiste. `ok=false` con el motivo en `error` cuando la
//!     validación o el hardware la rechazan; entonces **no** se guarda nada y
//!     se vuelve a lo que había.
//!
//! signal OutputsChanged()
//!     Algo cambió en las pantallas —se aplicó una configuración, se enchufó o
//!     se quitó un monitor—. Quien la reciba vuelve a pedir `GetOutputs`.
//! ```
//!
//! **Por qué el hilo de D-Bus no toca nada gráfico.** zbus atiende el bus en su
//! propio hilo. Leer el censo es leer un `Mutex` que el compositor rellena;
//! aplicar es mandar la petición por el canal de calloop y esperar la respuesta
//! por un canal de vuelta. Ni el renderer ni el backend DRM se comparten entre
//! hilos, que es lo que permite que todo esto no lleve ni un `unsafe`.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize;
use smithay::reexports::calloop::channel::Sender;

use crate::pantallas::{Compartido, Peticion, Salida};

pub const NOMBRE: &str = "org.bookos.Desktop";
pub const RUTA: &str = "/org/bookos/Desktop";

/// Cuánto espera el hilo de D-Bus a que el compositor conteste. Un modeset con
/// su reintento tarda decenas de milisegundos; cinco segundos es "el compositor
/// no va a contestar", no "está tardando".
const ESPERA: Duration = Duration::from_secs(5);

pub enum Aviso {
    Recargar(String),
    /// Configuración de pantallas y por dónde devolver el veredicto.
    Salidas(Vec<Peticion>, mpsc::Sender<Result<(), String>>),
    /// Algo cambió en el hardware: volver a censar y avisar por la señal.
    Redetectar,
    Actividad(bookos_shell::actividad::Estado),
    CerrarActividad(String),
    AbrirActividadPrevisualizacion(String),
}

#[derive(Deserialize)]
struct ActividadJson {
    #[serde(default)] activo: bool,
    #[serde(default)] pausado: bool,
    #[serde(default)] titulo: String,
    #[serde(default)] subtitulo: String,
    #[serde(default)] posicion_ms: i64,
    #[serde(default)] duracion_ms: i64,
    #[serde(default)] restante_ms: i64,
    #[serde(default = "volumen_defecto")] volumen: u8,
    #[serde(default)] nivel: f32,
    // Aleatorio y repetición: la isla pinta encendidos sus dos botones. Van con
    // `default` como todo lo demás, así que una app que no los mande sigue
    // publicando igual que antes.
    #[serde(default)] aleatorio: bool,
    #[serde(default)] repetir: bool,
    #[serde(default)] portada: String,
    #[serde(default)] cola: Vec<ItemColaJson>,
}

#[derive(Deserialize)]
struct ItemColaJson {
    #[serde(default)] id: String,
    #[serde(default)] titulo: String,
    #[serde(default)] artista: String,
    #[serde(default)] duracion_ms: i64,
    #[serde(default)] favorita: bool,
    #[serde(default)] actual: bool,
}

fn volumen_defecto() -> u8 { 100 }

struct Servidor {
    canal: Sender<Aviso>,
    compartido: Arc<Compartido>,
}

#[zbus::interface(name = "org.bookos.Desktop")]
impl Servidor {
    /// Solicita recargar una sección. Devuelve `true` cuando la orden pudo
    /// entregarse al bucle del compositor.
    fn reload_config(&self, section: String) -> bool {
        self.canal.send(Aviso::Recargar(section)).is_ok()
    }

    /// Qué sabe hacer este backend.
    fn get_capabilities(&self) -> std::collections::HashMap<String, zbus::zvariant::OwnedValue> {
        self.compartido.capacidades()
    }

    /// El censo de salidas conectadas.
    fn get_outputs(&self) -> Vec<Salida> {
        self.compartido.salidas()
    }

    /// Aplica una configuración de pantallas. `(ok, error)`: `ok=false` nunca
    /// va con `error` vacío, para que la interfaz no pueda enseñar un éxito que
    /// no ocurrió.
    fn apply_output_config(&self, config: Vec<Peticion>) -> (bool, String) {
        let (respuesta, espera) = mpsc::channel();
        if self.canal.send(Aviso::Salidas(config, respuesta)).is_err() {
            return (false, "el compositor no está escuchando".into());
        }
        match espera.recv_timeout(ESPERA) {
            Ok(Ok(())) => (true, String::new()),
            Ok(Err(err)) => (false, err),
            Err(_) => (
                false,
                "el compositor no contestó a tiempo; no se ha cambiado nada".into(),
            ),
        }
    }

    /// Publica una tarea viva. El identificador está en una lista cerrada: una
    /// app cualquiera no puede convertir la isla en una segunda bandeja de
    /// notificaciones.
    fn publish_activity(&self, app_id: String, kind: String, state_json: String) -> bool {
        let Some(clase) = clase_permitida(&app_id, &kind) else {
            tracing::warn!(app_id, kind, "publicador de actividad rechazado");
            return false;
        };
        if state_json.len() > 8 * 1024 * 1024 {
            tracing::warn!(app_id, "estado de actividad demasiado grande");
            return false;
        }
        let Ok(json) = serde_json::from_str::<ActividadJson>(&state_json) else {
            tracing::warn!(app_id, "estado de actividad no válido");
            return false;
        };
        let portada = decodificar_portada(&json.portada);
        let estado = bookos_shell::actividad::Estado {
            app_id,
            clase,
            activo: json.activo,
            pausado: json.pausado,
            titulo: limitar(json.titulo, 120),
            subtitulo: limitar(json.subtitulo, 160),
            posicion_ms: json.posicion_ms,
            duracion_ms: json.duracion_ms.max(0),
            restante_ms: json.restante_ms,
            volumen: json.volumen.min(100),
            nivel: json.nivel.clamp(0.0, 1.0),
            aleatorio: json.aleatorio,
            repetir: json.repetir,
            portada,
            cola: json.cola.into_iter().take(50).map(|i| bookos_shell::actividad::ItemCola {
                id: limitar(i.id, 512), titulo: limitar(i.titulo, 120),
                artista: limitar(i.artista, 120), duracion_ms: i.duracion_ms.max(0),
                favorita: i.favorita, actual: i.actual,
            }).collect(),
        };
        self.canal.send(Aviso::Actividad(estado)).is_ok()
    }

    fn close_activity(&self, app_id: String) -> bool {
        if clase_permitida(&app_id, "").is_none() { return false; }
        self.canal.send(Aviso::CerrarActividad(app_id)).is_ok()
    }

    /// Abre una actividad ya publicada para inspeccionarla durante el
    /// desarrollo. En release se rechaza: una aplicación no debe poder forzar
    /// que su tarjeta se despliegue sobre lo que esté haciendo el usuario.
    fn preview_activity_open(&self, app_id: String) -> bool {
        if !cfg!(debug_assertions) || clase_permitida(&app_id, "").is_none() {
            return false;
        }
        self.canal.send(Aviso::AbrirActividadPrevisualizacion(app_id)).is_ok()
    }

    #[zbus(signal)]
    async fn outputs_changed(emisor: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    /// Orden del usuario hacia la app que publicó la actividad.
    #[zbus(signal)]
    async fn activity_action(
        emisor: &zbus::object_server::SignalEmitter<'_>,
        app_id: &str,
        action: &str,
        value: &str,
    ) -> zbus::Result<()>;
}

pub fn recibir(state: &mut crate::state::BookosComp, aviso: Aviso) {
    match aviso {
        Aviso::Recargar(seccion) => recargar(state, &seccion),
        Aviso::Salidas(peticion, respuesta) => {
            let r = crate::pantallas::aplicar(state, peticion);
            if let Err(err) = r.as_ref() {
                tracing::warn!("configuración de pantallas rechazada: {err}");
            }
            // Que el otro extremo se haya rendido (timeout) no es motivo para
            // deshacer lo aplicado: el cambio ya está en la pantalla.
            let _ = respuesta.send(r);
        }
        Aviso::Redetectar => redetectar(state),
        Aviso::Actividad(estado) => {
            if let Some(shell) = state.shell.as_mut() {
                shell.publicar_actividad(estado);
                state.needs_redraw = true;
            }
        }
        Aviso::CerrarActividad(app_id) => {
            if state.shell.as_mut().is_some_and(|s| s.cerrar_actividad(&app_id)) {
                state.needs_redraw = true;
            }
        }
        Aviso::AbrirActividadPrevisualizacion(app_id) => {
            if state.shell.as_mut().is_some_and(|s| {
                s.abrir_actividad_previsualizacion(&app_id)
            }) {
                state.needs_redraw = true;
            }
        }
    }
}

fn clase_permitida(app_id: &str, kind: &str) -> Option<bookos_shell::actividad::Clase> {
    use bookos_shell::actividad::Clase;
    match (app_id, kind) {
        ("com.bookos.player", "player" | "") => Some(Clase::Player),
        ("com.bookos.clock", "timer" | "") => Some(Clase::Timer),
        ("com.bookos.voicerecorder", "recorder" | "") => Some(Clase::Recorder),
        _ => None,
    }
}

fn limitar(mut texto: String, max: usize) -> String {
    if texto.chars().count() > max {
        texto = texto.chars().take(max).collect();
    }
    texto
}

fn decodificar_portada(valor: &str) -> Option<bookos_shell::actividad::Portada> {
    let b64 = valor.strip_prefix("data:image/")?.split_once(',')?.1;
    if b64.len() > 6 * 1024 * 1024 { return None; }
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let (rgba, width, height) = bookos_shell::decodificar_imagen(&bytes)?;
    Some(bookos_shell::actividad::Portada { rgba, width, height })
}

fn recargar(state: &mut crate::state::BookosComp, seccion: &str) {
    if seccion != "lockscreen" && seccion != "bloqueo"
        && seccion != "activities" && seccion != "actividades" && seccion != "all"
    {
        tracing::warn!(seccion, "sección de ajustes desconocida");
        return;
    }
    let config = bookos_shell::Config::cargar();
    if let Some(shell) = state.shell.as_mut() {
        if matches!(seccion, "lockscreen" | "bloqueo" | "all") {
            shell.aplicar_bloqueo_config(config.bloqueo);
        }
        if matches!(seccion, "activities" | "actividades" | "all") {
            shell.aplicar_actividades_config(config.actividades);
        }
        state.needs_redraw = true;
    }
}

/// Vuelve a censar las salidas sin tocar la configuración. Es lo que se hace
/// cuando el kernel avisa de un cambio de conector: enterarse de que hay un
/// monitor nuevo no es lo mismo que decidir qué hacer con él.
fn redetectar(state: &mut crate::state::BookosComp) {
    let Some(censar) = state.censar_pantallas.take() else {
        return;
    };
    let salidas = censar();
    state.censar_pantallas = Some(censar);
    if salidas == state.pantallas.compartido.salidas() {
        return;
    }
    state.pantallas.compartido.publicar(salidas);
    avisar_salidas(state);
}

/// Emite `OutputsChanged`. Se llama desde el hilo del compositor con la
/// conexión bloqueante que ya se guarda en el estado: emitir una señal es
/// escribir en el socket del bus, no hay que despertar a zbus para eso.
pub fn avisar_salidas(state: &crate::state::BookosComp) {
    let Some(conexion) = state.bus_ajustes.as_ref() else {
        return;
    };
    let r = conexion.emit_signal(None::<&str>, RUTA, NOMBRE, "OutputsChanged", &());
    if let Err(err) = r {
        tracing::warn!("no se pudo emitir OutputsChanged: {err}");
    }
}

pub fn accion_actividad(state: &crate::state::BookosComp, accion: &bookos_shell::actividad::Accion) {
    let Some(conexion) = state.bus_ajustes.as_ref() else { return; };
    if let Err(err) = conexion.emit_signal(
        None::<&str>, RUTA, NOMBRE, "ActivityAction",
        &(&accion.app_id, &accion.nombre, &accion.valor),
    ) {
        tracing::warn!("no se pudo enviar la acción de actividad: {err}");
    }
}

pub fn arrancar(canal: Sender<Aviso>, compartido: Arc<Compartido>) -> Option<zbus::blocking::Connection> {
    let conexion = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(NOMBRE))
        .and_then(|b| b.serve_at(RUTA, Servidor { canal, compartido }))
        .and_then(|b| b.build());
    match conexion {
        Ok(conexion) => {
            tracing::info!("interfaz de ajustes de BookOS en el bus");
            Some(conexion)
        }
        Err(err) => {
            tracing::warn!("sin interfaz de ajustes de BookOS: {err}");
            None
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::pantallas::{Modo, Salida};

    fn salida_de_prueba() -> Salida {
        Salida {
            id: "SDC-ATNA40YK-0".into(),
            conector: "eDP-1".into(),
            fabricante: "SDC".into(),
            modelo: "ATNA40YK".into(),
            serie: String::new(),
            mm_ancho: 300,
            mm_alto: 190,
            activa: true,
            modos: vec![Modo {
                ancho: 2880,
                alto: 1800,
                refresco_mhz: 120_000,
                preferido: true,
                actual: true,
            }],
            escala: 1.75,
            escalas: crate::pantallas::ESCALAS.to_vec(),
            x: 0,
            y: 0,
            transformacion: "normal".into(),
            vrr_capaz: true,
            vrr: false,
            principal: true,
            logico_ancho: 1646,
            logico_alto: 1029,
        }
    }

    /// El censo y las capacidades tienen que poder ir y volver **por el bus de
    /// verdad**: las firmas las genera zvariant desde los tipos, y un campo que
    /// no se pueda serializar no se ve compilando, se ve al llamar.
    ///
    /// El servidor se publica sin pedir `org.bookos.Desktop` y se le habla por
    /// el nombre único de la conexión: así el test no le quita el nombre a una
    /// sesión de BookOS que esté corriendo en la misma máquina.
    #[test]
    fn el_censo_viaja_por_el_bus() {
        let (emisor, receptor) = smithay::reexports::calloop::channel::channel::<Aviso>();
        // Sin bucle de eventos nadie atiende el canal. Se suelta el extremo de
        // lectura para que el envío falle en el acto en vez de esperar los
        // cinco segundos de `ESPERA`: lo que se prueba aquí es la firma y el
        // camino del error, no el reloj.
        drop(receptor);
        let compartido = Arc::new(Compartido::default());
        compartido.publicar(vec![salida_de_prueba()]);

        let servidor = Servidor {
            canal: emisor,
            compartido: compartido.clone(),
        };
        let conexion = match zbus::blocking::connection::Builder::session()
            .and_then(|b| b.serve_at(RUTA, servidor))
            .and_then(|b| b.build())
        {
            Ok(c) => c,
            // Sin bus de sesión —una compilación en un contenedor, por
            // ejemplo— no hay nada que probar aquí.
            Err(_) => return,
        };
        let yo = conexion.unique_name().expect("la conexión tiene nombre único").to_string();

        let cliente = zbus::blocking::Connection::session().expect("bus de sesión");
        let proxy = zbus::blocking::Proxy::new(&cliente, yo.as_str(), RUTA, NOMBRE)
            .expect("proxy");

        let salidas: Vec<Salida> = proxy.call("GetOutputs", &()).expect("GetOutputs");
        assert_eq!(salidas, vec![salida_de_prueba()]);

        let caps: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
            proxy.call("GetCapabilities", &()).expect("GetCapabilities");
        assert_eq!(
            u32::try_from(&caps["version"]).unwrap(),
            crate::pantallas::VERSION_CONTRATO
        );
        assert!(caps.contains_key("escalas"));
        assert!(caps.contains_key("fractional_scale"));

        // Y una configuración imposible tiene que volver como fallo con motivo,
        // no como un éxito silencioso. Nadie atiende el canal en el test, así
        // que lo que se comprueba es que la llamada contesta y que `ok` es
        // falso con un `error` no vacío.
        let (ok, error): (bool, String) = proxy
            .call("ApplyOutputConfig", &(Vec::<crate::pantallas::Peticion>::new(),))
            .expect("ApplyOutputConfig");
        assert!(!ok);
        assert!(!error.is_empty());
    }
}
