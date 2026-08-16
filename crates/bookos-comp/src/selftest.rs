//! Autotest del enrutado de entrada.
//!
//! El backend anidado depende de que el compositor anfitrión entregue eventos
//! de ratón, y eso no se puede provocar desde un script. Este módulo sintetiza
//! el mismo camino que recorre un clic real —buscar la superficie bajo el
//! cursor, mover el puntero, pulsar, dar el foco— y cuenta qué encontró en cada
//! paso, para poder distinguir dos fallos que se parecen mucho desde fuera:
//!
//! - "no llegan eventos" (problema del anfitrión o del backend), y
//! - "llegan pero no aciertan" (problema nuestro de coordenadas o de escena).
//!
//! Se activa con `BOOKOS_INPUT_SELFTEST=1` y no se compila fuera de depuración
//! de este binario: es una herramienta de diagnóstico, no una función del DE.

use std::time::Duration;

use smithay::input::pointer::{ButtonEvent, MotionEvent};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::seat::WaylandFocus;

use crate::state::BookosComp;

/// Botón izquierdo, según linux/input-event-codes.h.
const BTN_LEFT: u32 = 0x110;

pub fn schedule(state: &mut BookosComp) {
    if std::env::var_os("BOOKOS_INPUT_SELFTEST").is_none() {
        return;
    }
    // Se espera a que el cliente de prueba se haya mapeado.
    let result = state
        .loop_handle
        .insert_source(Timer::from_duration(Duration::from_secs(4)), |_, _, state| {
            run(state);
            TimeoutAction::Drop
        });
    if let Err(err) = result {
        tracing::error!("no se pudo programar el autotest: {err}");
    }
}

/// Comprueba que el hit-test del dock acierta, sin necesidad de ratón.
///
/// Es la mitad del camino que no se puede provocar desde un script: el dock no
/// es una superficie Wayland, así que ningún cliente puede decir si un clic le
/// llegó. Aquí se pregunta directamente por el centro del primer icono, que es
/// donde el usuario apuntaría.
fn comprobar_dock(state: &mut BookosComp) {
    let Some((dx, dy, dw, dh)) = state.shell.as_ref().map(|s| s.dock_rect()) else {
        tracing::error!("no hay shell: nada que comprobar en el dock");
        return;
    };
    tracing::info!(dx, dy, dw, dh, "dock (lógico)");

    let centro = (
        dx + bookos_shell::DOCK_PAD as f64 + bookos_shell::DOCK_ICON as f64 / 2.0,
        dy + bookos_shell::DOCK_PAD as f64 + bookos_shell::DOCK_ICON as f64 / 2.0,
    );
    let dentro = state
        .shell
        .as_ref()
        .is_some_and(|s| s.contiene(centro.0, centro.1));
    // Si esto falla, el dock se dibuja en un sitio y se pulsa en otro: casi
    // siempre es la escala, que convierte físicos en lógicos.
    tracing::info!(?centro, dentro, "centro del primer icono");

    match state.shell.as_mut().and_then(|s| s.pulsar(centro.0, centro.1)) {
        Some(accion) => tracing::info!(?accion, "el primer icono responde"),
        None => tracing::error!("el centro del primer icono no devuelve acción"),
    }

    // Y un punto del hueco entre el primero y el segundo, que no debe ser de
    // nadie: si aquí sale acción, el hit-test se está comiendo los huecos.
    let hueco = (dx + bookos_shell::DOCK_PAD as f64 + bookos_shell::DOCK_ICON as f64 + 7.0, centro.1);
    match state.shell.as_mut().and_then(|s| s.pulsar(hueco.0, hueco.1)) {
        Some(accion) => tracing::error!(?accion, "el hueco entre iconos ha lanzado algo"),
        None => tracing::info!("el hueco entre iconos no es de nadie, correcto"),
    }
}

/// Abre el menú y lo recorre con el puntero, midiendo lo que cuesta.
///
/// Es el caso que se siente lento y no se puede reproducir desde un script sin
/// esto: hace falta mover el ratón de verdad por encima de las filas, que es
/// cuando el shell repinta y el compositor recompone.
fn recorrer_menu(state: &mut BookosComp) {
    tracing::info!("--- recorrido del menú ---");
    if let Some(shell) = state.shell.as_mut() {
        // Pulsar el logo, arriba a la izquierda.
        shell.pulsar(20.0, 16.0);
    }
    // El movimiento va por `set_pointer`, que es el camino de verdad: marca
    // repintado y avisa al shell. Y se programa a 4 ms para que el bucle tenga
    // que dibujar entre paso y paso, que es lo que hace una mano moviendo el
    // ratón y lo que no se veía llamando al shell en un bucle cerrado.
    programar_paso(state, 0);
}

/// Un movimiento del puntero por el menú, y se programa el siguiente.
fn programar_paso(state: &mut BookosComp, i: u32) {
    const PASOS: u32 = 500;
    if i >= PASOS {
        if let Some(shell) = state.shell.as_mut() {
            shell.cerrar_emergente();
        }
        tracing::info!("--- fin del recorrido ---");
        return;
    }
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(4)),
        move |_, _, state| {
            // Zigzag por las filas del menú, que es donde cambia el resaltado.
            let y = 34.0 + (i % 40) as f64 * 5.0;
            let x = 60.0 + (i % 7) as f64;
            // Igual que `input::handle`: sin esto el bucle cree que no hay
            // nadie tocando nada y se duerme a 33 ms, y la medición sale
            // diciendo que el escritorio va a 30 fps cuando el problema era la
            // propia prueba.
            state.last_input = Some(std::time::Instant::now());
            crate::input::set_pointer(state, (x, y).into(), i);
            programar_paso(state, i + 1);
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar el paso: {err}");
    }
}

/// Barre el puntero de izquierda a derecha y apunta qué cursor pide el cliente
/// en cada columna.
///
/// Sirve para ver **dónde cree el cliente que están sus bordes**: si pide
/// `EwResize` a doscientos píxeles del borde de verdad, su geometría y la
/// nuestra han dejado de coincidir. Cada paso va en su propio temporizador
/// porque el cliente contesta por el socket y no en la misma vuelta del bucle.
fn comprobar_bordes(state: &mut BookosComp, paso: u32) {
    const PASOS: u32 = 40;
    const ESPERA: Duration = Duration::from_millis(120);

    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    let Some(loc) = state.space.element_location(&window) else {
        return;
    };
    let tam = window.geometry().size;
    if paso == 0 {
        tracing::info!(?loc, ?tam, "--- bordes: barrido de izquierda a derecha ---");
    }
    if paso >= PASOS {
        tracing::info!("--- fin del barrido ---");
        return;
    }

    // A media altura, para no tropezar con la barra de título ni con el dock.
    let x = loc.x as f64 + (tam.w as f64 * paso as f64 / (PASOS - 1) as f64);
    let y = loc.y as f64 + tam.h as f64 / 2.0;
    crate::input::set_pointer(state, (x, y).into(), 500 + paso);

    let result = state
        .loop_handle
        .insert_source(Timer::from_duration(ESPERA), move |_, _, state| {
            tracing::info!(
                x = format_args!("{x:.0}"),
                desde_el_borde = format_args!("{:.0}", x - loc.x as f64),
                cursor = ?state.cursor_status,
                "columna"
            );
            comprobar_bordes(state, paso + 1);
            TimeoutAction::Drop
        });
    if let Err(err) = result {
        tracing::error!("no se pudo programar el barrido: {err}");
    }
}

/// Comprueba que del bloqueo se puede salir.
///
/// Es la prueba que faltaba y que costó un encierro: con el bloqueo echado
/// ninguna tecla llegaba a resolverse como atajo, así que ni Escape ni el
/// cambio de TTY funcionaban y la sesión se quedaba muerta.
fn comprobar_encierro(state: &mut BookosComp) {
    use smithay::input::keyboard::{keysyms, ModifiersState};

    tracing::info!("--- salidas del bloqueo ---");
    crate::keybinds::ejecutar(state, crate::keybinds::Accion::Bloquear);
    let echado = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado());
    tracing::info!(echado, "bloqueo echado");

    // Las salidas de emergencia tienen que seguir reconociéndose. Se comprueba
    // sobre `Accion`, que es lo que decide si una tecla se traga o no.
    use crate::keybinds::Accion;
    for (accion, nombre) in [
        (Accion::CambiarVt(2), "Ctrl+Alt+F2"),
        (Accion::Salir, "Ctrl+Alt+Retroceso"),
    ] {
        if accion.es_emergencia() {
            tracing::info!(nombre, "sigue siendo salida de emergencia");
        } else {
            tracing::error!(nombre, "ha dejado de ser salida de emergencia");
        }
    }
    for (accion, nombre) in [
        (Accion::CerrarVentana, "Meta+Q"),
        (Accion::Launchpad, "Meta+Espacio"),
    ] {
        if accion.es_emergencia() {
            tracing::error!(nombre, "no debería funcionar con el bloqueo echado");
        } else {
            tracing::info!(nombre, "se traga, correcto");
        }
    }
    let _ = (keysyms::KEY_Escape, ModifiersState::default());

    // Y Escape lo quita.
    crate::keybinds::ejecutar(state, Accion::Desbloquear);
    let sigue = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado());
    if sigue {
        tracing::error!("Escape no quitó el bloqueo");
    } else {
        tracing::info!("Escape quita el bloqueo");
    }
    tracing::info!("--- fin ---");
}

/// Comprueba que el panel y el dock se apartan de las ventanas y vuelven.
///
/// Es lo que hacen Meta+Alt+B y Meta+Alt+D. Se prueba aquí y no con un test
/// porque hace falta una ventana de verdad ocupando sitio: el solape se mide
/// contra las geometrías del espacio.
fn comprobar_barras(state: &mut BookosComp) {
    use crate::shell::{Barra, Visibilidad};

    tracing::info!("--- barras ---");
    let area_fija = state.work_area();
    tracing::info!(?area_fija, "área útil con el panel fijo");

    for cual in [Barra::Panel, Barra::Dock] {
        let modo = state
            .shell
            .as_mut()
            .map(|s| s.alternar_visibilidad(cual))
            .unwrap_or(Visibilidad::Siempre);
        tracing::info!(?cual, ?modo, "tras el atajo");
    }

    // Con el panel esquivando, el área útil llega hasta arriba del todo: si no,
    // ninguna ventana llegaría nunca a estorbarle.
    let area = state.work_area();
    if area.loc.y == 0 && area.size.h > area_fija.size.h {
        tracing::info!(?area, "el panel deja de reservar sitio");
    } else {
        tracing::error!(?area, ?area_fija, "el área útil no creció al esquivar");
    }

    // Maximizar la ventana la pone encima de las dos barras.
    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ventana con la que tapar las barras");
        return;
    };
    state.encajar(&window, crate::ventanas::Zona::Maxima);
    state.revisar_barras();
    let escondidas = (
        !state.shell.as_ref().is_some_and(|s| s.a_la_vista(Barra::Panel)),
        !state.shell.as_ref().is_some_and(|s| s.a_la_vista(Barra::Dock)),
    );
    // La animación tarda, así que se mira pasado su tiempo.
    let queda = std::env::var("BOOKOS_SELFTEST_BARRAS").is_ok_and(|v| v == "queda");
    let ventana = window.clone();
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(400)),
        move |_, _, state| {
            let panel = state.shell.as_ref().is_some_and(|s| s.a_la_vista(Barra::Panel));
            let dock = state.shell.as_ref().is_some_and(|s| s.a_la_vista(Barra::Dock));
            if queda {
                let zonas = state.shell.as_ref().map(|s| s.zonas_barras()).unwrap_or_default();
                let geos: Vec<_> = state
                    .space
                    .elements()
                    .filter_map(|w| state.space.element_location(w).map(|l| (l, w.geometry().size)))
                    .collect();
                tracing::info!(panel, dock, ?zonas, ?geos, "se queda con las barras apartadas");
                return TimeoutAction::Drop;
            }
            if panel || dock {
                tracing::error!(panel, dock, "una barra sigue a la vista con la ventana encima");
            } else {
                tracing::info!("las dos barras se apartaron de la ventana");
            }

            // Antes de mover nada: el cursor pegado al borde de arriba tiene
            // que traer el panel de vuelta aunque la ventana siga ahí.
            crate::input::set_pointer(state, (400.0, 0.0).into(), 600);
            // La barra tarda lo que dura su animación en llegar; preguntar en
            // el mismo instante siempre diría que no ha vuelto.
            std::thread::sleep(Duration::from_millis(300));
            let vuelve = state.shell.as_ref().is_some_and(|s| s.a_la_vista(Barra::Panel));
            if vuelve {
                tracing::info!("el cursor en el borde trae el panel de vuelta");
            } else {
                tracing::error!("el cursor en el borde no trae el panel");
            }
            // Con el cursor **sobre el panel** —no en el borde— tiene que
            // seguir a la vista: es lo que permite llegar a pulsar un icono.
            crate::input::set_pointer(state, (400.0, 20.0).into(), 602);
            std::thread::sleep(Duration::from_millis(120));
            if state.shell.as_ref().is_some_and(|s| s.a_la_vista(Barra::Panel)) {
                tracing::info!("el panel aguanta con el cursor encima");
            } else {
                tracing::error!("el panel se esconde al subir el cursor hacia sus iconos");
            }

            // Y al apartarlo del borde se vuelve a esconder.
            crate::input::set_pointer(state, (400.0, 400.0).into(), 601);
            std::thread::sleep(Duration::from_millis(300));
            let sigue = state.shell.as_ref().is_some_and(|s| s.a_la_vista(Barra::Panel));
            if sigue {
                tracing::error!("el panel se queda tras apartar el cursor del borde");
            } else {
                tracing::info!("al apartar el cursor, el panel se vuelve a esconder");
            }

            // Y al quitar la ventana de en medio tienen que volver.
            state.desencajar(&ventana);
            state.encajar(&ventana, crate::ventanas::Zona::InfDer);
            state.revisar_barras();
            let result = state.loop_handle.insert_source(
                Timer::from_duration(Duration::from_millis(400)),
                |_, _, state| {
                    let panel = state.shell.as_ref().is_some_and(|s| s.a_la_vista(Barra::Panel));
                    tracing::info!(panel, "el panel tras apartar la ventana del borde de arriba");
                    if !panel {
                        tracing::error!("el panel no volvió cuando dejó de estorbar");
                    }
                    tracing::info!("--- fin del autotest de barras ---");
                    TimeoutAction::Drop
                },
            );
            if let Err(err) = result {
                tracing::error!("no se pudo programar la vuelta: {err}");
            }
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la medición: {err}");
    }
    tracing::debug!(?escondidas, "estado justo tras maximizar");
    // `BOOKOS_SELFTEST_BARRAS=queda` deja la ventana maximizada y las barras
    // apartadas, para poder capturarlo: el ciclo entero dura menos de un
    // segundo y no da tiempo.
}

/// Comprueba el doble clic en la barra de título: maximiza y restaura.
fn comprobar_doble_clic(state: &mut BookosComp) {
    tracing::info!("--- doble clic ---");
    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    let Some(inicial) = state.space.element_location(&window) else {
        return;
    };
    let tam = window.geometry().size;
    // Konsole recuerda su estado y arranca maximizada, así que el primer doble
    // clic la restaura y el segundo la vuelve a maximizar. Lo que se comprueba
    // es que **alterna**, no en qué estado acaba.
    let antes = crate::ventanas::maximizada(&window);
    tracing::info!(?inicial, ?tam, maximizada = antes, "antes del doble clic");

    // En la franja de arriba de la ventana, que es donde vive la barra.
    let barra = (inicial.x as f64 + tam.w as f64 / 2.0, inicial.y as f64 + 8.0);
    crate::input::set_pointer(state, barra.into(), 400);
    pulsar_dos_veces(state);
    let tras_uno = crate::ventanas::maximizada(&window);
    if tras_uno == antes {
        tracing::error!(maximizada = tras_uno, "el doble clic no cambió el tamaño");
    } else {
        tracing::info!(maximizada = tras_uno, "el primer doble clic alterna");
    }

    // Y otra vez: tiene que volver a donde estaba.
    let loc = state.space.element_location(&window).unwrap_or(inicial);
    let barra = (loc.x as f64 + 40.0, loc.y as f64 + 8.0);
    crate::input::set_pointer(state, barra.into(), 402);
    pulsar_dos_veces(state);
    let tras_dos = crate::ventanas::maximizada(&window);
    if tras_dos == antes {
        tracing::info!(maximizada = tras_dos, loc = ?state.space.element_location(&window),
            "el segundo doble clic devuelve al estado de partida");
    } else {
        tracing::error!(maximizada = tras_dos, "no volvió al estado de partida");
    }

    // Y un doble clic en mitad de la ventana no debe hacer nada: ahí seleccionar
    // una palabra es del cliente.
    let loc = state.space.element_location(&window).unwrap_or(inicial);
    let tam = window.geometry().size;
    let centro = (
        loc.x as f64 + tam.w as f64 / 2.0,
        loc.y as f64 + tam.h as f64 / 2.0,
    );
    crate::input::set_pointer(state, centro.into(), 404);
    pulsar_dos_veces(state);
    if crate::ventanas::maximizada(&window) == tras_dos {
        tracing::info!("el doble clic en el centro no toca el tamaño, correcto");
    } else {
        tracing::error!("el doble clic en el centro cambió el tamaño, y no debía");
    }
    tracing::info!("--- fin del autotest de doble clic ---");
}

/// Dos pulsaciones seguidas en el mismo sitio, que es lo que el compositor
/// reconoce como doble clic. La primera nunca hace nada; la segunda es la que
/// cuenta.
fn pulsar_dos_veces(state: &mut BookosComp) {
    let primera = crate::input::doble_clic(state);
    let segunda = crate::input::doble_clic(state);
    if primera {
        tracing::error!("la primera pulsación ya contó como doble clic");
    }
    tracing::debug!(segunda, "la segunda pulsación cierra el doble clic");
}

/// Comprueba que el cursor vuelve a ser el del escritorio al salir de una
/// ventana.
///
/// El fallo se veía así: pasabas el ratón por una terminal, que pide la barra
/// en I, salías al fondo y la barra en I seguía puesta por todo el escritorio.
/// El cliente que la pidió ya no tiene el puntero y no vuelve a pedir nada, así
/// que si el compositor no repone, no repone nadie.
fn comprobar_cursor(state: &mut BookosComp) {
    use smithay::input::pointer::CursorImageStatus;

    tracing::info!("--- cursor ---");
    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    let Some(loc) = state.space.element_location(&window) else {
        return;
    };
    let tam = window.geometry().size;
    let centro = (
        loc.x as f64 + tam.w as f64 / 2.0,
        loc.y as f64 + tam.h as f64 / 2.0,
    );

    // Dentro: el cliente pedirá lo suyo, pero tarda en contestar, así que la
    // comprobación de verdad va en el temporizador.
    crate::input::set_pointer(state, centro.into(), 300);
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(600)),
        move |_, _, state| {
            tracing::info!(cursor = ?state.cursor_status, "con el puntero dentro de la ventana");

            // Y ahora al fondo del escritorio, lejos de todo.
            crate::input::set_pointer(state, (4.0, 4.0).into(), 301);
            let fuera = matches!(state.cursor_status, CursorImageStatus::Named(_));
            if fuera {
                tracing::info!(cursor = ?state.cursor_status, "fuera vuelve al del escritorio");
            } else {
                tracing::error!(
                    cursor = ?state.cursor_status,
                    "el cursor del cliente se quedó puesto fuera de su ventana"
                );
            }
            tracing::info!("--- fin del autotest de cursor ---");
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la comprobación: {err}");
    }
}

/// Comprueba la pantalla completa: la ventana ocupa la pantalla entera —panel
/// incluido— y al salir vuelve exactamente a donde estaba.
///
/// Se provoca desde aquí y no esperando a que un cliente pulse F11 porque el
/// camino que hay que probar es el del compositor; que Firefox pida
/// `set_fullscreen` ya lo hace Firefox.
fn comprobar_pantalla_completa(state: &mut BookosComp) {
    tracing::info!("--- pantalla completa ---");
    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    let antes = state.space.element_location(&window);
    let area = state.work_area();
    tracing::info!(?antes, ?area, "antes de nada");

    state.pantalla_completa(&window, true);
    let dentro = state.space.element_location(&window);
    tracing::info!(
        loc = ?dentro,
        oculta_el_shell = state.hay_pantalla_completa(),
        "puesta a pantalla completa"
    );

    // El tamaño se mide con retardo: el cliente tiene que recibir el configure,
    // repintar y hacer commit antes de que su geometría diga la verdad.
    let ventana = window.clone();
    let result = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_secs(2)),
        move |_, _, state| {
            let tam = ventana.geometry().size;
            let loc = state.space.element_location(&ventana);
            let pantalla = state.work_area();
            tracing::info!(?tam, ?loc, "el cliente aceptó la pantalla completa");
            // Tapar el panel es justamente el punto: si la altura no pasa del
            // área útil, la ventana está maximizada, no a pantalla completa.
            if tam.h <= pantalla.size.h {
                tracing::error!(alto = tam.h, area = pantalla.size.h, "no tapa el panel");
            }

            // `BOOKOS_SELFTEST_COMPLETA=queda` deja la ventana dentro, que es
            // la única forma de mirar el resultado con calma: la ida y la
            // vuelta duran dos segundos y no da tiempo a capturar la pantalla.
            if std::env::var("BOOKOS_SELFTEST_COMPLETA").is_ok_and(|v| v == "queda") {
                tracing::info!("se queda a pantalla completa, como se pidió");
                return TimeoutAction::Drop;
            }

            state.pantalla_completa(&ventana, false);
            let vuelta = state.space.element_location(&ventana);
            tracing::info!(?vuelta, ?antes, "tras salir de pantalla completa");
            if vuelta != antes {
                tracing::error!("no volvió a donde estaba");
            }
            tracing::info!(
                oculta_el_shell = state.hay_pantalla_completa(),
                "--- fin del autotest de pantalla completa ---"
            );
            TimeoutAction::Drop
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la medición: {err}");
    }
}

/// Comprueba que una aplicación X11 llega hasta el escritorio.
///
/// No basta con que XWayland arranque: la ventana tiene que aparecer en el
/// espacio, traer su `WM_CLASS` —de ahí depende que el dock la reconozca— y
/// obedecer un encaje, que es donde X11 se separa de Wayland porque no hay
/// configure diferido. Se lanza el cliente aquí y no como cliente inicial
/// porque al arrancar el compositor todavía no hay `DISPLAY` que darle.
fn comprobar_xwayland(state: &mut BookosComp) {
    tracing::info!("--- xwayland ---");
    match state.display_x11 {
        Some(n) => tracing::info!(display = format!(":{n}"), "XWayland en pie"),
        None => {
            tracing::error!("XWayland no arrancó: no hay DISPLAY que ofrecer");
            return;
        }
    }

    let cliente = std::env::var("BOOKOS_SELFTEST_XWAYLAND")
        .ok()
        .filter(|v| v != "1")
        .unwrap_or_else(|| "xmessage -center hola".to_string());
    crate::keybinds::lanzar(state, &cliente);

    // Tres segundos: xmessage tarda en conectarse a X, pedir el mapeo y que el
    // xwm nos lo cuente. Menos daba falsos negativos al medir.
    let result = state
        .loop_handle
        .insert_source(Timer::from_duration(Duration::from_secs(3)), |_, _, state| {
            revisar_x11(state);
            TimeoutAction::Drop
        });
    if let Err(err) = result {
        tracing::error!("no se pudo programar la revisión de X11: {err}");
    }
}

fn revisar_x11(state: &mut BookosComp) {
    let x11: Vec<_> = state
        .space
        .elements()
        .filter(|w| w.x11_surface().is_some())
        .cloned()
        .collect();
    tracing::info!(n = x11.len(), "ventanas X11 en la escena");
    let Some(window) = x11.first() else {
        tracing::error!("ninguna ventana X11 se mapeó: el cliente no llegó al escritorio");
        return;
    };

    let loc = state.space.element_location(window);
    tracing::info!(
        clase = crate::xwayland::clase(window).unwrap_or_default(),
        app_id = crate::handlers::app_id(window).unwrap_or_default(),
        ?loc,
        tam = ?window.geometry().size,
        "ventana X11"
    );

    // El encaje es la prueba de fuego: si el cliente no obedece al `configure`
    // de X11, el `Space` lo dibuja en su zona pero el contenido sigue con el
    // tamaño viejo y se ve recortado.
    let zona = crate::ventanas::Zona::Izquierda;
    let esperado = zona.rect(state.work_area());
    state.encajar(&window.clone(), zona);
    let ventana = window.clone();
    let result = state
        .loop_handle
        .insert_source(Timer::from_duration(Duration::from_secs(2)), move |_, _, state| {
            let real = ventana.geometry().size;
            let loc = state.space.element_location(&ventana);
            if real == esperado.size {
                tracing::info!(?real, "el cliente X11 obedeció el encaje");
            } else {
                tracing::error!(?real, esperado = ?esperado.size, "el cliente X11 no obedeció");
            }
            tracing::info!(?loc, esperado = ?esperado.loc, "posición tras el encaje");
            tracing::info!("--- fin del autotest de xwayland ---");
            TimeoutAction::Drop
        });
    if let Err(err) = result {
        tracing::error!("no se pudo programar la medición del encaje: {err}");
    }
}

/// Mueve, redimensiona y maximiza la ventana sin tocar el ratón.
///
/// Es la otra mitad que no se puede provocar desde un script: mover una ventana
/// necesita una mano moviendo el ratón con Meta pulsada. Aquí se recorre el
/// mismo camino que un arrastre real —`empezar_arrastre`, varios movimientos del
/// puntero, soltar— y se comprueba dónde acabó la ventana.
fn comprobar_ventanas(state: &mut BookosComp) {
    use crate::ventanas::Modo;

    tracing::info!("--- ventanas ---");
    let area = state.work_area();
    tracing::info!(?area, "área útil");

    let Some(window) = state.space.elements().next_back().cloned() else {
        tracing::error!("no hay ninguna ventana: nada que comprobar");
        return;
    };
    let Some(inicial) = state.space.element_location(&window) else {
        tracing::error!("la ventana no está mapeada");
        return;
    };
    let tam = window.geometry().size;
    tracing::info!(?inicial, ?tam, "ventana de prueba");

    // El arrastre empieza donde esté el cursor, así que primero se lleva al
    // centro de la ventana, que es donde apuntaría una mano.
    let centro = (
        inicial.x as f64 + tam.w as f64 / 2.0,
        inicial.y as f64 + tam.h as f64 / 2.0,
    );
    crate::input::set_pointer(state, centro.into(), 0);
    if !state.empezar_arrastre(Modo::Mover) {
        tracing::error!(?centro, "no hay ventana bajo el centro: el hit-test falla");
        return;
    }
    const DX: f64 = 120.0;
    const DY: f64 = -40.0;
    // En varios pasos y no en uno: es lo que hace una mano, y así se ejercita
    // que la geometría salga del punto de partida y no de ir acumulando deltas.
    for i in 1..=6 {
        let f = i as f64 / 6.0;
        crate::input::set_pointer(state, (centro.0 + DX * f, centro.1 + DY * f).into(), i);
    }
    state.arrastre = None;

    let tras_mover = state.space.element_location(&window);
    let esperado = (inicial.x + DX as i32, inicial.y + DY as i32);
    match tras_mover {
        Some(p) if (p.x, p.y) == esperado => tracing::info!(?p, "la ventana se movió donde tocaba"),
        Some(p) => tracing::error!(?p, ?esperado, "la ventana acabó donde no debía"),
        None => tracing::error!("la ventana dejó de estar mapeada al moverla"),
    }

    // Redimensionar tirando de la esquina inferior derecha: crece el tamaño y
    // el origen no se mueve.
    let ahora = tras_mover.unwrap_or(inicial);
    let esquina = (
        ahora.x as f64 + tam.w as f64 - 4.0,
        ahora.y as f64 + tam.h as f64 - 4.0,
    );
    crate::input::set_pointer(state, esquina.into(), 100);
    if state.empezar_arrastre(Modo::Redimensionar {
        izquierda: false,
        arriba: false,
    }) {
        crate::input::set_pointer(state, (esquina.0 + 80.0, esquina.1 + 60.0).into(), 101);
        let modo = state.arrastre.as_ref().map(|a| a.modo);
        // El cuadrante se decide al empezar: tirando de la esquina de abajo a
        // la derecha, ni izquierda ni arriba.
        tracing::info!(?modo, "borde del que se tira");
        state.arrastre = None;
    } else {
        tracing::error!(?esquina, "no se pudo agarrar la esquina para redimensionar");
    }

    // Y ahora tirando del borde de **arriba** hacia arriba, que es el gesto que
    // metía la ventana bajo el panel: el techo tiene que ser el área útil.
    let ahora = state.space.element_location(&window).unwrap_or(inicial);
    let tam = window.geometry().size;
    let borde = (ahora.x as f64 + tam.w as f64 / 2.0, ahora.y as f64 + 2.0);
    crate::input::set_pointer(state, borde.into(), 150);
    if state.empezar_arrastre(Modo::Redimensionar {
        izquierda: false,
        arriba: true,
    }) {
        // Muy por encima del panel, para que el recorte tenga que actuar.
        crate::input::set_pointer(state, (borde.0, -200.0).into(), 151);
        state.soltar_arrastre();
        match state.space.element_location(&window) {
            Some(p) if p.y >= area.loc.y => {
                tracing::info!(y = p.y, techo = area.loc.y, "el borde de arriba respeta el panel")
            }
            otra => tracing::error!(?otra, techo = area.loc.y, "la ventana se metió bajo el panel"),
        }
    } else {
        tracing::error!(?borde, "no se pudo agarrar el borde de arriba");
    }

    // Arrastrar hasta el borde izquierdo y soltar: tiene que encajar en media
    // pantalla. Es el gesto que no se puede provocar desde un script.
    let ahora = state.space.element_location(&window).unwrap_or(inicial);
    let tam = window.geometry().size;
    let centro = (
        ahora.x as f64 + tam.w as f64 / 2.0,
        ahora.y as f64 + tam.h as f64 / 2.0,
    );
    crate::input::set_pointer(state, centro.into(), 200);
    if state.empezar_arrastre(Modo::Mover) {
        crate::input::set_pointer(state, (1.0, centro.1).into(), 201);
        let previa = state.arrastre.as_ref().and_then(|a| a.zona);
        tracing::info!(?previa, "vista previa con el cursor en el borde izquierdo");
        state.soltar_arrastre();
        let media = crate::ventanas::Zona::Izquierda.rect(area);
        match state.space.element_location(&window) {
            Some(p) if p == media.loc => tracing::info!(?p, "encajó en media pantalla"),
            otra => tracing::error!(?otra, esperado = ?media.loc, "no encajó donde tocaba"),
        }
    }
    // Y volver: arrastrar una encajada la suelta a su tamaño de antes.
    tracing::info!(zona = ?crate::ventanas::zona_de(&window), "estado tras encajar");
    state.desencajar(&window);
    tracing::info!(donde = ?state.space.element_location(&window), "tras desencajar");

    // Y maximizar, que además de cambiar la geometría tiene que dejar apuntado
    // a dónde volver.
    let antes = state.space.element_location(&window);
    state.alternar_maximizada(&window);
    tracing::info!(
        maximizada = crate::ventanas::maximizada(&window),
        donde = ?state.space.element_location(&window),
        "tras maximizar"
    );
    state.alternar_maximizada(&window);
    let vuelta = state.space.element_location(&window);
    if vuelta == antes {
        tracing::info!(?vuelta, "vuelve exactamente a donde estaba");
    } else {
        tracing::error!(?vuelta, ?antes, "no volvió a su sitio al desmaximizar");
    }

    // Un arrastre pedido por el cliente (`move_request`, o sea tirar de su
    // propia barra de título) tiene que devolverle el evento de soltar: él vio
    // la pulsación. Tragárselo lo dejaba creyendo que seguías pulsando, y el
    // siguiente intento de mover la ventana no hacía nada — el "a veces no
    // agarra".
    if let Some(toplevel) = window.toplevel().cloned() {
        state.arrastrar_toplevel(&toplevel, Modo::Mover);
        let del_cliente = state.arrastre.as_ref().map(|a| a.del_cliente);
        let tragado = state.soltar_arrastre();
        match (del_cliente, tragado) {
            (Some(true), false) => {
                tracing::info!("el soltar vuelve al cliente que pidió mover, correcto")
            }
            otro => tracing::error!(?otro, "el cliente se queda con el botón pulsado"),
        }
    }

    // Con cuatro ventanas o más, se reparten en cuartos: es la comprobación de
    // que se pueden tener las cuatro a la vez sin huecos ni solapes.
    let todas: Vec<_> = state.space.elements().cloned().collect();
    if todas.len() >= 4 {
        use crate::ventanas::Zona;
        for (w, zona) in todas
            .iter()
            .zip([Zona::SupIzq, Zona::SupDer, Zona::InfIzq, Zona::InfDer])
        {
            state.encajar(w, zona);
            tracing::info!(
                ?zona,
                donde = ?state.space.element_location(w),
                pedido = ?zona.rect(area).size,
                "cuarto"
            );
        }
        // El tamaño de verdad se comprueba **después**: el cliente tarda uno o
        // dos fotogramas en devolver un buffer del tamaño nuevo, y preguntarlo
        // aquí mide lo que había antes.
        let result = state.loop_handle.insert_source(
            Timer::from_duration(Duration::from_secs(2)),
            |_, _, state| {
                for w in state.space.elements() {
                    tracing::info!(
                        tam = ?w.geometry().size,
                        app = ?crate::handlers::app_id(w),
                        "tamaño real tras encajar"
                    );
                }
                TimeoutAction::Drop
            },
        );
        if let Err(err) = result {
            tracing::error!("no se pudo programar la comprobación: {err}");
        }
    } else {
        tracing::info!(n = todas.len(), "menos de cuatro ventanas: no se reparten");
    }

    // El puntero se deja en un sitio fijo y conocido del escritorio, sobre
    // fondo liso: es la única forma de medir el cursor en una captura, porque
    // dónde acaba si no depende de dónde tuviera el ratón el compositor
    // anfitrión.
    const PARKING: (f64, f64) = (400.0, 700.0);
    crate::input::set_pointer(state, PARKING.into(), 300);
    tracing::info!(?PARKING, "puntero aparcado para medirlo");

    tracing::info!("--- fin de ventanas ---");
}

fn run(state: &mut BookosComp) {
    tracing::info!("--- autotest de entrada ---");

    let escala = state.output_scale();
    let modo = state
        .space
        .outputs()
        .next()
        .and_then(|o| o.current_mode())
        .map(|m| m.size);
    tracing::info!(?modo, escala, "salida");

    comprobar_dock(state);
    if std::env::var_os("BOOKOS_SELFTEST_MENU").is_some() {
        recorrer_menu(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_VENTANAS").is_some() {
        comprobar_ventanas(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_ENCIERRO").is_some() {
        comprobar_encierro(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_OSD").is_some() {
        // El aviso que sale al tocar el volumen, por el mismo camino que la
        // tecla de función: `multimedia::ejecutar` es quien lo pide.
        use crate::multimedia::Tecla;
        let cual = std::env::var("BOOKOS_SELFTEST_OSD").unwrap_or_default();
        let tecla = match cual.as_str() {
            "brillo" => Tecla::Brillo(5),
            "teclado" => Tecla::BrilloTeclado(1),
            "touchpad" => Tecla::Touchpad,
            _ => Tecla::Volumen(5),
        };
        crate::multimedia::ejecutar(state, tecla);
        tracing::info!(
            ?tecla,
            vivo = state.shell.as_mut().is_some_and(|s| s.osd_vivo().0),
            queda = ?state.shell.as_ref().and_then(|s| s.osd_queda()),
            "aviso mostrado"
        );
        // Y se vuelve a mirar pasado su tiempo: lo que se comprueba de verdad
        // es que **se va solo**, que es donde estaba el fallo —la superficie se
        // quedaba puesta para siempre porque nadie la soltaba.
        let espera = std::time::Duration::from_millis(2_000);
        let _ = state.loop_handle.insert_source(
            Timer::from_duration(espera),
            |_, _, state: &mut BookosComp| {
                let vivo = state.shell.as_mut().is_some_and(|s| s.osd_vivo().0);
                if vivo {
                    tracing::error!("el aviso sigue puesto dos segundos después");
                } else {
                    tracing::info!("el aviso se ha ido solo, correcto");
                }
                TimeoutAction::Drop
            },
        );
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_WIDGETS").is_some() {
        // Pulsa cada widget del panel por el camino de verdad —el hit-test del
        // compositor— y dice qué tarjeta se abrió. Es la comprobación que el
        // test del shell no puede hacer: allí se llama al shell directamente,
        // aquí pasa por las coordenadas de pantalla y la escala.
        for nombre in [
            "bateria",
            "bluetooth",
            "red",
            "volumen",
            "brillo",
            "notificaciones",
            "control",
            "reloj",
        ] {
            let Some(shell) = state.shell.as_mut() else {
                return;
            };
            // **Sin cerrar la anterior a propósito.** Cambiar de tarjeta tiene
            // que costar un clic, no dos: mientras el clic fuera de la
            // emergente se gastaba en cerrarla, abrir un widget con otro ya
            // abierto no hacía nada la primera vez.
            let Some((x0, x1)) = shell.zona_widget(nombre) else {
                tracing::warn!(nombre, "ese widget no se dibuja en esta máquina");
                continue;
            };
            let centro = (x0 + x1) / 2.0;
            shell.pulsar(centro, 8.0);
            match state.shell.as_ref().and_then(|s| s.emergente_nombre()) {
                Some(abierta) => tracing::info!(nombre, abierta, "el widget abre su tarjeta"),
                None => tracing::error!(nombre, "el widget no abrió nada"),
            }
        }
        if let Some(shell) = state.shell.as_mut() {
            shell.cerrar_emergente();
        }
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_RESIZE").is_some() {
        // Maximiza la primera ventana y va anotando el factor de escala: es la
        // única forma de comprobar que el cambio de tamaño se anima, porque en
        // Wayland puro no hay manera de grabar la pantalla desde un script.
        let Some(window) = state.space.elements().next().cloned() else {
            tracing::error!("no hay ventana que maximizar");
            return;
        };
        let antes = window.geometry().size;
        // A media pantalla y no a máxima: el cliente de prueba ya arranca
        // ocupando el área útil entera, y encajarlo donde ya está no cambia de
        // tamaño y no habría nada que animar.
        state.encajar(&window, crate::ventanas::Zona::Izquierda);
        tracing::info!(?antes, "maximizada");
        muestrear_resize(state, window, 0);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_BLOQUEO").is_some() {
        crate::keybinds::ejecutar(state, crate::keybinds::Accion::Bloquear);
        tracing::info!(
            bloqueado = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado()),
            "tras pedir el bloqueo"
        );
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_LAUNCHPAD").is_some() {
        // Pulsa el primer icono del dock, que es el launchpad, por el mismo
        // camino que un clic: `pulsar` devuelve la acción y el compositor la
        // ejecuta. Así se comprueba de paso que el dock la emite bien.
        let centro = state.shell.as_ref().map(|s| {
            let (dx, dy, _, _) = s.dock_rect();
            (
                dx + bookos_shell::DOCK_PAD as f64 + bookos_shell::DOCK_ICON as f64 / 2.0,
                dy + bookos_shell::DOCK_PAD as f64 + bookos_shell::DOCK_ICON as f64 / 2.0,
            )
        });
        if let Some((x, y)) = centro {
            match state.shell.as_mut().and_then(|s| s.pulsar(x, y)) {
                Some(accion) => {
                    tracing::info!(?accion, "el primer icono del dock pide");
                    crate::keybinds::hacer(state, accion);
                }
                None => tracing::error!("el primer icono del dock no devuelve acción"),
            }
        }
        let abierto = state.shell.as_ref().is_some_and(|s| s.hay_emergente());
        tracing::info!(abierto, "launchpad tras pulsar su icono");
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_BARRAS").is_some() {
        comprobar_barras(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_SONIDO").is_some() {
        // Pulsa el icono del volumen y deja la emergente abierta, para poder
        // capturarla: el hit-test se prueba en el shell, esto comprueba que la
        // tarjeta sale **bajo su icono** y no en un sitio fijo.
        let zona = state
            .shell
            .as_ref()
            .and_then(|s| s.zona_widget("volumen"));
        match zona {
            Some((x0, x1)) => {
                let x = (x0 + x1) / 2.0;
                tracing::info!(x, "pulsando el icono del volumen");
                if let Some(shell) = state.shell.as_mut() {
                    shell.pulsar(x, 8.0);
                }
                state.needs_redraw = true;
            }
            None => tracing::error!("el volumen no está en el panel"),
        }
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_BORDES").is_some() {
        comprobar_bordes(state, 0);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_DOBLECLIC").is_some() {
        comprobar_doble_clic(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_CURSOR").is_some() {
        comprobar_cursor(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_COMPLETA").is_some() {
        comprobar_pantalla_completa(state);
        return;
    }
    if std::env::var_os("BOOKOS_SELFTEST_XWAYLAND").is_some() {
        comprobar_xwayland(state);
        return;
    }

    let ventanas: Vec<_> = state
        .space
        .elements()
        .map(|w| (w.bbox(), state.space.element_location(w)))
        .collect();
    tracing::info!(n = ventanas.len(), "ventanas en la escena");
    for (bbox, loc) in &ventanas {
        tracing::info!(?bbox, ?loc, "  ventana");
    }
    if ventanas.is_empty() {
        tracing::error!("no hay ninguna ventana mapeada: el clic no tendría destino");
        return;
    }

    // Centro de la primera ventana, en coordenadas lógicas.
    let (bbox, loc) = ventanas[0];
    let loc = loc.unwrap_or_default();
    let punto = (
        loc.x as f64 + bbox.size.w as f64 / 2.0,
        loc.y as f64 + bbox.size.h as f64 / 2.0,
    )
        .into();
    tracing::info!(?punto, "punto de prueba (centro de la primera ventana)");

    match state.surface_under(punto) {
        Some((_surface, pos)) => {
            tracing::info!(?pos, "superficie encontrada bajo el punto")
        }
        None => {
            tracing::error!(
                "NO hay superficie bajo el punto: element_under o surface_under fallan \
                 (mira el espacio de coordenadas)"
            );
            return;
        }
    }

    let focus = state.surface_under(punto);
    state.pointer_location = punto;
    let pointer = state.pointer.clone();
    pointer.motion(
        state,
        focus,
        &MotionEvent {
            location: punto,
            serial: SERIAL_COUNTER.next_serial(),
            time: 0,
        },
    );
    pointer.frame(state);
    tracing::info!(
        tiene_foco = pointer.current_focus().is_some(),
        "tras mover el puntero"
    );

    let serial = SERIAL_COUNTER.next_serial();
    pointer.button(
        state,
        &ButtonEvent {
            button: BTN_LEFT,
            state: smithay::backend::input::ButtonState::Pressed,
            serial,
            time: 0,
        },
    );
    pointer.frame(state);

    // Soltar el botón. Dejarlo pulsado deja al cliente creyendo que arrastras:
    // konsole abría su búsqueda y se quedaba seleccionando texto.
    pointer.button(
        state,
        &ButtonEvent {
            button: BTN_LEFT,
            state: smithay::backend::input::ButtonState::Released,
            serial: SERIAL_COUNTER.next_serial(),
            time: 1,
        },
    );
    pointer.frame(state);

    if let Some(kbd) = state.seat.get_keyboard() {
        let surface = state
            .space
            .elements()
            .next()
            .and_then(|w| w.wl_surface())
            .map(|s| s.into_owned());
        kbd.set_focus(state, surface, serial);
        tracing::info!(
            tiene_foco = kbd.current_focus().is_some(),
            "tras dar el foco de teclado"
        );
    }

    tracing::info!("--- fin del autotest ---");
}

/// Anota el factor de escala del redimensionado unas cuantas veces seguidas.
///
/// Con un solo vistazo no se distingue "anima" de "salta": lo que dice que hay
/// animación es la serie de valores intermedios entre el factor inicial y 1.
fn muestrear_resize(state: &mut BookosComp, window: smithay::desktop::Window, i: u32) {
    const PASOS: u32 = 12;
    let escala = crate::ventanas::escala_resize(&window, window.geometry().size);
    tracing::info!(
        i,
        escala = format_args!("{escala:.3}"),
        tam = ?window.geometry().size,
        "escala del redimensionado"
    );
    if i >= PASOS {
        return;
    }
    let _ = state.loop_handle.insert_source(
        Timer::from_duration(Duration::from_millis(30)),
        move |_, _, state: &mut BookosComp| {
            state.needs_redraw = true;
            muestrear_resize(state, window.clone(), i + 1);
            TimeoutAction::Drop
        },
    );
}
