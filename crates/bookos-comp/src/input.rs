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
    AbsolutePositionEvent, Axis, ButtonState, Event, GestureBeginEvent, GesturePinchUpdateEvent,
    GestureSwipeUpdateEvent, InputBackend, InputEvent, KeyState, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::keyboard::{FilterResult, Keycode, keysyms};
use smithay::input::pointer::RelativeMotionEvent;
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use smithay::wayland::pointer_constraints::{PointerConstraint, with_pointer_constraint};

use bookos_shell::TeclaPulsada;

use crate::gestos::Gesto;
use crate::state::{ArrastreEscritorio, BookosComp};

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

fn modificador_conmutador_suelto(
    modo: bookos_shell::conmutador::Modo,
    modifiers: &smithay::input::keyboard::ModifiersState,
) -> bool {
    match modo {
        bookos_shell::conmutador::Modo::Aplicaciones => !modifiers.alt,
        bookos_shell::conmutador::Modo::Ventanas => !modifiers.logo,
    }
}

/// Una tecla, ya traducida por `teclas.conf`, camino de xkb y del cliente.
///
/// `codigo` es el de xkb (evdev + 8). Se conserva el código y no solo el keysym
/// porque algunas teclas Fn llegan sin keysym aunque el kernel sí haya
/// identificado correctamente su botón físico.
/// Una tecla, venga de libinput, de winit o de un autotest.
pub fn tecla(state: &mut BookosComp, codigo: Keycode, estado: KeyState, time: u32) {
    let Some(kbd) = state.seat.get_keyboard() else {
        return;
    };
    let serial = SERIAL_COUNTER.next_serial();
    let pulsada = estado == KeyState::Pressed;
    // Hay un menú abierto que aún no tiene el teclado: se le da ahora, antes
    // de que la tecla salga hacia el cliente. El grab de popups ignora los
    // cambios de foco mientras está puesto, así que se quita y se vuelve a
    // poner con el mismo serial.
    if pulsada && let Some(grab) = state.menu_sin_foco.take().filter(|g| !g.has_ended()) {
        kbd.unset_grab(state);
        kbd.set_focus(state, grab.current_grab(), serial);
        kbd.set_grab(
            state,
            smithay::desktop::PopupKeyboardGrab::new(&grab),
            grab.serial(),
        );
    }
    // El filtro corre con las teclas **antes** de reenviarlas, así que
    // los atajos del compositor funcionan aunque el cliente con foco
    // esté colgado. Esa es justamente la propiedad que hace que el
    // cambio de TTY sea una salida de emergencia fiable.
    let accion = kbd.input(
        state,
        codigo,
        estado,
        serial,
        time,
        |state, modifiers, handle| {
            let _ = modifiers;
            let bloqueado = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado());
            // Con la capa de captura abierta, el teclado es suyo: Esc
            // la cierra, Intro captura y las flechas cambian de modo.
            // Nada de eso puede llegar a la ventana de debajo.
            if state.shell.as_ref().is_some_and(|s| s.hay_captura()) {
                if !pulsada {
                    return FilterResult::Intercept(None);
                }
                let accion = traducir_tecla(&handle)
                    .and_then(|t| state.shell.as_mut().map(|s| s.captura_tecla(t)))
                    .and_then(|(_, accion)| accion);
                state.needs_redraw = true;
                // Consumida o no, la tecla se queda aquí: mientras
                // encuadras, el escritorio no responde a nada más.
                return FilterResult::Intercept(accion.map(crate::keybinds::Accion::DelShell));
            }
            // Solo la pulsación: interceptar también la suelta le
            // dejaría la tecla trabada al cliente. Bloqueado, tampoco
            // la suelta sale de aquí.
            if !pulsada {
                if bloqueado {
                    return FilterResult::Intercept(None);
                }
                // Con una excepción: soltar el modificador **resuelve**
                // el conmutador de Alt+Tab. Es lo que lo hace el de
                // macOS y no una lista que se queda abierta: eliges
                // mientras mantienes la tecla y confirmas al soltarla.
                //
                // La acción **no** se devuelve por `Intercept`, que es
                // lo que hacía antes: `Intercept` se queda la tecla y no
                // la reenvía (`KeyboardHandle::input` de Smithay), así
                // que el cliente al que llegas nunca ve que Alt se ha
                // soltado y se le queda trabada para siempre. Se marca
                // aquí, se devuelve `Forward` para que la suelta salga
                // hacia el cliente que la esperaba, y se resuelve al
                // volver de `kbd.input`.
                // Smithay actualiza el estado de modificadores antes de
                // llamar al filtro. Mirarlo evita depender del keysym
                // concreto de Alt/Meta en cada mapa de teclado.
                let cerrando = state.shell.as_ref().is_some_and(|s| s.hay_conmutador())
                    && !state.conmutador_pegado
                    && state
                        .conmutador_modo
                        .is_some_and(|modo| modificador_conmutador_suelto(modo, modifiers));
                if cerrando {
                    state.conmutador_resolver = true;
                }
                // Meta a secas: se soltó sin que llegara nada más por el
                // medio, así que era la tecla y no un modificador. El
                // launchpad se abre al volver de `kbd.input`, igual que
                // el conmutador y por lo mismo: la suelta tiene que
                // salir hacia el cliente o se le queda trabada.
                let meta = matches!(
                    handle.modified_sym().raw(),
                    keysyms::KEY_Super_L | keysyms::KEY_Super_R
                );
                if meta && std::mem::take(&mut state.meta_sola) {
                    state.abrir_launchpad = true;
                }
                return FilterResult::Forward;
            }
            // Cada tecla que llega, con sus modificadores. Es la única
            // forma de distinguir "el atajo no funciona" de "la tecla no
            // llega": anidado, el compositor de debajo se queda con
            // Alt+Tab y aquí no aparece nada.
            tracing::debug!(
                codigo = codigo.raw(),
                sym = handle.modified_sym().raw(),
                nombre = ?handle.modified_sym().name(),
                alt = modifiers.alt,
                logo = modifiers.logo,
                ctrl = modifiers.ctrl,
                shift = modifiers.shift,
                "tecla"
            );
            // Meta empieza a contar como «sola» solo si no hay nada más
            // pulsado; cualquier otra tecla la descarta hasta que se
            // vuelva a pulsar.
            state.meta_sola = matches!(
                handle.modified_sym().raw(),
                keysyms::KEY_Super_L | keysyms::KEY_Super_R
            ) && !modifiers.alt
                && !modifiers.ctrl
                && !modifiers.shift;
            // Escape cancela el conmutador sin ir a ninguna parte, y no
            // llega al cliente: es la salida del propio conmutador.
            if state.shell.as_ref().is_some_and(|s| s.hay_conmutador())
                && handle.modified_sym().raw() == keysyms::KEY_Escape
            {
                return FilterResult::Intercept(Some(crate::keybinds::Accion::ConmutarCancelar));
            }
            if let Some(accion) = crate::keybinds::resolver(modifiers, &handle, codigo.raw()) {
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
            if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
                // La capa es modal: mientras el usuario mantiene Alt o
                // Meta, ninguna tecla suelta debe acabar escribiéndose
                // en la ventana que hay debajo.
                return FilterResult::Intercept(None);
            }
            if bloqueado {
                // La tecla se compara por **keysym** y no por código de
                // tecla: `key_code` devuelve el de xkb, que es el de
                // evdev más ocho, y comparar con el 1 de evdev no
                // acertaba nunca. Ese fue el fallo que dejaba el
                // bloqueo sin salida.
                let sym = handle.modified_sym();
                let tecla = match sym.raw() {
                    keysyms::KEY_Escape => bookos_shell::TeclaPulsada::Escape,
                    keysyms::KEY_Return | keysyms::KEY_KP_Enter => {
                        bookos_shell::TeclaPulsada::Intro
                    }
                    keysyms::KEY_Left => bookos_shell::TeclaPulsada::Izquierda,
                    keysyms::KEY_Right => bookos_shell::TeclaPulsada::Derecha,
                    keysyms::KEY_Tab => bookos_shell::TeclaPulsada::Tabulador,
                    _ => bookos_shell::TeclaPulsada::Caracter(' '),
                };
                if let Some(shell) = state.shell.as_mut() {
                    let (consumida, peticion) = shell.bloqueo_confirmar_tecla(tecla);
                    if consumida {
                        state.needs_redraw = true;
                        return FilterResult::Intercept(
                            peticion.map(crate::keybinds::Accion::Energia),
                        );
                    }
                }
                let accion = match sym.raw() {
                    keysyms::KEY_F9 => Some(crate::keybinds::Accion::BloqueoHuella),
                    keysyms::KEY_Escape => Some(crate::keybinds::Accion::BloqueoLimpiar),
                    keysyms::KEY_BackSpace => Some(crate::keybinds::Accion::BloqueoBorrar),
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
                    return FilterResult::Intercept(accion.map(crate::keybinds::Accion::DelShell));
                }
            }
            FilterResult::Forward
        },
    );
    if let Some(Some(accion)) = accion {
        crate::keybinds::ejecutar(state, accion);
    }
    // El orden importa y es este: la suelta de Alt ya ha salido hacia el
    // cliente **antiguo** —que es quien vio la pulsación y la espera— y
    // el cambio de foco ocurre ahora, así que el `enter` del cliente
    // nuevo llega con el juego de teclas pulsadas ya sin Alt.
    if std::mem::take(&mut state.conmutador_resolver) {
        crate::keybinds::ejecutar(state, crate::keybinds::Accion::ConmutarFin);
    }
    if std::mem::take(&mut state.abrir_launchpad) {
        crate::keybinds::ejecutar(state, crate::keybinds::Accion::Launchpad);
    }
    if state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
        let caps_lock = kbd.modifier_state().caps_lock;
        if let Some(shell) = state.shell.as_mut() {
            shell.bloqueo_caps_lock(caps_lock);
        }
        state.needs_redraw = true;
    }
}

/// La acción de un remapeo. Con el bloqueo echado no hace nada: una tecla
/// cambiada no puede ser una forma de saltárselo.
fn remapeada(state: &mut BookosComp, accion: crate::keybinds::Accion) {
    if state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
        return;
    }
    // La tecla no pasa por xkb ni por el filtro, que es quien descarta el
    // «Meta sola»: sin esto, soltar Meta después abriría el launchpad.
    state.meta_sola = false;
    crate::keybinds::ejecutar(state, accion);
}

/// Si la app de teclas espera una tecla, se queda con esta pulsación y la
/// anuncia por `KeyCaptured`.
///
/// Los modificadores no se capturan: son la primera mitad del acorde de
/// Copilot, y al pulsarlos no se sabe si viene algo detrás. Quien quiera
/// remapear uno lo escribe a mano en `teclas.conf`.
fn capturar(state: &mut BookosComp, evdev: u32) -> bool {
    const MODIFICADORES: [u32; 8] = [29, 97, 42, 54, 56, 100, 125, 126];
    if !state.captura_tecla
        || MODIFICADORES.contains(&evdev)
        || state.shell.as_ref().is_some_and(|s| s.esta_bloqueado())
    {
        return false;
    }
    state.captura_tecla = false;
    let copilot = state.traductor.es_copilot(evdev);
    crate::ajustes::avisar_tecla_capturada(state, evdev, copilot);
    state.traductor.tragar_suelta(evdev);
    true
}

pub fn handle<B: InputBackend>(state: &mut BookosComp, event: InputEvent<B>) {
    state.last_input = Some(std::time::Instant::now());
    state.idle_notifier_state.notify_activity(&state.seat);
    crate::backend::despertar_dpms(state);
    if state
        .shell
        .as_ref()
        .is_some_and(|shell| shell.esta_bloqueado())
    {
        crate::backend::programar_suspension_inactividad(state);
    }

    match event {
        InputEvent::Keyboard { event } => {
            // `teclas.conf` habla en evdev, que es lo que dicen
            // `input-event-codes.h` y `libinput debug-events`; smithay da el
            // código de xkb, ocho más.
            let evdev = event.key_code().raw() - 8;
            let pulsada = event.state() == KeyState::Pressed;
            let time = event.time_msec();
            let eventos = if pulsada && capturar(state, evdev) {
                Vec::new()
            } else {
                state.traductor.traducir(&state.teclas, evdev, pulsada)
            };
            for evento in eventos {
                match evento {
                    crate::teclas::Evento::Tecla(evdev, pulsada) => {
                        let estado = if pulsada {
                            KeyState::Pressed
                        } else {
                            KeyState::Released
                        };
                        tecla(state, Keycode::new(evdev + 8), estado, time);
                    }
                    crate::teclas::Evento::Accion(accion) => remapeada(state, accion.accion()),
                    crate::teclas::Evento::Comando(cmd) => {
                        remapeada(state, crate::keybinds::Accion::Lanzar(cmd));
                    }
                }
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
            // El backend anidado entrega coordenadas relativas a su única
            // salida. Sumar su origen mantiene correcto el camino si la salida
            // fue recolocada desde el editor de pantallas.
            let origen = state
                .space
                .outputs()
                .next()
                .and_then(|o| state.space.output_geometry(o))
                .map(|r| r.loc)
                .unwrap_or_default();
            let destino = (
                origen.x as f64 + event.x_transformed(w),
                origen.y as f64 + event.y_transformed(h),
            )
                .into();
            set_pointer(state, destino, event.time_msec());
        }

        InputEvent::PointerButton { event } => {
            boton(state, event.button_code(), event.state(), event.time_msec());
        }

        InputEvent::PointerAxis { event } => {
            // Con una emergente abierta, el gesto es suyo: en el launchpad pasa
            // de página, como el swipe horizontal del plasmoide. `amount` viene
            // en píxeles lógicos con el touchpad y en muescas con la rueda, y a
            // las dos se les da el mismo trato porque el umbral está en píxeles.
            let dx = event.amount(Axis::Horizontal).unwrap_or(0.0);
            let dy = event.amount(Axis::Vertical).unwrap_or(0.0);
            if (dx != 0.0 || dy != 0.0) && state.shell.as_mut().is_some_and(|s| s.desplazar(dx, dy))
            {
                state.needs_redraw = true;
                return;
            }

            // No todo desplazamiento es una rueda. Firefox, entre otros,
            // mantiene estados distintos para una rueda (pasos) y un touchpad
            // (gesto continuo): etiquetar siempre como `Wheel` hace que un
            // gesto de dos dedos pueda quedarse abierto y el siguiente deje
            // de llegar. Se conserva la fuente real que da libinput.
            let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
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

        // --- Gestos de touchpad ---------------------------------------------
        // Solo llegan desde libinput: winit no los reenvía, así que esto está
        // muerto en el backend anidado. La lógica vive en `crate::gestos`, que
        // sí se puede probar sin touchpad.
        InputEvent::GestureSwipeBegin { event } => {
            state.gestos.deslizamiento_inicio(event.fingers());
        }

        InputEvent::GestureSwipeUpdate { event } => {
            if let Some(gesto) = state
                .gestos
                .deslizamiento_avance(event.delta_x(), event.delta_y())
            {
                tracing::debug!(?gesto, "gesto de touchpad");
                match gesto {
                    Gesto::Escritorio(pasos) => {
                        let destino = crate::escritorios::destino(
                            crate::escritorios::activo_aqui(state),
                            pasos,
                            state.escritorios.cuantos(),
                        );
                        crate::escritorios::cambiar_aqui(state, destino);
                    }
                    Gesto::DespejarEscritorio => crate::escritorios::despejar(state),
                    // Subir con el escritorio despejado devuelve las ventanas;
                    // subir estando en una aplicación cualquiera enseña los
                    // escritorios, que es lo que hace el mismo gesto en macOS.
                    Gesto::SubirCuatroDedos => {
                        if state.escritorios.despejado() {
                            crate::escritorios::recuperar(state);
                        } else {
                            crate::keybinds::ejecutar(
                                state,
                                crate::keybinds::Accion::VistaEscritorios,
                            );
                        }
                    }
                    // La exposición es el conmutador de ventanas sin tecla que
                    // sostener: se abre y se queda, y se cierra al elegir una
                    // miniatura, con Esc o con el gesto contrario. El `0` es que
                    // no mueva la selección al abrir —no hay Tab que aplicar—.
                    Gesto::Exponer => {
                        if !state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
                            crate::keybinds::ejecutar(
                                state,
                                crate::keybinds::Accion::Conmutar(
                                    bookos_shell::conmutador::Modo::Ventanas,
                                    0,
                                ),
                            );
                            state.conmutador_pegado =
                                state.shell.as_ref().is_some_and(|s| s.hay_conmutador());
                        }
                    }
                    Gesto::CerrarExposicion => {
                        if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
                            crate::keybinds::ejecutar(
                                state,
                                crate::keybinds::Accion::ConmutarCancelar,
                            );
                        }
                    }
                    // El pellizco no sale de un deslizamiento.
                    Gesto::AbrirLaunchpad | Gesto::CerrarLaunchpad => {}
                }
            }
        }

        InputEvent::GestureSwipeEnd { .. } => state.gestos.deslizamiento_fin(),

        InputEvent::GesturePinchBegin { event } => {
            state.gestos.pellizco_inicio(event.fingers());
        }

        InputEvent::GesturePinchUpdate { event } => {
            let abierto = state
                .shell
                .as_ref()
                .is_some_and(|s| s.emergente_nombre() == Some("launchpad"));
            if let Some(gesto) = state.gestos.pellizco_avance(event.scale(), abierto) {
                tracing::debug!(?gesto, escala = event.scale(), "gesto de touchpad");
                // Los dos gestos alternan lo mismo; `pellizco_avance` ya se ha
                // encargado de que solo salga el que corresponde al estado
                // actual, así que aquí no hay nada que distinguir.
                if let Some(shell) = state.shell.as_mut() {
                    shell.alternar_launchpad();
                }
                state.needs_redraw = true;
            }
        }

        InputEvent::GesturePinchEnd { .. } => state.gestos.pellizco_fin(),

        _ => {}
    }
}

/// Traduce un keysym de xkb a la tecla que entiende el shell.
///
/// El shell no conoce xkb a propósito: su enum es una docena de variantes sin
/// ninguna dependencia, así que la frontera aguanta aunque debajo cambie la
/// capa de teclado.
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
        keysyms::KEY_Home | keysyms::KEY_KP_Home => TeclaPulsada::Inicio,
        keysyms::KEY_End | keysyms::KEY_KP_End => TeclaPulsada::Fin,
        keysyms::KEY_Prior | keysyms::KEY_KP_Prior => TeclaPulsada::PaginaArriba,
        keysyms::KEY_Next | keysyms::KEY_KP_Next => TeclaPulsada::PaginaAbajo,
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => TeclaPulsada::Tabulador,
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

/// Un botón del ratón, venga de libinput, de winit o de un autotest: todos
/// pasan por aquí para que el clic de prueba recorra el mismo camino.
pub fn boton(state: &mut BookosComp, button: u32, pulsado: ButtonState, time: u32) {
    // Meta+clic mueve y redimensiona ventanas: si hubo clic, la tecla
    // era un modificador y no debe abrir nada al soltarla.
    state.meta_sola = false;
    // Bloqueado, los clics no salen hacia los clientes: lo único que
    // atienden es el botón de apagado de la propia pantalla de bloqueo.
    // La capa de captura se queda con el ratón mientras está: se está
    // encuadrando una foto y un clic no puede llegar a la ventana de
    // debajo. Va antes que el bloqueo porque son excluyentes y esta es
    // la que puede estar abierta con el escritorio en uso.
    if state.shell.as_ref().is_some_and(|s| s.hay_captura()) {
        let (x, y) = (state.pointer_location.x, state.pointer_location.y);
        let accion = match pulsado {
            ButtonState::Pressed => state.shell.as_mut().and_then(|s| s.captura_pulsar(x, y)),
            ButtonState::Released => {
                if let Some(shell) = state.shell.as_mut() {
                    shell.captura_soltar();
                }
                None
            }
        };
        if let Some(accion) = accion {
            crate::keybinds::hacer(state, accion);
        }
        state.needs_redraw = true;
        return;
    }
    if state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()) {
        if pulsado == ButtonState::Pressed {
            let (x, y) = (state.pointer_location.x, state.pointer_location.y);
            let peticion = state
                .shell
                .as_mut()
                .and_then(|shell| shell.bloqueo_pulsado(x, y));
            if let Some(peticion) = peticion {
                crate::keybinds::ejecutar(state, crate::keybinds::Accion::Energia(peticion));
            }
            state.needs_redraw = true;
        }
        return;
    }
    let serial = SERIAL_COUNTER.next_serial();

    if pulsado == ButtonState::Released && std::mem::take(&mut state.conmutador_clic) {
        return;
    }
    if pulsado == ButtonState::Pressed && state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
        state.conmutador_clic = true;
        let elegida = (button == BTN_LEFT)
            .then(|| {
                state.shell.as_ref().and_then(|s| {
                    s.conmutador_en(state.pointer_location.x, state.pointer_location.y)
                })
            })
            .flatten();
        if let Some(i) = elegida {
            if let Some(shell) = state.shell.as_mut() {
                shell.conmutador_elegir(i);
            }
            crate::keybinds::ejecutar(state, crate::keybinds::Accion::ConmutarFin);
        } else {
            crate::keybinds::ejecutar(state, crate::keybinds::Accion::ConmutarCancelar);
        }
        return;
    }

    // Con la vista de escritorios abierta, una ventana de una miniatura se
    // coge para llevarla a otro escritorio. Va antes que el shell porque la
    // vista cambia de escritorio al pulsar, y eso acabaría el arrastre antes
    // de empezar.
    if button == BTN_LEFT
        && crate::escritorios::boton_en_vista(state, pulsado == ButtonState::Pressed)
    {
        return;
    }

    // Soltar cierra el arrastre y encaja la ventana si el cursor estaba
    // en un borde. Que el evento llegue o no al cliente depende de quién
    // empezó: con Meta+ratón el clic nunca existió para él, pero si fue
    // él quien pidió el movimiento, ya vio la pulsación y **necesita**
    // ver el soltar — sin eso se queda creyendo que sigues pulsando y no
    // vuelve a pedir otro movimiento nunca más.
    if pulsado == ButtonState::Released {
        if button == BTN_RIGHT && std::mem::take(&mut state.menu_ventana_suelta) {
            return;
        }
        // La banda elástica o los iconos agarrados se cierran aquí, y
        // ese soltar no es de nadie más: empezó sobre el escritorio.
        if soltar_escritorio(state) {
            return;
        }
        // El shell primero: si tenía un deslizador agarrado, el soltar
        // es suyo aunque el puntero esté ya fuera de su superficie.
        if let Some(accion) = state.shell.as_mut().and_then(|s| s.soltar()) {
            crate::keybinds::hacer(state, accion);
        }
        // Un botón de la barra de título se dispara al soltar y sobre
        // el mismo botón donde se pulsó, como cualquier botón del
        // sistema: bajar el ratón en la ✕ y salirse antes de soltar no
        // cierra nada.
        if let Some((window, boton)) = crate::decoracion::soltar(state, state.pointer_location) {
            crate::decoracion::accionar(state, &window, boton);
            return;
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
        if let Some(modo) = modo
            && state.empezar_arrastre(modo)
        {
            return;
        }
    }

    // El shell se dibuja por encima de las ventanas, así que se mira
    // primero: un clic en el dock es del dock, no de lo que haya
    // debajo.
    let punto = state.pointer_location;
    // Clic derecho en el dock: menú contextual del icono, como en
    // Plasma. Fijar y cerrar salen de ahí, no del propio clic.
    // Y en el launchpad: el botón derecho saca las ✕ para quitar
    // aplicaciones de la rejilla, que es el mismo gesto de «esto tiene
    // más opciones» que en el dock.
    if pulsado == ButtonState::Pressed
        && button == BTN_RIGHT
        && !modificadores(state).logo
        && state.shell.as_mut().is_some_and(|s| s.launchpad_menu())
    {
        state.needs_redraw = true;
        return;
    }
    if pulsado == ButtonState::Pressed
        && button == BTN_RIGHT
        && !modificadores(state).logo
        && state
            .shell
            .as_ref()
            .is_some_and(|s| s.en_el_dock(punto.x, punto.y))
        && state
            .shell
            .as_mut()
            .is_some_and(|s| s.menu_dock_en(punto.x, punto.y))
    {
        state.needs_redraw = true;
        return;
    }
    if state
        .shell
        .as_ref()
        .is_some_and(|s| s.contiene(punto.x, punto.y))
    {
        // Ni la pulsación ni el soltar llegan al cliente: para él este
        // clic no ha existido. Reenviar solo el soltar le dejaría un
        // botón que se levanta sin haberse pulsado nunca.
        if pulsado == ButtonState::Pressed {
            if button == BTN_RIGHT && state.shell.as_mut().is_some_and(|s| s.editar_launchpad()) {
                state.needs_redraw = true;
                return;
            }
            // Solo el botón principal activa controles. Antes cualquier
            // botón caía por este camino cuando su menú contextual no
            // encontraba un item; en el dock eso convertía un clic
            // derecho en «activar» y minimizaba la app con la lámpara.
            if button == BTN_LEFT
                && let Some(accion) = state
                    .shell
                    .as_mut()
                    .and_then(|s| s.pulsar(punto.x, punto.y))
            {
                crate::keybinds::hacer(state, accion);
            }
        }
        return;
    }

    // La barra de título que dibujamos nosotros: sus clics no son de
    // nadie más. Va después del shell —el panel manda sobre la barra de
    // una ventana que llegue hasta él— y antes del reenvío, porque para
    // el cliente este clic no existe.
    if pulsado == ButtonState::Pressed
        && button == BTN_RIGHT
        && let Some((window, _)) = crate::decoracion::barra_en(state, punto)
    {
        state.menu_ventana_suelta = true;
        crate::keybinds::menu_ventana(state, window);
        return;
    }
    if pulsado == ButtonState::Pressed
        && button == BTN_LEFT
        && let Some((window, boton)) = crate::decoracion::barra_en(state, punto)
    {
        state.enfocar(&window);
        match boton {
            Some(boton) => crate::decoracion::pulsar(state, &window, boton),
            // Fuera de los botones, la barra es para agarrar la
            // ventana; y dos clics seguidos, para maximizarla.
            None if doble_clic_en_barra(state) => {
                state.alternar_maximizada(&window);
            }
            None => {
                state.arrastrar_ventana(&window, crate::ventanas::Modo::Mover, false);
            }
        }
        state.needs_redraw = true;
        return;
    }

    // El escritorio pelado: aquí no hay ventana ni barra ni shell, así
    // que el clic es de los iconos. Va lo último de todo porque el
    // escritorio está **detrás** de todo lo demás.
    if pulsado == ButtonState::Pressed
        && button == BTN_LEFT
        && state.space.element_under(punto).is_none()
        && pulsar_escritorio(state, punto)
    {
        state.needs_redraw = true;
        return;
    }
    // Y el mismo escritorio con el botón derecho: el menú de «Nueva
    // carpeta» y «Eliminar». El soltar que sigue se traga con el mismo
    // `menu_ventana_suelta` que usa el menú de la barra de título, dos
    // líneas más abajo de este bloque.
    if pulsado == ButtonState::Pressed
        && button == BTN_RIGHT
        && state.space.element_under(punto).is_none()
        && pulsar_derecho_escritorio(state, punto)
    {
        state.menu_ventana_suelta = true;
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
            time,
        },
    );
    pointer.frame(state);
}

/// Mueve el cursor a un punto lógico, lo confina al escritorio y reenvía el
/// movimiento al cliente que haya debajo.
pub fn set_pointer(state: &mut BookosComp, destino: Point<f64, Logical>, time: u32) {
    // No se confina al rectángulo envolvente: con una pantalla arriba y otra a
    // la derecha ese rectángulo contiene una esquina vacía donde el cursor
    // desaparecería. Se proyecta sobre la salida real más cercana y por eso
    // también funcionan coordenadas negativas (monitores a izquierda/arriba).
    let geometrías: Vec<_> = state
        .space
        .outputs()
        .filter_map(|o| state.space.output_geometry(o))
        .collect();
    let anterior = state.pointer_location;
    let delta_fisico = destino - anterior;
    let mut location = confinar_a_salidas(destino, &geometrías);

    // Juegos, máquinas virtuales y escritorios remotos usan
    // pointer-constraints junto con relative-pointer. Publicar esos globales y
    // activar la restricción no basta: el compositor tiene que obedecerla al
    // calcular la nueva posición. Antes el protocolo contestaba «locked» pero
    // el cursor seguía escapando de la ventana, dejando a VirtualBox con una
    // mitad de la captura (teclado o ratón) en un estado incoherente.
    let foco_anterior = state.pointer.current_focus().and_then(|surface| {
        state
            .surface_under(anterior)
            .filter(|(bajo, _)| *bajo == surface)
    });
    let pointer = state.pointer.clone();
    let mut bloqueado = false;
    if let Some((surface, origen)) = foco_anterior.as_ref() {
        let decision = with_pointer_constraint(surface, &pointer, |constraint| {
            let constraint = constraint.filter(|c| c.is_active())?;
            match &*constraint {
                PointerConstraint::Locked(_) => Some((true, false)),
                PointerConstraint::Confined(_) => {
                    let dentro = match constraint.region() {
                        Some(region) => {
                            let local = location - *origen;
                            region.contains((local.x.floor() as i32, local.y.floor() as i32))
                        }
                        // Sin región explícita, la propia superficie es la
                        // región. El hit-test incluye su input_region.
                        None => state
                            .surface_under(location)
                            .is_some_and(|(bajo, _)| bajo == *surface),
                    };
                    Some((false, dentro))
                }
            }
        });
        match decision {
            Some((true, _)) => {
                bloqueado = true;
                location = anterior;
            }
            Some((false, false)) => location = anterior,
            _ => {}
        }
    }
    state.pointer_location = location;
    // Mover el cursor **es** un cambio en pantalla. Sin esto el puntero solo se
    // repinta cuando algo más provoca un frame, y se arrastra a tirones.
    state.needs_redraw = true;

    // Con una ventana agarrada, el movimiento es del compositor: ni el shell ni
    // el cliente lo ven. Reenviarlo mientras arrastras una terminal la deja
    // creyendo que estás seleccionando texto.
    if state.seguir_arrastre() {
        state.space.refresh();
        let fallback = state.output_scale();
        state.broadcast_preferred_scale(fallback);
        return;
    }

    // Y con la banda elástica o unos iconos agarrados, igual: el gesto es del
    // escritorio y no llega a ningún cliente.
    if seguir_arrastre_escritorio(state, location) {
        return;
    }

    if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
        let celda = state
            .shell
            .as_ref()
            .and_then(|s| s.conmutador_en(location.x, location.y));
        if let (Some(i), Some(shell)) = (celda, state.shell.as_mut()) {
            state.needs_redraw |= shell.conmutador_elegir(i);
        }
        // Es una capa modal: lo de debajo no recibe hover mientras elegimos.
        let pointer = state.pointer.clone();
        pointer.motion(
            state,
            None,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(state);
        return;
    }

    // Con la capa de captura abierta el ratón es suyo entero: se está
    // encuadrando, y ni el dock ni la ventana de debajo tienen nada que decir.
    if state.shell.as_ref().is_some_and(|s| s.hay_captura()) {
        if let Some(shell) = state.shell.as_mut() {
            shell.captura_puntero(location.x, location.y);
        }
        state.needs_redraw = true;
        return;
    }
    if crate::escritorios::mover_en_vista(state) {
        state.needs_redraw = true;
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.puntero(location.x, location.y);
    }
    // El cursor pegado a un borde trae de vuelta a la barra que se apartó. Va
    // en físicos porque la franja sensible se mide contra la pantalla.
    //
    // Dos cosas que antes estaban mal y son la misma: las barras están en la
    // **principal**, así que el punto hay que acotarlo a ella y escalarlo con
    // **su** escala. Aquí se usaba `output_scale()`, que es la del monitor bajo
    // el puntero, contra un `self.screen` que son físicos de la principal —con
    // dos escalas distintas la comparación es sencillamente falsa—, y no se
    // miraba la x, así que llevar el ratón al borde de **cualquier** monitor
    // sacaba la barra oculta de la principal.
    let mut repintar = false;
    if let Some(shell) = state.shell.as_ref() {
        let (ancho, alto) = shell.area_principal();
        let escala = shell.escala_principal();
        let dentro =
            location.x >= 0.0 && location.y >= 0.0 && location.x < ancho && location.y < alto;
        if dentro {
            let fisico = (location.x * escala, location.y * escala);
            for cual in [crate::shell::Barra::Panel, crate::shell::Barra::Dock] {
                if let Some(shell) = state.shell.as_mut() {
                    repintar |= shell.reclamada(cual, fisico);
                }
            }
        }
    }
    if repintar {
        state.needs_redraw = true;
    }

    // El botón de la barra de título bajo el cursor. Repinta solo cuando
    // cambia: mover el ratón por una barra sin salir del mismo botón no tiene
    // por qué costar un rasterizado.
    if crate::decoracion::señalar(state, location) {
        state.needs_redraw = true;
    }

    let focus = if bloqueado {
        foco_anterior
    } else {
        state.surface_under(location)
    };
    let pointer = state.pointer.clone();
    // Un puntero bloqueado no cambia sus coordenadas absolutas; el cliente
    // recibe solo el movimiento relativo. Mandarle además `wl_pointer.motion`
    // contradice el bloqueo que acabamos de confirmar.
    if !bloqueado {
        pointer.motion(
            state,
            focus.clone(),
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        // Una restricción persistente se desactiva al perder el foco (por
        // ejemplo, al abrir la capa de captura). Al volver a entrar en la
        // superficie hay que activarla de nuevo; `new_constraint` no vuelve a
        // llamarse porque el objeto del cliente sigue siendo el mismo.
        if let Some((surface, origen)) = focus.as_ref() {
            with_pointer_constraint(surface, &pointer, |constraint| {
                let Some(constraint) = constraint.filter(|c| !c.is_active()) else {
                    return;
                };
                let local = location - *origen;
                let dentro = constraint.region().is_none_or(|region| {
                    region.contains((local.x.floor() as i32, local.y.floor() as i32))
                });
                if dentro {
                    constraint.activate();
                }
            });
        }
    }
    pointer.relative_motion(
        state,
        focus,
        &RelativeMotionEvent {
            // La posición puede quedarse quieta por un lock, un borde o una
            // región confinada; el movimiento físico no. Ese delta es justo lo
            // que consumen una VM o un juego para mover su cursor interno.
            delta: delta_fisico,
            delta_unaccel: delta_fisico,
            utime: u64::from(time) * 1_000,
        },
    );
    pointer.frame(state);
}

fn confinar_a_salidas(
    destino: Point<f64, Logical>,
    salidas: &[smithay::utils::Rectangle<i32, Logical>],
) -> Point<f64, Logical> {
    let dentro = |r: &smithay::utils::Rectangle<i32, Logical>| {
        destino.x >= r.loc.x as f64
            && destino.y >= r.loc.y as f64
            && destino.x < (r.loc.x + r.size.w) as f64
            && destino.y < (r.loc.y + r.size.h) as f64
    };
    if salidas.iter().any(dentro) {
        return destino;
    }

    salidas
        .iter()
        .filter(|r| r.size.w > 0 && r.size.h > 0)
        .map(|r| {
            let x = destino
                .x
                .clamp(r.loc.x as f64, (r.loc.x + r.size.w - 1) as f64);
            let y = destino
                .y
                .clamp(r.loc.y as f64, (r.loc.y + r.size.h - 1) as f64);
            let distancia = (destino.x - x).powi(2) + (destino.y - y).powi(2);
            (distancia, Point::from((x, y)))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, p)| p)
        .unwrap_or_else(|| Point::from((0.0, 0.0)))
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
    let punto = state.pointer_location;
    if !es_doble_clic(state) {
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

/// ¿Este clic llega lo bastante seguido y lo bastante cerca del anterior?
///
/// Apunta el clic actual pase lo que pase: dos clics lentos no son un doble,
/// pero el segundo sí puede ser el primero de otro par.
fn es_doble_clic(state: &mut BookosComp) -> bool {
    let ahora = std::time::Instant::now();
    let punto = state.pointer_location;
    let previo = state.ultimo_clic.replace((ahora, punto));
    previo.is_some_and(|(t, p)| {
        ahora.duration_since(t) <= DOBLE_CLIC
            && (p.x - punto.x).abs() <= DOBLE_CLIC_RADIO
            && (p.y - punto.y).abs() <= DOBLE_CLIC_RADIO
    })
}

/// El doble clic sobre **nuestra** barra: no hace falta mirar dónde cae dentro
/// de la ventana, porque la barra entera es zona de título.
fn doble_clic_en_barra(state: &mut BookosComp) -> bool {
    if !es_doble_clic(state) {
        return false;
    }
    // El tercer clic no vuelve a disparar: se olvida el par ya usado.
    state.ultimo_clic = None;
    true
}

/// Lo que hay que alejarse del punto de pulsación para que agarrar un icono
/// cuente como arrastrarlo, en píxeles lógicos.
///
/// Cuatro es el mismo umbral que usa el gestor de ventanas para distinguir un
/// clic de un tirón. Sin umbral, una mano poco firme recoloca el icono cada vez
/// que lo selecciona, y esa posición se guarda en disco.
const UMBRAL_ARRASTRE: f64 = 4.0;

/// Un clic sobre el escritorio pelado: abrir, seleccionar o empezar un gesto.
///
/// Devuelve `true` si el clic era suyo, que lo es siempre que haya shell: el
/// escritorio es el último que mira, y por debajo de él no hay nadie.
fn pulsar_escritorio(state: &mut BookosComp, punto: Point<f64, Logical>) -> bool {
    // Los iconos viven en la principal, igual que el resto del shell. El
    // hit-test de un icono ya se acota solo —ningún rectángulo cae fuera—, pero
    // el hueco vacío no: sin esto, un clic en el escritorio pelado de la
    // pantalla de al lado arrancaba una banda elástica en la principal.
    if !state
        .shell
        .as_ref()
        .is_some_and(|s| s.escritorio_alcanza(punto.x, punto.y))
    {
        return false;
    }
    // Se apunta el clic pase lo que pase, aunque no sea doble: el siguiente
    // puede serlo con este de pareja.
    let doble = es_doble_clic(state);
    let ctrl = modificadores(state).ctrl;
    let icono = state
        .shell
        .as_ref()
        .and_then(|s| s.escritorio_en(punto.x, punto.y));

    match icono {
        Some(i) if doble => {
            // El tercer clic no vuelve a abrir lo mismo.
            state.ultimo_clic = None;
            if let Some(accion) = state.shell.as_ref().and_then(|s| s.escritorio_abrir(i)) {
                crate::keybinds::hacer(state, accion);
            }
        }
        Some(i) => {
            if let Some(shell) = state.shell.as_mut() {
                // Pulsar sobre algo que **ya** estaba seleccionado no deshace la
                // selección: es lo que permite arrastrar varios iconos de una
                // vez, como en cualquier gestor de archivos.
                let ya = shell
                    .escritorio_seleccion()
                    .get(i)
                    .copied()
                    .unwrap_or(false);
                if ctrl || !ya {
                    shell.escritorio_seleccionar(Some(i), ctrl);
                }
            }
            state.arrastre_escritorio = Some(ArrastreEscritorio::Iconos {
                origen: punto,
                movido: false,
            });
        }
        None => {
            let previa = match ctrl {
                true => state
                    .shell
                    .as_ref()
                    .map(|s| s.escritorio_seleccion())
                    .unwrap_or_default(),
                false => Vec::new(),
            };
            if !ctrl && let Some(shell) = state.shell.as_mut() {
                shell.escritorio_seleccionar(None, false);
            }
            state.arrastre_escritorio = Some(ArrastreEscritorio::Banda {
                origen: punto,
                previa,
            });
        }
    }
    true
}

/// El clic derecho sobre el escritorio pelado: el menú de «Nueva carpeta» y
/// «Eliminar».
///
/// Reutiliza `MenuDock::ventana_en` —el mismo menú genérico que ya usa el
/// clic derecho de una barra de título, pero anclado al punto del clic en vez
/// de centrado— en vez de montar una emergente propia: montar una entera para
/// dos entradas es más superficie de la que hace falta.
///
/// `true` si el clic era del escritorio y no debe llegar a nadie más.
fn pulsar_derecho_escritorio(state: &mut BookosComp, punto: Point<f64, Logical>) -> bool {
    if !state
        .shell
        .as_ref()
        .is_some_and(|s| s.escritorio_alcanza(punto.x, punto.y))
    {
        return false;
    }
    // Clic derecho sobre un icono que no estaba seleccionado: lo selecciona
    // solo, como el clic izquierdo. Sobre uno que ya lo estaba, se deja la
    // selección tal cual, para poder eliminar varios a la vez.
    let icono = state
        .shell
        .as_ref()
        .and_then(|s| s.escritorio_en(punto.x, punto.y));
    if let Some(i) = icono {
        let ya = state
            .shell
            .as_ref()
            .map(|s| s.escritorio_seleccion())
            .and_then(|sel| sel.get(i).copied())
            .unwrap_or(false);
        if !ya && let Some(shell) = state.shell.as_mut() {
            shell.escritorio_seleccionar(Some(i), false);
        }
    }
    let hay_seleccion = state
        .shell
        .as_ref()
        .is_some_and(|s| s.escritorio_seleccion().iter().any(|&s| s));
    let mut opciones = vec![(
        "Nueva carpeta".to_string(),
        bookos_shell::Accion::EscritorioNuevaCarpeta,
    )];
    if hay_seleccion {
        opciones.push((
            "Eliminar".to_string(),
            bookos_shell::Accion::EscritorioEliminarSeleccion,
        ));
    }
    // Sobre un icono, el menú sale debajo de él, no encima tapando lo que se
    // acaba de seleccionar. En hueco vacío, junto al propio clic.
    let origen = icono
        .and_then(|i| state.shell.as_ref().and_then(|s| s.escritorio_rect(i)))
        .map(|(rx, ry, _, rh)| (rx, ry + rh))
        .unwrap_or((punto.x as f32, punto.y as f32));
    if let Some(shell) = state.shell.as_mut() {
        shell.menu_ventana_en(opciones, origen);
    }
    state.needs_redraw = true;
    true
}

/// Mueve la banda elástica o los iconos agarrados. `true` si el movimiento era
/// del escritorio y no debe llegar a nadie más.
fn seguir_arrastre_escritorio(state: &mut BookosComp, location: Point<f64, Logical>) -> bool {
    // Se saca y se vuelve a poner porque el gesto y el shell viven los dos en
    // `state`: con el gesto prestado no se puede tocar el shell.
    let Some(arrastre) = state.arrastre_escritorio.take() else {
        return false;
    };
    let arrastre = match arrastre {
        ArrastreEscritorio::Banda { origen, previa } => {
            let rect = (
                origen.x.min(location.x),
                origen.y.min(location.y),
                (location.x - origen.x).abs(),
                (location.y - origen.y).abs(),
            );
            if let Some(shell) = state.shell.as_mut() {
                shell.escritorio_banda(rect, &previa);
            }
            state.needs_redraw = true;
            ArrastreEscritorio::Banda { origen, previa }
        }
        ArrastreEscritorio::Iconos { origen, movido } => {
            let delta = (location.x - origen.x, location.y - origen.y);
            let movido = movido || delta.0.abs().max(delta.1.abs()) >= UMBRAL_ARRASTRE;
            if movido {
                if let Some(shell) = state.shell.as_mut() {
                    shell.escritorio_arrastrar(delta);
                }
                state.needs_redraw = true;
            }
            ArrastreEscritorio::Iconos { origen, movido }
        }
    };
    state.arrastre_escritorio = Some(arrastre);
    true
}

/// Cierra el gesto del escritorio: quita la banda o deja los iconos en su celda.
fn soltar_escritorio(state: &mut BookosComp) -> bool {
    let Some(arrastre) = state.arrastre_escritorio.take() else {
        return false;
    };
    if let Some(shell) = state.shell.as_mut() {
        match arrastre {
            ArrastreEscritorio::Banda { .. } => {
                shell.escritorio_quitar_banda();
            }
            ArrastreEscritorio::Iconos { .. } => {
                shell.escritorio_soltar();
            }
        }
    }
    state.needs_redraw = true;
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

#[cfg(test)]
mod tests {
    use super::*;
    use bookos_shell::conmutador::Modo;
    use smithay::input::keyboard::ModifiersState;

    #[test]
    fn el_cursor_recorre_salidas_con_coordenadas_negativas() {
        let salidas = [
            smithay::utils::Rectangle::new((-1280, 0).into(), (1280, 1024).into()),
            smithay::utils::Rectangle::new((0, 0).into(), (1920, 1080).into()),
        ];
        assert_eq!(
            confinar_a_salidas((-400.0, 500.0).into(), &salidas),
            (-400.0, 500.0).into()
        );
        assert_eq!(
            confinar_a_salidas((-2000.0, 500.0).into(), &salidas),
            (-1280.0, 500.0).into()
        );
    }

    #[test]
    fn el_cursor_no_cae_en_huecos_del_escritorio_virtual() {
        let salidas = [
            smithay::utils::Rectangle::new((0, 0).into(), (1920, 1080).into()),
            smithay::utils::Rectangle::new((1920, 500).into(), (1280, 1024).into()),
        ];
        // A la derecha de la principal pero por encima de la secundaria hay un
        // hueco; se conserva el cursor en el borde real más próximo.
        assert_eq!(
            confinar_a_salidas((2000.0, 100.0).into(), &salidas),
            (1919.0, 100.0).into()
        );
    }

    #[test]
    fn cada_selector_se_cierra_solo_con_su_modificador() {
        let alt = ModifiersState {
            alt: true,
            ..Default::default()
        };
        assert!(!modificador_conmutador_suelto(Modo::Aplicaciones, &alt));
        assert!(modificador_conmutador_suelto(Modo::Ventanas, &alt));

        let meta = ModifiersState {
            logo: true,
            ..Default::default()
        };
        assert!(modificador_conmutador_suelto(Modo::Aplicaciones, &meta));
        assert!(!modificador_conmutador_suelto(Modo::Ventanas, &meta));
    }
}
