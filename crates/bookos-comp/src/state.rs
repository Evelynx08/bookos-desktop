//! Estado del compositor y su bucle de eventos.
//!
//! Dos decisiones de este fichero condicionan todo el consumo del escritorio:
//!
//! 1. **El bucle es calloop, no un bucle activo.** El ejemplo `minimal` de
//!    Smithay gira sin parar llamando a `dispatch_new_events`; eso mantiene un
//!    core despierto el 100% del tiempo. Aquí el proceso duerme en `epoll` y
//!    solo despierta cuando hay algo real que atender.
//! 2. **Redibujar es opt-in.** `needs_redraw` solo se marca cuando un cliente
//!    hace commit o cambia el layout. Sin clientes pidiendo frame callback, no
//!    se dibuja nada y la GPU no se toca.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::desktop::{Space, Window};
use smithay::input::pointer::PointerHandle;
use smithay::input::{Seat, SeatState};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use smithay::wayland::compositor::{CompositorClientState, CompositorState};
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shm::ShmState;

/// Lo que se está escribiendo en la pantalla de bloqueo.
///
/// Vive en el compositor y **no** en el shell: el shell corre dentro de un
/// `catch_unwind` y su estado se descarta al primer panic, que es el último
/// sitio donde debe estar una contraseña. Aquí, además, es donde se puede
/// borrar de verdad al terminar.
#[derive(Default)]
pub struct Bloqueo {
    pub escrito: String,
    pub comprobando: Option<crate::autenticar::Comprobacion>,
    /// El último intento falló y aún no se ha vuelto a escribir.
    pub fallo: bool,
}

pub struct BookosComp {
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, BookosComp>,
    pub loop_signal: LoopSignal,
    pub start_time: Instant,
    pub socket_name: OsString,

    /// Marca de trabajo pendiente para el backend. El backend la consulta en
    /// su punto de dibujo y la limpia; nadie dibuja "por si acaso".
    pub needs_redraw: bool,
    /// Lo que se lleva escrito en la pantalla de bloqueo.
    pub bloqueo: Bloqueo,
    /// Cuándo llegó el último evento de entrada. Decide si el backend anidado
    /// sondea rápido o se relaja.
    pub last_input: Option<Instant>,
    pub frames: FrameStats,

    pub space: Space<Window>,
    pub seat: Seat<Self>,
    pub pointer: PointerHandle<Self>,
    /// Qué cursor toca pintar: el del tema, o una superficie del cliente.
    pub cursor_status: smithay::input::pointer::CursorImageStatus,
    pub cursor_theme: Option<crate::cursor::CursorTheme>,
    /// El fondo del escritorio, ya subido a textura. `None` si no se encontró
    /// ninguno: entonces se ve el color liso de siempre.
    pub fondo: Option<crate::fondo::Fondo>,
    /// El shader del fondo esmerilado y su textura de trabajo. `None` si el
    /// driver no compiló el shader: el escritorio sigue, con el panel opaco.
    pub cristal: Option<std::rc::Rc<std::cell::RefCell<crate::desenfoque::Cristal>>>,
    /// El contador de cambios del cristal. Sube en cada frame porque lo que
    /// desenfoca —el escritorio de debajo— puede haber cambiado.
    pub cristal_commit: smithay::backend::renderer::utils::CommitCounter,
    /// Posición del cursor en coordenadas lógicas del compositor.
    pub pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    /// El shell in-process. `None` hasta que el backend sabe el tamaño de la
    /// pantalla.
    pub shell: Option<crate::shell::ShellHost>,
    /// La configuración del escritorio, leída una vez al arrancar.
    ///
    /// El backend la consulta para la escala **antes** de que exista el shell
    /// —hay que saberla para crear la salida— y luego se la entrega a él, que
    /// es quien la consume. Es `Option` justamente por ese traspaso.
    pub config: Option<bookos_shell::Config>,
    /// La escala que pide la configuración, si pide alguna.
    ///
    /// Es copia de lo que hay en `config`, y existe porque esa se la lleva el
    /// shell al construirse: el backend sigue necesitando el dato después, cada
    /// vez que la pantalla cambia de tamaño.
    pub escala_forzada: Option<f64>,
    /// Lo que hay que pedirle a libinput sobre cada dispositivo que aparezca.
    pub entrada: bookos_shell::Entrada,
    /// Si el touchpad está encendido. Se guarda aquí y no en el dispositivo
    /// porque tras un cambio de TTY libinput los vuelve a crear de cero: sin
    /// esto, el touchpad que habías apagado volvería encendido.
    pub touchpad_activo: bool,
    /// Cómo encender y apagar los touchpads que ya están abiertos. Lo rellena
    /// el backend de sesión real; anidado se queda en `None` porque ahí la
    /// entrada la da el compositor de debajo.
    pub aplicar_touchpad: Option<Box<dyn Fn(bool)>>,
    /// Ruta del fondo que pide la configuración, copiada por lo mismo que la
    /// escala: la `Config` se la lleva el shell al construirse.
    pub fondo_config: Option<String>,
    /// Tamaño lógico del cursor. Copia de la configuración por lo mismo que
    /// `escala_forzada`: el tema se vuelve a cargar cada vez que cambia la
    /// escala, y para entonces la `Config` ya se la llevó el shell.
    pub cursor_nominal: u32,
    /// Cuándo y dónde fue la última pulsación del botón izquierdo, para poder
    /// reconocer el doble clic. Se guarda el punto además del instante porque
    /// dos clics seguidos en sitios distintos son dos clics, no uno doble.
    pub ultimo_clic: Option<(Instant, smithay::utils::Point<f64, smithay::utils::Logical>)>,
    /// La ventana que el usuario está moviendo o redimensionando ahora mismo.
    /// Mientras dura, los eventos del puntero son del compositor y no llegan al
    /// cliente: si no, la terminal que arrastras cree que estás seleccionando
    /// texto.
    pub arrastre: Option<crate::ventanas::Arrastre>,

    /// Cómo saltar a otro terminal virtual. Lo rellena el backend de sesión
    /// real; anidado se queda en `None` porque el TTY no es nuestro.
    ///
    /// Es un cierre y no la `LibSeatSession` para que este fichero no dependa
    /// de libseat: con la característica `udev` apagada el tipo ni existe.
    pub cambiar_vt: Option<Box<dyn Fn(i32)>>,
    /// Programas lanzados desde la sesión. Se guardan solo para recogerlos y
    /// que no queden zombis colgando del compositor.
    pub hijos: Vec<std::process::Child>,

    /// El gestor de ventanas X11, cuando XWayland ya está en pie. `None`
    /// mientras arranca, y para siempre si `Xwayland` no está instalado.
    pub xwm: Option<smithay::xwayland::X11Wm>,
    /// El número de pantalla de XWayland, para el `DISPLAY` de los hijos.
    pub display_x11: Option<u32>,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    /// Nadie lo lee, pero tiene que seguir vivo: al soltarse se retiran los
    /// globales `wl_output` y `xdg_output_manager` y los clientes dejan de ver
    /// las pantallas.
    #[allow(dead_code)]
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    /// `wp_fractional_scale` + `wp_viewporter`: la pareja que permite a un
    /// cliente dibujar a 1,75× y decirle al compositor "recórtame a este
    /// tamaño lógico". Sin viewporter, la escala fraccional no sirve de nada.
    #[allow(dead_code)]
    pub fractional_scale_state: FractionalScaleManagerState,
    #[allow(dead_code)]
    pub viewporter_state: ViewporterState,
    /// `wp_cursor_shape_v1`: el cliente dice **qué forma** quiere —"texto",
    /// "mano", "redimensionar"— y la dibuja el compositor con su tema, en vez
    /// de mandar él una superficie con la imagen ya pintada.
    ///
    /// Sin esto, cada toolkit elige el tema por su cuenta: medido, konsole
    /// dentro de esta sesión mandaba `Surface` con el cursor de Breeze que le
    /// da la integración de KDE, y el escritorio enseñaba dos cursores
    /// distintos según sobre qué estuviera el puntero. Con el protocolo pasa a
    /// pedir `Named(Text)` y lo dibujamos nosotros.
    #[allow(dead_code)]
    pub cursor_shape_state: smithay::wayland::cursor_shape::CursorShapeManagerState,
    /// El globales `xwayland_shell_v1`, por el que XWayland asocia cada ventana
    /// X11 con su `wl_surface`. Sin él las ventanas X11 nunca se emparejan con
    /// un buffer y se ven como huecos negros.
    pub xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState,
}

/// Qué distribución de teclado usar.
///
/// `XkbConfig::default()` es **us**, y eso es lo que salía: un teclado español
/// escribiendo en inglés. La configuración de teclado de esta máquina está
/// dicha en tres sitios a la vez —`XKB_DEFAULT_LAYOUT`, `localectl` y
/// `/etc/vconsole.conf`— y ninguno lo lee xkb por su cuenta.
///
/// Se mira, por orden: lo que diga `panel.conf`, la variable de entorno —que
/// es el estándar de facto y la que ponen las sesiones de Wayland— y el
/// `KEYMAP` de vconsole, que es lo que queda en un TTY pelado.
///
/// El `Box::leak` es deliberado: `XkbConfig` presta `&str` y esto se lee una
/// vez al arrancar y vive lo que dure la sesión. Un `String` en el estado
/// obligaría a arrastrar el préstamo por medio compositor para ahorrar treinta
/// bytes.
fn distribucion_teclado(config: Option<&str>) -> smithay::input::keyboard::XkbConfig<'static> {
    let layout = config
        .map(str::to_string)
        .or_else(|| std::env::var("XKB_DEFAULT_LAYOUT").ok())
        .or_else(keymap_de_vconsole)
        .filter(|s| !s.trim().is_empty());
    let variant = std::env::var("XKB_DEFAULT_VARIANT").ok();
    let options = std::env::var("XKB_DEFAULT_OPTIONS").ok();
    tracing::info!(?layout, ?variant, "distribución de teclado");
    smithay::input::keyboard::XkbConfig {
        layout: layout.map(|s| &*Box::leak(s.into_boxed_str())).unwrap_or(""),
        variant: variant.map(|s| &*Box::leak(s.into_boxed_str())).unwrap_or(""),
        options: options.map(|s| Box::leak(s.into_boxed_str()).to_string()),
        ..Default::default()
    }
}

/// `KEYMAP="es"` de `/etc/vconsole.conf`, que es lo que queda cuando no hay
/// sesión gráfica que ponga las variables.
fn keymap_de_vconsole() -> Option<String> {
    let texto = std::fs::read_to_string("/etc/vconsole.conf").ok()?;
    texto.lines().find_map(|l| {
        let valor = l.trim().strip_prefix("KEYMAP=")?;
        Some(valor.trim_matches('"').to_string())
    })
}

impl BookosComp {
    /// ¿Hay alguna animación en marcha?
    ///
    /// El backend lo usa para no relajarse mientras algo se mueve. Sin esto, el
    /// ritmo se decide **después** de dibujar, cuando `needs_redraw` ya se ha
    /// limpiado, y una animación de 180 ms se dibujaba a 33 ms por fotograma:
    /// medido, cuatro fotogramas para abrir el launchpad en vez de veintidós.
    pub fn hay_animacion(&self) -> bool {
        // El aviso solo cuenta como animación **mientras se está yendo**: los
        // 1,2 s que está quieto no cambian un píxel, y repintarlos sería
        // despertar ciento cuarenta veces para enseñar lo mismo.
        let osd_saliendo = self
            .shell
            .as_ref()
            .and_then(|s| s.osd_queda())
            .is_some_and(|q| q <= bookos_shell::osd::SALIDA);
        osd_saliendo
            || self
                .shell
                .as_ref()
                .is_some_and(|s| s.animando() || s.barras_animando())
            || self.space.elements().any(crate::ventanas::animando)
            || self.arrastre.as_ref().is_some_and(|a| a.animando())
    }

    pub fn new(
        event_loop: &mut EventLoop<'static, BookosComp>,
        display: Display<BookosComp>,
    ) -> anyhow::Result<Self> {
        let dh = display.handle();
        let loop_handle = event_loop.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let fractional_scale_state = FractionalScaleManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let cursor_shape_state =
            smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Self>(&dh);
        let xwayland_shell_state =
            smithay::wayland::xwayland_shell::XWaylandShellState::new::<Self>(&dh);

        // La configuración se lee antes que el teclado: la distribución sale de
        // ahí si el fichero la dice.
        let mut config = bookos_shell::Config::cargar();
        let teclado_config = config.teclado.clone();

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "bookos");
        // Repetición de teclas: 25/s tras 600 ms, los valores por defecto de KDE.
        seat.add_keyboard(distribucion_teclado(teclado_config.as_deref()), 600, 25)?;
        let pointer = seat.add_pointer();

        let socket_name = Self::init_socket(&loop_handle, display)?;

        let escala_forzada = config.escala;
        // Se saca de la `Config` antes de que el shell se la lleve entera: los
        // dispositivos no se configuran al arrancar sino cada vez que libinput
        // anuncia uno, y eso vuelve a pasar tras cada cambio de TTY.
        let entrada = std::mem::take(&mut config.entrada);
        let cursor_nominal = config.cursor;
        let fondo_config = config.fondo.clone();

        Ok(Self {
            display_handle: dh,
            loop_handle,
            loop_signal: event_loop.get_signal(),
            start_time: Instant::now(),
            socket_name,
            needs_redraw: true,
            bloqueo: Bloqueo::default(),
            last_input: None,
            frames: FrameStats::default(),
            space: Space::default(),
            seat,
            pointer,
            cursor_status: smithay::input::pointer::CursorImageStatus::default_named(),
            cursor_theme: None,
            fondo: None,
            cristal: None,
            cristal_commit: Default::default(),
            pointer_location: (0.0, 0.0).into(),
            shell: None,
            config: Some(config),
            escala_forzada,
            entrada,
            touchpad_activo: true,
            aplicar_touchpad: None,
            cursor_nominal,
            fondo_config,
            arrastre: None,
            ultimo_clic: None,
            cambiar_vt: None,
            hijos: Vec::new(),
            xwm: None,
            display_x11: None,
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            fractional_scale_state,
            viewporter_state,
            cursor_shape_state,
            xwayland_shell_state,
        })
    }

    /// Publica el socket Wayland y engancha tanto las conexiones nuevas como el
    /// tráfico de las existentes al bucle de eventos.
    fn init_socket(
        loop_handle: &LoopHandle<'static, BookosComp>,
        display: Display<BookosComp>,
    ) -> anyhow::Result<OsString> {
        use smithay::reexports::wayland_server::ListeningSocket;

        let listener = ListeningSocket::bind_auto("wayland", 1..32)?;
        let socket_name = listener
            .socket_name()
            .ok_or_else(|| anyhow::anyhow!("el socket no tiene nombre"))?
            .to_os_string();

        // ListeningSocket no es una fuente de calloop por sí misma; se vigila su
        // fd y se acepta a mano.
        let mut dh = display.handle();
        loop_handle
            .insert_source(
                Generic::new(listener, Interest::READ, Mode::Level),
                move |_, listener, _state| {
                    while let Some(stream) = listener.accept()? {
                        if let Err(err) = dh.insert_client(stream, Arc::new(ClientState::default()))
                        {
                            tracing::warn!("no se pudo aceptar el cliente: {err}");
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|err| anyhow::anyhow!("insert_source(listener): {err}"))?;

        // Tráfico de los clientes ya conectados. El fd del display se vigila con
        // epoll: el proceso duerme mientras nadie hable.
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // SAFETY: calloop da acceso exclusivo al Display en el callback.
                    let r = unsafe { display.get_mut().dispatch_clients(state) };
                    // El error **no** se propaga a propósito. Un `?` aquí sale
                    // del callback, calloop lo trata como fallo del bucle y
                    // `run` termina: la sesión entera se cae porque un solo
                    // cliente mandó algo que Smithay no supo despachar. El
                    // cliente ofensor ya queda desconectado por su cuenta; el
                    // compositor tiene que seguir de pie.
                    if let Err(err) = r {
                        tracing::warn!("fallo despachando a un cliente: {err}");
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|err| anyhow::anyhow!("insert_source(display): {err}"))?;

        Ok(socket_name)
    }

    /// Qué superficie hay bajo un punto, y en qué posición está dibujada.
    ///
    /// Devuelve la coordenada *relativa a la superficie*, que es lo que espera
    /// el puntero de Smithay: el cliente razona en su propio sistema, no en el
    /// del escritorio.
    pub fn surface_under(
        &self,
        point: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        smithay::utils::Point<f64, smithay::utils::Logical>,
    )> {
        use smithay::desktop::WindowSurfaceType;
        // El panel y el dock se dibujan encima de las ventanas: mientras el
        // puntero esté sobre ellos, para los clientes no hay nada bajo el
        // cursor. Sin esto la ventana de debajo se cree señalada y cambia la
        // forma del cursor al pasar por encima del dock.
        if self.shell.as_ref().is_some_and(|s| s.contiene(point.x, point.y)) {
            return None;
        }
        let (window, location) = self.space.element_under(point)?;
        window
            .surface_under(point - location.to_f64(), WindowSurfaceType::ALL)
            .map(|(surface, offset)| (surface, (location + offset).to_f64()))
    }

    /// Todo lo que hay que hacer justo antes de volver a dormir en `epoll`.
    pub fn post_dispatch(&mut self) {
        // Una emergente entrando pide fotogramas aunque no pase nada más. Es la
        // única vez que este compositor dibuja sin que haya ocurrido un evento,
        // y dura lo que dura la animación: 280 ms.
        if self
            .shell
            .as_ref()
            .is_some_and(|s| s.animando() || s.barras_animando())
        {
            self.needs_redraw = true;
        } else if let Some(shell) = self.shell.as_mut() {
            // Ya no se mueve nada: se sueltan los buffers de lo que se cerró.
            shell.fin_animacion();
        }
        // Y lo mismo con las ventanas: una que está apareciendo o llegando a su
        // sitio pide fotogramas aunque el cliente no haga commit. `any` corta en
        // cuanto encuentra una, y con el escritorio quieto es un recorrido de
        // punteros sobre una lista de tres elementos.
        if self.space.elements().any(crate::ventanas::animando)
            || self.arrastre.as_ref().is_some_and(|a| a.animando())
        {
            self.needs_redraw = true;
        }
        self.frames.maybe_report();
        self.space.refresh();
        self.display_handle.flush_clients().ok();
    }
}

/// Contadores del bucle de dibujo.
///
/// Existen para poder demostrar —y no solo afirmar— que el compositor está
/// quieto cuando la pantalla está quieta. `submitted` es lo que llega a la
/// pantalla; `skipped` son las veces que se evaluó el damage y no había nada
/// que cambiar. En un escritorio en reposo, `submitted` debe tender a cero.
#[derive(Default)]
pub struct FrameStats {
    pub submitted: u64,
    pub skipped: u64,
    /// Cuánto se ha tardado dibujando, en total y en el peor caso.
    ///
    /// La media sola engaña: un escritorio que dibuja a 2 ms de media pero se
    /// va a 40 en un fotograma de cada diez se ve peor que uno constante a 8, y
    /// es exactamente lo que se percibe como que "va a tirones".
    tiempo: Duration,
    peor: Duration,
    last_report: Option<Instant>,
}

impl FrameStats {
    /// Vuelca un resumen como mucho una vez cada 5 s, y solo si hubo actividad.
    /// Apunta lo que ha costado un fotograma.
    pub fn dibujado(&mut self, cuanto: Duration) {
        self.tiempo += cuanto;
        self.peor = self.peor.max(cuanto);
    }

    pub fn maybe_report(&mut self) {
        const EVERY: Duration = Duration::from_secs(5);
        let now = Instant::now();
        let last = *self.last_report.get_or_insert(now);
        if now.duration_since(last) < EVERY {
            return;
        }
        let secs = now.duration_since(last).as_secs_f64();
        if self.submitted > 0 || self.skipped > 0 {
            let n = (self.submitted + self.skipped).max(1) as f64;
            tracing::info!(
                fps_reales = format_args!("{:.1}", self.submitted as f64 / secs),
                saltados = format_args!("{:.1}/s", self.skipped as f64 / secs),
                media_ms = format_args!("{:.2}", self.tiempo.as_secs_f64() * 1000.0 / n),
                peor_ms = format_args!("{:.2}", self.peor.as_secs_f64() * 1000.0),
                "ritmo de dibujo"
            );
        }
        self.submitted = 0;
        self.skipped = 0;
        self.tiempo = Duration::ZERO;
        self.peor = Duration::ZERO;
        self.last_report = Some(now);
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, id: ClientId) {
        tracing::debug!(?id, "cliente conectado");
    }

    fn disconnected(&self, id: ClientId, reason: DisconnectReason) {
        tracing::debug!(?id, ?reason, "cliente desconectado");
    }
}
