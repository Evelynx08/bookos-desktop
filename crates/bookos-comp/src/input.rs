//! Tratamiento de la entrada, común a los dos backends.
//!
//! Está escrito contra `InputBackend` en vez de contra un backend concreto, así
//! que el mismo código sirve para winit (anidado) y para libinput (sesión real).
//! Es deliberado: un fallo de coordenadas o de foco arreglado aquí queda
//! arreglado en los dos sitios, y ya nos ha mordido una vez.
//!
//! La diferencia entre ambos es solo cómo llega el puntero: winit da posición
//! absoluta normalizada y libinput da desplazamientos relativos. Los dos
//! terminan en el mismo sitio, [`set_pointer`].

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
    KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::keyboard::{keysyms, FilterResult};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use bookos_shell::TeclaPulsada;

use crate::state::BookosComp;

/// Botones del ratón, según linux/input-event-codes.h.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Los modificadores pulsados ahora mismo.
///
/// Un evento de botón no los trae: hay que preguntárselos al teclado, que es
/// quien lleva la cuenta.
fn modificadores(state: &BookosComp) -> smithay::input::keyboard::ModifiersState {
    state
        .seat
        .get_keyboard()
        .map(|kbd| kbd.modifier_state())
        .unwrap_or_default()
}

pub fn handle<B: InputBackend>(state: &mut BookosComp, event: InputEvent<B>) {
    state.last_input = Some(std::time::Instant::now());

    match event {
        InputEvent::Keyboard { event } => {
            let Some(kbd) = state.seat.get_keyboard() else {
                return;
            };
            let serial = SERIAL_COUNTER.next_serial();
            let time = event.time_msec();
            let pulsada = event.state() == KeyState::Pressed;
            // El filtro corre con las teclas **antes** de reenviarlas, así que
            // los atajos del compositor funcionan aunque el cliente con foco
            // esté colgado. Esa es justamente la propiedad que hace que el
            // cambio de TTY sea una salida de emergencia fiable.
            let accion = kbd.input(
                state,
                event.key_code(),
                event.state(),
                serial,
                time,
                |state, modifiers, handle| {
                    let bloqueado = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado());
                    // Solo la pulsación: interceptar también la suelta le
                    // dejaría la tecla trabada al cliente. Bloqueado, tampoco
                    // la suelta sale de aquí.
                    if !pulsada {
                        return if bloqueado {
                            FilterResult::Intercept(None)
                        } else {
                            FilterResult::Forward
                        };
                    }
                    if let Some(accion) = crate::keybinds::resolver(modifiers, &handle) {
                        // Con el bloqueo echado solo valen las salidas de
                        // emergencia. Que el cambio de TTY siga funcionando no
                        // es un descuido: es la puerta trasera con la que se
                        // recupera la máquina si el bloqueo falla, y la tiene
                        // cualquier escritorio. Lo demás —cerrar ventanas,
                        // maximizar, el launchpad— se traga.
                        if !bloqueado || accion.es_emergencia() {
                            return FilterResult::Intercept(Some(accion));
                        }
                        return FilterResult::Intercept(None);
                    }
                    if bloqueado {
                        // La tecla se compara por **keysym** y no por código de
                        // tecla: `key_code` devuelve el de xkb, que es el de
                        // evdev más ocho, y comparar con el 1 de evdev no
                        // acertaba nunca. Ese fue el fallo que dejaba el
                        // bloqueo sin salida.
                        let sym = handle.modified_sym();
                        let accion = match sym.raw() {
                            keysyms::KEY_Escape => Some(crate::keybinds::Accion::BloqueoLimpiar),
                            keysyms::KEY_BackSpace => {
                                Some(crate::keybinds::Accion::BloqueoBorrar)
                            }
                            keysyms::KEY_Return | keysyms::KEY_KP_Enter => {
                                Some(crate::keybinds::Accion::BloqueoComprobar)
                            }
                            _ => sym
                                .key_char()
                                // Los caracteres de control —tabulador, avance
                                // de línea— tienen `key_char` y no son parte de
                                // ninguna contraseña.
                                .filter(|c| !c.is_control())
                                .map(crate::keybinds::Accion::BloqueoEscribir),
                        };
                        // Ninguna tecla llega a los clientes, se entienda o no:
                        // es la propiedad que define un bloqueo.
                        return FilterResult::Intercept(accion);
                    }
                    // Una superficie emergente abierta se queda con las teclas
                    // que entiende: es lo que permite cerrar el menú con Esc y
                    // recorrerlo con las flechas sin que el cliente con foco
                    // llegue a verlas. `Intercept(None)` es "consumida, pero no
                    // es un atajo del compositor".
                    if let Some(tecla) = traducir_tecla(&handle) {
                        let (consumida, accion) = state
                            .shell
                            .as_mut()
                            .map(|s| s.tecla(tecla))
                            .unwrap_or((false, None));
                        if consumida {
                            state.needs_redraw = true;
                            // La acción no se ejecuta aquí: seguimos dentro del
                            // filtro de `kbd.input`, con el estado prestado.
                            return FilterResult::Intercept(
                                accion.map(crate::keybinds::Accion::DelShell),
                            );
                        }
                    }
                    FilterResult::Forward
                },
            );
            if let Some(Some(accion)) = accion {
                crate::keybinds::ejecutar(state, accion);
            }
        }

        // libinput: desplazamiento relativo respecto de donde estaba el cursor.
        InputEvent::PointerMotion { event } => {
            let delta = (event.delta_x(), event.delta_y());
            let destino = state.pointer_location + Point::from(delta);
            set_pointer(state, destino, event.time_msec());
        }

        // winit: posición absoluta, normalizada al tamaño de la ventana.
        InputEvent::PointerMotionAbsolute { event } => {
            let (w, h) = logical_size(state);
            let destino = (event.x_transformed(w), event.y_transformed(h)).into();
            set_pointer(state, destino, event.time_msec());
        }

        InputEvent::PointerButton { event } => {
            // Bloqueado, los clics tampoco salen de aquí.
            if state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
                return;
            }
            let serial = SERIAL_COUNTER.next_serial();
            let button = event.button_code();
            let pulsado = event.state();

            // Soltar cierra el arrastre y encaja la ventana si el cursor estaba
            // en un borde. Que el evento llegue o no al cliente depende de quién
            // empezó: con Meta+ratón el clic nunca existió para él, pero si fue
            // él quien pidió el movimiento, ya vio la pulsación y **necesita**
            // ver el soltar — sin eso se queda creyendo que sigues pulsando y no
            // vuelve a pedir otro movimiento nunca más.
            if pulsado == ButtonState::Released {
                // El shell primero: si tenía un deslizador agarrado, el soltar
                // es suyo aunque el puntero esté ya fuera de su superficie.
                if let Some(shell) = state.shell.as_mut() {
                    shell.soltar();
                }
                if state.soltar_arrastre() {
                    return;
                }
            }

            // Meta + botón mueve o redimensiona sin necesidad de acertarle a un
            // borde. Es el gesto de KWin y de casi todos los gestores de
            // ventanas de Wayland, y aquí es además la **única** forma de mover
            // una ventana que se dibuje sin decoración cliente.
            if pulsado == ButtonState::Pressed && modificadores(state).logo {
                let modo = match button {
                    BTN_LEFT => Some(crate::ventanas::Modo::Mover),
                    BTN_RIGHT => Some(crate::ventanas::Modo::Redimensionar {
                        izquierda: false,
                        arriba: false,
                    }),
                    _ => None,
                };
                if let Some(modo) = modo {
                    if state.empezar_arrastre(modo) {
                        return;
                    }
                }
            }

            // El shell se dibuja por encima de las ventanas, así que se mira
            // primero: un clic en el dock es del dock, no de lo que haya
            // debajo.
            let punto = state.pointer_location;
            // Clic derecho en el dock: menú contextual del icono, como en
            // Plasma. Fijar y cerrar salen de ahí, no del propio clic.
            if pulsado == ButtonState::Pressed
                && button == BTN_RIGHT
                && !modificadores(state).logo
                && state.shell.as_ref().is_some_and(|s| s.en_el_dock(punto.x, punto.y))
            {
                if state
                    .shell
                    .as_mut()
                    .is_some_and(|s| s.menu_dock_en(punto.x, punto.y))
                {
                    state.needs_redraw = true;
                    return;
                }
            }
            if state.shell.as_ref().is_some_and(|s| s.contiene(punto.x, punto.y)) {
                // Ni la pulsación ni el soltar llegan al cliente: para él este
                // clic no ha existido. Reenviar solo el soltar le dejaría un
                // botón que se levanta sin haberse pulsado nunca.
                if pulsado == ButtonState::Pressed {
                    if let Some(accion) = state.shell.as_mut().and_then(|s| s.pulsar(punto.x, punto.y))
                    {
                        crate::keybinds::hacer(state, accion);
                    }
                }
                return;
            }

            // Doble clic en la franja de arriba: maximiza y restaura, como en
            // cualquier escritorio. Va antes del reenvío porque si el clic llega
            // al cliente, la terminal lo entiende como seleccionar una palabra.
            if pulsado == ButtonState::Pressed && button == BTN_LEFT && doble_clic(state) {
                return;
            }

            // Pulsar sobre una ventana le da también el foco de teclado y la
            // eleva: es lo que espera cualquiera al hacer clic.
            if pulsado == ButtonState::Pressed {
                focus_under_pointer(state);
            }

            let pointer = state.pointer.clone();
            pointer.button(
                state,
                &ButtonEvent {
                    button,
                    state: pulsado,
                    serial,
                    time: event.time_msec(),
                },
            );
            pointer.frame(state);
        }

        InputEvent::PointerAxis { event } => {
            // Con una emergente abierta, el gesto es suyo: en el launchpad pasa
            // de página, como el swipe horizontal del plasmoide. `amount` viene
            // en píxeles lógicos con el touchpad y en muescas con la rueda, y a
            // las dos se les da el mismo trato porque el umbral está en píxeles.
            let dx = event.amount(Axis::Horizontal).unwrap_or(0.0);
            let dy = event.amount(Axis::Vertical).unwrap_or(0.0);
            if (dx != 0.0 || dy != 0.0)
                && state
                    .shell
                    .as_mut()
                    .is_some_and(|s| s.desplazar(dx, dy))
            {
                state.needs_redraw = true;
                return;
            }

            let mut frame = AxisFrame::new(event.time_msec()).source(AxisSource::Wheel);
            for axis in [Axis::Horizontal, Axis::Vertical] {
                if let Some(discreto) = event.amount_v120(axis) {
                    frame = frame.v120(axis, discreto as i32);
                }
                match event.amount(axis) {
                    Some(cantidad) if cantidad != 0.0 => frame = frame.value(axis, cantidad),
                    // Un 0 explícito significa "aquí se acabó el gesto": hay que
                    // reenviarlo o el cliente sigue creyendo que se desplaza.
                    Some(_) => frame = frame.stop(axis),
                    None => {}
                }
            }
            let pointer = state.pointer.clone();
            pointer.axis(state, frame);
            pointer.frame(state);
        }

        _ => {}
    }
}

/// Traduce un keysym de xkb a la tecla que entiende el shell.
///
/// El shell no conoce xkb a propósito: su enum tiene ocho variantes y ninguna
/// dependencia, así que la frontera aguanta aunque debajo cambie la capa de
/// teclado.
fn traducir_tecla(handle: &smithay::input::keyboard::KeysymHandle<'_>) -> Option<TeclaPulsada> {
    use smithay::input::keyboard::keysyms;
    let sym = handle.modified_sym();
    let tecla = match sym.raw() {
        keysyms::KEY_Escape => TeclaPulsada::Escape,
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => TeclaPulsada::Intro,
        keysyms::KEY_Up => TeclaPulsada::Arriba,
        keysyms::KEY_Down => TeclaPulsada::Abajo,
        keysyms::KEY_Left => TeclaPulsada::Izquierda,
        keysyms::KEY_Right => TeclaPulsada::Derecha,
        keysyms::KEY_BackSpace => TeclaPulsada::Retroceso,
        // Lo demás solo interesa si escribe algo: es lo que alimentará la
        // búsqueda del launchpad. Los controles se descartan para que un
        // Ctrl+C no acabe metiendo un carácter raro en un campo de texto.
        _ => {
            let c = sym.key_char()?;
            if c.is_control() {
                return None;
            }
            TeclaPulsada::Caracter(c)
        }
    };
    Some(tecla)
}

/// Mueve el cursor a un punto lógico, lo confina a la pantalla y reenvía el
/// movimiento al cliente que haya debajo.
pub fn set_pointer(state: &mut BookosComp, destino: Point<f64, Logical>, time: u32) {
    let (w, h) = logical_size(state);
    // Sin confinar, el puntero se puede ir fuera de la pantalla y no hay forma
    // de traerlo de vuelta: en una sesión real eso es un cursor perdido.
    let location = (
        destino.x.clamp(0.0, w as f64 - 1.0),
        destino.y.clamp(0.0, h as f64 - 1.0),
    )
        .into();
    state.pointer_location = location;
    // Mover el cursor **es** un cambio en pantalla. Sin esto el puntero solo se
    // repinta cuando algo más provoca un frame, y se arrastra a tirones.
    state.needs_redraw = true;

    // Con una ventana agarrada, el movimiento es del compositor: ni el shell ni
    // el cliente lo ven. Reenviarlo mientras arrastras una terminal la deja
    // creyendo que estás seleccionando texto.
    if state.seguir_arrastre() {
        return;
    }

    if let Some(shell) = state.shell.as_mut() {
        shell.puntero(location.x, location.y);
    }
    // El cursor pegado a un borde trae de vuelta a la barra que se apartó. Va
    // en físicos porque la franja sensible se mide contra la pantalla.
    let escala = state.output_scale();
    let fisico = (location.x * escala, location.y * escala);
    let mut repintar = false;
    for cual in [crate::shell::Barra::Panel, crate::shell::Barra::Dock] {
        if let Some(shell) = state.shell.as_mut() {
            repintar |= shell.reclamada(cual, fisico);
        }
    }
    if repintar {
        state.needs_redraw = true;
    }

    let focus = state.surface_under(location);
    let pointer = state.pointer.clone();
    pointer.motion(
        state,
        focus,
        &MotionEvent {
            location,
            serial: SERIAL_COUNTER.next_serial(),
            time,
        },
    );
    pointer.frame(state);
}

/// Lo que se considera "la barra de título" de una ventana, en lógicos.
///
/// El compositor no sabe dónde acaba la barra de un cliente que se decora solo:
/// eso vive dentro de su buffer y no se anuncia por ningún sitio. 40 cubre las
/// barras de Qt y de GTK, que rondan las 30-36, sin llegar a la primera fila de
/// contenido de una terminal.
const BARRA: i32 = 40;

/// Cuánto puede tardar el segundo clic para seguir siendo un doble clic.
///
/// 400 ms es el valor por defecto de Qt y de GTK, o sea el que el usuario ya
/// tiene en los dedos del resto del sistema.
const DOBLE_CLIC: std::time::Duration = std::time::Duration::from_millis(400);

/// Cuánto se puede mover el ratón entre los dos clics.
///
/// Sin este margen, un temblor de un píxel entre pulsaciones convierte el doble
/// clic en dos clics sueltos y la ventana no hace nada.
const DOBLE_CLIC_RADIO: f64 = 6.0;

/// ¿Este clic completa un doble clic sobre la barra de título? Si sí, alterna
/// el tamaño de la ventana y se lo traga.
///
/// Público para el autotest: es la única forma de probar el gesto sin una mano
/// dando dos clics seguidos.
pub fn doble_clic(state: &mut BookosComp) -> bool {
    let ahora = std::time::Instant::now();
    let punto = state.pointer_location;
    let previo = state.ultimo_clic.replace((ahora, punto));

    let doble = previo.is_some_and(|(t, p)| {
        ahora.duration_since(t) <= DOBLE_CLIC
            && (p.x - punto.x).abs() <= DOBLE_CLIC_RADIO
            && (p.y - punto.y).abs() <= DOBLE_CLIC_RADIO
    });
    if !doble {
        return false;
    }

    let Some((window, loc)) = state
        .space
        .element_under(punto)
        .map(|(w, loc)| (w.clone(), loc))
    else {
        return false;
    };
    // Solo la franja de arriba: doble clic en medio de una terminal es
    // seleccionar una palabra, y eso es del cliente.
    if punto.y - loc.y as f64 > BARRA as f64 {
        return false;
    }
    // El tercer clic no vuelve a disparar: se olvida el par ya usado.
    state.ultimo_clic = None;
    state.alternar_maximizada(&window);
    true
}

/// Da el foco de teclado a la ventana bajo el cursor y la sube del todo.
fn focus_under_pointer(state: &mut BookosComp) {
    let bajo = state
        .space
        .element_under(state.pointer_location)
        .map(|(w, _)| w.clone());
    if let Some(window) = bajo {
        state.enfocar(&window);
    }
}

/// Tamaño de la pantalla en píxeles **lógicos**.
///
/// El modo de la salida está en físicos, pero el puntero y la escena razonan en
/// lógicos: sin dividir por la escala, a 1,75 los clics caen desplazados un 75 %.
fn logical_size(state: &BookosComp) -> (i32, i32) {
    state
        .space
        .outputs()
        .next()
        .and_then(|o| Some((o.current_mode()?, o.current_scale().fractional_scale())))
        .map(|(mode, scale)| {
            (
                (mode.size.w as f64 / scale).round().max(1.0) as i32,
                (mode.size.h as f64 / scale).round().max(1.0) as i32,
            )
        })
        .unwrap_or((1, 1))
}
