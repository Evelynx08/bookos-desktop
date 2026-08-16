//! Atajos que atiende el compositor antes que nadie.
//!
//! Hasta ahora `input.rs` reenviaba **todas** las teclas al cliente con foco.
//! Eso deja la sesión en un callejón sin salida: sin cambio de TTY no se puede
//! volver a un terminal de texto ni a la sesión de KDE, y sin forma de lanzar
//! nada, cerrar la última ventana te deja delante de un escritorio que no
//! responde. Las dos cosas se arreglan aquí, en el único sitio que ve las
//! teclas antes de que salgan hacia un cliente.
//!
//! Criterio de qué entra: solo lo que **no se puede hacer de otra manera** una
//! vez el compositor tiene el KMS. El resto de atajos son cosa del escritorio y
//! llegarán con la configuración, no cableados en el compositor.

use smithay::input::keyboard::{keysyms, KeysymHandle, ModifiersState};
use smithay::utils::{IsAlive, SERIAL_COUNTER};

use crate::state::BookosComp;

/// Terminal que abre Meta+Return si `BOOKOS_TERMINAL` no dice otra cosa.
const TERMINAL: &str = "konsole";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accion {
    /// Ctrl+Alt+F1..F12 — saltar a otro terminal virtual.
    CambiarVt(i32),
    /// Meta+Return — abrir un terminal dentro de la sesión.
    Terminal,
    /// Meta+Q — cerrar la ventana con foco (petición educada al cliente).
    CerrarVentana,
    /// Ctrl+Alt+Retroceso — terminar el compositor y devolver el TTY.
    Salir,
    /// Meta+Espacio — abrir o cerrar el launchpad.
    Launchpad,
    /// Meta+F — maximizar la ventana con foco, o devolverla a su sitio.
    Maximizar,
    /// Meta+Alt+B y Meta+Alt+D — alternar si la barra se aparta de las ventanas
    /// o está siempre a la vista.
    AlternarBarra(crate::shell::Barra),
    /// Meta+L — echar la pantalla de bloqueo.
    Bloquear,
    /// Quitar el bloqueo. Hoy la pide Escape, porque todavía no autentica.
    Desbloquear,
    /// Un carácter más en la contraseña del bloqueo.
    BloqueoEscribir(char),
    BloqueoBorrar,
    BloqueoLimpiar,
    BloqueoComprobar,
    /// Las teclas de función del portátil: volumen, brillo, touchpad.
    Multimedia(crate::multimedia::Tecla),
    /// Meta+flechas — encajar la ventana con foco, como en KDE: la primera
    /// pulsación la manda a media pantalla y la siguiente, en perpendicular, a
    /// un cuarto. Es la única forma de llegar a un cuarto sin apuntar a una
    /// esquina con el ratón.
    Encajar(crate::ventanas::Direccion),
    /// Meta+Tab — pasar el foco a la siguiente ventana.
    SiguienteVentana,
    /// Lo que ha pedido una superficie emergente del shell. Llega por aquí y no
    /// se ejecuta en el sitio porque se decide dentro del filtro de
    /// `kbd.input`, con el estado del compositor ya prestado.
    DelShell(bookos_shell::Accion),
}

/// Decide si una tecla es un atajo del compositor.
///
/// Se llama **solo con la pulsación**, no con la suelta: interceptar también la
/// suelta dejaría al cliente con la tecla marcada como pulsada para siempre.
pub fn resolver(modifiers: &ModifiersState, handle: &KeysymHandle<'_>) -> Option<Accion> {
    let sym = handle.modified_sym().raw();

    // Ctrl+Alt+Fn. Con la configuración de xkb por defecto la propia capa de
    // teclado ya traduce esa combinación a XF86Switch_VT_n, así que no hay que
    // mirar los modificadores: si llega este keysym, es que la quería.
    const VT1: u32 = keysyms::KEY_XF86Switch_VT_1;
    const VT12: u32 = keysyms::KEY_XF86Switch_VT_12;
    if (VT1..=VT12).contains(&sym) {
        return Some(Accion::CambiarVt((sym - VT1 + 1) as i32));
    }

    // Red de seguridad para distribuciones de teclado que no traen la regla
    // `srvr_ctrl` y por tanto nunca emiten XF86Switch_VT_n. Ahí Ctrl+Alt+F3
    // llega como una F3 normal y hay que reconocerla a mano, mirando el símbolo
    // **sin** modificadores: con Ctrl+Alt aplicados podría no ser ya una F.
    if modifiers.ctrl && modifiers.alt {
        let base = handle
            .raw_syms()
            .first()
            .map(|s| s.raw())
            .unwrap_or(sym);
        const F1: u32 = keysyms::KEY_F1;
        const F12: u32 = keysyms::KEY_F12;
        if (F1..=F12).contains(&base) {
            return Some(Accion::CambiarVt((base - F1 + 1) as i32));
        }
        if base == keysyms::KEY_BackSpace {
            return Some(Accion::Salir);
        }
    }

    // Las teclas de función no llevan modificador y valen aunque el foco lo
    // tenga una aplicación a pantalla completa: subir el volumen en un vídeo
    // tiene que funcionar sin salir de él.
    if let Some(tecla) = crate::multimedia::resolver(sym) {
        return Some(Accion::Multimedia(tecla));
    }

    // Meta+Alt va antes que Meta a secas: si no, `Meta+Alt+D` entraría por el
    // brazo de Meta y no llegaría nunca aquí.
    if modifiers.logo && modifiers.alt {
        match sym {
            keysyms::KEY_b | keysyms::KEY_B => {
                return Some(Accion::AlternarBarra(crate::shell::Barra::Panel))
            }
            keysyms::KEY_d | keysyms::KEY_D => {
                return Some(Accion::AlternarBarra(crate::shell::Barra::Dock))
            }
            _ => {}
        }
    }

    if modifiers.logo {
        match sym {
            keysyms::KEY_Return | keysyms::KEY_KP_Enter => return Some(Accion::Terminal),
            keysyms::KEY_q | keysyms::KEY_Q => return Some(Accion::CerrarVentana),
            // Meta+Espacio abre el launchpad. En Plasma es Meta sola, pero
            // Meta sola es un modificador: distinguir "la he pulsado y soltado
            // sin nada más" exige recordar la suelta, y esa heurística es justo
            // la que deja el menú abriéndose solo al usar cualquier atajo.
            keysyms::KEY_space => return Some(Accion::Launchpad),
            keysyms::KEY_f | keysyms::KEY_F => return Some(Accion::Maximizar),
            keysyms::KEY_l | keysyms::KEY_L => return Some(Accion::Bloquear),
            keysyms::KEY_Left => {
                return Some(Accion::Encajar(crate::ventanas::Direccion::Izquierda))
            }
            keysyms::KEY_Right => {
                return Some(Accion::Encajar(crate::ventanas::Direccion::Derecha))
            }
            keysyms::KEY_Up => return Some(Accion::Encajar(crate::ventanas::Direccion::Arriba)),
            keysyms::KEY_Down => return Some(Accion::Encajar(crate::ventanas::Direccion::Abajo)),
            // Con Meta pulsada, xkb entrega el tabulador como ISO_Left_Tab en
            // cuanto entra Mayús. Se aceptan los dos porque Meta+Mayús+Tab es el
            // reflejo de cualquiera que venga de otro escritorio.
            keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => {
                return Some(Accion::SiguienteVentana)
            }
            _ => {}
        }
    }

    None
}

impl Accion {
    /// ¿Es una salida de emergencia? Son las únicas que siguen funcionando con
    /// el bloqueo echado: sin ellas, un fallo del bloqueo deja la máquina
    /// inservible hasta apagarla a lo bruto.
    pub fn es_emergencia(&self) -> bool {
        matches!(self, Accion::CambiarVt(_) | Accion::Salir)
    }
}

pub fn ejecutar(state: &mut BookosComp, accion: Accion) {
    match accion {
        Accion::CambiarVt(vt) => cambiar_vt(state, vt),
        Accion::Terminal => {
            let cmd = std::env::var("BOOKOS_TERMINAL").unwrap_or_else(|_| TERMINAL.to_string());
            lanzar(state, &cmd);
        }
        Accion::CerrarVentana => cerrar_ventana(state),
        Accion::Salir => {
            tracing::info!("salida pedida con Ctrl+Alt+Retroceso");
            state.loop_signal.stop();
        }
        Accion::Launchpad => {
            if let Some(shell) = state.shell.as_mut() {
                shell.alternar_launchpad();
            }
            state.needs_redraw = true;
        }
        Accion::AlternarBarra(cual) => {
            let modo = state.shell.as_mut().map(|s| s.alternar_visibilidad(cual));
            if let Some(modo) = modo {
                tracing::info!(?cual, ?modo, "visibilidad de la barra");
                // Al pasar a esquivar, el área útil crece; al volver a fija,
                // mengua. Las ventanas encajadas hay que recolocarlas o se
                // quedan con el hueco del panel de más o de menos.
                state.recolocar_encajadas();
                state.revisar_barras();
                state.needs_redraw = true;
            }
        }
        Accion::Bloquear => bloquear(state),
        Accion::Desbloquear => {
            state.bloqueo = Default::default();
            if let Some(shell) = state.shell.as_mut() {
                shell.desbloquear();
            }
            state.needs_redraw = true;
        }
        Accion::BloqueoEscribir(c) => {
            // Mientras se comprueba no se admite nada más: el campo está en
            // "comprobando" y aceptar teclas ahí deja escrito lo que se teclee
            // encima del intento anterior.
            if state.bloqueo.comprobando.is_none() {
                state.bloqueo.fallo = false;
                state.bloqueo.escrito.push(c);
                refrescar_bloqueo(state);
            }
        }
        Accion::BloqueoBorrar => {
            if state.bloqueo.comprobando.is_none() {
                state.bloqueo.fallo = false;
                state.bloqueo.escrito.pop();
                refrescar_bloqueo(state);
            }
        }
        Accion::BloqueoLimpiar => {
            if state.bloqueo.comprobando.is_none() {
                state.bloqueo.escrito.clear();
                state.bloqueo.fallo = false;
                refrescar_bloqueo(state);
            }
        }
        Accion::BloqueoComprobar => comprobar_bloqueo(state),
        Accion::Multimedia(tecla) => crate::multimedia::ejecutar(state, tecla),
        Accion::Maximizar => {
            if let Some(window) = state.ventana_con_foco() {
                state.alternar_maximizada(&window);
            }
        }
        Accion::Encajar(hacia) => {
            if let Some(window) = state.ventana_con_foco() {
                let actual = crate::ventanas::zona_de(&window);
                match crate::ventanas::empujar(actual, hacia) {
                    Some(zona) => state.encajar(&window, zona),
                    None => state.desencajar(&window),
                }
            }
        }
        Accion::SiguienteVentana => state.siguiente_ventana(),
        Accion::DelShell(accion) => hacer(state, accion),
    }
}

/// Ejecuta lo que ha pedido el shell: lanzar un programa, o traer al frente el
/// que ya está abierto.
pub fn hacer(state: &mut BookosComp, accion: bookos_shell::Accion) {
    match accion {
        bookos_shell::Accion::Lanzar(cmd) => lanzar(state, &cmd),
        bookos_shell::Accion::Launchpad => {
            if let Some(shell) = state.shell.as_mut() {
                shell.alternar_launchpad();
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Emergente(widget) => {
            if let Some(shell) = state.shell.as_mut() {
                shell.abrir_de_widget(widget);
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Acerca => {
            if let Some(shell) = state.shell.as_mut() {
                shell.abrir_acerca();
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Anclar {
            app_id,
            exec,
            icono,
        } => {
            let Some(shell) = state.shell.as_mut() else {
                return;
            };
            if let Some(lista) = shell.anclar(&app_id, &exec, &icono) {
                if let Err(err) = bookos_shell::guardar_dock(&lista) {
                    tracing::warn!("no se pudo guardar el dock: {err}");
                }
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Cerrar { app_id } => {
            // Se pide el cierre a todas sus ventanas, no solo a la primera: el
            // "Cerrar" del dock cierra la aplicación, y una que tenga tres
            // ventanas abiertas dejaría dos si se cerrara solo una.
            for window in state.ventanas_de_app(&app_id) {
                cerrar(&window);
            }
        }
        bookos_shell::Accion::Activar { app_id, exec } => {
            match state.ventana_de_app(&app_id) {
                Some(window) => state.enfocar(&window),
                // El dock creía que estaba abierta y no la encontramos: pasa si
                // el cliente declara un `app_id` distinto del de su `.desktop`.
                // Lanzar es mejor que quedarse quieto — un icono que no responde
                // se lee como que el dock está roto.
                None => {
                    tracing::debug!(app_id, "sin ventana que activar; se lanza");
                    lanzar(state, &exec);
                }
            }
        }
    }
}

fn cambiar_vt(state: &mut BookosComp, vt: i32) {
    let Some(cambiar) = state.cambiar_vt.as_ref() else {
        // Backend anidado: el TTY lo manda el compositor de debajo.
        tracing::debug!(vt, "cambio de TTY ignorado: no hay sesión propia");
        return;
    };
    tracing::info!(vt, "cambiando de TTY");
    cambiar(vt);
}

/// Le cuenta al shell cuántos puntos dibujar y en qué estado está.
fn refrescar_bloqueo(state: &mut BookosComp) {
    use bookos_shell::bloqueo::Estado;
    let estado = match (&state.bloqueo.comprobando, state.bloqueo.fallo) {
        (Some(_), _) => Estado::Comprobando,
        (None, true) => Estado::Fallo,
        (None, false) => Estado::Escribiendo,
    };
    let escritos = state.bloqueo.escrito.chars().count();
    if let Some(shell) = state.shell.as_mut() {
        shell.bloqueo_estado(escritos, estado);
    }
    state.needs_redraw = true;
}

/// Manda la contraseña al ayudante de PAM y programa la recogida.
///
/// No se espera al hijo: `pam_unix` tarda del orden de segundos cuando la
/// contraseña falla —es su defensa contra la fuerza bruta— y el compositor no
/// puede quedarse parado ahí. Mientras tanto el campo enseña "comprobando".
fn comprobar_bloqueo(state: &mut BookosComp) {
    if state.bloqueo.comprobando.is_some() || state.bloqueo.escrito.is_empty() {
        return;
    }
    let usuario = crate::autenticar::usuario();
    let Some(comprobacion) =
        crate::autenticar::Comprobacion::lanzar(&usuario, &state.bloqueo.escrito)
    else {
        // Sin ayudante en el sistema no hay forma de comprobar nada. Se dice en
        // el log y el campo se pone en rojo: colar a quien sea porque falte un
        // binario es lo contrario de lo que hace un bloqueo.
        tracing::error!("no hay unix_chkpwd: el bloqueo no puede autenticar");
        state.bloqueo.escrito.clear();
        state.bloqueo.fallo = true;
        refrescar_bloqueo(state);
        return;
    };
    state.bloqueo.comprobando = Some(comprobacion);
    refrescar_bloqueo(state);
    recoger_comprobacion(state);
}

/// Mira cada poco si el ayudante ya contestó.
///
/// Cien milisegundos: es la resolución con la que se nota que el campo pasa de
/// "comprobando" a abierto, y son diez despertares por segundo solo mientras
/// dura la comprobación, no en reposo.
fn recoger_comprobacion(state: &mut BookosComp) {
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    let result = state.loop_handle.insert_source(
        Timer::from_duration(std::time::Duration::from_millis(100)),
        |_, _, state: &mut BookosComp| {
            let Some(comprobacion) = state.bloqueo.comprobando.as_mut() else {
                return TimeoutAction::Drop;
            };
            match comprobacion.resultado() {
                None => TimeoutAction::ToDuration(std::time::Duration::from_millis(100)),
                Some(true) => {
                    ejecutar(state, Accion::Desbloquear);
                    TimeoutAction::Drop
                }
                Some(false) => {
                    // Lo escrito se borra al fallar, como en cualquier pantalla
                    // de acceso: reintentar con la mitad de una contraseña mal
                    // escrita solo alarga el fallo.
                    state.bloqueo.comprobando = None;
                    state.bloqueo.escrito.clear();
                    state.bloqueo.fallo = true;
                    refrescar_bloqueo(state);
                    TimeoutAction::Drop
                }
            }
        },
    );
    if let Err(err) = result {
        tracing::error!("no se pudo programar la recogida del bloqueo: {err}");
    }
}

fn cerrar_ventana(state: &mut BookosComp) {
    let foco = state
        .seat
        .get_keyboard()
        .and_then(|kbd| kbd.current_focus())
        .and_then(|surface| state.window_for_surface(&surface));
    let Some(window) = foco else {
        return;
    };
    cerrar(&window);
}

/// Pide a una ventana que se cierre, sea nativa o de XWayland.
///
/// Es una petición, no una orden: el cliente puede enseñar un "¿guardar los
/// cambios?" y quedarse. Matar el proceso sería lo contrario de lo que espera
/// quien pulsa "Cerrar".
fn cerrar(window: &smithay::desktop::Window) {
    if let Some(toplevel) = window.toplevel() {
        toplevel.send_close();
        return;
    }
    if let Some(x11) = window.x11_surface() {
        if let Err(err) = x11.close() {
            tracing::warn!("no se pudo cerrar la ventana X11: {err}");
        }
    }
}

/// Arranca un programa como cliente de esta sesión.
///
/// Centralizado aquí porque los dos backends lo necesitan para el cliente
/// inicial y el atajo del terminal quiere exactamente el mismo entorno: si se
/// escribiera dos veces, tarde o temprano una de las copias se olvidaría de
/// quitar `DISPLAY` y el programa acabaría en el X11 del anfitrión.
pub fn lanzar(state: &mut BookosComp, cmd: &str) {
    // Los hijos ya terminados se recogen aquí, no con un manejador de SIGCHLD:
    // el bucle es de un solo hilo y esto no necesita señales ni dependencias.
    state.hijos.retain_mut(|hijo| !matches!(hijo.try_wait(), Ok(Some(_))));

    // `sh -c` para que valga cualquier cosa que se escriba en BOOKOS_TERMINAL,
    // con sus argumentos; `exec` evita dejar un shell de más colgando.
    //
    // Pero **solo con un comando suelto**: `exec A || B` no ejecuta B si A no
    // existe, porque `exec` con un programa inexistente termina el shell antes
    // de llegar al `||`. Así se quedaron sin funcionar las entradas del menú que
    // llevan respaldo — el log decía "exec: bookos-about: no encontrado" y ahí
    // se acababa. Cuando hay encadenado, el shell se queda de padre.
    let linea = if cmd.contains("||") || cmd.contains("&&") || cmd.contains(';') {
        cmd.to_string()
    } else {
        format!("exec {cmd}")
    };
    // El tema de cursor va en el entorno porque en Wayland **lo dibuja cada
    // cliente**: Qt y GTK cargan el tema ellos mismos y mandan su propia
    // superficie de cursor. Sin esto, el puntero cambia de aspecto según sobre
    // qué ventana esté — el nuestro sobre el escritorio y el de Adwaita sobre
    // una ventana GTK.
    let (tema, tamano) = state
        .cursor_theme
        .as_ref()
        .map(|t| (t.nombre().to_string(), t.tamano_logico().to_string()))
        .unwrap_or_default();

    let mut orden = std::process::Command::new("/bin/sh");
    orden
        .arg("-c")
        .arg(linea)
        .env("WAYLAND_DISPLAY", &state.socket_name)
        .env("XCURSOR_THEME", tema)
        .env("XCURSOR_SIZE", tamano);
    // DISPLAY solo si XWayland ya contestó. Ponerlo antes es peor que no
    // ponerlo: una aplicación que sabe hablar los dos protocolos prefiere X11
    // en cuanto ve un DISPLAY, y acabaría yendo por el camino largo hacia un
    // servidor que todavía no escucha.
    match state.display_x11 {
        Some(n) => orden.env("DISPLAY", format!(":{n}")),
        None => orden.env_remove("DISPLAY"),
    };

    match orden.spawn() {
        Ok(hijo) => {
            tracing::info!(cmd, pid = hijo.id(), "cliente lanzado");
            state.hijos.push(hijo);
        }
        Err(err) => tracing::error!(cmd, "no se pudo lanzar: {err}"),
    }
}

/// Echa la pantalla de bloqueo.
///
/// **Todavía no autentica.** Es una vista previa para poder mirar el diseño en
/// una sesión de verdad: se sale con Escape. Mientras siga así no protege
/// nada, y por eso lo dice el log en voz alta: un bloqueo que parece bloquear
/// sin bloquear es peor que no tener ninguno.
fn bloquear(state: &mut BookosComp) {
    let pantalla = state.pantalla_logica();
    let hora = bookos_shell::Shell::hora_bloqueo();
    if let Some(shell) = state.shell.as_mut() {
        shell.bloquear(hora, pantalla);
    }
    tracing::warn!("bloqueo echado: VISTA PREVIA, todavía no pide contraseña (Escape para salir)");
    state.needs_redraw = true;
}

/// Da el foco de teclado a la ventana que quede más arriba.
///
/// Se llama al destruirse un toplevel: sin esto el foco se queda apuntando a la
/// superficie muerta y las ventanas que siguen abiertas no reciben teclas.
pub fn refocalizar(state: &mut BookosComp) {
    // `alive()` porque la ventana destruida sigue mapeada en el espacio hasta
    // el siguiente `refresh()`: sin filtrarla se le devolvería el foco al
    // muerto.
    let heredera = state
        .space
        .elements()
        .filter(|w| w.alive())
        .next_back()
        .cloned();
    match heredera {
        // Por `enfocar` y no por `set_focus` a secas: la ventana que hereda el
        // foco también tiene que enterarse de que ahora está activa, o se queda
        // dibujándose con la barra de título apagada.
        Some(window) => state.enfocar(&window),
        None => {
            if let Some(kbd) = state.seat.get_keyboard() {
                kbd.set_focus(state, None, SERIAL_COUNTER.next_serial());
            }
        }
    }
}
