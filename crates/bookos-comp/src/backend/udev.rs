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
use smithay::backend::udev::{all_gpus, primary_gpu};
use smithay::desktop::utils::surface_primary_scanout_output;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::drm::control::{connector, crtc, Device as _, ModeTypeFlags};
use smithay::reexports::input::{AccelProfile, ClickMethod, Device as InputDevice, Libinput};
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::DeviceFd;

use crate::cursor::OverlayElement;
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

    let (output, surface) = crear_salida(&mut manager, &mut renderer, state)?;

    let escala = output.current_scale().fractional_scale();
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
    state.cursor_theme = Some(crate::cursor::CursorTheme::con_tamano(escala, state.cursor_nominal));
    state.cristal = crate::desenfoque::Cristal::new(&mut renderer);
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
        en_vuelo: false,
        activa: true,
    }));

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
                    if state.needs_redraw {
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
        if state.needs_redraw {
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

/// Enciende el primer conector conectado con su modo preferido.
fn crear_salida(
    manager: &mut Manager,
    renderer: &mut GlesRenderer,
    state: &mut BookosComp,
) -> anyhow::Result<(Output, Surface)> {
    let recursos = manager.device().resource_handles()?;

    let (conector, crtc, modo) = recursos
        .connectors()
        .iter()
        .filter_map(|handle| manager.device().get_connector(*handle, true).ok())
        .filter(|con| con.state() == connector::State::Connected)
        .find_map(|con| {
            // El modo preferido es el que anuncia el panel como nativo.
            let modo = con
                .modes()
                .iter()
                .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
                .or_else(|| con.modes().first())
                .copied()?;
            let crtc = primer_crtc(manager, &recursos, &con)?;
            Some((con, crtc, modo))
        })
        .ok_or_else(|| anyhow::anyhow!("no hay ninguna pantalla conectada"))?;

    let nombre = format!("{:?}-{}", conector.interface(), conector.interface_id());
    let (mm_w, mm_h) = conector.size().unwrap_or((0, 0));
    let (w, h) = modo.size();
    tracing::info!(
        pantalla = %nombre,
        modo = %format!("{}x{}@{}", w, h, modo.vrefresh()),
        "encendiendo salida"
    );

    let output = Output::new(
        nombre,
        PhysicalProperties {
            size: (mm_w as i32, mm_h as i32).into(),
            subpixel: Subpixel::Unknown,
            make: "BookOS".into(),
            model: "KMS".into(),
        },
    );
    let output_mode = OutputMode {
        size: (w as i32, h as i32).into(),
        refresh: (modo.vrefresh() * 1000) as i32,
    };
    output.create_global::<BookosComp>(&state.display_handle);

    // La escala la manda la configuración; si no dice nada, se deduce del
    // tamaño físico que anuncia el panel. Estaba cableada a 1,0, y en un
    // portátil de 242 DPI eso significaba un panel de 32 px físicos y texto
    // ilegible: el escritorio solo se podía usar anidado.
    let escala = state
        .config
        .as_ref()
        .and_then(|c| c.escala)
        .unwrap_or_else(|| super::escala_sugerida((w as i32, h as i32), (mm_w as i32, mm_h as i32)));
    tracing::info!(
        escala,
        automatica = state.config.as_ref().and_then(|c| c.escala).is_none(),
        logico = %format!("{}x{}", (w as f64 / escala) as i32, (h as f64 / escala) as i32),
        "escala de la pantalla"
    );
    output.change_current_state(
        Some(output_mode),
        Some(smithay::utils::Transform::Normal),
        Some(Scale::Fractional(escala)),
        Some((0, 0).into()),
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

    Ok((output, surface))
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
