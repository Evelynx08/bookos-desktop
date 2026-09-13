//! `zwlr_screencopy_v1`: copiar una salida a un buffer del cliente.
//!
//! Es lo que hace posible **hacer una captura de pantalla y compartir
//! pantalla**, que hasta ahora no se podía ninguna de las dos cosas. No hace
//! falta escribir un backend de portal: `xdg-desktop-portal-wlr` habla este
//! mismo protocolo, así que con esto puesto funcionan tanto `grim` como el
//! selector de «compartir pantalla» de un navegador.
//!
//! ## Por qué a mano y no con Smithay
//!
//! Smithay 0.7 no trae este protocolo. Lo que sí trae es el XML generado
//! —`wayland_protocols_wlr::screencopy`, que ya entra en el árbol con
//! `data_control`— y `RenderFrameResult::blit_frame_result`, que es justo el
//! camino pensado para esto. O sea que no hay dependencia nueva: lo que falta
//! son los `Dispatch`, que es este fichero.
//!
//! ## Cuándo se copia
//!
//! No al recibir `copy`, sino **después del siguiente fotograma**. La copia
//! sale del resultado de dibujar la salida, que es lo que ya está en la GPU;
//! atenderla en el momento obligaría a componer la escena otra vez solo para
//! esto. El cliente pide, se apunta en `pendientes`, y el backend la sirve al
//! terminar de dibujar esa salida.
//!
//! ## Qué no hace todavía
//!
//! - **Solo `wl_shm`.** El evento `linux_dmabuf` no se anuncia, así que el
//!   cliente pide memoria compartida. Para una captura da igual; para compartir
//!   pantalla a 60 fps sería una copia GPU→CPU→GPU que se puede quitar después.
//! - **`copy_with_damage` se sirve como `copy`**: se contesta con el daño de la
//!   salida entera. Es correcto —el daño anunciado puede ser de más, nunca de
//!   menos— y evita llevar la cuenta por cliente.

use std::sync::Mutex;

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Buffer as BufferCoord, Rectangle, Size};

use crate::state::BookosComp;

/// La versión que se anuncia. La 3 es la que pide `xdg-desktop-portal-wlr`.
const VERSION: u32 = 3;

/// El formato en el que se entrega la copia.
///
/// `Xrgb8888` y no `Argb8888`: lo que sale de componer una salida es opaco —el
/// fondo tapa siempre— y anunciar alfa haría que el visor de la captura
/// multiplicara por un canal que no significa nada. Es también lo que esperan
/// `grim` y el portal.
const FORMATO: wl_shm::Format = wl_shm::Format::Xrgb8888;

/// El global vivo. Soltarlo lo retira y los clientes dejarían de ver el
/// protocolo, así que hay que guardarlo aunque nadie lo lea.
#[derive(Debug)]
pub struct CapturaState {
    #[allow(dead_code)]
    global: smithay::reexports::wayland_server::backend::GlobalId,
}

impl CapturaState {
    pub fn new(dh: &DisplayHandle) -> Self {
        let global = dh.create_global::<BookosComp, ZwlrScreencopyManagerV1, _>(VERSION, ());
        Self { global }
    }
}

/// Lo que el compositor guarda de cada `zwlr_screencopy_frame_v1`.
#[derive(Debug)]
pub struct DatosFrame {
    output: Output,
    /// La región pedida, en píxeles **del buffer** de la salida. La captura
    /// entera es la salida completa.
    region: Rectangle<i32, BufferCoord>,
    /// Si hay que dibujar el cursor encima.
    con_cursor: bool,
    /// Se sirve como mucho una vez: el segundo `copy` es un error de protocolo.
    servido: Mutex<bool>,
}

/// Una copia pedida y todavía sin servir.
#[derive(Debug)]
pub struct Pendiente {
    pub frame: ZwlrScreencopyFrameV1,
    pub output: Output,
    pub region: Rectangle<i32, BufferCoord>,
    /// Si el cliente pidió el cursor encima.
    ///
    /// Hoy da igual: el cursor se compone dentro de la escena como un elemento
    /// más y separarlo pediría componerla dos veces, así que sale siempre. Se
    /// guarda porque es lo que pidió el cliente, y quitar el campo escondería
    /// que no se le está haciendo caso.
    #[allow(dead_code)]
    pub con_cursor: bool,
    pub buffer: smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for BookosComp {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        recurso: New<ZwlrScreencopyManagerV1>,
        _datos: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(recurso, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for BookosComp {
    fn request(
        state: &mut Self,
        _client: &Client,
        _recurso: &ZwlrScreencopyManagerV1,
        peticion: zwlr_screencopy_manager_v1::Request,
        _datos: &(),
        _dh: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_screencopy_manager_v1::Request;
        let (frame, overlay_cursor, output, region) = match peticion {
            Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => (frame, overlay_cursor, output, None),
            Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => (
                frame,
                overlay_cursor,
                output,
                Some(Rectangle::new((x, y).into(), (width, height).into())),
            ),
            Request::Destroy => return,
            _ => return,
        };

        let Some(output) = Output::from_resource(&output) else {
            // La salida se desenchufó entre que el cliente la pidió y esto:
            // se contesta `failed` en vez de dejarlo esperando para siempre.
            let frame = init.init(frame, DatosFrame::muerto());
            frame.failed();
            return;
        };
        let tamano = tamano_de(&output);
        let completa = Rectangle::from_size(tamano);
        // Recortada contra la salida: una región que se sale devolvería filas
        // de fuera del framebuffer, y eso es leer memoria que no es nuestra.
        let region = region
            .map(|r| Rectangle::new(r.loc, r.size).intersection(completa))
            .unwrap_or(Some(completa));
        let Some(region) = region.filter(|r| r.size.w > 0 && r.size.h > 0) else {
            let frame = init.init(frame, DatosFrame::muerto());
            frame.failed();
            return;
        };

        let frame = init.init(
            frame,
            DatosFrame {
                output,
                region,
                con_cursor: overlay_cursor != 0,
                servido: Mutex::new(false),
            },
        );
        // Cuatro bytes por píxel: es lo que mide `Xrgb8888`.
        let stride = region.size.w as u32 * 4;
        frame.buffer(FORMATO, region.size.w as u32, region.size.h as u32, stride);
        if frame.version() >= 3 {
            frame.buffer_done();
        }
        let _ = state;
    }
}

impl DatosFrame {
    /// Un frame que ya nace fallado: hace falta porque el protocolo obliga a
    /// crear el objeto antes de poder decirle que no.
    fn muerto() -> Self {
        Self {
            output: Output::new(
                "muerta".into(),
                smithay::output::PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: smithay::output::Subpixel::Unknown,
                    make: "BookOS".into(),
                    model: "ninguna".into(),
                },
            ),
            region: Rectangle::default(),
            con_cursor: false,
            servido: Mutex::new(true),
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, DatosFrame> for BookosComp {
    fn request(
        state: &mut Self,
        _client: &Client,
        recurso: &ZwlrScreencopyFrameV1,
        peticion: zwlr_screencopy_frame_v1::Request,
        datos: &DatosFrame,
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_screencopy_frame_v1::Request;
        let buffer = match peticion {
            Request::Copy { buffer } | Request::CopyWithDamage { buffer } => buffer,
            Request::Destroy => {
                // Se olvida la petición pendiente: el cliente se ha ido y su
                // buffer ya no es válido.
                state.capturas_pantalla.retain(|p| p.frame != *recurso);
                return;
            }
            _ => return,
        };

        {
            let mut servido = datos.servido.lock().expect("cerrojo del frame de captura");
            if *servido {
                recurso.post_error(
                    zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                    "este frame ya se había copiado",
                );
                return;
            }
            *servido = true;
        }

        // El buffer tiene que ser de memoria compartida y del tamaño y formato
        // que se anunciaron: es lo único que se sabe rellenar.
        let esperado = (datos.region.size.w, datos.region.size.h);
        match smithay::wayland::shm::with_buffer_contents(&buffer, |_, _, datos_shm| {
            (datos_shm.width, datos_shm.height, datos_shm.format)
        }) {
            Ok((w, h, formato)) if (w, h) == esperado && formato == FORMATO => {}
            Ok(_) => {
                recurso.post_error(
                    zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                    "el buffer no tiene el tamaño o el formato que se anunciaron",
                );
                return;
            }
            Err(_) => {
                recurso.post_error(
                    zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                    "se esperaba un buffer de wl_shm",
                );
                return;
            }
        }

        // Se apunta y se sirve tras el próximo fotograma de esa salida. Y se
        // pide dibujar: si el escritorio está quieto no habría fotograma que
        // copiar y la captura se quedaría esperando.
        state.capturas_pantalla.push(Pendiente {
            frame: recurso.clone(),
            output: datos.output.clone(),
            region: datos.region,
            con_cursor: datos.con_cursor,
            buffer,
        });
        state.needs_redraw = true;
    }

    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        recurso: &ZwlrScreencopyFrameV1,
        _datos: &DatosFrame,
    ) {
        state.capturas_pantalla.retain(|p| p.frame != *recurso);
    }
}

/// ¿Es el compositor el dueño del portapapeles y ofrece una imagen?
///
/// Para el autotest. **No pega**: Smithay no deja leer la propia selección del
/// servidor —`request_data_device_client_selection` devuelve
/// `ServerSideSelection` a propósito, porque esa función existe para que el
/// compositor lea la selección de un *cliente*—. Y ese error es justo la
/// respuesta que se busca: dice que el dueño somos nosotros y que el tipo que
/// se pregunta está entre los ofrecidos. Sin selección puesta contestaría
/// `NoSelection`, y con otra lista de tipos, `InvalidMimetype`.
///
/// Que los bytes lleguen de verdad se comprueba con un cliente que pegue; eso
/// no cabe aquí dentro.
pub fn el_portapapeles_tiene_imagen(state: &crate::state::BookosComp) -> bool {
    use smithay::reexports::rustix;
    use smithay::wayland::selection::data_device::{
        SelectionRequestError, request_data_device_client_selection,
    };
    let Ok((_lectura, escritura)) = rustix::pipe::pipe() else {
        return false;
    };
    match request_data_device_client_selection(&state.seat, "image/png".into(), escritura) {
        Err(SelectionRequestError::ServerSideSelection) => true,
        otro => {
            tracing::warn!(?otro, "el portapapeles no ofrece image/png del compositor");
            false
        }
    }
}

/// Pasa un rectángulo con el origen **arriba** a la izquierda al que espera
/// `copy_framebuffer`, que lo tiene **abajo**.
///
/// `GlesRenderer::copy_framebuffer` le da la `y` de la región tal cual a
/// `glReadPixels`, y el origen de OpenGL es la esquina inferior izquierda. Sin
/// esta vuelta, pedir un recorte devuelve la franja simétrica de la pantalla:
/// medido, un recuadro pedido en y=262 salía con el contenido de y=1014, que es
/// `1801 − 262 − 525`. La captura de pantalla **entera** no lo notaba porque
/// con `y = 0` y el alto completo el rectángulo es el mismo del derecho y del
/// revés, y por eso el fallo solo aparecía al recortar.
pub fn region_gl(
    region: Rectangle<i32, BufferCoord>,
    alto_total: i32,
) -> Rectangle<i32, BufferCoord> {
    Rectangle::new(
        (region.loc.x, alto_total - region.loc.y - region.size.h).into(),
        region.size,
    )
}

/// El tamaño del buffer de una salida, que es lo que se copia.
pub fn tamano_de(output: &Output) -> Size<i32, BufferCoord> {
    let modo = output.current_mode().map(|m| m.size).unwrap_or_default();
    let transformado = output.current_transform().transform_size(modo);
    (transformado.w, transformado.h).into()
}

/// Copia los píxeles al buffer del cliente y le dice que ya está.
///
/// `pixeles` viene tal cual lo devuelve `map_texture`: empaquetado, cuatro
/// bytes por píxel y tantas filas como alto tiene la región. El buffer del
/// cliente puede tener un `stride` mayor que su ancho —lo elige él—, así que se
/// copia **fila a fila** y no de un tirón.
///
/// Con `invertida`, las filas de `pixeles` vienen de abajo arriba y se le dan
/// la vuelta al copiarlas. Es lo que devuelve OpenGL, cuyo origen está en la
/// esquina **inferior** izquierda: sin esto la captura sale del revés, con el
/// panel abajo y el dock arriba —comprobado mirando el PNG—. Se invierte aquí y
/// no con la bandera `YInvert` del protocolo porque la copia ya va fila a fila:
/// darles la vuelta no cuesta nada y así el cliente recibe siempre lo mismo, sin
/// depender de que sepa interpretar la bandera.
///
/// Devuelve `false` si el buffer se murió entre medias, que pasa cuando el
/// cliente se va justo después de pedir la copia.
pub fn volcar(pendiente: &Pendiente, pixeles: &[u8], invertida: bool) -> bool {
    let (ancho, alto) = (
        pendiente.region.size.w as usize,
        pendiente.region.size.h as usize,
    );
    let fila = ancho * 4;
    if pixeles.len() < fila * alto {
        tracing::error!(
            tiene = pixeles.len(),
            hacen_falta = fila * alto,
            "la copia de la pantalla salió más corta de lo que mide la región"
        );
        pendiente.frame.failed();
        return false;
    }
    let escrito = smithay::wayland::shm::with_buffer_contents_mut(
        &pendiente.buffer,
        |destino, largo, datos| {
            let stride = datos.stride as usize;
            // SAFETY: `with_buffer_contents_mut` da un puntero al mapeo del
            // cliente y su largo; se escribe solo dentro de esos límites, que
            // es lo que comprueban el `min` y el `largo` de abajo.
            let destino = unsafe { std::slice::from_raw_parts_mut(destino, largo) };
            for y in 0..alto {
                let origen = if invertida { alto - 1 - y } else { y };
                let (o, d) = (origen * fila, y * stride);
                if d + fila > destino.len() {
                    break;
                }
                destino[d..d + fila].copy_from_slice(&pixeles[o..o + fila]);
            }
        },
    );
    if escrito.is_err() {
        // El buffer se fue con su cliente: no hay a quién contestarle.
        return false;
    }
    true
}

/// Le dice al cliente que su copia está lista.
///
/// El instante va en el reloj monotónico, que es el mismo con el que se anuncia
/// `wp_presentation`: para un cliente que graba, las dos marcas tienen que
/// poder compararse.
pub fn contestar(pendiente: &Pendiente, tamano: Size<i32, BufferCoord>) {
    use zwlr_screencopy_frame_v1::Flags;
    // Sin `YInvert`: la vuelta se la damos nosotros en `volcar`, así que lo que
    // el cliente tiene en el buffer ya está de arriba abajo.
    pendiente.frame.flags(Flags::empty());
    if pendiente.frame.version() >= 2 {
        pendiente
            .frame
            .damage(0, 0, tamano.w.max(0) as u32, tamano.h.max(0) as u32);
    }
    let ahora = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = ahora.as_secs();
    pendiente.frame.ready(
        (secs >> 32) as u32,
        (secs & 0xFFFF_FFFF) as u32,
        ahora.subsec_nanos(),
    );
}
