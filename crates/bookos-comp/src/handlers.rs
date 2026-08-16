//! Implementación de los protocolos Wayland que el compositor expone.
//!
//! Cada `commit` de un cliente marca `needs_redraw`. Ese es el único camino por
//! el que se llega a dibujar: si nadie hace commit, no hay frame.

use smithay::desktop::Window;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::{wl_buffer, wl_seat, wl_surface::WlSurface};
use smithay::reexports::wayland_server::Client;
use smithay::utils::Serial;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler, CompositorState,
};
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::fractional_scale::{with_fractional_scale, FractionalScaleHandler};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{
    delegate_compositor, delegate_data_device, delegate_fractional_scale, delegate_output,
    delegate_seat, delegate_shm, delegate_viewporter, delegate_xdg_shell,
};

use crate::state::{BookosComp, ClientState};

impl CompositorHandler for BookosComp {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    /// XWayland no se conecta como los demás: Smithay le pone su propio
    /// `ClientData` al lanzarlo, así que buscar aquí el nuestro devuelve `None`
    /// y antes eso era un `unwrap` — el compositor se caía en el primer commit
    /// de la primera ventana X11, antes incluso de que se viera.
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(datos) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
            return &datos.compositor_state;
        }
        &client
            .get_data::<ClientState>()
            .expect("todo cliente entra por nuestro socket o es XWayland")
            .compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        use smithay::backend::renderer::utils::on_commit_buffer_handler;
        on_commit_buffer_handler::<Self>(surface);

        // Sube al toplevel: el estado de la ventana vive en la superficie raíz.
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }

        // Una sola búsqueda para todo lo que viene. Antes se recorría el
        // espacio dos veces —aquí y otra vez dentro de `send_initial_configure`
        // — en **cada** commit de **cada** superficie, subsuperficies y cursor
        // incluidos.
        let Some(window) = self.window_for_surface(&root) else {
            // El commit no es de una ventana. Solo obliga a repintar si es la
            // superficie que el cliente ha puesto de cursor: lo demás no se ve,
            // y marcarlo arrastraba un frame entero por cada commit invisible.
            if self.cursor_surface_is(surface) {
                self.needs_redraw = true;
            }
            return;
        };

        if !is_sync_subsurface(surface) {
            window.on_commit();
        }

        // La primera configuración de un xdg_surface tiene que salir antes de
        // que el cliente pueda adjuntar buffer; si no, se queda esperando.
        self.send_initial_configure(&window, &root);
        self.colocar_si_es_nueva(&window);
        self.needs_redraw = true;
    }
}

impl BookosComp {
    /// Reenvía la escala preferida a todas las superficies vivas.
    ///
    /// Hace falta al cambiar de escala en caliente: `new_fractional_scale` solo
    /// cubre a los clientes que se conectan después.
    pub fn broadcast_preferred_scale(&mut self, scale: f64) {
        use smithay::wayland::compositor::with_states;
        let surfaces: Vec<_> = self
            .space
            .elements()
            .filter_map(|w| w.wl_surface().map(|s| s.into_owned()))
            .collect();
        for surface in surfaces {
            with_states(&surface, |states| {
                with_fractional_scale(states, |fractional| {
                    fractional.set_preferred_scale(scale);
                });
            });
        }
    }

    /// La escala de la pantalla. `Scale::Fractional` la guarda tal cual;
    /// `wl_output` solo sabe de enteros, pero `wp_fractional_scale` no.
    pub fn output_scale(&self) -> f64 {
        self.space
            .outputs()
            .next()
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(1.0)
    }

    /// La ventana cuya superficie raíz es `surface`.
    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.wl_surface().is_some_and(|s| s.as_ref() == surface))
            .cloned()
    }

    /// Le pasa al dock qué aplicaciones tienen ventana.
    ///
    /// Se llama al mapear, al cerrar y cuando un cliente fija su `app_id`, no
    /// por frame ni por commit: son los tres únicos momentos en que la lista
    /// puede cambiar, y recorrer las ventanas leyendo su estado no es algo que
    /// deba pasar sesenta veces por segundo.
    pub fn actualizar_dock(&mut self) {
        use smithay::utils::IsAlive;
        let ids: Vec<String> = self
            .space
            .elements()
            .filter(|w| w.alive())
            .filter_map(app_id)
            .collect();
        if self.shell.as_mut().is_some_and(|s| s.ventanas(&ids)) {
            self.needs_redraw = true;
        }
    }

    /// ¿Es `surface` la que el cliente ha puesto como imagen de cursor?
    ///
    /// Sirve para no repintar por commits que no se ven: el cursor es la única
    /// superficie que se dibuja sin estar en el `Space`.
    fn cursor_surface_is(&self, surface: &WlSurface) -> bool {
        use smithay::input::pointer::CursorImageStatus;
        matches!(&self.cursor_status, CursorImageStatus::Surface(s) if s == surface)
    }

    fn send_initial_configure(&mut self, window: &Window, surface: &WlSurface) {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let pending = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .map(|d| d.lock().unwrap().initial_configure_sent)
                .unwrap_or(true)
        });
        if !pending {
            toplevel.send_configure();
        }
    }
}

/// El `app_id` que el cliente ha declarado, si ya lo ha hecho.
pub fn app_id(window: &Window) -> Option<String> {
    use smithay::wayland::compositor::with_states;
    use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;

    // Una ventana X11 no tiene `xdg_toplevel` y su `wl_surface` no lleva
    // ningún `app_id`: lo que la identifica es su `WM_CLASS`.
    if let Some(clase) = crate::xwayland::clase(window) {
        return Some(clase);
    }

    let surface = window.wl_surface()?;
    with_states(&surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()?
            .lock()
            .ok()?
            .app_id
            .clone()
    })
}

impl XdgShellHandler for BookosComp {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let area = self.work_area();
        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Activated);
            // `size = None` es "elige tú": el cliente sale con su tamaño
            // natural, que es lo que hace flotante al escritorio. Antes se le
            // imponía el área útil y `Maximized`, y con eso **todas** las
            // ventanas caían en el mismo sitio tapándose por completo: abrir un
            // segundo terminal parecía no hacer nada.
            state.size = None;
            // El límite sí se le dice, para que un cliente que se maximiza solo
            // no pida más pantalla de la que hay bajo el panel.
            state.bounds = Some(area.size);
        });
        let window = Window::new_wayland_window(surface);
        // Se mapea ya para que exista en el espacio, pero la posición de verdad
        // se decide en el primer commit con tamaño (`colocar_si_es_nueva`);
        // hasta entonces `ventanas::animacion` la deja invisible.
        self.space.map_element(window, area.loc, false);
        self.needs_redraw = true;
        self.actualizar_dock();
    }

    /// El `app_id` llega en un commit posterior a `new_toplevel`, así que el
    /// dock no puede saber de quién es la ventana hasta aquí.
    fn app_id_changed(&mut self, _surface: ToplevelSurface) {
        self.actualizar_dock();
    }

    fn toplevel_destroyed(&mut self, _surface: ToplevelSurface) {
        // Al cerrarse una ventana puede quedar libre el sitio de una barra.
        self.revisar_barras();
        // El foco de teclado apuntaba a la ventana que acaba de morir: si no se
        // reasigna, las que siguen abiertas dejan de recibir teclas y el
        // escritorio parece congelado sin estarlo.
        crate::keybinds::refocalizar(self);
        self.needs_redraw = true;
        // La ventana muerta sigue en el espacio hasta el siguiente `refresh()`,
        // así que `actualizar_dock` la filtra por `alive()`.
        self.actualizar_dock();
    }

    /// El cliente pide mover su propia ventana: es lo que hace GTK cuando
    /// arrastras su barra de título, que la dibuja él y no nosotros.
    ///
    /// El serial se ignora a propósito. Lo correcto sería comprobar que
    /// corresponde a un clic reciente para que un cliente no pueda secuestrar el
    /// puntero sin que lo hayan tocado; aquí el arrastre se cancela al soltar
    /// cualquier botón, así que lo peor que consigue es que la ventana siga al
    /// cursor hasta el siguiente clic.
    fn move_request(&mut self, surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        self.arrastrar_toplevel(&surface, crate::ventanas::Modo::Mover);
    }

    /// Y lo mismo para el borde: GTK y Qt dibujan sus propios agarraderos.
    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        use xdg_toplevel::ResizeEdge as E;
        // El cliente dice de qué borde tira; se cree, porque él sabe dónde ha
        // puesto su agarradero mejor que nosotros mirando cuadrantes.
        let modo = crate::ventanas::Modo::Redimensionar {
            izquierda: matches!(edges, E::Left | E::TopLeft | E::BottomLeft),
            arriba: matches!(edges, E::Top | E::TopLeft | E::TopRight),
        };
        self.arrastrar_toplevel(&surface, modo);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_de_toplevel(&surface) {
            if !crate::ventanas::maximizada(&window) {
                self.alternar_maximizada(&window);
            }
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_de_toplevel(&surface) {
            if crate::ventanas::maximizada(&window) {
                self.alternar_maximizada(&window);
            }
        }
    }

    /// Pantalla completa: el cliente la pide con F11, con su propio botón o al
    /// darle a "reproducir a pantalla completa". La salida `output` se ignora
    /// porque de momento solo hay una pantalla.
    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        if let Some(window) = self.window_de_toplevel(&surface) {
            self.pantalla_completa(&window, true);
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_de_toplevel(&surface) {
            self.pantalla_completa(&window, false);
        }
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        self.needs_redraw = true;
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }
}

impl SeatHandler for BookosComp {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        // El cliente propone; el compositor dispone (y dibuja). Con
        // `wp_cursor_shape_v1` lo que llega es `Named` —"texto", "mano"— y se
        // resuelve contra nuestro tema; sin él, el cliente manda su propia
        // superficie ya pintada y no queda más que componerla tal cual.
        tracing::debug!(?image, "el cliente pide un cursor");
        self.cursor_status = image;
        self.needs_redraw = true;
    }
}

impl OutputHandler for BookosComp {}

impl FractionalScaleHandler for BookosComp {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        // En cuanto un cliente pregunta, se le dice a qué escala va a dibujarse.
        // Si no, asume 1 y se ve borroso en un panel de 2880×1800.
        let scale = self.output_scale();
        smithay::wayland::compositor::with_states(&surface, |states| {
            with_fractional_scale(states, |fractional| {
                fractional.set_preferred_scale(scale);
            });
        });
    }
}

impl BufferHandler for BookosComp {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for BookosComp {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SelectionHandler for BookosComp {
    type SelectionUserData = ();
}

impl DataDeviceHandler for BookosComp {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for BookosComp {}
impl ServerDndGrabHandler for BookosComp {}

delegate_compositor!(BookosComp);
delegate_xdg_shell!(BookosComp);
delegate_shm!(BookosComp);
delegate_seat!(BookosComp);
delegate_data_device!(BookosComp);
delegate_output!(BookosComp);
delegate_fractional_scale!(BookosComp);
delegate_viewporter!(BookosComp);

/// `wp_cursor_shape_v1` comparte el manejo del cursor con el de las tabletas,
/// así que exige este trait aunque aquí no haya ninguna. Todos sus métodos
/// tienen implementación por defecto: declararlo basta, y el día que haya una
/// Wacom el sitio donde escribir su cursor ya está.
impl smithay::wayland::tablet_manager::TabletSeatHandler for BookosComp {}
smithay::delegate_cursor_shape!(BookosComp);
