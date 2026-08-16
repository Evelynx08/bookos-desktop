//! Shell de BookOS: los widgets del escritorio, dibujados **dentro** del
//! proceso del compositor.
//!
//! Por qué in-process, y no un `plasmashell` aparte:
//!
//! - No hay IPC ni un segundo proceso que arrancar. El panel y el dock pueden
//!   estar pintados en el mismo primer frame que el compositor, que es lo que
//!   hace que no haya pantalla de carga al iniciar sesión.
//! - No se cargan dos veces las librerías de dibujo ni se abren dos contextos.
//!
//! El precio es que un panic aquí se lleva la sesión por delante. Por eso este
//! crate **no puede hacer panic hacia fuera**: el compositor envuelve las
//! llamadas en `catch_unwind` y, si algo revienta, se queda sin shell pero con
//! las ventanas vivas.
//!
//! El rasterizado es `iced_tiny_skia`, en CPU, sobre buffers que el compositor
//! sube como texturas. Un panel repinta una franja diminuta y muy de vez en
//! cuando (el reloj, una vez por minuto), así que no compensa abrir un segundo
//! backend GPU con `wgpu` solo para eso.

use iced_core::mouse::Cursor;
use iced_core::renderer::Style;
use iced_core::{Color, Font, Pixels, Rectangle, Size, Theme};
use iced_graphics::Viewport;
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_tiny_skia::Renderer;

mod apps;
pub mod bloqueo;
pub mod osd;
mod config;
mod dock;
mod icono;
pub mod medios;
/// El logo, incrustado en el binario. Público porque el "Acerca de" del menú
/// lo pinta grande.
pub mod marca;
mod emergente;
mod state;
/// Los tokens del sistema de diseño. Público porque el compositor anima con
/// las mismas curvas y duraciones que el shell: dos tablas de movimiento en el
/// mismo escritorio se separan a la primera.
pub mod tema;
mod view;
mod widget;
mod widgets;

pub use emergente::{Ancla, Emergente};

pub use dock::{
    icon_path as icon_debug, Dock, DockItem, ICON as DOCK_ICON, MARGIN as DOCK_MARGIN,
    PAD as DOCK_PAD,
};
pub use config::{guardar_dock, Config, Entrada};
pub use state::PanelData;
pub use widget::Widget;

/// Alto del panel en píxeles lógicos.
pub const PANEL_HEIGHT: u32 = 32;

/// Lo que el shell le pide al compositor tras una interacción.
///
/// El shell no lanza el programa él mismo aunque podría: el entorno correcto
/// —`WAYLAND_DISPLAY` de esta sesión, `DISPLAY` fuera— lo sabe el compositor, y
/// duplicar eso aquí es la forma segura de que una de las dos copias se quede
/// atrás.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accion {
    Lanzar(String),
    /// Traer al frente lo que ya está abierto, en vez de abrirlo otra vez.
    ///
    /// Lleva el `exec` además del `app_id` porque el emparejamiento por `app_id`
    /// puede fallar —un cliente que no lo declara, o que usa otro distinto del
    /// que dice su `.desktop`— y en ese caso lanzar es mejor que no hacer nada:
    /// desde fuera, un icono que no responde parece el dock roto.
    Activar { app_id: String, exec: String },
    /// Abrir o cerrar el launchpad. No es un programa que lanzar: vive dentro
    /// del shell, así que el dock no puede pedirlo con un `exec`.
    Launchpad,
    /// Fijar la aplicación al dock, o soltarla si ya lo estaba. La escribe el
    /// compositor en la configuración: el shell no toca el disco.
    Anclar {
        app_id: String,
        exec: String,
        icono: String,
    },
    /// Abrir «Acerca de este PC», que es una superficie del propio shell.
    Acerca,
    /// Abrir otra tarjeta del shell por su nombre de widget. Es lo que hace el
    /// centro de control al pulsar «Wi-Fi»: la lista de redes ya existe como
    /// emergente y no tiene sentido dibujarla dos veces.
    Emergente(&'static str),
    /// Cerrar las ventanas de esa aplicación. Quien las conoce es el
    /// compositor, que tiene el `Space`.
    Cerrar { app_id: String },
}

/// Qué emergente abre cada widget del panel al pulsarlo.
///
/// `None` para los que todavía no tienen una: se pulsan y no pasa nada, que es
/// lo que hace hoy la red.
fn emergente_de(nombre: &str) -> Option<fn() -> Emergente> {
    match nombre {
        "reloj" => Some(Emergente::calendario),
        "volumen" => Some(Emergente::sonido),
        "brillo" => Some(Emergente::brillo),
        "bateria" => Some(Emergente::energia),
        "red" => Some(Emergente::red),
        "bluetooth" => Some(Emergente::bluetooth),
        "notificaciones" => Some(Emergente::notificaciones),
        "control" => Some(Emergente::centro),
        _ => None,
    }
}

/// El brillo de la pantalla ahora mismo, en tanto por ciento.
pub fn brillo_actual() -> Option<u8> {
    state::read_brightness()
}

/// El dispositivo de retroiluminación y su valor crudo máximo.
pub fn backlight() -> Option<(String, u32)> {
    let d = state::backlight_device()?;
    let max = state::backlight_max(&d)?;
    Some((d, max))
}

/// Decodifica una imagen a RGBA sin premultiplicar.
///
/// Vive aquí y no en el compositor porque `image` ya es dependencia de este
/// crate —la usa el dock para los iconos que no son SVG— y no hacía falta
/// repetirla al otro lado.
pub fn decodificar_rgba(ruta: &std::path::Path) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::ImageReader::open(ruta).ok()?.decode().ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some((rgba.into_raw(), w, h))
}

/// ¿El `app_id` de un lanzador y el de una ventana son del mismo programa?
///
/// La comparación no es exacta porque casi nunca coinciden: el lanzador dice
/// `konsole` y la ventana se presenta como `org.kde.konsole`. Se acepta el
/// sufijo tras un punto, que cubre la convención de nombre inverso de dominio
/// sin casar cosas que solo se parecen — `konsole-profile` no cuenta. Un
/// lanzador que necesite otra cosa puede decir su `app_id` en la configuración.
///
/// Es pública porque el compositor tiene que emparejar exactamente igual que el
/// dock al buscar qué ventana traer al frente: dos copias de esta regla se
/// desincronizan y el icono deja de responder solo para algunos programas.
pub fn mismo_programa(lanzador: &str, ventana: &str) -> bool {
    ventana.eq_ignore_ascii_case(lanzador)
        || ventana
            .rsplit('.')
            .next()
            .is_some_and(|ultimo| ultimo.eq_ignore_ascii_case(lanzador))
}

/// Las teclas que le importan al shell.
///
/// Es un enum propio y diminuto en vez del keysym de xkb porque el shell no
/// tiene por qué saber de xkb: el compositor traduce, y aquí solo llegan las
/// cuatro teclas que una superficie emergente necesita. `Caracter` es para el
/// campo de búsqueda del launchpad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeclaPulsada {
    Escape,
    Intro,
    Arriba,
    Abajo,
    Izquierda,
    Derecha,
    Retroceso,
    Caracter(char),
}

/// Una zona repintada, en píxeles físicos.
///
/// Es un tipo propio y no el `Rectangle` de iced a propósito: así el
/// compositor no necesita conocer iced para hablar con el shell, y cambiar de
/// toolkit no obliga a tocar el compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Una superficie del shell: su caché de layout y su geometría.
///
/// El `Renderer` **no** vive aquí: es uno solo para todo el shell. Cada
/// `Renderer` de iced arrastra su propio motor de texto (cosmic-text) con su
/// FontSystem, así que tener uno por superficie cargaría las fuentes del
/// sistema dos veces dentro del proceso del compositor.
struct Canvas {
    cache: Option<Cache>,
    size: Size<f32>,
    scale: f32,
    painted_once: bool,
}

/// Ajusta un tamaño lógico para que en píxeles físicos salga **entero**.
///
/// El compositor le dice a Smithay el tamaño lógico de la superficie y Smithay
/// lo multiplica por la escala de la salida para saber dónde pintarla. Si el
/// resultado no coincide con los píxeles que de verdad tiene el buffer, la
/// textura se reescala y el filtrado bilineal emborrona **todo** el contenido,
/// por poco que sea el desajuste.
///
/// Medido a escala 1,75: una tarjeta de 335 lógicos da un buffer de 586 px y el
/// compositor pide 586,25 —un cuarto de píxel—, y con eso el texto y los iconos
/// salen borrosos. Con 336 lógicos el físico es 588 exacto y se ve nítido. El
/// panel no lo sufría porque su ancho es el de la pantalla, que siempre encaja.
///
/// Se sube como mucho unos pocos píxeles lógicos: para una escala de la forma
/// `n/d` basta llegar al siguiente múltiplo de `d`, y las que se usan de verdad
/// —1,25, 1,5, 1,75, 2— tienen denominadores de 4 o menos.
pub fn a_pixel_entero(logico: f32, escala: f32) -> f32 {
    if escala <= 0.0 {
        return logico;
    }
    let base = logico.ceil();
    for extra in 0..16 {
        let candidato = base + extra as f32;
        let fisico = candidato * escala;
        if (fisico - fisico.round()).abs() < 0.001 {
            return candidato;
        }
    }
    base
}

impl Canvas {
    fn new(size: Size<f32>, scale: f32) -> Self {
        // El tamaño se cuadra a píxel entero aquí, que es por donde pasan
        // todas las superficies del shell: el panel, el dock, las emergentes,
        // el aviso y el bloqueo.
        let size = Size::new(
            a_pixel_entero(size.width, scale),
            a_pixel_entero(size.height, scale),
        );
        Self {
            cache: Some(Cache::default()),
            size,
            scale,
            painted_once: false,
        }
    }

    /// Tamaño del buffer en píxeles físicos.
    fn buffer_size(&self) -> (u32, u32) {
        (
            (self.size.width * self.scale).round().max(1.0) as u32,
            (self.size.height * self.scale).round().max(1.0) as u32,
        )
    }
}

pub struct Shell {
    renderer: Renderer,
    theme: Theme,

    panel: Canvas,
    widgets: widget::Panel,

    dock: Canvas,
    dock_items: Dock,

    /// La pantalla de bloqueo, cuando está echada.
    ///
    /// Va con su propio `Canvas` y no dentro de `emergente` porque no es una
    /// emergente: no cuelga de nada, ocupa la pantalla entera y no se cierra
    /// pulsando fuera. Comparte el renderer con el resto del shell, que es lo
    /// que hace que el texto se dibuje: un `Renderer` recién creado mide el
    /// texto pero no llega a rasterizarlo hasta la siguiente vuelta, y el
    /// reloj salía en blanco.
    bloqueo: Option<(bloqueo::Bloqueo, Canvas)>,

    /// El aviso de volumen, brillo y demás, mientras dura.
    osd: Option<(osd::Osd, Canvas)>,

    /// La superficie emergente abierta, si hay alguna. Solo puede haber una:
    /// abrir el calendario con el menú desplegado cierra el menú, que es lo que
    /// hace cualquier barra de menús.
    emergente: Option<(Emergente, Canvas)>,
}

impl Shell {
    pub fn new(width: u32, scale: f32) -> Self {
        Self::con_config(width, scale, Config::cargar())
    }

    /// Como [`Shell::new`] pero con una configuración dada.
    ///
    /// `new` lee `~/.config/bookos/panel.conf`, y eso ata cualquier prueba a lo
    /// que tenga el usuario en su casa: el test del panel empezó a fallar en
    /// cuanto hubo un fichero de configuración de verdad en la máquina.
    pub fn con_config(width: u32, scale: f32, config: Config) -> Self {
        // Un nombre que no existe en la configuración se ignora y se avisa: el
        // panel se queda sin ese widget, no sin panel.
        let construir = |nombre: &String| match widgets::por_nombre(nombre) {
            Some(w) => Some(w),
            None => {
                tracing::warn!(nombre, "widget desconocido en la configuración");
                None
            }
        };
        let widgets = widget::Panel::new(
            config.centro.as_ref().and_then(construir),
            config.derecha.iter().filter_map(construir).collect(),
        );
        let dock_items = Dock::from_config(&config.dock);
        let (dw, dh) = dock_items.size();
        Self {
            // Font::DEFAULT resuelve contra las fuentes del sistema vía fontdb.
            renderer: Renderer::new(Font::DEFAULT, Pixels(13.0)),
            theme: view::theme(),
            panel: Canvas::new(Size::new(width as f32, PANEL_HEIGHT as f32), scale),
            widgets,
            dock: Canvas::new(Size::new(dw, dh), scale),
            dock_items,
            emergente: None,
            bloqueo: None,
            osd: None,
        }
    }

    pub fn resize(&mut self, width: u32, scale: f32) {
        if self.panel.size.width != width as f32 || self.panel.scale != scale {
            self.panel.size.width = width as f32;
            self.panel.scale = scale;
            self.panel.painted_once = false;
            self.dock.scale = scale;
            self.dock.painted_once = false;
        }
    }

    /// Tamaño **lógico** del panel: el que ve el escritorio, independiente de
    /// la escala. No confundir con `panel_buffer_size`, que son píxeles reales.
    pub fn panel_logical_size(&self) -> (i32, i32) {
        (self.panel.size.width as i32, self.panel.size.height as i32)
    }

    /// Tamaño **lógico** del dock.
    pub fn dock_logical_size(&self) -> (i32, i32) {
        (self.dock.size.width as i32, self.dock.size.height as i32)
    }

    pub fn panel_buffer_size(&self) -> (u32, u32) {
        self.panel.buffer_size()
    }

    pub fn dock_buffer_size(&self) -> (u32, u32) {
        self.dock.buffer_size()
    }

    /// Alto del panel en píxeles **lógicos**. Es el que se usa para colocar
    /// ventanas: el `Space` de Smithay razona en lógico, no en físico.
    pub fn panel_height(&self) -> i32 {
        PANEL_HEIGHT as i32
    }

    pub fn scale(&self) -> f32 {
        self.panel.scale
    }

    pub fn dock(&self) -> &Dock {
        &self.dock_items
    }

    /// Relee los estados. Devuelve `true` si hay que repintar el panel.
    ///
    /// Esta es la puerta que mantiene el shell callado: si nada de lo que se
    /// enseña ha cambiado, nadie dibuja nada. Se llama tanto en el cambio de
    /// minuto como cuando el kernel avisa de un cambio de hardware, así que
    /// tiene que ser barata y no dar por hecho cada cuánto la llaman.
    pub fn refresh(&mut self) -> bool {
        let cambio = self.widgets.refrescar();
        // La emergente abierta también relee lo suyo: la lista de redes cambia
        // mientras la tienes delante, y sin esto se quedaría con la foto del
        // instante en que se abrió.
        if let Some((e, canvas)) = self.emergente.as_mut() {
            if e.refrescar() {
                canvas.painted_once = false;
            }
        }
        cambio || !self.panel.painted_once
    }

    /// Cuánto falta para que algún widget tenga algo nuevo que enseñar.
    ///
    /// Es el mínimo de las alarmas de los widgets vivos. Antes esto era una
    /// función suelta que daba por hecho que el único con reloj era el reloj;
    /// ahora un widget nuevo con su propia cadencia entra sin tocar nada.
    pub fn next_tick(&self) -> Option<std::time::Duration> {
        self.widgets.proxima_alarma()
    }

    /// Subsistemas de udev que hay que vigilar para este panel.
    pub fn subsistemas(&self) -> Vec<&'static str> {
        self.widgets.subsistemas()
    }


    /// ¿Hace falta pintar el dock? Su contenido solo cambia al señalar un
    /// icono, así que la primera vez, tras un cambio de tamaño y en el hover.
    pub fn dock_needs_paint(&self) -> bool {
        !self.dock.painted_once
    }

    /// Señala el icono que haya en `punto`, en coordenadas **lógicas relativas
    /// al dock**. `None` = el puntero se ha ido del dock.
    ///
    /// Devuelve `true` si hay que repintar. El que llama tiene que acotar esto
    /// al rectángulo del dock: si cada movimiento del ratón por la pantalla
    /// llegara aquí, se pagaría el recorrido por nada.
    pub fn dock_hover(&mut self, punto: Option<(f32, f32)>) -> bool {
        let item = punto.and_then(|(x, y)| self.dock_items.item_en(x, y));
        if !self.dock_items.set_hover(item) {
            return false;
        }
        self.dock.painted_once = false;
        true
    }

    // --- Superficies emergentes --------------------------------------------

    /// Abre una emergente, cerrando la que hubiera.
    pub fn abrir(&mut self, emergente: Emergente) {
        let (w, h) = emergente.size();
        let canvas = Canvas::new(Size::new(w, h), self.panel.scale);
        tracing::debug!(que = emergente.nombre(), w, h, "abriendo emergente");
        self.emergente = Some((emergente, canvas));
    }

    /// Abre la tarjeta de un widget del panel por su nombre. No hace nada si
    /// ese widget no tiene ninguna.
    pub fn abrir_de_widget(&mut self, widget: &str) {
        if let Some(constructor) = emergente_de(widget) {
            self.abrir(constructor());
        }
    }

    /// Cierra la emergente abierta. `true` si había alguna.
    pub fn cerrar_emergente(&mut self) -> bool {
        self.emergente.take().is_some()
    }

    /// El nombre de la emergente abierta, para las trazas y las pruebas.
    pub fn emergente_nombre(&self) -> Option<&'static str> {
        self.emergente.as_ref().map(|(e, _)| e.nombre())
    }

    pub fn hay_emergente(&self) -> bool {
        self.emergente.is_some()
    }

    /// Tamaño **lógico** de la emergente abierta, y dónde quiere colocarse.
    pub fn emergente_geometria(&self) -> Option<((i32, i32), Ancla)> {
        let (e, canvas) = self.emergente.as_ref()?;
        Some((
            (canvas.size.width as i32, canvas.size.height as i32),
            e.ancla(),
        ))
    }

    pub fn emergente_buffer_size(&self) -> Option<(u32, u32)> {
        self.emergente.as_ref().map(|(_, c)| c.buffer_size())
    }

    pub fn emergente_needs_paint(&self) -> bool {
        self.emergente
            .as_ref()
            // Con una animación de contenido en marcha hay que repintar en cada
            // fotograma: lo que se mueve está **dentro** del buffer, así que no
            // basta con que el compositor recoloque la superficie.
            .is_some_and(|(e, c)| !c.painted_once || e.animando())
    }

    /// ¿Está la emergente animando su contenido? Lo consulta el compositor para
    /// no dormirse a mitad de la transición.
    pub fn emergente_animando(&self) -> bool {
        self.emergente.as_ref().is_some_and(|(e, _)| e.animando())
    }

    /// Mueve el puntero sobre la emergente. `punto` es relativo a su esquina;
    /// `None` cuando está fuera. Devuelve `true` si hay que repintar.
    pub fn emergente_puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return false;
        };
        if !e.puntero(punto) {
            return false;
        }
        // Una emergente con realce propio resuelve el hover moviendo esa
        // superficie, sin tocar su buffer. Las demás son pequeñas y se repintan
        // enteras, que sale más barato que llevar la cuenta.
        if !e.usa_realce() {
            canvas.painted_once = false;
        }
        true
    }

    /// Pulsación sobre la emergente, en coordenadas relativas a su esquina.
    pub fn emergente_pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let (e, canvas) = self.emergente.as_mut()?;
        let accion = e.pulsar(x, y);
        // Hay tarjetas que crecen con lo que se pulsa: «No molestar» despliega
        // sus cuatro duraciones. Sin esto el contenido nuevo se pintaba fuera
        // del buffer y desaparecía —el canvas se hizo al abrir y nadie lo
        // volvía a mirar.
        let (w, h) = e.size();
        // Cuadrado a píxel entero antes de comparar: `Canvas::new` lo hace por
        // dentro, y sin cuadrarlo aquí el tamaño pedido nunca coincidiría con el
        // guardado y se tiraría la caché de iced en cada clic.
        let objetivo = Size::new(
            a_pixel_entero(w, canvas.scale),
            a_pixel_entero(h, canvas.scale),
        );
        if canvas.size != objetivo {
            *canvas = Canvas::new(objetivo, canvas.scale);
        }
        // Elegir algo del menú lo cierra, como cualquier menú. Pulsar en un
        // hueco no: ahí el usuario ha fallado la puntería, no ha decidido nada.
        if accion.is_some() {
            self.emergente = None;
        }
        accion
    }

    /// Desplazamiento sobre la emergente abierta. `true` si hay que repintar.
    pub fn emergente_desplazar(&mut self, dx: f32, dy: f32) -> bool {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return false;
        };
        if !e.desplazar(dx, dy) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    /// Una tecla para la emergente. Devuelve si la ha consumido y, si toca,
    /// qué hay que ejecutar.
    pub fn emergente_tecla(&mut self, tecla: TeclaPulsada) -> (bool, Option<Accion>) {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return (false, None);
        };
        match e.tecla(tecla) {
            emergente::Tecla::Cerrar => {
                self.emergente = None;
                (true, None)
            }
            emergente::Tecla::Consumida => {
                canvas.painted_once = false;
                (true, None)
            }
            // Lanzar algo cierra la emergente, igual que elegirlo con el ratón.
            emergente::Tecla::Hacer(accion) => {
                self.emergente = None;
                (true, Some(accion))
            }
            emergente::Tecla::Ignorada => (false, None),
        }
    }

    /// El velo a pantalla completa que pide la emergente abierta, en RGBA.
    pub fn emergente_velo(&self) -> Option<[f32; 4]> {
        let (e, _) = self.emergente.as_ref()?;
        let c = e.velo()?;
        Some([c.r, c.g, c.b, c.a])
    }

    /// ¿Tapa la emergente abierta la pantalla entera?
    pub fn emergente_tapa_la_pantalla(&self) -> bool {
        self.emergente
            .as_ref()
            .is_some_and(|(e, _)| e.tapa_la_pantalla())
    }

    /// El realce de lo señalado dentro de la emergente: `(x, y, ancho, alto)`
    /// lógicos relativos a su esquina, y si lleva marco de acento.
    pub fn emergente_realce(&self) -> Option<(f32, f32, f32, f32, bool)> {
        let (e, _) = self.emergente.as_ref()?;
        let (r, marco) = e.realce()?;
        Some((r.x, r.y, r.width, r.height, marco))
    }

    /// Pinta el realce en `buf`, que debe medir lo que diga
    /// [`Shell::emergente_realce`].
    ///
    /// Es una superficie propia y diminuta: repintarla cuesta lo que una celda,
    /// no lo que la rejilla entera, y mover el ratón solo cambia su posición.
    pub fn draw_realce(&mut self, buf: &mut [u8], ancho: f32, alto: f32, marco: bool) {
        use iced_core::{Border, Length};
        use iced_widget::{container, Space};

        let vista: view::PanelElement<'_> = container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(
                    if marco {
                        Color { a: 0.18, ..tema::ACENTO }
                    } else {
                        tema::HOVER
                    }
                    .into(),
                ),
                border: Border {
                    radius: tema::R_POPOVER.into(),
                    width: if marco { 2.0 } else { 0.0 },
                    color: Color { a: 0.7, ..tema::ACENTO },
                },
                ..Default::default()
            })
            .into();

        let escala = self.panel.scale;
        let mut canvas = Canvas::new(Size::new(ancho, alto), escala);
        Self::paint(&mut canvas, &mut self.renderer, &self.theme, vista, buf);
    }

    /// Pinta la emergente. `buf` debe tener el tamaño de
    /// [`Shell::emergente_buffer_size`].
    pub fn draw_emergente(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return Vec::new();
        };
        let view = e.view();
        Self::paint(canvas, &mut self.renderer, &self.theme, view, buf)
    }

    /// Qué pasa al pulsar en el panel, en coordenadas lógicas de pantalla.
    ///
    /// Las zonas son aritméticas, como en el dock: el nombre a la izquierda
    /// abre el menú y el reloj del centro abrirá el calendario. Son las mismas
    /// que dibuja [`view::panel`], y por eso los anchos viven ahí.
    pub fn panel_pulsado(&mut self, x: f32, _y: f32) -> Option<Accion> {
        // El logo no es un widget: es zona fija a la izquierda del todo.
        let que: Option<fn() -> Emergente> = if x <= view::ANCHO_LOGO {
            Some(Emergente::menu)
        } else {
            // Y el resto sale de dónde dice cada widget que se ha dibujado, en
            // vez de compararse con dos posiciones fijas: así responden todos y
            // no solo el reloj, y siguen respondiendo si cambian de orden en la
            // configuración.
            let nombre = self
                .zonas_panel()
                .into_iter()
                .find(|(_, x0, x1)| x >= *x0 && x <= *x1)
                .map(|(nombre, _, _)| nombre);
            nombre.and_then(emergente_de)
        };
        // Pulsar el mismo sitio con su emergente ya abierta la cierra, como
        // cualquier barra de menús. Pulsar *otro* sitio cambia de emergente.
        if let Some(que) = que {
            let abierta = self.emergente.as_ref().map(|(e, _)| e.nombre());
            let nueva = que();
            if abierta == Some(nueva.nombre()) {
                self.cerrar_emergente();
            } else {
                self.abrir(nueva);
            }
        }
        None
    }

    /// Dónde cae cada widget del panel, en lógicos: `(nombre, x0, x1)`.
    ///
    /// Es lo que convierte un clic en el panel en "has pulsado el volumen".
    pub fn zonas_panel(&self) -> Vec<(&'static str, f32, f32)> {
        self.widgets
            .zonas_derecha(self.panel.size.width, view::MARGEN_PANEL, view::HUECO)
    }

    /// El botón se ha soltado. `true` si hay que repintar.
    pub fn soltar(&mut self) -> bool {
        match self.emergente.as_mut() {
            Some((e, _)) => e.soltar(),
            None => false,
        }
    }

    /// ¿La emergente abierta tiene algo agarrado?
    pub fn emergente_agarrada(&self) -> bool {
        self.emergente.as_ref().is_some_and(|(e, _)| e.agarrado())
    }

    /// La hora del bloqueo: solo `H:mm`, sin fecha. En el panel la fecha ayuda
    /// —es donde se mira de reojo—, pero en una pantalla que se lee de lejos
    /// sobra todo lo que no sea la hora.
    pub fn hora_bloqueo() -> String {
        state::local_hhmm()
    }

    /// Enseña un aviso: el icono, su nivel y, si no lo tiene, un texto.
    ///
    /// Sustituye al que hubiera: subir el volumen dos veces seguidas no apila
    /// dos tarjetas, reinicia la misma.
    pub fn mostrar_osd(&mut self, icono: &str, nivel: Option<u8>, texto: Option<String>) {
        let osd = osd::Osd::new(icono, nivel, texto);
        let (w, h) = osd.size();
        let canvas = Canvas::new(Size::new(w, h), self.panel.scale);
        self.osd = Some((osd, canvas));
    }

    /// El aviso, si sigue a la vista. Se retira solo al agotarse su tiempo.
    pub fn osd_vivo(&mut self) -> bool {
        if self.osd.as_ref().is_some_and(|(o, _)| o.queda().is_none()) {
            self.osd = None;
        }
        self.osd.is_some()
    }

    /// Cuánto falta para que el aviso desaparezca, para programar el despertar.
    pub fn osd_queda(&self) -> Option<std::time::Duration> {
        self.osd.as_ref().and_then(|(o, _)| o.queda())
    }

    pub fn osd_alfa(&self) -> f32 {
        self.osd.as_ref().map(|(o, _)| o.alfa()).unwrap_or(0.0)
    }

    pub fn osd_buffer_size(&self) -> Option<(u32, u32)> {
        self.osd.as_ref().map(|(_, c)| c.buffer_size())
    }

    pub fn osd_logical_size(&self) -> Option<(f32, f32)> {
        self.osd.as_ref().map(|(o, _)| o.size())
    }

    pub fn draw_osd(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((osd, canvas)) = self.osd.as_mut() else {
            return Vec::new();
        };
        let vista = osd.view();
        Self::paint(canvas, &mut self.renderer, &self.theme, vista, buf)
    }

    /// Echa el bloqueo, con la hora ya formateada.
    ///
    /// `pantalla` es el tamaño **lógico**: el bloqueo la ocupa entera.
    pub fn bloquear(&mut self, hora: String, pantalla: (f32, f32)) {
        let canvas = Canvas::new(Size::new(pantalla.0, pantalla.1), self.panel.scale);
        self.bloqueo = Some((bloqueo::Bloqueo::new(hora), canvas));
    }

    pub fn desbloquear(&mut self) {
        self.bloqueo = None;
    }

    pub fn esta_bloqueado(&self) -> bool {
        self.bloqueo.is_some()
    }

    /// El bloqueo, para cambiarle lo que enseña.
    pub fn bloqueo_mut(&mut self) -> Option<&mut bloqueo::Bloqueo> {
        self.bloqueo.as_mut().map(|(b, _)| b)
    }

    /// Pone cuántos caracteres lleva escritos la contraseña y en qué estado
    /// está la comprobación. `true` si hay que repintar.
    ///
    /// La contraseña **no entra aquí**: el shell solo sabe cuántos puntos
    /// dibujar. Quien la guarda es el compositor, que es quien habla con el
    /// ayudante de PAM.
    pub fn bloqueo_estado(&mut self, escritos: usize, estado: bloqueo::Estado) -> bool {
        let Some((b, canvas)) = self.bloqueo.as_mut() else {
            return false;
        };
        if b.escritos == escritos && b.estado == estado {
            return false;
        }
        b.escritos = escritos;
        b.estado = estado;
        canvas.painted_once = false;
        true
    }

    pub fn bloqueo_buffer_size(&self) -> Option<(u32, u32)> {
        self.bloqueo.as_ref().map(|(_, c)| c.buffer_size())
    }

    /// Pinta el bloqueo. El buffer sale **transparente** donde no hay nada: el
    /// fondo lo pone el compositor con la imagen del escritorio.
    pub fn draw_bloqueo(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((bloqueo, canvas)) = self.bloqueo.as_mut() else {
            return Vec::new();
        };
        let vista = bloqueo.view((canvas.size.width, canvas.size.height));
        Self::paint(canvas, &mut self.renderer, &self.theme, vista, buf)
    }

    /// Abre el menú contextual del icono que haya en ese punto del dock, en
    /// coordenadas lógicas relativas al dock. `true` si se abrió.
    pub fn dock_menu(&mut self, x: f32, y: f32) -> bool {
        let Some(i) = self.dock_items.item_en(x, y) else {
            return false;
        };
        let item = &self.dock_items.items()[i];
        // El launchpad no es una aplicación: no se abre en otra ventana, ni se
        // suelta del dock, ni se cierra.
        if item.exec() == config::LAUNCHPAD {
            return false;
        }
        let objetivo = emergente::Objetivo {
            app_id: item.app_id().to_string(),
            exec: item.exec().to_string(),
            icono: item.icono_nombre().to_string(),
            etiqueta: item.label.clone(),
            anclada: item.anclada(),
            abierta: item.abierta(),
            x: self.dock_items.centro_de(i),
        };
        self.abrir(Emergente::menu_dock(objetivo));
        true
    }

    /// Ancla una aplicación al dock, o la desancla si ya estaba.
    ///
    /// Devuelve la lista de anclados que hay que guardar, para que el
    /// compositor la escriba en `panel.conf`: el shell no toca el disco.
    pub fn anclar(&mut self, app_id: &str, exec: &str, icono: &str) -> Vec<String> {
        self.dock_items.alternar_anclado(app_id, exec, icono);
        self.dock.painted_once = false;
        let (dw, dh) = self.dock_items.size();
        self.dock = Canvas::new(Size::new(dw, dh), self.dock.scale);
        self.dock_items.como_configuracion()
    }

    /// Le dice al dock si está apoyado en el borde de la pantalla, para que se
    /// dibuje como parte del marco y no como algo que flota. `true` si hay que
    /// repintar.
    pub fn dock_pegado(&mut self, pegado: bool) -> bool {
        if !self.dock_items.set_pegado(pegado) {
            return false;
        }
        self.dock.painted_once = false;
        true
    }

    /// ¿Está esta aplicación anclada al dock?
    pub fn esta_anclada(&self, app_id: &str) -> bool {
        self.dock_items.esta_anclada(app_id)
    }

    /// Marca en el dock qué aplicaciones tienen ventana, a partir de los
    /// `app_id` que hay en el escritorio. Devuelve `true` si hay que repintar.
    pub fn dock_ventanas(&mut self, app_ids: &[String]) -> bool {
        if !self.dock_items.set_abiertas(app_ids) {
            return false;
        }
        // El dock cambia de ancho al entrar o salir una aplicación abierta, así
        // que no basta con repintar: hay que rehacer el lienzo.
        let (dw, dh) = self.dock_items.size();
        self.dock = Canvas::new(Size::new(dw, dh), self.dock.scale);
        true
    }

    /// Qué hacer al pulsar en `punto`, en coordenadas **lógicas relativas al
    /// dock**.
    pub fn dock_pulsar(&self, x: f32, y: f32) -> Option<Accion> {
        let item = self.dock_items.pulsado(x, y)?;
        if item.exec() == config::LAUNCHPAD {
            return Some(Accion::Launchpad);
        }
        Some(if item.abierta() {
            Accion::Activar {
                app_id: item.app_id().to_string(),
                exec: item.exec().to_string(),
            }
        } else {
            Accion::Lanzar(item.exec().to_string())
        })
    }

    /// Pinta el panel. Ver [`Shell::draw_dock`] para el formato de `buf`.
    pub fn draw_panel(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        // Escotilla para comprobar que el aislamiento de pánicos del compositor
        // funciona de verdad. Sin esto, "el shell no puede tumbar la sesión" es
        // una afirmación sin probar.
        //
        // Con el nombre de un widget (`=reloj`) revienta solo ese widget y el
        // panel sigue: son dos redes distintas y se prueban por separado.
        if std::env::var("BOOKOS_SHELL_PANIC_TEST").is_ok_and(|v| v == "shell") {
            panic!("panic de prueba del shell");
        }
        let view = view::panel(&self.widgets);
        let Self {
            renderer,
            theme,
            panel,
            ..
        } = self;
        Self::paint(panel, renderer, theme, view, buf)
    }

    /// Pinta el dock en `buf`, que debe tener el tamaño de
    /// [`Shell::dock_buffer_size`].
    ///
    /// El formato de salida es premultiplicado y con los bytes en orden
    /// **B,G,R,A** — o sea `Fourcc::Argb8888` en little-endian. No es una
    /// elección nuestra sino lo que deja iced, y conviene no fiarse de la
    /// intuición aquí: el compositor tuvo puesto Abgr8888 por lo que parecía
    /// evidente y lo que se veía era el azul del acento en naranja.
    ///
    /// Devuelve las zonas dañadas en píxeles físicos. De momento se repinta
    /// entero: son pocos KB y ocurre una vez, no compensa afinar por widget
    /// hasta que el dock tenga contenido que cambie.
    pub fn draw_dock(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let view = self.dock_items.view();
        let Self {
            renderer,
            theme,
            dock,
            ..
        } = self;
        Self::paint(dock, renderer, theme, view, buf)
    }

    fn paint(
        canvas: &mut Canvas,
        renderer: &mut Renderer,
        theme: &Theme,
        view: view::PanelElement<'_>,
        buf: &mut [u8],
    ) -> Vec<Damage> {
        let (w, h) = canvas.buffer_size();
        let Some(mut pixmap) = tiny_skia::PixmapMut::from_bytes(buf, w, h) else {
            tracing::error!(w, h, len = buf.len(), "buffer del shell con tamaño incoherente");
            return Vec::new();
        };

        let viewport = Viewport::with_physical_size(Size::new(w, h), canvas.scale);
        let logical = viewport.logical_size();

        let mut ui =
            UserInterface::build(view, logical, canvas.cache.take().unwrap_or_default(), renderer);
        ui.draw(
            renderer,
            theme,
            &Style {
                text_color: view::TEXT,
            },
            Cursor::Unavailable,
        );
        canvas.cache = Some(ui.into_cache());

        // Se repinta entero, y **el damage parcial no ayuda**: se probó pasando
        // aquí solo los rectángulos de las celdas que cambiaban, y señalar una
        // celda del launchpad seguía costando 23 ms de los 52 del repintado
        // completo. La razón está en `iced_tiny_skia::engine::adjust_clip_mask`,
        // que hace `clip_mask.clear()` —o sea, borra la máscara del tamaño del
        // buffer entero— **por cada capa y por cada zona dañada**. El coste va
        // con el tamaño del buffer, no con lo que cambia. Medido, con el mismo
        // contenido: buffer de 2881x1801, 23 ms; de 1440x900, 10,8 ms; de
        // 720x450, 3,6 ms.
        //
        // Por eso la palanca es el **tamaño**: el velo del launchpad lo pinta
        // el compositor y su realce va en una superficie de una celda.
        let zonas = [Rectangle {
            x: 0.0,
            y: 0.0,
            width: logical.width,
            height: logical.height,
        }];

        let mut mask = tiny_skia::Mask::new(w, h).expect("máscara del tamaño del pixmap");
        renderer.draw(&mut pixmap, &mut mask, &viewport, &zonas, Color::TRANSPARENT);

        canvas.painted_once = true;
        vec![Damage {
            x: 0,
            y: 0,
            width: w as i32,
            height: h as i32,
        }]
    }
}

/// Cabe cualquier `Length` en el panel; se reexporta para la vista.
pub(crate) const FILL: iced_core::Length = iced_core::Length::Fill;
