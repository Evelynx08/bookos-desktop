//! Backends de salida y entrada.
//!
//! - `winit`: anidado dentro de otra sesión, para desarrollo.
//! - `udev`:  DRM/KMS atómico + libinput + libseat. El de producción.
//!
//! Aquí vive lo que comparten: cómo se arma la escena y cada cuánto despierta
//! el panel. Que la escena se construya en un solo sitio importa — es lo que
//! garantiza que lo que ves anidado es exactamente lo que verás en la sesión
//! real.

#[cfg(feature = "udev")]
pub mod udev;
#[cfg(feature = "winit")]
pub mod winit;

use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::utils::RescaleRenderElement;
use smithay::backend::renderer::element::AsRenderElements;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Scale};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::cursor::OverlayElement;
use crate::state::BookosComp;

/// Arma la lista de elementos de un frame, de arriba abajo: cursor, shell y
/// ventanas.
/// Ya no es genérica sobre el renderer: los dos backends componen con GLES y
/// el cristal necesita copiar el framebuffer, que es GL crudo. La genericidad
/// solo servía para un segundo renderer que no existe.
pub fn escena(
    state: &mut BookosComp,
    renderer: &mut GlesRenderer,
    output: &Output,
) -> Vec<OverlayElement> {
    let scale = output.current_scale().fractional_scale();
    let cursor_en = (
        state.pointer_location.x * scale,
        state.pointer_location.y * scale,
    )
        .into();

    // El cursor va delante: los elementos se apilan de delante hacia atrás, así
    // que el primero de la lista queda arriba del todo.
    let _t = std::time::Instant::now();
    let mut elementos = crate::cursor::elements(
        renderer,
        &state.cursor_status,
        state.cursor_theme.as_ref(),
        cursor_en,
        scale,
    );

    let t_cursor = _t.elapsed();
    let _t = std::time::Instant::now();
    // Con una ventana a pantalla completa el shell no se dibuja: un vídeo con
    // el panel encima no está a pantalla completa, está maximizado. El cursor
    // sí sigue delante, que es lo que hace todo el mundo.
    let completa = state.hay_pantalla_completa();
    // Retirar el aviso caducado antes de componer: su superficie vive en el
    // compositor y nadie más la suelta. Sin esto el OSD se quedaba en pantalla
    // para siempre —el alfa llegaba a cero, pero el buffer seguía puesto y
    // volvía a verse en cuanto algo cambiaba la escena.
    if let Some(shell) = state.shell.as_mut() {
        if shell.osd_vivo().1 {
            state.needs_redraw = true;
        }
        shell.animar_emergente();
    }
    elementos.extend(
        state
            .shell
            .as_ref()
            .filter(|_| !completa)
            .map(|shell| shell.elements(renderer))
            .unwrap_or_default()
            .into_iter()
            .map(OverlayElement::Memory),
    );
    let t_shell = _t.elapsed();
    let _t = std::time::Instant::now();

    // La vista previa del encaje va delante de las ventanas —tiene que verse
    // sobre la que estás arrastrando— pero detrás del panel y del dock, que
    // mandan siempre.
    let area = state.work_area();
    elementos.extend(
        state
            .arrastre
            .as_ref()
            .map(|a| a.previa(area, scale))
            .unwrap_or_default()
            .into_iter()
            .map(OverlayElement::Color),
    );

    // El velo del launchpad va detrás del shell y delante de las ventanas: es
    // lo que hace que el escritorio se vea apagado por debajo sin que el shell
    // tenga que rasterizar la pantalla entera en CPU.
    elementos.extend(
        state
            .shell
            .as_ref()
            .filter(|_| !completa)
            .and_then(|shell| shell.velo())
            .map(OverlayElement::Color),
    );

    // El cristal va detrás del panel, del dock y del velo, y delante de las
    // ventanas: se dibuja cuando estas ya están en el framebuffer, que es de
    // donde copia lo que desenfoca.
    //
    // Detrás del velo y no delante porque el launchpad desenfoca la pantalla
    // entera: con el cristal por delante, el velo quedaba tapado y el
    // escritorio se veía nítido y sin apagar debajo de los iconos.
    //
    // El commit sube en cada frame a propósito. El desenfoque depende de lo que
    // haya debajo, así que cualquier cambio en la escena lo invalida; llevar la
    // cuenta de qué zonas de debajo cambiaron sería lo suyo, pero hoy el
    // compositor solo dibuja cuando algo cambió, y en reposo no dibuja nada.
    let (ancho_pantalla, alto_pantalla) = output
        .current_mode()
        .map(|m| (m.size.w, m.size.h))
        .unwrap_or((1, 1));
    if !completa {
        if let (Some(cristal), Some(shell)) = (state.cristal.clone(), state.shell.as_ref()) {
            let zonas = shell.zonas_cristal();
            // El commit **no** sube por frame: el cristal desenfoca el fondo,
            // que no cambia. Solo se invalida cuando la barra se mueve, y de eso
            // se encarga el propio rectángulo del elemento.
            // Sin fondo subido no hay nada que desenfocar: el panel y el dock
            // se dibujan con su color, que es lo que hacían antes del cristal.
            if let Some((textura, fondo_tam)) = cristal.borrow().textura() {
            let pantalla = (ancho_pantalla, alto_pantalla);
            for (i, (rect, radio, fuerza)) in zonas.into_iter().enumerate() {
                elementos.push(OverlayElement::Cristal(crate::desenfoque::Desenfoque::new(
                    id_cristal(i),
                    state.cristal_commit,
                    rect,
                    radio,
                    fondo_tam,
                    pantalla,
                    textura.clone(),
                    cristal.clone(),
                    fuerza,
                )));
            }
            }
        }
    }


    // Con el bloqueo echado no se dibuja ninguna ventana. No es cosmética: si
    // se dibujan, el escritorio se ve por debajo de la pantalla de bloqueo y
    // esta deja de proteger nada. Se comprobó en pantalla — konsole se veía
    // entera detrás del reloj.
    let bloqueado = state.shell.as_ref().is_some_and(|s| s.esta_bloqueado());

    // Y por último las ventanas, que quedan debajo de todo lo anterior.
    //
    // Se generan a mano en vez de con `space_render_elements` para no arrastrar
    // el tipo envuelto `SpaceRenderElements` al enum: aquí solo queremos
    // superficies. `elements()` va de abajo arriba, así que se recorre al revés.
    let ventanas: Vec<_> = if bloqueado {
        Vec::new()
    } else {
        state
            .space
            .elements()
            .rev()
            .filter_map(|w| state.space.element_location(w).map(|loc| (w.clone(), loc)))
            .collect()
    };
    for (window, loc) in ventanas {
        let (alfa, zoom) = crate::ventanas::animacion(&window);
        // Al maximizar o encajar, el buffer nuevo entra escalado desde el
        // tamaño viejo: es la parte de la animación que faltaba, porque
        // `posicion` solo movía la esquina y el tamaño cambiaba de golpe.
        let zoom = zoom * crate::ventanas::escala_resize(&window, window.geometry().size);
        // Una ventana que aún no tiene sitio no se dibuja. Enseñarla mientras
        // tanto significa un fotograma con el cliente pegado a la esquina
        // superior izquierda antes de saltar al centro, y ese salto se ve.
        if alfa <= 0.0 {
            continue;
        }
        // La posición de dibujo descuenta el marco del cliente: `geometry().loc`
        // es el hueco entre el borde del buffer y la ventana visible (sombras).
        let animada = crate::ventanas::posicion(&window, loc);
        let origen = (animada - window.geometry().loc.to_f64()).to_physical_precise_round(scale);
        let superficies = window.render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
            renderer,
            origen,
            Scale::from(scale),
            alfa,
        );

        if zoom == 1.0 {
            elementos.extend(superficies.into_iter().map(OverlayElement::Surface));
            continue;
        }
        if std::env::var_os("BOOKOS_PERFIL").is_some() {
            // La animación de una ventana no se puede capturar con las
            // herramientas del sistema —en Wayland puro no hay `import -window
            // root`—, así que la forma de comprobar que de verdad se dibujan
            // fotogramas intermedios es verlos pasar aquí.
            tracing::info!(
                alfa = format_args!("{alfa:.2}"),
                zoom = format_args!("{zoom:.3}"),
                "fotograma de entrada"
            );
        }
        // El zoom crece desde el centro de la ventana: con el origen en la
        // esquina, la ventana se despliega hacia abajo y a la derecha y parece
        // que entre deslizándose en diagonal, no que aparezca.
        let geo = window.geometry().size;
        let centro = (
            animada.x + geo.w as f64 / 2.0,
            animada.y + geo.h as f64 / 2.0,
        );
        let centro = Point::<f64, Logical>::from(centro).to_physical_precise_round(scale);
        elementos.extend(superficies.into_iter().map(|elemento| {
            OverlayElement::Escalada(RescaleRenderElement::from_element(elemento, centro, zoom))
        }));
    }

    // El fondo va el último de la lista, o sea el más atrás de todo: detrás de
    // las ventanas, del shell y del cristal.
    if let Some(fondo) = state.fondo.as_ref() {
        let (lw, lh) = state.pantalla_logica();
        if let Some(elemento) = fondo.elemento(renderer, (lw as i32, lh as i32)) {
            elementos.push(OverlayElement::Memory(elemento));
        }
    }

    if std::env::var_os("BOOKOS_PERFIL").is_some() {
        let t_ventanas = _t.elapsed();
        if t_cursor.as_millis() > 1 || t_shell.as_millis() > 1 || t_ventanas.as_millis() > 1 {
            tracing::info!(
                cursor_ms = format_args!("{:.2}", t_cursor.as_secs_f64() * 1000.0),
                shell_ms = format_args!("{:.2}", t_shell.as_secs_f64() * 1000.0),
                ventanas_ms = format_args!("{:.2}", t_ventanas.as_secs_f64() * 1000.0),
                "escena lenta"
            );
        }
    }
    elementos
}

/// El identificador estable de cada zona de cristal.
///
/// Tiene que ser el mismo de un frame al siguiente o el damage tracker cree que
/// cada frame trae elementos nuevos y repinta la pantalla entera.
fn id_cristal(i: usize) -> smithay::backend::renderer::element::Id {
    use smithay::backend::renderer::element::Id;
    use std::sync::OnceLock;
    static IDS: OnceLock<[Id; 2]> = OnceLock::new();
    IDS.get_or_init(|| std::array::from_fn(|_| Id::new()))[i.min(1)].clone()
}

/// A cuántos puntos por pulgada se ve un escritorio del tamaño correcto.
///
/// No son los 96 de la convención de X11, y por eso hay una constante y no una
/// división directa: 96 sale de los monitores CRT de los noventa y en un panel
/// moderno da escalas disparatadas. Esta pantalla —2880×1800 en 300×190 mm, o
/// sea 242 DPI— pide un 175 %, que es lo que se acaba eligiendo a mano en
/// cualquier escritorio; 242/1,75 = 138, de ahí el número.
///
/// Comprobado contra los casos que importan, y sale lo que la gente elige:
/// 1080p en 15,6" → 1,0; 1440p en 27" → 1,0; 4K en 27" → 1,25; 4K en 15,6" → 2,0.
const DPI_OBJETIVO: f64 = 138.0;

/// La escala que le toca a una pantalla por su tamaño físico.
///
/// Devuelve 1,0 cuando el monitor no dice cuánto mide —muchos proyectores y
/// algunas KVM mandan 0×0— porque inventar una escala a partir de un dato que
/// no existe es peor que quedarse en la que siempre funciona.
pub fn escala_sugerida(px: (i32, i32), mm: (i32, i32)) -> f64 {
    if mm.0 <= 0 || mm.1 <= 0 || px.0 <= 0 || px.1 <= 0 {
        return 1.0;
    }
    const MM_POR_PULGADA: f64 = 25.4;
    // Se promedian los dos ejes: un EDID con los milímetros redondeados hacia
    // distinto lado da DPI distintos en horizontal y en vertical, y elegir uno
    // de los dos sería arbitrario.
    let dpi_x = px.0 as f64 * MM_POR_PULGADA / mm.0 as f64;
    let dpi_y = px.1 as f64 * MM_POR_PULGADA / mm.1 as f64;
    let escala = (dpi_x + dpi_y) / 2.0 / DPI_OBJETIVO;

    // A cuartos, que es el paso que usan todos los escritorios: una escala de
    // 1,73 no se distingue de 1,75 y en cambio hace que ningún tamaño en
    // píxeles caiga redondo.
    let escala = (escala * 4.0).round() / 4.0;
    // Por debajo de 1 no se baja: encoger el escritorio no arregla ninguna
    // pantalla, y sí rompe el panel, que tiene alturas pensadas en lógicos.
    escala.clamp(1.0, 3.0)
}

#[cfg(test)]
mod tests {
    use super::escala_sugerida;

    #[test]
    fn esta_pantalla_pide_el_175_por_ciento() {
        // El Book5 Pro: 2880×1800 en 300×190 mm, leído con `--drm-info`. 175 %
        // es lo que se elige a mano en KDE en esta misma máquina, y es el caso
        // contra el que está calibrada la constante.
        assert_eq!(escala_sugerida((2880, 1800), (300, 190)), 1.75);
    }

    #[test]
    fn las_pantallas_normales_se_quedan_al_cien() {
        // Un portátil de 15,6" a 1080p y un monitor de 27" a 1440p: los dos
        // rondan los 110-140 DPI y no necesitan escalado. Si alguno de estos dos
        // sale a 1,25, el escritorio se ve enorme en media España.
        assert_eq!(escala_sugerida((1920, 1080), (345, 194)), 1.0);
        assert_eq!(escala_sugerida((2560, 1440), (597, 336)), 1.0);
    }

    #[test]
    fn el_4k_sube_segun_el_tamano_del_panel() {
        // El mismo número de píxeles pide cosas distintas según cuánto midan:
        // en 27" basta un cuarto más, en un portátil de 15,6" hace falta el
        // doble. Es justo lo que se pierde al cablear una escala fija.
        assert_eq!(escala_sugerida((3840, 2160), (597, 336)), 1.25);
        assert_eq!(escala_sugerida((3840, 2160), (345, 194)), 2.0);
    }

    #[test]
    fn sin_tamano_fisico_no_se_inventa_nada() {
        // Proyectores y algunas KVM anuncian 0×0. Deducir una escala de un dato
        // que no existe es peor que quedarse en la que siempre funciona.
        assert_eq!(escala_sugerida((1920, 1080), (0, 0)), 1.0);
        assert_eq!(escala_sugerida((0, 0), (300, 190)), 1.0);
    }

    #[test]
    fn siempre_sale_un_cuarto_exacto() {
        // De esto depende que los tamaños en lógicos caigan redondos. Se barren
        // pantallas plausibles en vez de un par de casos elegidos.
        for ancho in [1366, 1920, 2256, 2880, 3840] {
            for mm in [200, 250, 300, 345, 600] {
                let e = escala_sugerida((ancho, ancho * 10 / 16), (mm, mm * 10 / 16));
                assert_eq!(e * 4.0, (e * 4.0).round(), "{ancho}px en {mm}mm da {e}");
                assert!((1.0..=3.0).contains(&e));
            }
        }
    }
}

/// Programa el siguiente repintado del panel.
///
/// El momento lo eligen los widgets, no esta función: el reloj pide el cambio
/// de minuto, no un tick de 1 Hz, porque un temporizador de un segundo para
/// algo que solo enseña minutos son 59 despertares de más por minuto. Cada uno
/// saca a la CPU de su C-state, y esa es la clase de goteo que arruina la
/// autonomía en reposo.
///
/// Si ningún widget pide alarma no se programa **nada**: un panel de widgets
/// que solo reaccionan a eventos deja el proceso durmiendo en `epoll`.
pub fn schedule_panel_tick(state: &mut BookosComp) {
    let Some(delay) = state.shell.as_ref().and_then(|s| s.next_tick()) else {
        return;
    };
    let result = state
        .loop_handle
        .insert_source(Timer::from_duration(delay), |_, _, state| {
            refresh_panel(state);
            schedule_panel_tick(state);
            TimeoutAction::Drop
        });
    if let Err(err) = result {
        tracing::error!("no se pudo programar el tick del panel: {err}");
    }
}

/// Relee los estados y marca repintado **solo** si algo cambió de verdad.
fn refresh_panel(state: &mut BookosComp) {
    if state.shell.as_mut().is_some_and(|s| s.refresh()) {
        state.needs_redraw = true;
    }
}

/// Despierta el panel cuando el kernel avisa de un cambio de hardware.
///
/// El tick del reloj es de un minuto, y para la hora está bien. Para el resto
/// de estados no: enchufar el cargador, perder el wifi o subir el brillo tienen
/// que verse en el momento, y la alternativa —sondear sysfs varias veces por
/// segundo— es justo el goteo de despertares que este compositor evita.
///
/// El kernel ya manda esos avisos por netlink; aquí solo se escuchan. Entre
/// evento y evento el descriptor duerme en `epoll` y no cuesta nada.
#[cfg(feature = "udev")]
pub fn watch_hardware(state: &mut BookosComp) {
    use smithay::reexports::calloop::generic::Generic;
    use smithay::reexports::calloop::{Interest, Mode, PostAction};
    use smithay::reexports::udev::MonitorBuilder;

    // Los subsistemas los declaran los widgets: si el panel no lleva el de
    // brillo, el kernel no nos despierta al mover la tecla. Antes la lista
    // estaba a fuego y cualquier evento releía las cuatro fuentes de sysfs.
    //
    // `backlight` no incluye el brillo del teclado (`leds`) a propósito: no se
    // enseña, y despertar por cada pulsación de la tecla de retroiluminación
    // sería trabajo para nada.
    let subsistemas = state
        .shell
        .as_ref()
        .map(|s| s.subsistemas())
        .unwrap_or_default();
    if subsistemas.is_empty() {
        tracing::debug!("ningún widget pide avisos de hardware");
        return;
    }
    let monitor = MonitorBuilder::new()
        .and_then(|m| subsistemas.iter().try_fold(m, |m, sub| m.match_subsystem(sub)))
        .and_then(|m| m.listen());
    let monitor = match monitor {
        Ok(monitor) => monitor,
        Err(err) => {
            // Sin esto el panel sigue funcionando, solo que al ritmo del reloj.
            tracing::warn!("sin avisos de hardware, el panel irá al minuto: {err}");
            return;
        }
    };

    let source = Generic::new(monitor, Interest::READ, Mode::Level);
    let result = state
        .loop_handle
        .insert_source(source, |_, monitor, state| {
            // Hay que vaciar la cola sí o sí: la fuente es de nivel, así que un
            // evento sin leer volvería a despertarnos para siempre. Da igual
            // *qué* cambió — releer los cuatro ficheros de sysfs cuesta menos
            // que averiguarlo.
            let cambios = monitor.iter().count();
            if cambios > 0 {
                tracing::debug!(cambios, "aviso de hardware, releyendo el panel");
                refresh_panel(state);
            }
            Ok(PostAction::Continue)
        });
    if let Err(err) = result {
        tracing::error!("no se pudo escuchar los cambios de hardware: {err}");
    }
}

/// Sin el backend real no hay libudev enlazado. El panel se queda con el tick
/// del reloj, que para desarrollar anidado sobra.
#[cfg(not(feature = "udev"))]
pub fn watch_hardware(_state: &mut BookosComp) {}
