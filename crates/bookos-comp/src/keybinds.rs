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

use smithay::desktop::Window;
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
    /// El buscador de Meta+Espacio: aplicaciones, comandos y estados.
    Buscador,
    /// Meta+W — vista general y gestión de escritorios.
    VistaEscritorios,
    /// Meta+F — maximizar la ventana con foco, o devolverla a su sitio.
    Maximizar,
    /// Meta+H — mandar la ventana con foco al dock. Vuelve pulsando su icono.
    Minimizar,
    /// Meta+Alt+B y Meta+Alt+D — alternar si la barra se aparta de las ventanas
    /// o está siempre a la vista.
    AlternarBarra(crate::shell::Barra),
    /// Meta+L — echar la pantalla de bloqueo.
    Bloquear,
    /// Quitar el bloqueo. La pide el ayudante de PAM cuando la contraseña vale.
    Desbloquear,
    /// Apagar, reiniciar o suspender desde el menú de la pantalla de bloqueo.
    Energia(bookos_shell::bloqueo::Peticion),
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
    /// Alt+Tab abre aplicaciones; Meta+Tab, ventanas con miniaturas. El entero
    /// es el sentido: `-1` con Mayús.
    Conmutar(bookos_shell::conmutador::Modo, i32),
    /// Soltar el modificador: se va a la aplicación elegida.
    ConmutarFin,
    /// Escape con el conmutador abierto: se cierra sin ir a ninguna parte.
    ConmutarCancelar,
    /// Ir a un escritorio concreto: Meta+1..5.
    Escritorio(usize),
    /// Moverse uno a la izquierda o a la derecha: Meta+Ctrl+flechas y el gesto
    /// de cuatro dedos a los lados. Va aparte de [`Accion::Escritorio`] porque
    /// el destino depende de dónde estés, y eso no se sabe al resolver la tecla.
    EscritorioRelativo(i32),
    /// Meta+Ctrl+D y el gesto de cuatro dedos abajo: apartar todas las ventanas
    /// para ver el escritorio, o devolverlas.
    MostrarEscritorio,
    /// Lo que ha pedido una superficie emergente del shell. Llega por aquí y no
    /// se ejecuta en el sitio porque se decide dentro del filtro de
    /// `kbd.input`, con el estado del compositor ya prestado.
    DelShell(bookos_shell::Accion),
    /// El diálogo del botón de encendido: dormir, bloquear, salir, reiniciar,
    /// apagar.
    DialogoEnergia,
}

/// El conmutador de aplicaciones: `Alt+Tab` y `Meta+Tab`, con Mayús al revés.
///
/// Está aparte de [`resolver`] para poder probarlo: `resolver` necesita un
/// `KeysymHandle`, que envuelve el estado de xkb y no se construye en un test.
/// Sin esto, "el atajo no responde" y "la tecla no llega al compositor" son
/// indistinguibles desde fuera — y anidado dentro de Plasma pasa lo segundo, que
/// es lo que hay que poder descartar.
fn conmutador_de(sym: u32, modifiers: &ModifiersState) -> Option<Accion> {
    if !matches!(sym, keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab) {
        return None;
    }
    // Con Ctrl no: `Ctrl+Alt+Tab` es de otra cosa en varios escritorios y no
    // conviene pisarlo.
    if modifiers.ctrl || modifiers.alt == modifiers.logo {
        return None;
    }
    // Con Mayús se recorre al revés, como en todas partes. Se mira el
    // modificador y no el keysym porque xkb ya convierte el tabulador en
    // `ISO_Left_Tab` al entrar Mayús, y los dos llegan aquí.
    let modo = if modifiers.alt {
        bookos_shell::conmutador::Modo::Aplicaciones
    } else {
        bookos_shell::conmutador::Modo::Ventanas
    };
    Some(Accion::Conmutar(modo, if modifiers.shift { -1 } else { 1 }))
}

/// Abre el conmutador o avanza en él.
///
/// Alt toma la ventana más reciente de cada aplicación; Meta conserva todas
/// las ventanas. Por eso dos terminales no hacen nada con Alt —solo hay una
/// aplicación—, pero aparecen separadas y con título propio en Meta.
///
/// Solo entran las ventanas **de este escritorio**: las de los demás están
/// desmapeadas, y saltar a una cambiaría de escritorio sin avisar, que no es lo
/// que espera quien abre cualquiera de los dos selectores.
fn conmutar(state: &mut BookosComp, modo: bookos_shell::conmutador::Modo, sentido: i32) {
    if state.shell.as_ref().is_some_and(|s| s.hay_conmutador()) {
        if state.conmutador_modo != Some(modo) {
            state.conmutador_destinos.clear();
            state.conmutador_modo = None;
            if let Some(shell) = state.shell.as_mut() {
                shell.cancelar_conmutador();
            }
        } else {
            if let Some(shell) = state.shell.as_mut() {
                shell.conmutador_mover(sentido);
            }
            state.needs_redraw = true;
            return;
        }
    }

    let ventanas: Vec<Window> = state.space.elements().cloned().collect();
    let sellos: Vec<_> = ventanas.iter().map(crate::ventanas::enfocada_en).collect();
    let app_ids: Vec<_> = ventanas
        .iter()
        .map(|w| crate::handlers::app_id(w).unwrap_or_default())
        .collect();
    // Una celda por ventana en los dos modos. Alt tiene tope —la fila de iconos
    // se sale de la pantalla pasadas doce— y Meta no, porque su rejilla encoge
    // las miniaturas hasta que quepan.
    let orden = match modo {
        bookos_shell::conmutador::Modo::Aplicaciones => {
            crate::conmutador::orden(&sellos, bookos_shell::conmutador::MAXIMO)
        }
        bookos_shell::conmutador::Modo::Ventanas => crate::conmutador::orden(&sellos, usize::MAX),
    };

    let entradas: Vec<_> = orden
        .iter()
        .map(|i| {
            let window = &ventanas[*i];
            let app_id = &app_ids[*i];
            // El título también en Alt: es lo único que distingue dos ventanas
            // de la misma aplicación, que ahora tienen celda propia.
            let titulo = crate::handlers::titulo(window).unwrap_or_default();
            bookos_shell::entrada_de_ventana(app_id, &titulo)
        })
        .collect();
    state.conmutador_destinos = orden.iter().map(|i| ventanas[*i].clone()).collect();

    let abierto = state
        .shell
        .as_mut()
        .is_some_and(|s| s.abrir_conmutador(modo, entradas));
    if !abierto {
        // Nunca en silencio: que este atajo fallara sin decir nada es lo que
        // hizo que diagnosticarlo costara una tarde.
        tracing::info!(
            ventanas = ventanas.len(),
            celdas = orden.len(),
            hay_shell = state.shell.is_some(),
            "el conmutador no se abre"
        );
        state.conmutador_destinos.clear();
        state.conmutador_modo = None;
        return;
    }
    state.conmutador_modo = Some(modo);
    // El modelo nace señalando la ventana actual (índice 0); esta primera
    // pulsación aplica el mismo paso que las repeticiones. Hacia atrás cae así
    // directamente en la última celda.
    if let Some(shell) = state.shell.as_mut() {
        shell.conmutador_mover(sentido);
    }
    tracing::debug!(celdas = orden.len(), "conmutador abierto");
    state.needs_redraw = true;
}

/// Se ha soltado el modificador: a la ventana elegida.
fn conmutar_fin(state: &mut BookosComp) {
    state.conmutador_pegado = false;
    let elegida = state.shell.as_mut().and_then(|s| s.cerrar_conmutador());
    let destinos = std::mem::take(&mut state.conmutador_destinos);
    state.conmutador_modo = None;
    // Repintar pase lo que pase: la tarjeta ya no está y alguien tiene que
    // borrarla de la pantalla, se haya elegido algo o no.
    state.needs_redraw = true;
    let Some(window) = elegida.and_then(|i| destinos.get(i).cloned()) else {
        return;
    };
    // La ventana pudo cerrarse con el conmutador abierto: el `Window` clonado
    // sigue vivo aquí aunque su cliente se haya ido, así que enfocarlo sin
    // comprobarlo daría el foco a un fantasma. Antes esto no hacía falta porque
    // se rebuscaba en el `Space`, que ya solo tiene ventanas vivas.
    if !window.alive() {
        tracing::warn!("la ventana elegida se cerró mientras se elegía");
        return;
    }
    state.enfocar(&window);
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
        let base = handle.raw_syms().first().map(|s| s.raw()).unwrap_or(sym);
        const F1: u32 = keysyms::KEY_F1;
        const F12: u32 = keysyms::KEY_F12;
        if (F1..=F12).contains(&base) {
            return Some(Accion::CambiarVt((base - F1 + 1) as i32));
        }
        if base == keysyms::KEY_BackSpace {
            return Some(Accion::Salir);
        }
    }

    // El botón de encendido y `Meta+Esc`, los dos al mismo diálogo. El segundo
    // existe porque en un sobremesa el botón está en la caja, debajo de la
    // mesa, y porque en un portátil puede que logind se quede la tecla antes
    // de que llegue aquí (`HandlePowerKey`).
    if sym == XF86_ENCENDIDO || (modifiers.logo && sym == keysyms::KEY_Escape) {
        return Some(Accion::DialogoEnergia);
    }

    // Las teclas de función no llevan modificador y valen aunque el foco lo
    // tenga una aplicación a pantalla completa: subir el volumen en un vídeo
    // tiene que funcionar sin salir de él.
    if let Some(tecla) = crate::multimedia::resolver(sym) {
        return Some(Accion::Multimedia(tecla));
    }

    // Alt+Tab, el de toda la vida. Estaba solo en Meta+Tab y eso es pedirle al
    // usuario que desaprenda el atajo que usa desde hace veinte años: aquí lo
    // que hay que teclear tiene que ser lo que ya sabe.
    //
    // Con Alt pulsada, xkb entrega el tabulador como ISO_Left_Tab en cuanto
    // entra Mayús, así que se aceptan los dos. Va **antes** que el brazo de
    // Ctrl+Alt para que no lo intercepte el cambio de VT.
    if let Some(accion) = conmutador_de(sym, modifiers) {
        return Some(accion);
    }

    // Meta+Ctrl, para los escritorios: va antes que Meta a secas por lo mismo
    // que Meta+Alt —si no, `Meta+Ctrl+Izquierda` encajaría la ventana a medio
    // lado en vez de cambiar de escritorio—. Son los atajos de KDE.
    if modifiers.logo && modifiers.ctrl {
        match sym {
            keysyms::KEY_Left => return Some(Accion::EscritorioRelativo(-1)),
            keysyms::KEY_Right => return Some(Accion::EscritorioRelativo(1)),
            keysyms::KEY_d | keysyms::KEY_D => return Some(Accion::MostrarEscritorio),
            _ => {}
        }
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
            // Meta+Espacio abre el **buscador**, que es donde lo tiene KRunner
            // y donde lo busca cualquiera que venga de Plasma o de macOS. El
            // launchpad se quedó con Meta sola, que ya funcionaba: son dos
            // gestos distintos para dos cosas distintas —ver todo lo instalado
            // frente a escribir lo que quieres.
            keysyms::KEY_space => return Some(Accion::Buscador),
            keysyms::KEY_w | keysyms::KEY_W => return Some(Accion::VistaEscritorios),
            keysyms::KEY_f | keysyms::KEY_F => return Some(Accion::Maximizar),
            // H de *hide*, como en macOS: Meta+M es «minimizar» en Windows pero
            // aquí Meta+M ya no está libre en cuanto haya un menú.
            keysyms::KEY_h | keysyms::KEY_H => return Some(Accion::Minimizar),
            keysyms::KEY_l | keysyms::KEY_L => return Some(Accion::Bloquear),
            keysyms::KEY_Left => {
                return Some(Accion::Encajar(crate::ventanas::Direccion::Izquierda))
            }
            keysyms::KEY_Right => {
                return Some(Accion::Encajar(crate::ventanas::Direccion::Derecha))
            }
            keysyms::KEY_Up => return Some(Accion::Encajar(crate::ventanas::Direccion::Arriba)),
            keysyms::KEY_Down => return Some(Accion::Encajar(crate::ventanas::Direccion::Abajo)),
            // Meta+1..5, el atajo directo a cada escritorio. Los keysyms de los
            // dígitos son consecutivos desde KEY_1, así que la cuenta sale sola
            // y no hace falta un brazo por escritorio.
            _ if (keysyms::KEY_1..keysyms::KEY_1 + crate::escritorios::MAXIMO as u32)
                .contains(&sym) =>
            {
                return Some(Accion::Escritorio((sym - keysyms::KEY_1) as usize))
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
        Accion::Escritorio(n) => crate::escritorios::cambiar_a(state, n),
        Accion::EscritorioRelativo(pasos) => {
            let destino = crate::escritorios::destino(
                state.escritorios.activo(),
                pasos,
                state.escritorios.cuantos(),
            );
            crate::escritorios::cambiar_a(state, destino);
        }
        Accion::MostrarEscritorio => crate::escritorios::alternar_despejado(state),
        Accion::Launchpad => {
            if let Some(shell) = state.shell.as_mut() {
                shell.alternar_launchpad();
            }
            state.needs_redraw = true;
        }
        Accion::Buscador => {
            if let Some(shell) = state.shell.as_mut() {
                shell.alternar_buscador();
            }
            state.needs_redraw = true;
        }
        Accion::VistaEscritorios => alternar_vista_escritorios(state),
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
        Accion::DialogoEnergia => alternar_dialogo_energia(state),
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
        // Con `-i` para ignorar inhibidores, igual que el menú del panel: desde
        // la pantalla de bloqueo no hay forma de contestarle a un diálogo de
        // «hay una aplicación que impide apagar».
        Accion::Energia(peticion) => {
            use bookos_shell::bloqueo::Peticion;
            let orden = match peticion {
                Peticion::Apagar => "systemctl poweroff -i || systemctl poweroff --force",
                Peticion::Reiniciar => "systemctl reboot -i || systemctl reboot --force",
                Peticion::Suspender => "systemctl suspend -i",
            };
            lanzar(state, orden);
        }
        Accion::Minimizar => {
            if let Some(window) = state.ventana_con_foco() {
                crate::ventanas::minimizar(state, window);
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
        Accion::Conmutar(modo, sentido) => conmutar(state, modo, sentido),
        Accion::ConmutarFin => conmutar_fin(state),
        Accion::ConmutarCancelar => {
            state.conmutador_destinos.clear();
            state.conmutador_modo = None;
            state.conmutador_pegado = false;
            if state
                .shell
                .as_mut()
                .is_some_and(|s| s.cancelar_conmutador())
            {
                state.needs_redraw = true;
            }
        }
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
        bookos_shell::Accion::Escritorio(n) => crate::escritorios::cambiar_a(state, n),
        bookos_shell::Accion::VistaEscritorios => alternar_vista_escritorios(state),
        bookos_shell::Accion::CrearEscritorio => {
            if crate::escritorios::crear(state) {
                guardar_y_actualizar_escritorios(state);
            }
        }
        bookos_shell::Accion::EliminarEscritorio(i) => {
            if crate::escritorios::eliminar(state, i) {
                guardar_y_actualizar_escritorios(state);
            }
        }
        bookos_shell::Accion::RenombrarEscritorio { indice, nombre } => {
            if crate::escritorios::renombrar(state, indice, &nombre) {
                guardar_y_actualizar_escritorios(state);
            }
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
        bookos_shell::Accion::Apariencia { tema, acento } => {
            let Some(shell) = state.shell.as_mut() else {
                return;
            };
            if shell.aplicar_apariencia(tema, acento) {
                // Se guarda **después** de aplicarlo: si escribir falla —disco
                // lleno, `$HOME` de solo lectura— el escritorio ya ha cambiado
                // de color y lo que se pierde es que se recuerde, que es el
                // fallo menos malo de los dos.
                if let Err(err) = bookos_shell::guardar_apariencia(tema, acento) {
                    tracing::warn!("no se pudo guardar la apariencia: {err}");
                }
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::CerrarNotificacion(id) => {
            let cerrada = state
                .shell
                .as_mut()
                .is_some_and(|s| s.cerrar_notificacion(id));
            if cerrada {
                // La aplicación que la mandó espera saber que ya no está: es lo
                // que le permite no repetirla y limpiar lo suyo.
                crate::notificaciones::cerrada(
                    state.bus_notificaciones.as_ref(),
                    id,
                    crate::notificaciones::CERRADA_POR_EL_USUARIO,
                );
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::BorrarNotificaciones => {
            let ids = state
                .shell
                .as_mut()
                .map(|s| s.borrar_notificaciones())
                .unwrap_or_default();
            for id in ids {
                crate::notificaciones::cerrada(
                    state.bus_notificaciones.as_ref(),
                    id,
                    crate::notificaciones::CERRADA_POR_EL_USUARIO,
                );
            }
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Actividad(accion) => {
            crate::ajustes::accion_actividad(state, &accion);
            state.needs_redraw = true;
        }
        bookos_shell::Accion::Bloquear => bloquear(state),
        bookos_shell::Accion::CerrarSesion => {
            tracing::info!("cierre de sesión pedido desde el diálogo de energía");
            state.loop_signal.stop();
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
            // El icono funciona como alternador: minimizada vuelve con la
            // lámpara inversa; abierta se va al dock con la lámpara normal.
            // Primero las minimizadas para no esconder otra ventana de la misma
            // aplicación que se haya quedado abierta detrás.
            if crate::ventanas::hay_minimizada(state, &app_id) {
                crate::ventanas::restaurar_minimizada(state, &app_id);
                return;
            }
            // Mientras sigue viajando aún figura en el `Space`; una segunda
            // orden duplicaría su entrada en la lista de minimización.
            if crate::ventanas::esta_minimizando(state, &app_id) {
                return;
            }
            match state.ventana_de_app(&app_id) {
                Some(window) => crate::ventanas::minimizar(state, window),
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

fn alternar_vista_escritorios(state: &mut BookosComp) {
    let activo = state.escritorios.activo();
    let nombres = state.escritorios.nombres().to_vec();
    if let Some(shell) = state.shell.as_mut() {
        shell.alternar_vista_escritorios(activo, nombres);
    }
    state.needs_redraw = true;
}

fn guardar_y_actualizar_escritorios(state: &mut BookosComp) {
    let activo = state.escritorios.activo();
    let nombres = state.escritorios.nombres().to_vec();
    if let Err(err) = bookos_shell::guardar_escritorios(&nombres) {
        tracing::warn!("no se pudieron guardar los escritorios: {err}");
    }
    if let Some(shell) = state.shell.as_mut() {
        shell.actualizar_vista_escritorios(activo, nombres);
    }
    state.needs_redraw = true;
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
    state
        .hijos
        .retain_mut(|hijo| !matches!(hijo.try_wait(), Ok(Some(_))));

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
/// El keysym de la tecla de encendido. Como los demás `XF86`, a mano: su valor
/// está fijado desde hace treinta años y no depende de cómo lo llame la versión
/// de turno de la biblioteca de teclado.
const XF86_ENCENDIDO: u32 = 0x1008FF2A;

/// Abre el diálogo de energía, o lo cierra si ya estaba: pulsar dos veces el
/// botón de encendido tiene que dejar el escritorio como estaba, no apilar
/// diálogos.
fn alternar_dialogo_energia(state: &mut BookosComp) {
    let Some(shell) = state.shell.as_mut() else {
        return;
    };
    if shell.emergente_nombre() == Some("apagar") {
        shell.cerrar_emergente();
    } else {
        shell.abrir_de_widget("apagar");
    }
    state.needs_redraw = true;
}

fn bloquear(state: &mut BookosComp) {
    let pantalla = state.pantalla_logica();
    let hora = bookos_shell::Shell::hora_bloqueo();
    let fecha = bookos_shell::Shell::fecha_bloqueo();
    if let Some(shell) = state.shell.as_mut() {
        shell.bloquear(hora, fecha, pantalla);
    }
    // El bloqueo ya está puesto antes de preguntar por MPRIS: un reproductor
    // lento nunca puede retrasar la barrera de seguridad ni el primer frame.
    state.bloqueo_generacion = state.bloqueo_generacion.wrapping_add(1);
    let generacion = state.bloqueo_generacion;
    let canal = state.medios_bloqueo.clone();
    std::thread::Builder::new()
        .name("bookos-lock-media".into())
        .spawn(move || {
            let _ = canal.send((generacion, bookos_shell::medios::Sonando::leer()));
        })
        .inspect_err(|err| tracing::warn!("no se pudo consultar MPRIS para el bloqueo: {err}"))
        .ok();
    tracing::info!("bloqueo echado");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(alt: bool, logo: bool, shift: bool, ctrl: bool) -> ModifiersState {
        ModifiersState {
            alt,
            logo,
            shift,
            ctrl,
            ..Default::default()
        }
    }

    /// Alt+Tab y Meta+Tab abren el conmutador, y con Mayús va al revés.
    ///
    /// Si esto pasa y el atajo no responde en la máquina, la tecla no está
    /// llegando: anidado, el compositor de debajo se queda con Alt+Tab.
    #[test]
    fn el_tabulador_con_modificador_conmuta() {
        use bookos_shell::conmutador::Modo;
        let tab = keysyms::KEY_Tab;
        assert_eq!(
            conmutador_de(tab, &mods(true, false, false, false)),
            Some(Accion::Conmutar(Modo::Aplicaciones, 1)),
            "Alt+Tab"
        );
        assert_eq!(
            conmutador_de(tab, &mods(false, true, false, false)),
            Some(Accion::Conmutar(Modo::Ventanas, 1)),
            "Meta+Tab"
        );
        assert_eq!(
            conmutador_de(tab, &mods(true, false, true, false)),
            Some(Accion::Conmutar(Modo::Aplicaciones, -1)),
            "Alt+Mayús+Tab va hacia atrás"
        );
        // Y con Mayús, xkb entrega otro keysym: tiene que valer igual.
        assert_eq!(
            conmutador_de(keysyms::KEY_ISO_Left_Tab, &mods(true, false, true, false)),
            Some(Accion::Conmutar(Modo::Aplicaciones, -1)),
            "ISO_Left_Tab es el mismo gesto"
        );
    }

    /// Un tabulador sin modificador es del cliente —se escribe—, y con Ctrl es
    /// de otra cosa.
    #[test]
    fn el_tabulador_solo_no_es_del_compositor() {
        let tab = keysyms::KEY_Tab;
        assert_eq!(conmutador_de(tab, &mods(false, false, false, false)), None);
        assert_eq!(
            conmutador_de(tab, &mods(true, false, false, true)),
            None,
            "Ctrl+Alt+Tab"
        );
        assert_eq!(
            conmutador_de(tab, &mods(true, true, false, false)),
            None,
            "Alt+Meta+Tab"
        );
        assert_eq!(
            conmutador_de(keysyms::KEY_a, &mods(true, false, false, false)),
            None
        );
    }
}
