//! Backend de sesión real: DRM/KMS + libinput + libseat.
//!
//! Este es el camino de producción. A diferencia del anidado, aquí **no hay
//! ningún compositor debajo**: hablamos con el kernel directamente, así que es
//! el único sitio donde las mediciones de consumo significan algo y donde se
//! pueden usar las tres optimizaciones grandes del plan:
//!
//! - **Direct scanout.** `DrmOutput` envuelve un `DrmCompositor`, que reparte
//!   los elementos entre los planos del hardware. Una ventana opaca a pantalla
//!   completa va directa al plano primario sin componer nada en la GPU.
//! - **Cursor en plano de hardware.** Los elementos marcados `Kind::Cursor` se
//!   asignan al plano de cursor cuando cabe, y mover el ratón deja de repintar
//!   la escena.
//! - **Ritmo por vblank.** No hay temporizador de sondeo: el siguiente frame se
//!   dibuja cuando el hardware avisa de que enseñó el anterior. En reposo no se
//!   despierta nadie.
//!
//! Todo eso lo decide `DrmCompositor` según lo que quepa en los planos; el
//! Book5 anuncia 12, así que hay sitio de sobra.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::drm::compositor::FrameFlags;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmNode};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent};
use smithay::desktop::utils::surface_primary_scanout_output;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::drm::control::{connector, crtc, Device as _, ModeTypeFlags};
use smithay::reexports::drm::control::property::Value as PropValue;
use smithay::reexports::input::{AccelProfile, ClickMethod, Device as InputDevice, Libinput};
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::DeviceFd;

use crate::cursor::OverlayElement;
use crate::pantallas::{self, Aplicado, Modo, Peticion, Salida};
use crate::state::BookosComp;

type Alloc = GbmAllocator<DrmDeviceFd>;
/// El exportador convierte los buffers GBM en framebuffers de DRM. No vale el
/// `GbmDevice` pelado: hace falta este envoltorio.
type Exporter = GbmFramebufferExporter<DrmDeviceFd>;
type Manager = DrmOutputManager<Alloc, Exporter, (), DrmDeviceFd>;
type Surface = DrmOutput<Alloc, Exporter, (), DrmDeviceFd>;

/// Lo que el backend necesita y el estado del compositor no debe conocer.
struct Udev {
    manager: Manager,
    renderer: GlesRenderer,
    output: Output,
    surface: Surface,
    /// El conector encendido y su identificador estable. Se guardan porque el
    /// censo y el aplicador tienen que saber **cuál** de las pantallas
    /// enumeradas es la que se está dibujando, y el `Output` de Smithay solo
    /// lleva el nombre del conector, que no es estable entre arranques.
    conector: connector::Handle,
    id: String,
    /// Hay un frame en el aire esperando su vblank. Encolar otro antes de que
    /// llegue agota los buffers del swapchain.
    en_vuelo: bool,
    /// La sesión está en segundo plano (has cambiado de TTY): no se dibuja.
    activa: bool,
}

pub fn run(
    event_loop: &mut EventLoop<'static, BookosComp>,
    state: &mut BookosComp,
    client: Option<String>,
) -> anyhow::Result<()> {
    // libseat nos da el asiento: acceso a la GPU y a los dispositivos de
    // entrada sin ser root, y el cambio de TTY.
    let (mut session, notifier) = LibSeatSession::new()
        .map_err(|err| anyhow::anyhow!("no se pudo abrir la sesión de libseat: {err}"))?;

    // El cambio de TTY se expone al resto del compositor como un cierre: es lo
    // único que `keybinds` necesita de libseat, y así el estado no arrastra el
    // tipo de sesión. `change_vt` es asíncrono — libseat contesta con un
    // PauseSession que ya se atiende más abajo.
    {
        let session = session.clone();
        state.cambiar_vt = Some(Box::new(move |vt| {
            // `session` es Clone y comparte el asiento, pero `change_vt` pide
            // &mut; el clon local lo hace posible sin sincronización.
            let mut session = session.clone();
            if let Err(err) = session.change_vt(vt) {
                tracing::error!(vt, "no se pudo cambiar de TTY: {err}");
            }
        }));
    }

    let gpu = elegir_gpu(&session)?;
    tracing::info!(?gpu, "GPU elegida");

    let fd = session
        .open(
            gpu.dev_path().as_deref().unwrap_or(std::path::Path::new("")),
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(|err| anyhow::anyhow!("no se pudo abrir la GPU: {err}"))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));

    // `true` desactiva los conectores que el kernel dejó encendidos: partimos de
    // un estado conocido en vez de heredar lo que hubiera antes.
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd.clone())?;

    // El renderizador va sobre el mismo dispositivo GBM, así que lo que dibuja
    // se puede escanear directamente sin copiar entre GPUs.
    let display = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&display)?;
    let mut renderer = unsafe { GlesRenderer::new(context)? };

    let render_formats = renderer.egl_context().dmabuf_render_formats().clone();
    let allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let mut manager = Manager::new(
        drm,
        allocator,
        GbmFramebufferExporter::new(gbm.clone(), Some(gpu)),
        Some(gbm),
        // Formatos de color aceptables para el plano primario, en orden de
        // preferencia. Con alfa primero para poder componer sin copia extra.
        [
            smithay::backend::allocator::Fourcc::Argb8888,
            smithay::backend::allocator::Fourcc::Xrgb8888,
        ],
        render_formats,
    );

    let (output, surface, conector, id_salida) = crear_salida(&mut manager, &mut renderer, state)?;

    let escala = output.current_scale().fractional_scale();
    // La escala puede venir de `pantallas.conf` y no de `panel.conf`, así que
    // hay que reponerla aquí: el resto del compositor la lee de este campo
    // cuando la pantalla cambia de tamaño, y si no, volvería a la de la
    // configuración vieja.
    state.escala_forzada = Some(escala);
    let modo = output.current_mode().map(|m| m.size).unwrap_or_default();
    state.shell = Some(crate::shell::ShellHost::new(
        modo.w.max(1) as u32,
        modo.h.max(1) as u32,
        escala as f32,
        state.config.take(),
    ));
    if let Some(shell) = state.shell.as_mut() {
        shell.refresh();
    }
    // El indicador de escritorios no puede saber cuántos hay: se lo decimos
    // antes del primer frame, para que salga pintado con el resto del panel y
    // no aparezca un instante después.
    crate::escritorios::avisar_al_panel(state);
    state.cursor_theme = Some(crate::cursor::CursorTheme::con_tamano(escala, state.cursor_nominal));
    state.cristal = crate::desenfoque::Cristal::new(&mut renderer);
    state.genio = crate::genio::Genio::new(&mut renderer);
    state.fondo = crate::fondo::Fondo::cargar(state.fondo_config.as_deref());
    // El cristal desenfoca el fondo, así que necesita su propia copia con
    // mipmaps. Se sube aquí, una vez, y no se vuelve a tocar.
    if let (Some(fondo), Some(cristal)) = (state.fondo.as_ref(), state.cristal.as_ref()) {
        let (rgba, tam) = fondo.rgba();
        cristal.borrow_mut().preparar(&mut renderer, rgba, tam);
    }
    state.space.map_output(&output, (0, 0));

    let udev = Rc::new(RefCell::new(Udev {
        manager,
        renderer,
        output,
        surface,
        conector,
        id: id_salida,
        en_vuelo: false,
        activa: true,
    }));

    // --- Pantallas ---------------------------------------------------------
    // Los dos cierres que van del hilo del compositor a KMS. Nadie más toca el
    // backend: D-Bus solo empuja peticiones por el canal de calloop, y estas se
    // ejecutan aquí, entre fotogramas.
    *state.pantallas.compartido.backend.lock().unwrap_or_else(|e| e.into_inner()) = "udev";
    {
        let udev = udev.clone();
        state.censar_pantallas = Some(Box::new(move || match udev.try_borrow() {
            Ok(u) => censar(&u),
            Err(_) => Vec::new(),
        }));
    }
    {
        let udev = udev.clone();
        state.aplicar_pantallas = Some(Box::new(move |peticion| {
            let mut u = udev
                .try_borrow_mut()
                .map_err(|_| "el backend está ocupado dibujando".to_string())?;
            aplicar_en_kms(&mut u, peticion)
        }));
    }
    {
        let salidas = censar(&udev.borrow());
        state.pantallas.ultima = pantallas::peticion_de(&salidas);
        state.pantallas.compartido.publicar(salidas);
    }

    // Enchufar o quitar un monitor lo cuenta el kernel por netlink: no hay
    // ningún temporizador mirando si cambió algo. Solo se vuelve a censar y se
    // avisa por `OutputsChanged`; decidir qué hacer con la pantalla nueva es
    // cosa de quien configure.
    match UdevBackend::new(session.seat()) {
        Ok(backend) => {
            let r = event_loop.handle().insert_source(backend, |evento, _, state| {
                if let UdevEvent::Changed { .. } = evento {
                    crate::ajustes::recibir(state, crate::ajustes::Aviso::Redetectar);
                }
            });
            if let Err(err) = r {
                tracing::warn!("sin aviso de conexión de monitores: {err}");
            }
        }
        Err(err) => tracing::warn!("sin aviso de conexión de monitores: {err}"),
    }

    // --- Entrada -----------------------------------------------------------
    // Los touchpads abiertos se guardan para poder encenderlos y apagarlos con
    // la tecla de función: `DeviceAdded` solo llega con los que aparecen, y el
    // del portátil aparece al arrancar.
    let touchpads: Rc<RefCell<Vec<InputDevice>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let touchpads = touchpads.clone();
        state.aplicar_touchpad = Some(Box::new(move |activo| {
            for device in touchpads.borrow_mut().iter_mut() {
                let _ = device.config_send_events_set_mode(if activo {
                    smithay::reexports::input::SendEventsMode::ENABLED
                } else {
                    smithay::reexports::input::SendEventsMode::DISABLED
                });
            }
        }));
    }

    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    libinput
        .udev_assign_seat(&session.seat())
        .map_err(|_| anyhow::anyhow!("no se pudo asignar el asiento a libinput"))?;
    event_loop
        .handle()
        .insert_source(LibinputInputBackend::new(libinput.clone()), move |event, _, state| {
            // Los ajustes se aplican aquí y no en `input::handle` porque ese
            // está escrito contra `InputBackend` genérico y no puede tocar la
            // API de libinput. Llega uno por dispositivo al arrancar y otra
            // tanda tras cada cambio de TTY, cuando libseat los devuelve.
            if let smithay::backend::input::InputEvent::DeviceAdded { device } = &event {
                let mut device = device.clone();
                configurar_dispositivo(&mut device, &state.entrada, state.touchpad_activo);
                if device.config_tap_finger_count() > 0 {
                    touchpads.borrow_mut().push(device);
                }
            }
            crate::input::handle(state, event);
        })
        .map_err(|err| anyhow::anyhow!("insert_source(libinput): {err}"))?;

    // --- Vblank ------------------------------------------------------------
    {
        let udev = udev.clone();
        event_loop
            .handle()
            .insert_source(drm_notifier, move |event, _, state| match event {
                DrmEvent::VBlank(_crtc) => {
                    let mut u = udev.borrow_mut();
                    // Cerrar el frame anterior antes de plantearse otro: si no,
                    // el swapchain se queda sin buffers libres.
                    if let Err(err) = u.surface.frame_submitted() {
                        tracing::warn!("frame_submitted: {err}");
                    }
                    u.en_vuelo = false;
                    // Igual que en winit: una animación en marcha pide el
                    // siguiente fotograma aunque nadie haya marcado nada, porque
                    // su paso se calcula al componer.
                    if state.needs_redraw || state.hay_animacion() {
                        dibujar(state, &mut u);
                    }
                }
                DrmEvent::Error(err) => tracing::error!("error de DRM: {err}"),
            })
            .map_err(|err| anyhow::anyhow!("insert_source(drm): {err}"))?;
    }

    // --- Cambio de TTY -----------------------------------------------------
    {
        let udev = udev.clone();
        let mut libinput = libinput;
        event_loop
            .handle()
            .insert_source(notifier, move |event, _, state| {
                let mut u = udev.borrow_mut();
                match event {
                    SessionEvent::PauseSession => {
                        // Otro TTY toma el control: se suelta el KMS y se deja
                        // de dibujar hasta volver.
                        tracing::info!("sesión en pausa (cambio de TTY)");
                        libinput.suspend();
                        u.manager.pause();
                        u.activa = false;
                        u.en_vuelo = false;
                    }
                    SessionEvent::ActivateSession => {
                        tracing::info!("sesión reactivada");
                        if libinput.resume().is_err() {
                            tracing::error!("libinput no pudo reanudar");
                        }
                        u.activa = true;
                        // Reactivar fuerza un modeset: mientras no éramos
                        // nosotros, otro pudo dejar la pantalla en otro modo.
                        // `false` = no apagar los conectores que ya estaban.
                        if let Err(err) = u.manager.activate(false) {
                            tracing::warn!("activate: {err}");
                        }
                        u.surface.reset_buffers();
                        state.needs_redraw = true;
                        dibujar(state, &mut u);
                    }
                }
            })
            .map_err(|err| anyhow::anyhow!("insert_source(session): {err}"))?;
    }

    crate::backend::schedule_panel_tick(state);
    crate::backend::watch_hardware(state);

    // Primer frame: deja el escritorio pintado antes de que arranque nada más,
    // que es lo que evita el parpadeo al iniciar sesión.
    dibujar(state, &mut udev.borrow_mut());

    tracing::info!(socket = ?state.socket_name, "compositor listo (sesión real)");

    tracing::info!(
        "atajos: Ctrl+Alt+F1..F12 cambia de TTY · Meta+Return abre un terminal · \
         Meta+Q cierra la ventana · Ctrl+Alt+Retroceso sale"
    );

    if let Some(cmd) = client {
        crate::keybinds::lanzar(state, &cmd);
    }

    let udev_loop = udev.clone();
    event_loop.run(None, state, move |state| {
        // Si hay trabajo pendiente y no hay frame en el aire, se dibuja al
        // cerrar la vuelta del bucle. El resto del tiempo el proceso duerme en
        // epoll: no hay sondeo de ningún tipo.
        if state.needs_redraw || state.hay_animacion() {
            match udev_loop.try_borrow_mut() {
                Ok(mut u) => dibujar(state, &mut u),
                // No debería pasar: este callback corre entre vueltas del
                // bucle, con todos los callbacks de las fuentes ya cerrados.
                // Pero si pasara, descartar el frame en silencio dejaría la
                // pantalla congelada para siempre en un escritorio en reposo:
                // `needs_redraw` sigue a `true` y nadie vuelve a mirarlo hasta
                // el siguiente evento, que puede no llegar nunca. El timer
                // fuerza otra vuelta.
                Err(err) => {
                    tracing::warn!("el backend estaba prestado al ir a dibujar: {err}");
                    let r = state.loop_handle.insert_source(
                        Timer::immediate(),
                        |_, _, _| TimeoutAction::Drop,
                    );
                    if let Err(err) = r {
                        tracing::error!("no se pudo reintentar el dibujo: {err}");
                    }
                }
            }
        }
        state.post_dispatch();
    })?;
    Ok(())
}

/// La GPU que manda. Con una sola integrada esto es trivial, pero conviene
/// dejarlo explícito para cuando haya híbridas.
fn elegir_gpu(session: &LibSeatSession) -> anyhow::Result<DrmNode> {
    if let Some(path) = primary_gpu(session.seat())? {
        return Ok(DrmNode::from_path(path)?);
    }
    let path = all_gpus(session.seat())?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no hay ninguna GPU disponible en este asiento"))?;
    Ok(DrmNode::from_path(path)?)
}

/// Enciende el primer conector conectado, con lo que diga la configuración
/// guardada o, si no hay nada guardado para él, con su modo preferido.
///
/// **Solo se enciende uno.** El resto de conectores se enumeran en el censo
/// —Settings los ve, con sus modos y su EDID— pero no se les asigna un crtc:
/// para eso hace falta una `DrmOutput` por salida y componer la escena varias
/// veces por frame, y eso es un cambio de otra magnitud. Ver `censar`.
fn crear_salida(
    manager: &mut Manager,
    renderer: &mut GlesRenderer,
    state: &mut BookosComp,
) -> anyhow::Result<(Output, Surface, connector::Handle, String)> {
    let recursos = manager.device().resource_handles()?;
    let guardadas = pantallas::cargar();

    let (conector, crtc, modo, guardada) = recursos
        .connectors()
        .iter()
        .filter_map(|handle| manager.device().get_connector(*handle, true).ok())
        .filter(|con| con.state() == connector::State::Connected)
        .find_map(|con| {
            let id = identificador(manager.device(), &con);
            // Lo guardado manda sobre el modo preferido, pero solo si el modo
            // sigue existiendo: un monitor puede perder modos al cambiar de
            // cable, y aplicar uno que ya no está deja la pantalla apagada.
            let guardada = guardadas
                .iter()
                .find(|g| g.id == id && g.activa)
                .and_then(|g| {
                    let existe = con.modes().iter().any(|m| {
                        let (w, h) = m.size();
                        w as u32 == g.ancho && h as u32 == g.alto
                            && m.vrefresh() * 1000 == g.refresco_mhz
                    });
                    existe.then(|| g.clone())
                });
            let modo = guardada
                .as_ref()
                .and_then(|g| {
                    con.modes().iter().find(|m| {
                        let (w, h) = m.size();
                        w as u32 == g.ancho && h as u32 == g.alto
                            && m.vrefresh() * 1000 == g.refresco_mhz
                    })
                })
                // El modo preferido es el que anuncia el panel como nativo.
                .or_else(|| {
                    con.modes()
                        .iter()
                        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
                })
                .or_else(|| con.modes().first())
                .copied()?;
            let crtc = primer_crtc(manager, &recursos, &con)?;
            Some((con, crtc, modo, guardada))
        })
        .ok_or_else(|| anyhow::anyhow!("no hay ninguna pantalla conectada"))?;

    let nombre = format!("{:?}-{}", conector.interface(), conector.interface_id());
    let id = identificador(manager.device(), &conector);
    let (mm_w, mm_h) = conector.size().unwrap_or((0, 0));
    let (w, h) = modo.size();
    let (fabricante, modelo_edid, _serie) = datos_edid(manager.device(), &conector);
    tracing::info!(
        pantalla = %nombre,
        id = %id,
        modo = %format!("{}x{}@{}", w, h, modo.vrefresh()),
        restaurada = guardada.is_some(),
        "encendiendo salida"
    );

    let output = Output::new(
        nombre,
        PhysicalProperties {
            size: (mm_w as i32, mm_h as i32).into(),
            subpixel: Subpixel::Unknown,
            // Lo que dice el EDID, no "BookOS": es lo que los clientes enseñan
            // cuando preguntan en qué pantalla están.
            make: if fabricante.is_empty() { "BookOS".into() } else { fabricante },
            model: if modelo_edid.is_empty() { "KMS".into() } else { modelo_edid },
        },
    );
    let output_mode = OutputMode {
        size: (w as i32, h as i32).into(),
        refresh: (modo.vrefresh() * 1000) as i32,
    };
    output.create_global::<BookosComp>(&state.display_handle);

    // La escala la manda lo guardado; si no, la configuración; si tampoco, se
    // deduce del tamaño físico que anuncia el panel. Estaba cableada a 1,0, y
    // en un portátil de 242 DPI eso significaba un panel de 32 px físicos y
    // texto ilegible: el escritorio solo se podía usar anidado.
    let escala = guardada
        .as_ref()
        .map(|g| g.escala)
        .or_else(|| state.config.as_ref().and_then(|c| c.escala))
        .unwrap_or_else(|| super::escala_sugerida((w as i32, h as i32), (mm_w as i32, mm_h as i32)));
    let transformacion = guardada
        .as_ref()
        .and_then(|g| pantallas::transformacion(&g.transformacion))
        .unwrap_or(smithay::utils::Transform::Normal);
    let posicion: smithay::utils::Point<i32, smithay::utils::Logical> =
        guardada.as_ref().map_or((0, 0).into(), |g| (g.x, g.y).into());
    tracing::info!(
        escala,
        automatica = guardada.is_none() && state.config.as_ref().and_then(|c| c.escala).is_none(),
        logico = %format!("{}x{}", (w as f64 / escala) as i32, (h as f64 / escala) as i32),
        "escala de la pantalla"
    );
    output.change_current_state(
        Some(output_mode),
        Some(transformacion),
        Some(Scale::Fractional(escala)),
        Some(posicion),
    );
    output.set_preferred(output_mode);

    let surface = manager
        .initialize_output(
            crtc,
            modo,
            &[conector.handle()],
            &output,
            None,
            renderer,
            &DrmOutputRenderElements::<GlesRenderer, OverlayElement>::default(),
        )
        .map_err(|err| anyhow::anyhow!("no se pudo inicializar la salida: {err}"))?;

    // La frecuencia variable no se hereda: si la configuración guardada la
    // pedía, se pide aquí, y si el panel no puede se sigue sin ella.
    if guardada.as_ref().is_some_and(|g| g.vrr) {
        let r = surface.with_compositor(|c| c.use_vrr(true));
        if let Err(err) = r {
            tracing::warn!("no se pudo activar la frecuencia variable: {err}");
        }
    }

    Ok((output, surface, conector.handle(), id))
}

// ── Censo de pantallas ───────────────────────────────────────────────────

/// Fabricante, modelo y número de serie del EDID del monitor.
///
/// Se leen a mano y no con `libdisplay-info` porque hacen falta tres campos de
/// los primeros 128 bytes y la biblioteca traería una dependencia de sistema
/// nueva para eso. El formato de esos bytes lleva congelado desde EDID 1.0.
fn datos_edid(device: &DrmDevice, con: &connector::Info) -> (String, String, String) {
    let Some(bytes) = blob_edid(device, con) else {
        return (String::new(), String::new(), String::new());
    };
    if bytes.len() < 128 {
        return (String::new(), String::new(), String::new());
    }

    // Bytes 8-9: tres letras de cinco bits, en big-endian, con 'A' = 1.
    let id = u16::from_be_bytes([bytes[8], bytes[9]]);
    let letra = |desplazamiento: u16| -> char {
        let v = ((id >> desplazamiento) & 0x1f) as u8;
        if (1..=26).contains(&v) {
            (b'A' + v - 1) as char
        } else {
            '?'
        }
    };
    let fabricante: String = [letra(10), letra(5), letra(0)].iter().collect();

    // Los cuatro descriptores de 18 bytes que empiezan en el 54. El 0xFC lleva
    // el nombre del monitor y el 0xFF su número de serie, ambos en ASCII
    // terminado en 0x0A y rellenado con espacios.
    let mut modelo = String::new();
    let mut serie = String::new();
    for i in 0..4 {
        let d = &bytes[54 + i * 18..54 + i * 18 + 18];
        if d[0] != 0 || d[1] != 0 || d[2] != 0 {
            continue;
        }
        let texto: String = d[5..18]
            .iter()
            .take_while(|&&b| b != 0x0a)
            .map(|&b| b as char)
            .collect();
        match d[3] {
            0xfc => modelo = texto.trim().to_string(),
            0xff => serie = texto.trim().to_string(),
            _ => {}
        }
    }
    if modelo.is_empty() {
        // Sin descriptor de nombre queda el código de producto, que al menos
        // distingue dos paneles del mismo fabricante.
        modelo = format!("{:04X}", u16::from_le_bytes([bytes[10], bytes[11]]));
    }
    if serie.is_empty() {
        let n = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        if n != 0 {
            serie = n.to_string();
        }
    }
    (fabricante, modelo, serie)
}

fn blob_edid(device: &DrmDevice, con: &connector::Info) -> Option<Vec<u8>> {
    let props = device.get_properties(con.handle()).ok()?;
    let (handles, valores) = props.as_props_and_values();
    for (handle, valor) in handles.iter().zip(valores.iter()) {
        let info = device.get_property(*handle).ok()?;
        if info.name().to_str().ok()? != "EDID" {
            continue;
        }
        if let PropValue::Blob(id) = info.value_type().convert_value(*valor) {
            return device.get_property_blob(id).ok();
        }
    }
    None
}

/// ¿El conector admite frecuencia variable? Es una propiedad del conector, así
/// que se puede saber sin encenderlo.
fn vrr_capaz(device: &DrmDevice, con: &connector::Info) -> bool {
    let Ok(props) = device.get_properties(con.handle()) else {
        return false;
    };
    let (handles, valores) = props.as_props_and_values();
    for (handle, valor) in handles.iter().zip(valores.iter()) {
        let Ok(info) = device.get_property(*handle) else {
            continue;
        };
        if info.name().to_str() != Ok("vrr_capable") {
            continue;
        }
        return match info.value_type().convert_value(*valor) {
            PropValue::Boolean(v) => v,
            PropValue::UnsignedRange(v) => v != 0,
            _ => false,
        };
    }
    false
}

/// Identificador estable de una pantalla. Ver la cabecera de
/// [`crate::pantallas`] para el porqué.
fn identificador(device: &DrmDevice, con: &connector::Info) -> String {
    let (fabricante, modelo, serie) = datos_edid(device, con);
    let conector = format!("{:?}-{}", con.interface(), con.interface_id());
    let bruto = if fabricante.is_empty() && modelo.is_empty() {
        conector
    } else if serie.is_empty() {
        // Sin número de serie dos monitores idénticos comparten identificador,
        // así que se desempata con el conector. Es peor que el EDID solo —
        // cambiar de puerto cambia el id— pero es lo único que los distingue.
        format!("{fabricante}-{modelo}-{conector}")
    } else {
        format!("{fabricante}-{modelo}-{serie}")
    };
    bruto
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Los modos de un conector, ya en la forma que viaja por D-Bus.
///
/// El refresco sale de `vrefresh`, que el kernel da redondeado a hercios
/// enteros: 90 y 120 salen exactos, y un modo de 59,94 se anuncia como 60000
/// mHz. Es la misma cuenta que ya hacía el `OutputMode` de esta salida, y
/// mantenerla igual es lo que permite emparejar por número lo que pide Settings
/// con lo que tiene el conector.
fn modos_de(con: &connector::Info, actual: Option<&smithay::reexports::drm::control::Mode>) -> Vec<Modo> {
    con.modes()
        .iter()
        .map(|m| {
            let (w, h) = m.size();
            Modo {
                ancho: w as u32,
                alto: h as u32,
                refresco_mhz: m.vrefresh() * 1000,
                preferido: m.mode_type().contains(ModeTypeFlags::PREFERRED),
                actual: actual.is_some_and(|a| a.size() == m.size() && a.vrefresh() == m.vrefresh()),
            }
        })
        .collect()
}

/// El censo de todo lo que hay enchufado.
///
/// La única salida con `activa = true` es la que tiene crtc. Las demás se
/// enumeran enteras —modos, EDID, tamaño físico— para que Settings pueda
/// enseñarlas y avisar de que todavía no se pueden encender, en vez de hacer
/// como que no existen.
fn censar(u: &Udev) -> Vec<Salida> {
    let device = u.manager.device();
    let Ok(recursos) = device.resource_handles() else {
        return Vec::new();
    };
    let escala = u.output.current_scale().fractional_scale();
    let transformacion = pantallas::nombre_transformacion(u.output.current_transform());
    let posicion = u.output.current_location();
    let modo_actual = u.surface.with_compositor(|c| c.pending_mode());
    let vrr = u.surface.with_compositor(|c| c.vrr_enabled());

    recursos
        .connectors()
        .iter()
        .filter_map(|handle| device.get_connector(*handle, true).ok())
        .filter(|con| con.state() == connector::State::Connected)
        .map(|con| {
            let encendida = con.handle() == u.conector;
            let (fabricante, modelo, serie) = datos_edid(device, &con);
            let (mm_w, mm_h) = con.size().unwrap_or((0, 0));
            let modos = modos_de(&con, encendida.then_some(&modo_actual));
            let escala = if encendida {
                escala
            } else {
                // Todavía no está encendida: se enseña lo que se le pondría.
                let preferido = modos.iter().find(|m| m.preferido).or_else(|| modos.first());
                preferido.map_or(1.0, |m| {
                    super::escala_sugerida(
                        (m.ancho as i32, m.alto as i32),
                        (mm_w as i32, mm_h as i32),
                    )
                })
            };
            let (ancho, alto) = modos
                .iter()
                .find(|m| m.actual)
                .or_else(|| modos.iter().find(|m| m.preferido))
                .or_else(|| modos.first())
                .map_or((0, 0), |m| (m.ancho, m.alto));
            let transformacion = if encendida { transformacion } else { "normal" };
            let (logico_ancho, logico_alto) =
                pantallas::tamano_logico(ancho, alto, escala, transformacion);
            Salida {
                id: identificador(device, &con),
                conector: format!("{:?}-{}", con.interface(), con.interface_id()),
                fabricante,
                modelo,
                serie,
                mm_ancho: mm_w,
                mm_alto: mm_h,
                activa: encendida,
                modos,
                escala,
                escalas: pantallas::ESCALAS.to_vec(),
                x: if encendida { posicion.x } else { 0 },
                y: if encendida { posicion.y } else { 0 },
                transformacion: transformacion.to_string(),
                vrr_capaz: vrr_capaz(device, &con),
                vrr: encendida && vrr,
                principal: encendida,
                logico_ancho,
                logico_alto,
            }
        })
        .collect()
}

/// Aplica una configuración ya validada al hardware.
///
/// Devuelve el censo releído, no lo que se pidió: si el driver acabó en otro
/// sitio, lo que gana es lo que hay.
fn aplicar_en_kms(u: &mut Udev, peticion: &[Peticion]) -> Result<Aplicado, String> {
    let mia = peticion
        .iter()
        .find(|p| p.id == u.id)
        .ok_or_else(|| format!("la configuración no dice nada de «{}»", u.id))?;

    if !mia.activa {
        return Err(
            "este backend enciende una sola pantalla: apagarla dejaría la sesión sin ninguna".into(),
        );
    }
    if let Some(otra) = peticion.iter().find(|p| p.id != u.id && p.activa) {
        return Err(format!(
            "«{}» no se puede encender todavía: el compositor solo maneja una salida",
            otra.id
        ));
    }

    let transformacion = pantallas::transformacion(&mia.transformacion)
        .ok_or_else(|| format!("rotación «{}» desconocida", mia.transformacion))?;

    // El modo de DRM que corresponde a lo pedido. Se busca en el conector y no
    // se construye a mano: un `Mode` inventado no lleva los tiempos reales y el
    // modeset lo rechaza.
    let device = u.manager.device();
    let con = device
        .get_connector(u.conector, true)
        .map_err(|err| format!("no se pudo leer el conector: {err}"))?;
    let modo_drm = con
        .modes()
        .iter()
        .find(|m| {
            let (w, h) = m.size();
            w as u32 == mia.ancho && h as u32 == mia.alto && m.vrefresh() * 1000 == mia.refresco_mhz
        })
        .copied()
        .ok_or_else(|| {
            format!(
                "el conector no tiene el modo {}x{}@{}",
                mia.ancho, mia.alto, mia.refresco_mhz
            )
        })?;

    let modo_previo = u.surface.with_compositor(|c| c.pending_mode());
    let cambia_modo = modo_previo.size() != modo_drm.size()
        || modo_previo.vrefresh() != modo_drm.vrefresh();
    if cambia_modo {
        let Udev { surface, renderer, .. } = u;
        surface
            .use_mode(
                modo_drm,
                renderer,
                &DrmOutputRenderElements::<GlesRenderer, OverlayElement>::default(),
            )
            .map_err(|err| format!("el hardware rechazó el modo: {err}"))?;
    }

    // El `DrmCompositor` toma la escala y la rotación del `Output` en cada
    // frame (`OutputModeSource::Auto`), así que cambiarlas aquí basta: no hace
    // falta ningún modeset para rotar ni para reescalar.
    let output_mode = OutputMode {
        size: (mia.ancho as i32, mia.alto as i32).into(),
        refresh: mia.refresco_mhz as i32,
    };
    u.output.change_current_state(
        Some(output_mode),
        Some(transformacion),
        Some(Scale::Fractional(mia.escala)),
        Some((mia.x, mia.y).into()),
    );

    if u.surface.with_compositor(|c| c.vrr_enabled()) != mia.vrr {
        let r = u.surface.with_compositor(|c| c.use_vrr(mia.vrr));
        if let Err(err) = r {
            // No es motivo para tirar el resto de la configuración: el modo y
            // la escala ya están puestos y se ven.
            tracing::warn!(vrr = mia.vrr, "no se pudo cambiar la frecuencia variable: {err}");
        }
    }

    if cambia_modo {
        // Los buffers del swapchain tienen el tamaño del modo anterior.
        u.surface.reset_buffers();
    }

    Ok(Aplicado {
        salidas: censar(u),
        mapa: vec![(u.output.clone(), (mia.x, mia.y).into())],
        principal: 0,
    })
}

/// Un crtc capaz de manejar este conector.
fn primer_crtc(
    manager: &Manager,
    recursos: &smithay::reexports::drm::control::ResourceHandles,
    con: &connector::Info,
) -> Option<crtc::Handle> {
    con.encoders()
        .iter()
        .filter_map(|enc| manager.device().get_encoder(*enc).ok())
        .find_map(|enc| recursos.filter_crtcs(enc.possible_crtcs()).first().copied())
}

/// Dibuja un frame y lo encola para escaneo.
fn dibujar(state: &mut BookosComp, u: &mut Udev) {
    if !u.activa || u.en_vuelo {
        return;
    }

    let elementos = crate::backend::escena(state, &mut u.renderer, &u.output);

    // `FrameFlags::DEFAULT` deja que el compositor use los planos: es lo que
    // habilita el direct scanout y el cursor por hardware.
    let resultado = u.surface.render_frame(
        &mut u.renderer,
        &elementos,
        [0.05, 0.05, 0.06, 1.0],
        FrameFlags::DEFAULT,
    );

    let resultado = match resultado {
        Ok(r) => r,
        Err(err) => {
            tracing::error!("fallo al dibujar: {err}");
            return;
        }
    };

    if resultado.is_empty {
        // Nada cambió: no se toca la pantalla. Este es el caso normal en un
        // escritorio en reposo.
        state.needs_redraw = false;
        state.frames.skipped += 1;
        return;
    }

    match u.surface.queue_frame(()) {
        Ok(()) => {
            u.en_vuelo = true;
            state.frames.submitted += 1;
        }
        Err(err) => tracing::error!("queue_frame: {err}"),
    }
    state.needs_redraw = false;

    // Los frame callbacks salen después de encolar: es la señal de "puedes
    // dibujar el siguiente".
    let tiempo = state.start_time.elapsed();
    for window in state.space.elements() {
        window.send_frame(
            &u.output,
            tiempo,
            Some(Duration::ZERO),
            surface_primary_scanout_output,
        );
    }
    if state
        .shell
        .as_ref()
        .is_some_and(|s| s.vista_escritorios_abierta())
    {
        let activo = state.escritorios.activo();
        for (i, ventanas) in crate::escritorios::ventanas_para_vista(state).iter().enumerate() {
            if i == activo {
                continue;
            }
            for (window, _) in ventanas {
                window.send_frame(
                    &u.output,
                    tiempo,
                    Some(Duration::ZERO),
                    surface_primary_scanout_output,
                );
            }
        }
    }
}

/// Aplica los ajustes de entrada a un dispositivo recién aparecido.
///
/// Sin esto los dispositivos se quedan con los valores de fábrica de libinput,
/// que en un portátil significa touchpad lento, sin toque para hacer clic y con
/// el scroll invertido respecto a lo que hace todo lo demás. No es un detalle
/// de comodidad: es la diferencia entre poder usar el escritorio o no.
///
/// El `Device` llega clonado, y da igual: el clon es un `libinput_device_ref`
/// del mismo dispositivo, no una copia de sus ajustes.
fn configurar_dispositivo(
    device: &mut InputDevice,
    cfg: &bookos_shell::Entrada,
    touchpad_activo: bool,
) {
    // Un `config_*_set_*` que falla casi siempre es "este dispositivo no tiene
    // eso" —un teclado no tiene aceleración— y no un error que merezca traza.
    // Lo que sí merece traza es el resultado, para poder ver de un vistazo qué
    // le tocó a cada dispositivo cuando algo se sienta raro.
    let touchpad = device.config_tap_finger_count() > 0;
    let mut aplicado = Vec::new();

    if device.config_accel_is_available() {
        let velocidad = if touchpad {
            cfg.velocidad_touchpad
        } else {
            cfg.velocidad_raton
        };
        // Adaptativo y no plano: es el que acelera según lo rápido que muevas,
        // o sea el que deja llegar de esquina a esquina de una pasada sin
        // perder precisión al ir despacio. Es también lo que hacen KWin y
        // GNOME por defecto.
        let _ = device.config_accel_set_profile(AccelProfile::Adaptive);
        if device.config_accel_set_speed(velocidad).is_ok() {
            aplicado.push(format!("velocidad={velocidad}"));
        }
    }

    if touchpad {
        // Apagado con la tecla de función, el dispositivo se desactiva entero:
        // así no llegan ni toques ni desplazamientos, que es lo que se espera
        // al apagarlo para escribir sin que la palma mueva el cursor.
        let _ = device.config_send_events_set_mode(if touchpad_activo {
            smithay::reexports::input::SendEventsMode::ENABLED
        } else {
            smithay::reexports::input::SendEventsMode::DISABLED
        });
        if device.config_tap_set_enabled(cfg.toque_para_clic).is_ok() {
            aplicado.push(format!("toque={}", cfg.toque_para_clic));
        }
        // Clickfinger en vez de zonas: en un clickpad sin botones físicos,
        // repartir la mitad inferior en zonas se come sitio útil y hace que un
        // clic normal abajo a la derecha salga como botón derecho sin querer.
        let _ = device.config_click_set_method(ClickMethod::Clickfinger);
        // El puntero se congela mientras se escribe. Sin esto, la palma roza
        // el touchpad a mitad de una frase y el cursor se va a otra línea.
        if device.config_dwt_is_available() {
            let _ = device.config_dwt_set_enabled(true);
        }
    }

    if device.config_scroll_has_natural_scroll()
        && device
            .config_scroll_set_natural_scroll_enabled(cfg.scroll_natural)
            .is_ok()
    {
        aplicado.push(format!("scroll_natural={}", cfg.scroll_natural));
    }

    if !aplicado.is_empty() {
        tracing::info!(
            dispositivo = device.name(),
            touchpad,
            ajustes = aplicado.join(" "),
            "entrada configurada"
        );
    }
}
