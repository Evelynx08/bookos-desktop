//! XWayland: las aplicaciones que solo hablan X11.
//!
//! Sin esto no arranca nada que no sea Wayland nativo, que en un escritorio
//! real es bastante: Steam, Wine, buena parte de lo que trae Electron viejo y
//! cualquier programa de los 2000 que uno siga usando. El compositor se
//! convierte en el gestor de ventanas X11 de esa gente, hablando el protocolo
//! con `x11rb` a través de Smithay.
//!
//! **Todo XWayland es un solo cliente de Wayland.** Sus ventanas X11 no son
//! `xdg_toplevel`; llegan por aquí como [`X11Surface`] y se envuelven en el
//! mismo `desktop::Window` que las demás, de modo que el resto del compositor
//! —animaciones, encaje, dock, foco— no necesita saber de dónde vienen. Los
//! únicos sitios que distinguen son los que mandan un tamaño al cliente:
//! X11 no tiene `configure` diferido, así que se le dice y punto.
//!
//! **Se arranca al vuelo y su fallo no es fatal.** Si `Xwayland` no está
//! instalado se avisa y la sesión sigue: perder los programas X11 es malo,
//! pero mucho menos que no tener escritorio.
//!
//! La escala se deja fuera a propósito. X11 no sabe de escalas fraccionarias
//! —el protocolo es de 1987— y lo que hacen KWin y Mutter es enseñarle a
//! XWayland una pantalla en píxeles físicos y dejar que las aplicaciones se
//! vean pequeñas, o mentirle con `Xft.dpi`. Aquí las ventanas X11 se tratan en
//! lógicos como las demás, que en un panel de 242 DPI significa que se ven al
//! tamaño correcto aunque un poco borrosas al ampliarlas.

use smithay::utils::{Logical, Rectangle};
use smithay::wayland::xwayland_shell::XWaylandShellState;
use smithay::xwayland::xwm::{Reorder, ResizeEdge, XwmId};
use smithay::xwayland::{X11Surface, X11Wm, XWayland, XWaylandEvent};

use crate::state::BookosComp;

/// Lanza XWayland y deja programado el arranque de su gestor de ventanas.
///
/// El `XWaylandEvent::Ready` no llega hasta que el servidor está escuchando, y
/// hasta entonces no hay con quién hablar X11: por eso el gestor se crea en el
/// callback y no aquí.
pub fn arrancar(state: &mut BookosComp) {
    let (xwayland, cliente) = match XWayland::spawn(
        &state.display_handle,
        // Sin número de pantalla fijo: que Smithay busque el primero libre. Si
        // se pidiera el :0 y hubiera otra sesión X viva, esto fallaría entero.
        None,
        std::iter::empty::<(String, String)>(),
        true,
        std::process::Stdio::null(),
        std::process::Stdio::null(),
        |_| {},
    ) {
        Ok(par) => par,
        Err(err) => {
            tracing::warn!("no se pudo arrancar XWayland, la sesión sigue sin X11: {err}");
            return;
        }
    };

    let resultado = state
        .loop_handle
        .insert_source(xwayland, move |evento, _, state| match evento {
            XWaylandEvent::Ready {
                x11_socket,
                display_number,
            } => {
                let wm = X11Wm::start_wm(state.loop_handle.clone(), x11_socket, cliente.clone());
                match wm {
                    Ok(wm) => {
                        // DISPLAY solo se publica cuando el gestor está en pie.
                        // Publicarlo antes deja una ventana de tiempo en la que
                        // un cliente X11 se conecta, no encuentra gestor y se
                        // dibuja sin decoración ni foco.
                        state.xwm = Some(wm);
                        state.display_x11 = Some(display_number);
                        tracing::info!(display = format!(":{display_number}"), "XWayland listo");
                    }
                    Err(err) => tracing::error!("no se pudo arrancar el gestor X11: {err}"),
                }
            }
            XWaylandEvent::Error => {
                tracing::error!("XWayland murió al arrancar; la sesión sigue sin X11");
            }
        });
    if let Err(err) = resultado {
        tracing::error!("no se pudo escuchar a XWayland: {err}");
    }
}

/// El `app_id` equivalente de una ventana X11.
///
/// En X11 el par que identifica al programa es `WM_CLASS`, y de sus dos campos
/// el que se parece a un `app_id` de Wayland es la clase: `firefox` da
/// `instance="Navigator"` pero `class="firefox"`, que es lo que el dock busca.
pub fn clase(window: &smithay::desktop::Window) -> Option<String> {
    let x11 = window.x11_surface()?;
    let clase = x11.class();
    (!clase.is_empty()).then_some(clase)
}

impl BookosComp {
    /// La ventana que envuelve a una superficie X11.
    fn window_de_x11(&self, surface: &X11Surface) -> Option<smithay::desktop::Window> {
        self.space
            .elements()
            .find(|w| w.x11_surface() == Some(surface))
            .cloned()
    }
}

impl smithay::xwayland::XwmHandler for BookosComp {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("el gestor X11 existe mientras el bucle le entrega eventos")
    }

    /// Existe pero aún no se ve. No se mapea nada todavía: muchas ventanas X11
    /// se crean para no enseñarse nunca —diálogos que el programa se guarda,
    /// ventanas de utilidad de las toolkits— y mapearlas aquí llenaría el
    /// escritorio de cosas invisibles ocupando sitio en el espacio.
    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    /// La ventana pide enseñarse. Hay que concedérselo explícitamente: en X11
    /// el gestor de ventanas puede decir que no, y si nadie dice que sí la
    /// ventana no aparece nunca.
    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_mapped(true);
        let area = self.work_area();
        // El límite se le manda ya: un cliente X11 que se maximice solo no debe
        // meterse bajo el panel.
        let geo = window.geometry();
        let _ = window.configure(Rectangle::new(geo.loc, geo.size));
        let elemento = smithay::desktop::Window::new_x11_window(window);
        self.space.map_element(elemento, area.loc, false);
        self.needs_redraw = true;
        self.actualizar_dock();
    }

    /// Ya está enseñada de verdad. Aquí es donde se coloca, y no en
    /// `map_window_request`: hasta este momento la geometría que anuncia puede
    /// ser todavía la de fábrica (1x1 en varias toolkits).
    fn map_window_notify(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elemento) = self.window_de_x11(&window) {
            self.colocar_si_es_nueva(&elemento);
        }
        self.needs_redraw = true;
    }

    /// Las override-redirect son menús, tooltips y demás: el gestor no manda
    /// sobre ellas y se limita a ponerlas donde el cliente diga.
    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let geo = window.geometry();
        let elemento = smithay::desktop::Window::new_x11_window(window);
        self.space.map_element(elemento, geo.loc, true);
        self.needs_redraw = true;
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elemento) = self.window_de_x11(&window) {
            self.space.unmap_elem(&elemento);
        }
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
        crate::keybinds::refocalizar(self);
        self.needs_redraw = true;
        self.actualizar_dock();
    }

    fn destroyed_window(&mut self, _xwm: XwmId, _window: X11Surface) {
        crate::keybinds::refocalizar(self);
        self.needs_redraw = true;
        self.actualizar_dock();
    }

    /// El cliente pide un tamaño o una posición. Se le concede tal cual salvo
    /// que esté encajado: ahí manda el compositor, o una ventana a media
    /// pantalla se saldría de su zona en cuanto el programa decidiera crecer.
    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let elemento = self.window_de_x11(&window);
        if let Some(elemento) = &elemento {
            if crate::ventanas::encajada(elemento) {
                let geo = window.geometry();
                let _ = window.configure(geo);
                return;
            }
        }
        let anterior = window.geometry();
        let destino = Rectangle::new(
            (x.unwrap_or(anterior.loc.x), y.unwrap_or(anterior.loc.y)).into(),
            (
                w.map(|v| v as i32).unwrap_or(anterior.size.w).max(1),
                h.map(|v| v as i32).unwrap_or(anterior.size.h).max(1),
            )
                .into(),
        );
        let _ = window.configure(destino);
    }

    /// El cliente ya se movió o cambió de tamaño. Para las override-redirect
    /// esto es lo único que hay: se mueven cuando quieren y el espacio tiene
    /// que seguirlas o el menú se queda pintado donde ya no está.
    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        let Some(elemento) = self.window_de_x11(&window) else {
            return;
        };
        if self.space.element_location(&elemento) != Some(geometry.loc) {
            self.space.map_element(elemento, geometry.loc, false);
        }
        self.needs_redraw = true;
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elemento) = self.window_de_x11(&window) {
            if !crate::ventanas::maximizada(&elemento) {
                self.alternar_maximizada(&elemento);
            }
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elemento) = self.window_de_x11(&window) {
            if crate::ventanas::maximizada(&elemento) {
                self.alternar_maximizada(&elemento);
            }
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elemento) = self.window_de_x11(&window) {
            self.pantalla_completa(&elemento, true);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elemento) = self.window_de_x11(&window) {
            self.pantalla_completa(&elemento, false);
        }
    }

    /// Mover y redimensionar pedidos por el cliente, que en X11 llegan con el
    /// botón ya agarrado. Se reutiliza el mismo arrastre que el resto: la
    /// ventana ya está en el espacio y el cursor ya está donde tiene que estar.
    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32, edge: ResizeEdge) {
        let Some(elemento) = self.window_de_x11(&window) else {
            return;
        };
        use ResizeEdge as E;
        let modo = crate::ventanas::Modo::Redimensionar {
            izquierda: matches!(edge, E::Left | E::TopLeft | E::BottomLeft),
            arriba: matches!(edge, E::Top | E::TopLeft | E::TopRight),
        };
        self.arrastrar_ventana(&elemento, modo);
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32) {
        if let Some(elemento) = self.window_de_x11(&window) {
            self.arrastrar_ventana(&elemento, crate::ventanas::Modo::Mover);
        }
    }

    /// XWayland se ha caído. Se suelta el gestor para que nadie siga mandándole
    /// órdenes —`enfocar` le pide subir ventanas en cada clic— y la sesión
    /// continúa sin X11, que es lo mismo que si nunca hubiera arrancado.
    fn disconnected(&mut self, _xwm: XwmId) {
        tracing::warn!("XWayland se desconectó; la sesión sigue sin X11");
        self.xwm = None;
        self.display_x11 = None;
    }
}

impl smithay::wayland::xwayland_shell::XWaylandShellHandler for BookosComp {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

smithay::delegate_xwayland_shell!(BookosComp);
