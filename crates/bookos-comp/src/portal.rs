//! El backend de `xdg-desktop-portal` para compartir pantalla y capturarla.
//!
//! Es lo que hace que «compartir pantalla» funcione desde un navegador: Meet,
//! Discord u OBS no hablan Wayland para esto, hablan
//! `org.freedesktop.portal.ScreenCast`, y `xdg-desktop-portal` reenvía a la
//! interfaz `impl` que implementa este fichero.
//!
//! ## Por qué va dentro del compositor y no en un proceso aparte
//!
//! Un backend externo —`xdg-desktop-portal-wlr`, `-gnome`, `-kde`— es un
//! cliente Wayland más: para ver la pantalla tiene que pedirla por
//! `zwlr_screencopy`, recibirla en memoria compartida y volver a subirla a
//! PipeWire. Eso es una copia de ida y otra de vuelta sobre la que ya duele.
//! Aquí el fotograma sale del mismo composite que ya se hace ([`crate::backend::servir_capturas`])
//! y va directo al hilo de PipeWire.
//!
//! Y el diálogo de permiso lo dibuja el propio escritorio. Un backend externo
//! tendría que traerse GTK o Qt para enseñar una tarjeta; el shell ya está en
//! este proceso.
//!
//! ## Cómo se reparte el trabajo
//!
//! Igual que [`crate::notificaciones`]: `zbus` bloqueante en su propio hilo y
//! un canal de `calloop` hacia el bucle de frames. Este hilo **no toca nada
//! gráfico**; manda un [`Aviso`] y espera la respuesta.
//!
//! Esperar **no puede ser bloqueando**. zbus atiende toda la conexión en un
//! ejecutor de un solo hilo, así que un método síncrono parado deja muerto el
//! portal entero: medido, con el diálogo del permiso abierto ni siquiera se
//! podía leer una propiedad —`busctl ... Properties Get` daba tiempo de espera
//! a los 8 s—, y por tanto `xdg-desktop-portal` tampoco podía cancelar ni
//! abrir una segunda sesión. Por eso los métodos que esperan son `async` y
//! esperan sobre [`Promesa`], que es un `Future` de verdad: mientras el
//! usuario mira el diálogo, la conexión sigue atendiendo lo demás.
//!
//! También sirve `org.freedesktop.impl.portal.Settings`, que es por donde las
//! aplicaciones **de fuera** —GTK, Qt, Tauri— se enteran del tema, del acento y
//! de los efectos reducidos de BookOS. Ver [`Ajustes`].
//!
//! ## Lo que todavía no hay
//!
//! - **Solo pantallas enteras**, no ventanas sueltas: compartir una ventana
//!   pide componerla en su propio buffer, con su propio seguimiento de daño.
//!   Por eso `AvailableSourceTypes` anuncia solo `MONITOR`.
//! - **El puntero siempre sale.** `escena()` lo pinta como un elemento más;
//!   quitarlo obligaría a componer la salida dos veces. `AvailableCursorModes`
//!   anuncia solo `EMBEDDED`, que es la verdad.
//! - La interacción de `ScreenCast.Start` publica un objeto `Request`: cerrar
//!   la petición retira inmediatamente el diálogo y el nodo en preparación.

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use smithay::reexports::calloop::channel::Sender;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const NOMBRE: &str = "org.bookos.portal.desktop";
const RUTA: &str = "/org/freedesktop/portal/desktop";

/// Códigos de respuesta del protocolo: 0 correcto, 1 cancelado por el usuario,
/// 2 fallo.
const CORRECTO: u32 = 0;
const CANCELADO: u32 = 1;
const FALLO: u32 = 2;

/// El buzón compartido entre quien espera y quien contesta.
struct Buzon<T> {
    valor: Option<T>,
    /// El emisario se fue sin dejar nada. No es un error: significa «no», y es
    /// lo que ocurre cuando el diálogo se cierra de cualquier otra forma.
    cerrado: bool,
    waker: Option<Waker>,
}

/// El extremo que contesta. Vive en el hilo del compositor.
///
/// Contesta una sola vez —[`Self::entregar`] lo consume— y si se suelta sin
/// contestar, quien esperaba recibe `None`. Eso es lo que hace que ninguna
/// espera pueda quedarse colgada sin necesidad de un plazo inventado: todos los
/// caminos o contestan o sueltan.
pub struct Emisario<T>(Arc<Mutex<Buzon<T>>>);

impl<T> Emisario<T> {
    pub fn entregar(self, valor: T) {
        // El bloque cierra el préstamo antes de que corra el `Drop` de `self`,
        // que vuelve a coger el mismo mutex.
        {
            let mut buzon = self
                .0
                .lock()
                .expect("el buzón no se comparte con nada que entre en pánico");
            buzon.valor = Some(valor);
            if let Some(waker) = buzon.waker.take() {
                waker.wake();
            }
        }
    }
}

impl<T> Drop for Emisario<T> {
    fn drop(&mut self) {
        let mut buzon = self
            .0
            .lock()
            .expect("el buzón no se comparte con nada que entre en pánico");
        buzon.cerrado = true;
        if let Some(waker) = buzon.waker.take() {
            waker.wake();
        }
    }
}

/// El extremo que espera. Se `.await`ea desde un método del portal.
pub struct Promesa<T>(Arc<Mutex<Buzon<T>>>);

impl<T> std::future::Future for Promesa<T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let mut buzon = self
            .0
            .lock()
            .expect("el buzón no se comparte con nada que entre en pánico");
        if let Some(valor) = buzon.valor.take() {
            return Poll::Ready(Some(valor));
        }
        if buzon.cerrado {
            return Poll::Ready(None);
        }
        buzon.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

pub fn promesa<T>() -> (Emisario<T>, Promesa<T>) {
    let buzon = Arc::new(Mutex::new(Buzon {
        valor: None,
        cerrado: false,
        waker: None,
    }));
    (Emisario(buzon.clone()), Promesa(buzon))
}

/// Lo que el hilo del portal le pide al bucle de frames.
pub enum Aviso {
    /// Enseña el diálogo de permiso y, si el usuario acepta, abre el nodo.
    Consentir {
        sesion: u32,
        ruta: OwnedObjectPath,
        app: String,
        /// El tamaño en píxeles del buffer de la salida elegida. Soltarlo sin
        /// contestar es la negativa.
        respuesta: Emisario<(u32, u32)>,
        /// Va tal cual al hilo de PipeWire, que contesta con el `node_id`.
        nodo: Emisario<u32>,
    },
    /// La aplicación cerró la sesión.
    Cerrar { sesion: u32 },
    /// El portal canceló la interacción antes de que `Start` terminara.
    Cancelar { sesion: u32 },
    /// Un socket de PipeWire ya conectado para dárselo al cliente.
    Descriptor { respuesta: Emisario<OwnedFd> },
    /// Una captura de la pantalla entera a un fichero.
    Captura { respuesta: Emisario<PathBuf> },
}

/// Lo que el hilo guarda entre llamadas.
struct Estado {
    canal: Sender<Aviso>,
    /// De la ruta de sesión del portal al identificador que conoce el hilo de
    /// PipeWire. Se comparte con las tarjetas de `Session`, que solo saben la
    /// suya.
    sesiones: Mutex<HashMap<OwnedObjectPath, u32>>,
    siguiente: AtomicU32,
}

impl Estado {
    fn avisar(&self, aviso: Aviso) -> bool {
        self.canal.send(aviso).is_ok()
    }
}

struct ScreenCast {
    estado: Arc<Estado>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    /// Solo pantallas enteras (`MONITOR`).
    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        1
    }

    /// Solo `EMBEDDED`: el cursor va pintado dentro del fotograma y no hay
    /// forma de sacarlo aparte sin componer la salida dos veces.
    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        2
    }

    /// En minúscula: así se llama la propiedad en el XML del portal, y zbus
    /// convertiría `version` en `Version` si no se le dice.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }

    async fn create_session(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] servidor: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let sesion = self.estado.siguiente.fetch_add(1, Ordering::Relaxed);
        let ruta: OwnedObjectPath = session_handle.to_owned().into();
        self.estado
            .sesiones
            .lock()
            .expect("el mutex de sesiones no se comparte con nada que entre en pánico")
            .insert(ruta.clone(), sesion);

        // La tarjeta `Session` vive en la ruta que manda el portal; es por
        // donde llega el `Close` cuando la aplicación termina la llamada.
        let tarjeta = Sesion {
            estado: self.estado.clone(),
            ruta: ruta.clone(),
        };
        if let Err(err) = servidor.at(ruta.as_str(), tarjeta).await {
            tracing::warn!("no se pudo publicar la sesión del portal: {err}");
            return (FALLO, HashMap::new());
        }
        tracing::info!(sesion, %ruta, "sesión de captura creada");
        (CORRECTO, HashMap::new())
    }

    /// No hay nada que elegir todavía: la pantalla se elige en el diálogo de
    /// `Start`, que es donde el usuario está mirando. Se acepta y ya.
    fn select_sources(
        &self,
        _handle: ObjectPath<'_>,
        _session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        (CORRECTO, HashMap::new())
    }

    async fn start(
        &self,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] servidor: &zbus::ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let ruta: OwnedObjectPath = session_handle.to_owned().into();
        let Some(sesion) = self
            .estado
            .sesiones
            .lock()
            .expect("el mutex de sesiones no se comparte con nada que entre en pánico")
            .get(&ruta)
            .copied()
        else {
            tracing::warn!(%ruta, "Start sobre una sesión que no existe");
            return (FALLO, HashMap::new());
        };

        let ruta_solicitud: OwnedObjectPath = handle.to_owned().into();
        let cancelada = Arc::new(AtomicBool::new(false));
        let solicitud = Solicitud {
            estado: self.estado.clone(),
            ruta: ruta_solicitud.clone(),
            sesion,
            cancelada: cancelada.clone(),
        };
        if let Err(err) = servidor.at(ruta_solicitud.as_str(), solicitud).await {
            tracing::warn!(%ruta_solicitud, "no se pudo publicar Request: {err}");
            return (FALLO, HashMap::new());
        }

        let (respuesta, espera) = promesa();
        let (nodo, espera_nodo) = promesa();
        let app = nombre_legible(app_id);
        if !self.estado.avisar(Aviso::Consentir {
            sesion,
            ruta,
            app,
            respuesta,
            nodo,
        }) {
            let _ = servidor
                .remove::<Solicitud, _>(ruta_solicitud.as_str())
                .await;
            return (FALLO, HashMap::new());
        }

        let respuesta_usuario = espera.await;
        let resultado = if cancelada.load(Ordering::Acquire) {
            (CANCELADO, HashMap::new())
        } else if let Some((ancho, alto)) = respuesta_usuario {
            match espera_nodo.await {
                Some(node_id) if !cancelada.load(Ordering::Acquire) => {
                    respuesta_start(sesion, node_id, ancho, alto)
                }
                Some(_) => {
                    self.estado.avisar(Aviso::Cerrar { sesion });
                    (CANCELADO, HashMap::new())
                }
                None => {
                    tracing::warn!(sesion, "PipeWire no dio identificador de nodo");
                    self.estado.avisar(Aviso::Cerrar { sesion });
                    (FALLO, HashMap::new())
                }
            }
        } else {
            (CANCELADO, HashMap::new())
        };

        if let Err(err) = servidor
            .remove::<Solicitud, _>(ruta_solicitud.as_str())
            .await
        {
            tracing::debug!(%ruta_solicitud, "Request ya retirado: {err}");
        }
        resultado
    }

    async fn open_pipe_wire_remote(
        &self,
        _session_handle: ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        let (respuesta, espera) = promesa();
        if !self.estado.avisar(Aviso::Descriptor { respuesta }) {
            return Err(zbus::fdo::Error::Failed(
                "el compositor se está cerrando".into(),
            ));
        }
        match espera.await {
            Some(fd) => Ok(fd.into()),
            None => Err(zbus::fdo::Error::Failed(
                "no se pudo abrir una conexión a PipeWire".into(),
            )),
        }
    }
}

fn respuesta_start(
    sesion: u32,
    node_id: u32,
    ancho: u32,
    alto: u32,
) -> (u32, HashMap<String, OwnedValue>) {
    // `streams` es `a(ua{sv})`: una tupla por flujo con su nodo y sus
    // propiedades. `source_type = 1` es MONITOR.
    let mut props: HashMap<String, Value<'_>> = HashMap::new();
    props.insert("size".into(), Value::from((ancho as i32, alto as i32)));
    props.insert("source_type".into(), Value::from(1u32));
    let flujos = vec![(node_id, props)];
    let mut resultados = HashMap::new();
    match OwnedValue::try_from(Value::from(flujos)) {
        Ok(v) => {
            resultados.insert("streams".to_string(), v);
        }
        Err(err) => {
            tracing::warn!("no se pudo empaquetar la lista de flujos: {err}");
            return (FALLO, HashMap::new());
        }
    }
    tracing::info!(sesion, node_id, ancho, alto, "compartiendo pantalla");
    (CORRECTO, resultados)
}

/// Objeto efímero que permite a xdg-desktop-portal cancelar el diálogo de
/// `Start` mientras su llamada asíncrona sigue esperando.
struct Solicitud {
    estado: Arc<Estado>,
    ruta: OwnedObjectPath,
    sesion: u32,
    cancelada: Arc<AtomicBool>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Solicitud {
    async fn close(&self, #[zbus(object_server)] servidor: &zbus::ObjectServer) {
        self.cancelada.store(true, Ordering::Release);
        self.estado.avisar(Aviso::Cancelar {
            sesion: self.sesion,
        });
        if let Err(err) = servidor.remove::<Solicitud, _>(self.ruta.as_str()).await {
            tracing::debug!(ruta = %self.ruta, "Request ya retirado: {err}");
        }
    }
}

/// La sesión que el portal cierra cuando la aplicación termina.
struct Sesion {
    estado: Arc<Estado>,
    ruta: OwnedObjectPath,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl Sesion {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    async fn close(&self, #[zbus(object_server)] servidor: &zbus::ObjectServer) {
        let sesion = self
            .estado
            .sesiones
            .lock()
            .expect("el mutex de sesiones no se comparte con nada que entre en pánico")
            .remove(&self.ruta);
        if let Some(sesion) = sesion {
            tracing::info!(sesion, "la aplicación cerró la sesión de captura");
            self.estado.avisar(Aviso::Cerrar { sesion });
        }
        // Se retira la tarjeta de su ruta: si no, una sesión nueva con la misma
        // ruta —el portal las reutiliza— chocaría con esta.
        if let Err(err) = servidor.remove::<Sesion, _>(self.ruta.as_str()).await {
            tracing::warn!(ruta = %self.ruta, "no se pudo retirar la sesión del portal: {err}");
        }
    }
}

/// El aspecto del sistema, para las aplicaciones que no son de BookOS.
///
/// Es lo que hace que una app **GTK o Tauri** siga el tema y el acento del
/// escritorio sin instalarle nada: GTK 4 y libadwaita ≥ 1.6 preguntan aquí al
/// arrancar y se quedan escuchando la señal, así que cambiar el acento en
/// Apariencia les llega en caliente. Tauri va sobre WebKitGTK, o sea que
/// también le llega —su CSS ve el `prefers-color-scheme` correcto—.
///
/// Es mejor que el tema GTK de `BookOS-GTK-Theme`, que se instala copiando
/// ficheros y solo trae dos acentos fijos: aquí van los diez y cambian sin
/// reiniciar la aplicación.
///
/// **Los valores no se guardan aquí.** El tema, el acento, el contraste y los efectos
/// reducidos son estáticos atómicos del proceso (`tema::actual()`,
/// `tema::acento()`, `tema::efectos_reducidos()`), así que este hilo los lee
/// directamente sin canal ni copia que se pueda quedar vieja.
struct Ajustes;

/// El espacio de nombres que miran GTK, Qt y quien siga la especificación.
const APARIENCIA: &str = "org.freedesktop.appearance";

impl Ajustes {
    /// `color-scheme`: 0 sin preferencia, 1 oscuro, 2 claro.
    fn color_scheme() -> u32 {
        if bookos_shell::tema::es_claro() { 2 } else { 1 }
    }

    /// `accent-color` es una tupla de tres dobles de 0 a 1, no un entero de 32
    /// bits: comprobado contra lo que sirve el portal de KDE en esta máquina.
    fn accent_color() -> (f64, f64, f64) {
        let c = bookos_shell::tema::acento();
        (c.r as f64, c.g as f64, c.b as f64)
    }

    /// `reduced-motion`: 0 no, 1 sí. Sale de los efectos reducidos de BookOS,
    /// así que apagarlos en el escritorio también calma las animaciones de las
    /// aplicaciones que lo respetan.
    fn reduced_motion() -> u32 {
        u32::from(bookos_shell::tema::efectos_reducidos())
    }

    fn valores() -> HashMap<String, OwnedValue> {
        let mut m = HashMap::new();
        let mete = |m: &mut HashMap<String, OwnedValue>, k: &str, v: Value<'_>| {
            match OwnedValue::try_from(v) {
                Ok(v) => {
                    m.insert(k.to_string(), v);
                }
                Err(err) => tracing::warn!(k, "no se pudo empaquetar el ajuste: {err}"),
            }
        };
        mete(&mut m, "color-scheme", Value::from(Self::color_scheme()));
        mete(&mut m, "accent-color", Value::from(Self::accent_color()));
        mete(
            &mut m,
            "reduced-motion",
            Value::from(Self::reduced_motion()),
        );
        mete(
            &mut m,
            "contrast",
            Value::from(u32::from(bookos_shell::tema::alto_contraste())),
        );
        m
    }

    fn uno(clave: &str) -> Option<OwnedValue> {
        let v = match clave {
            "color-scheme" => Value::from(Self::color_scheme()),
            "accent-color" => Value::from(Self::accent_color()),
            "reduced-motion" => Value::from(Self::reduced_motion()),
            "contrast" => Value::from(u32::from(bookos_shell::tema::alto_contraste())),
            _ => return None,
        };
        OwnedValue::try_from(v).ok()
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Settings")]
impl Ajustes {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        // La lista admite comodines por prefijo y la vacía significa «todo».
        let quiere = namespaces.is_empty()
            || namespaces
                .iter()
                .any(|n| n.is_empty() || APARIENCIA.starts_with(n.trim_end_matches('*')));
        let mut fuera = HashMap::new();
        if quiere {
            fuera.insert(APARIENCIA.to_string(), Self::valores());
        }
        fuera
    }

    /// `ReadOne` es la de la versión 2. `Read` hace lo mismo y sigue viva
    /// porque las aplicaciones antiguas solo conocen esa.
    fn read_one(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        if namespace != APARIENCIA {
            return Err(zbus::fdo::Error::UnknownProperty(format!(
                "BookOS no sirve el espacio «{namespace}»"
            )));
        }
        Self::uno(key)
            .ok_or_else(|| zbus::fdo::Error::UnknownProperty(format!("no hay ajuste «{key}»")))
    }

    fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        self.read_one(namespace, key)
    }

    #[zbus(signal)]
    async fn setting_changed(
        emisor: &zbus::object_server::SignalEmitter<'_>,
        namespace: &str,
        key: &str,
        value: Value<'_>,
    ) -> zbus::Result<()>;
}

/// Avisa a las aplicaciones de que el aspecto ha cambiado.
///
/// Se llama desde el bucle de frames al aplicar apariencia o efectos. Sin la
/// señal, una app GTK ya abierta se queda con el tema con el que arrancó.
pub fn apariencia_cambiada(conexion: Option<&zbus::blocking::Connection>) {
    let Some(conexion) = conexion else {
        return;
    };
    for (clave, valor) in [
        ("color-scheme", Value::from(Ajustes::color_scheme())),
        ("accent-color", Value::from(Ajustes::accent_color())),
        ("reduced-motion", Value::from(Ajustes::reduced_motion())),
        (
            "contrast",
            Value::from(u32::from(bookos_shell::tema::alto_contraste())),
        ),
    ] {
        if let Err(err) = conexion.emit_signal(
            None::<&str>,
            RUTA,
            "org.freedesktop.impl.portal.Settings",
            "SettingChanged",
            &(APARIENCIA, clave, valor),
        ) {
            tracing::warn!(clave, "no se pudo avisar del cambio de aspecto: {err}");
        }
    }
}

struct Screenshot {
    estado: Arc<Estado>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    /// La 1 y no la 2: la 2 añade `PickColor`, que aquí no existe. Anunciar una
    /// versión que no se cumple es peor que anunciar la que sí.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    async fn screenshot(
        &self,
        _handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let (respuesta, espera) = promesa();
        if !self.estado.avisar(Aviso::Captura { respuesta }) {
            return (FALLO, HashMap::new());
        }
        let Some(ruta) = espera.await else {
            return (FALLO, HashMap::new());
        };
        let mut resultados = HashMap::new();
        let uri = format!("file://{}", ruta.to_string_lossy());
        match OwnedValue::try_from(Value::from(uri)) {
            Ok(v) => {
                resultados.insert("uri".to_string(), v);
            }
            Err(err) => {
                tracing::warn!("no se pudo empaquetar la ruta de la captura: {err}");
                return (FALLO, HashMap::new());
            }
        }
        (CORRECTO, resultados)
    }
}

/// El nombre que se le enseña al usuario en el diálogo.
///
/// El `app_id` que llega es el del `.desktop` —`org.mozilla.firefox`— o la
/// cadena vacía si la aplicación no está en un contenedor y el portal no pudo
/// averiguarlo. Sin nombre, «Una aplicación»: es más honesto que enseñar un
/// hueco.
fn nombre_legible(app_id: &str) -> String {
    if app_id.is_empty() {
        return "Una aplicación".into();
    }
    let corto = app_id.rsplit('.').next().unwrap_or(app_id);
    let mut letras = corto.chars();
    match letras.next() {
        Some(primera) => primera.to_uppercase().collect::<String>() + letras.as_str(),
        None => "Una aplicación".into(),
    }
}

/// Devuelve la conexión, que hay que **guardar**: al soltarla se cierra el bus
/// y el nombre se pierde.
pub fn arrancar(canal: Sender<Aviso>) -> Option<zbus::blocking::Connection> {
    let estado = Arc::new(Estado {
        canal,
        sesiones: Mutex::new(HashMap::new()),
        siguiente: AtomicU32::new(1),
    });
    let conexion = zbus::blocking::connection::Builder::session()
        .and_then(|b| {
            b.serve_at(
                RUTA,
                ScreenCast {
                    estado: estado.clone(),
                },
            )
        })
        .and_then(|b| {
            b.serve_at(
                RUTA,
                Screenshot {
                    estado: estado.clone(),
                },
            )
        })
        .and_then(|b| b.serve_at(RUTA, Ajustes))
        // El nombre se pide **al final**: hasta que las dos interfaces están
        // publicadas, un `xdg-desktop-portal` que ya estuviera esperando podría
        // preguntar por una que todavía no existe.
        .and_then(|b| b.name(NOMBRE))
        .and_then(|b| b.build());
    match conexion {
        Ok(conexion) => {
            tracing::info!("portal de escritorio de BookOS en el bus");
            Some(conexion)
        }
        Err(err) => {
            tracing::warn!("sin portal de escritorio: {err}");
            None
        }
    }
}

/// Lo que llega del hilo del portal, ya en el hilo del compositor.
pub fn recibir(state: &mut crate::state::BookosComp, aviso: Aviso) {
    match aviso {
        Aviso::Consentir {
            sesion,
            ruta,
            app,
            respuesta,
            nodo,
        } => {
            consentir(state, sesion, ruta, app, respuesta, nodo);
        }
        Aviso::Cerrar { sesion } => {
            state.emisiones.quitar(sesion);
        }
        Aviso::Cancelar { sesion } => cancelar(state, sesion),
        Aviso::Descriptor { respuesta } => {
            // Si el hilo de PipeWire no arranca, el portal se queda sin
            // respuesta y vence su plazo: mejor eso que contestarle con un
            // descriptor inventado.
            if let Some(emisor) = state.emisiones.hilo() {
                emisor.enviar(crate::pw::Orden::Descriptor { respuesta });
            }
        }
        Aviso::Captura { respuesta } => capturar(state, respuesta),
    }
}

fn cancelar(state: &mut crate::state::BookosComp, sesion: u32) {
    if state
        .consentimiento
        .as_ref()
        .is_some_and(|consentimiento| consentimiento.sesion == sesion)
    {
        // Soltar los emisarios despierta `Start` con cancelación.
        state.consentimiento = None;
        if let Some(shell) = state.shell.as_mut() {
            shell.cerrar_emergente();
        }
        state.needs_redraw = true;
    }
    state.emisiones.quitar(sesion);
}

fn consentir(
    state: &mut crate::state::BookosComp,
    sesion: u32,
    ruta: OwnedObjectPath,
    app: String,
    respuesta: Emisario<(u32, u32)>,
    nodo: Emisario<u32>,
) {
    // Un diálogo cada vez. Dos tarjetas de permiso superpuestas no se pueden
    // contestar por separado, y la segunda taparía a la primera.
    if state.consentimiento.is_some() {
        return;
    }
    let salidas: Vec<smithay::output::Output> = state.space.outputs().cloned().collect();
    if salidas.is_empty() {
        return;
    }
    let pantallas: Vec<bookos_shell::PantallaCompartible> = salidas
        .iter()
        .map(|output| {
            let tamano = crate::captura::tamano_de(output);
            bookos_shell::PantallaCompartible {
                nombre: output.name(),
                ancho: tamano.w.max(0) as u32,
                alto: tamano.h.max(0) as u32,
            }
        })
        .collect();
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    shell.abrir_compartir(sesion, app, pantallas);
    state.consentimiento = Some(crate::state::Consentimiento {
        sesion,
        ruta,
        respuesta,
        nodo,
        salidas,
    });
    state.needs_redraw = true;
}

/// La respuesta del usuario a la tarjeta.
pub fn responder(state: &mut crate::state::BookosComp, sesion: u32, pantalla: Option<usize>) {
    let Some(consentimiento) = state.consentimiento.take() else {
        return;
    };
    // Una tarjeta que ya no es la que está abierta: se deja como estaba.
    if consentimiento.sesion != sesion {
        state.consentimiento = Some(consentimiento);
        return;
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.cerrar_emergente();
    }
    state.needs_redraw = true;

    let elegida = pantalla.and_then(|i| consentimiento.salidas.get(i).cloned());
    let Some(output) = elegida else {
        // Los dos emisarios se sueltan aquí sin contestar, que es la negativa.
        return;
    };

    let tamano = crate::captura::tamano_de(&output);
    let (ancho, alto) = (tamano.w.max(0) as u32, tamano.h.max(0) as u32);
    // El tope de fotogramas es el de la pantalla, con 60 como techo: por encima
    // de eso lo que se gana es tráfico, no fluidez, y el camino todavía pasa
    // por la CPU. `refresh` viene en milihercios.
    let fps = output
        .current_mode()
        .map(|modo| (modo.refresh as u32).div_ceil(1000))
        .unwrap_or(60)
        .clamp(1, 60);
    let periodo = std::time::Duration::from_micros(1_000_000 / fps as u64);

    let Some(emisor) = state.emisiones.hilo() else {
        return;
    };
    emisor.enviar(crate::pw::Orden::Abrir {
        sesion,
        ancho,
        alto,
        fps,
        respuesta: consentimiento.nodo,
    });
    state.emisiones.anadir(crate::emision::Emision {
        sesion,
        ruta: consentimiento.ruta,
        output,
        tamano,
        periodo,
        // Atrasado un periodo para que el primer fotograma salga en el
        // siguiente dibujo y no haya un hueco negro al empezar la llamada.
        ultimo: std::time::Instant::now() - periodo,
    });
    consentimiento.respuesta.entregar((ancho, alto));
}

/// Avisa al portal cuando el compositor tiene que cortar una sesión por su
/// cuenta, por ejemplo porque el modo de la salida ya no coincide con el
/// formato negociado con PipeWire.
pub fn sesion_cerrada(conexion: Option<&zbus::blocking::Connection>, ruta: &OwnedObjectPath) {
    let Some(conexion) = conexion else { return };
    if let Err(err) = conexion.emit_signal(
        None::<&str>,
        ruta.as_str(),
        "org.freedesktop.impl.portal.Session",
        "Closed",
        &(),
    ) {
        tracing::warn!(%ruta, "no se pudo emitir Session.Closed: {err}");
    }
}

/// La red de seguridad: si la tarjeta ya no está pero nadie contestó, se
/// contesta que no.
///
/// Pasa con un clic fuera de la tarjeta, que la cierra sin pasar por ninguna
/// acción, y pasaría también si el shell se cayera con la tarjeta abierta. El
/// hilo del portal está bloqueado esperando y dejarlo hasta que venza el plazo
/// se ve, desde la aplicación, como que el escritorio se ha colgado.
pub fn denegar_si_se_cerro(state: &mut crate::state::BookosComp) {
    if state.consentimiento.is_none() {
        return;
    }
    let abierta = state
        .shell
        .as_ref()
        .and_then(|shell| shell.emergente_nombre())
        == Some("compartir");
    if abierta {
        return;
    }
    // Soltarlo sin contestar es la negativa.
    state.consentimiento = None;
}

/// La captura de pantalla que pide el portal: la principal entera, a fichero.
fn capturar(state: &mut crate::state::BookosComp, respuesta: Emisario<PathBuf>) {
    // La principal tiene el origen en (0,0); es la única que sabe capturar
    // `servir_captura_propia`, y sus coordenadas son las de esa pantalla.
    let principal = state
        .space
        .outputs()
        .find(|o| o.current_location() == (0, 0).into())
        .cloned();
    let Some(principal) = principal else {
        return;
    };
    let Some(geometria) = state.space.output_geometry(&principal) else {
        return;
    };
    state.captura_portal = Some(respuesta);
    state.captura_pedida = Some(crate::state::CapturaPedida {
        x: 0,
        y: 0,
        ancho: geometria.size.w,
        alto: geometria.size.h,
        guardar: true,
    });
    state.needs_redraw = true;
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Lo que se le sirve a GTK es lo que el escritorio está pintando.
    ///
    /// Los números son los del propio tema, no una copia: si alguien cambia el
    /// azul de BookOS, esto lo sigue. Lo que se fija aquí es la **forma**, que
    /// es lo que la especificación obliga y lo que se comprobó contra el portal
    /// de KDE de esta máquina: `color-scheme` entero con 1 oscuro y 2 claro, y
    /// `accent-color` tres dobles de 0 a 1.
    #[test]
    fn el_aspecto_que_se_sirve_es_el_del_escritorio() {
        use bookos_shell::tema::{self, Acento, Tema};

        let antes = (tema::actual(), tema::acento_actual());

        tema::aplicar(Tema::Oscuro);
        tema::aplicar_acento(Acento::Verde);
        assert_eq!(Ajustes::color_scheme(), 1, "oscuro es 1");
        let c = tema::acento();
        assert_eq!(
            Ajustes::accent_color(),
            (c.r as f64, c.g as f64, c.b as f64),
            "el acento servido no es el del tema"
        );

        tema::aplicar(Tema::Claro);
        assert_eq!(Ajustes::color_scheme(), 2, "claro es 2");
        assert_ne!(
            Ajustes::accent_color(),
            (c.r as f64, c.g as f64, c.b as f64),
            "el verde claro y el oscuro son colores distintos"
        );

        // Las cuatro claves están y `ReadOne` da lo mismo que `ReadAll`.
        let todo = Ajustes::valores();
        for clave in ["color-scheme", "accent-color", "reduced-motion", "contrast"] {
            let uno = Ajustes::uno(clave).unwrap_or_else(|| panic!("falta «{clave}»"));
            assert_eq!(
                todo.get(clave),
                Some(&uno),
                "«{clave}» no cuadra entre las dos"
            );
        }
        assert!(
            Ajustes::uno("lo-que-sea").is_none(),
            "una clave que no existe no puede dar valor"
        );

        tema::aplicar(antes.0);
        tema::aplicar_acento(antes.1);
    }

    #[test]
    fn el_nombre_de_la_aplicacion_sale_legible() {
        assert_eq!(nombre_legible("org.mozilla.firefox"), "Firefox");
        assert_eq!(nombre_legible("chromium"), "Chromium");
        assert_eq!(nombre_legible(""), "Una aplicación");
    }
}
