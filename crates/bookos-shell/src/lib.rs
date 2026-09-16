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

pub mod actividad;
mod apps;
pub mod bloqueo;
pub mod confirmacion;
pub mod captura;
mod config;
/// El conmutador de Alt+Tab. Público porque el compositor le da la lista de
/// aplicaciones por orden de uso: el shell no ve el foco.
pub mod conmutador;
/// La barra de título que el escritorio dibuja por las ventanas que la aceptan.
/// Pública porque el compositor decide quién la lleva y qué hace cada botón.
pub mod decoracion;
pub mod diagnostico;
mod dock;
mod emergente;
/// Los iconos del escritorio. Público porque el compositor los coloca sobre el
/// fondo, por debajo de las ventanas, y le da los clics que no son de nadie.
pub mod escritorio;
pub mod fondos;
mod icono;
/// El logo, incrustado en el binario. Público porque el "Acerca de" del menú
/// lo pinta grande.
pub mod marca;
pub mod medios;
/// La cola de notificaciones. Pública porque quien las recibe es el compositor:
/// el shell no habla D-Bus.
pub mod notificaciones;
pub mod osd;
mod state;
/// Los tokens del sistema de diseño. Público porque el compositor anima con
/// las mismas curvas y duraciones que el shell: dos tablas de movimiento en el
/// mismo escritorio se separan a la primera.
pub mod tema;
/// El aviso de una notificación recién llegada.
pub mod toast;
mod view;
mod widget;
mod widgets;

/// Qué hacer con una tecla que llega a una capa del shell. Lo necesita el
/// compositor para la capa de captura, que gobierna él.
pub use emergente::Tecla;
/// Un cambio de vista del launchpad que anima el compositor. Público porque es
/// él quien tiene las dos superficies que hay que cruzar.
pub use emergente::launchpad::Transicion as TransicionLaunchpad;
/// Cuánto hay que mover el ratón para que un clic pase a ser un arrastre. Lo
/// comparten el launchpad y el dock: el gesto es el mismo y con dos umbrales
/// distintos sacar un icono del dock respondería antes o después que moverlo
/// por la rejilla.
pub use emergente::launchpad::UMBRAL_ARRASTRE;
pub use emergente::{Ancla, Emergente, ModoProyeccion, PantallaCompartible};
/// El icono ya cargado de una aplicación. Sale al exterior porque el conmutador
/// lo arma el compositor, que conoce las ventanas pero no el tema de iconos.
pub use icono::Icono;

/// Busca el icono de una aplicación por su nombre, en el tema del sistema.
pub fn icono_de_app(nombre: &str) -> Option<Icono> {
    icono::cargar(nombre)
}

/// Una celda del conmutador, a partir de la ventana que representa.
///
/// El `app_id` casi nunca sirve como nombre ni como icono —`org.kde.konsole` no
/// es «Konsole» ni `utilities-terminal`—, así que se resuelve su `.desktop`,
/// que es lo mismo que hace el dock.
///
/// El rótulo es el **título** de la ventana, que es lo único que distingue dos
/// ventanas del mismo programa. Cuando viene vacío —hay clientes que tardan en
/// ponerlo— se cae al nombre de la aplicación: una celda sin renglón descoloca
/// la tarjeta entera.
pub fn entrada_de_ventana(app_id: &str, titulo: &str) -> conmutador::Entrada {
    let app = apps::por_app_id(app_id);
    let nombre = match (titulo.trim(), &app) {
        ("", Some(app)) => app.nombre.clone(),
        ("", None) => app_id.to_string(),
        (titulo, _) => titulo.to_string(),
    };
    conmutador::Entrada {
        nombre,
        // Sin `.desktop` se prueba con el propio `app_id`: hay programas cuyo
        // icono se llama igual que ellos, y un hueco es peor que intentarlo.
        icono: icono::cargar(app.as_ref().map_or(app_id, |a| a.icono.as_str())),
    }
}

pub use config::{
    Actividades as ConfigActividades, Bloqueo as ConfigBloqueo, Config, Efectos, Entrada,
    MAXIMO_ESCRITORIOS, guardar_apariencia, guardar_configuracion, guardar_dock, guardar_efectos,
    guardar_escritorios, guardar_fondo, olvidar_fondo,
};
pub use dock::{
    Dock, DockItem, ICON as DOCK_ICON, MARGIN as DOCK_MARGIN, PAD as DOCK_PAD,
    icon_path as icon_debug,
};
pub use state::PanelData;
/// La hora local de ahora, en `(hora, minuto)`. La necesita el compositor para
/// el temporizador del tema automático.
pub use state::hora_local_ahora;
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
    EnergiaConfirmada(confirmacion::Energia),
    CerrarEmergente,
    VentanaEncima,
    VentanaEscritorio(usize),
    VentanaMonitor(String),
    Lanzar(String),
    /// Traer al frente lo que ya está abierto, en vez de abrirlo otra vez.
    ///
    /// Lleva el `exec` además del `app_id` porque el emparejamiento por `app_id`
    /// puede fallar —un cliente que no lo declara, o que usa otro distinto del
    /// que dice su `.desktop`— y en ese caso lanzar es mejor que no hacer nada:
    /// desde fuera, un icono que no responde parece el dock roto.
    Activar {
        app_id: String,
        exec: String,
    },
    /// Abrir o cerrar el launchpad. No es un programa que lanzar: vive dentro
    /// del shell, así que el dock no puede pedirlo con un `exec`.
    Launchpad,
    /// Fijar la aplicación al dock o soltarla. La escribe el compositor en la
    /// configuración: el shell no toca el disco.
    Anclar {
        app_id: String,
        exec: String,
        icono: String,
        /// `None` alterna, que es lo que quiere el menú del clic derecho: una
        /// sola entrada que dice «Fijar» o «Soltar» según esté.
        ///
        /// Arrastrando **no** vale alternar: soltar sobre el dock algo que ya
        /// estaba anclado lo desanclaría, que es lo contrario de lo que acaba
        /// de hacer la mano. Por eso el arrastre manda `Some(true)` o
        /// `Some(false)` y dice lo que quiere.
        fijar: Option<bool>,
    },
    /// Abrir «Acerca de este PC», que es una superficie del propio shell.
    Acerca,
    /// Cambiar el tema y el acento. Los dos juntos y no una acción por cada uno
    /// porque la tarjeta de Apariencia manda **su estado entero**: así el
    /// compositor escribe las dos claves de una vez y no puede quedarse con un
    /// fichero a medio cambiar si algo falla entre medias.
    Apariencia {
        /// Lo **elegido**, que puede ser «automático». El tema concreto que
        /// toca lo resuelve el compositor con el reloj: la tarjeta no tiene por
        /// qué saber la hora.
        modo: tema::ModoTema,
        acento: tema::Acento,
    },
    /// Abrir otra tarjeta del shell por su nombre de widget. Es lo que hace el
    /// centro de control al pulsar «Wi-Fi»: la lista de redes ya existe como
    /// emergente y no tiene sentido dibujarla dos veces.
    Emergente(&'static str),
    /// Capturar ese rectángulo de la pantalla, en lógicos. Lo pide la capa de
    /// captura al soltar el arrastre o al pulsar «Pantalla».
    Capturar {
        x: i32,
        y: i32,
        ancho: i32,
        alto: i32,
        /// A un fichero. Con `false` va al portapapeles, que es lo normal: la
        /// mayoría de las capturas se pegan en un chat y no se vuelven a mirar.
        guardar: bool,
    },
    /// Poner una familia de fondos: la imagen clara y la oscura de golpe.
    ///
    /// Las **dos** y no la que toca ahora, que es lo que permite que el cambio
    /// de tema se lleve el fondo con él. Es lo mismo que hace Apariencia con el
    /// tema y el acento: la tarjeta manda su estado entero.
    Fondo {
        claro: std::path::PathBuf,
        oscuro: std::path::PathBuf,
    },
    /// Elegir cómo se reparten dos pantallas desde el selector de Fn+F4.
    Proyeccion(ModoProyeccion),
    /// Contestar al permiso de compartir pantalla del portal de escritorio.
    ///
    /// `pantalla` es el índice dentro de la lista que se le pasó a la tarjeta,
    /// y `None` es la negativa. Lleva la sesión porque puede haber más de una
    /// petición en vuelo y la respuesta tiene que llegar a la suya.
    Compartir {
        sesion: u32,
        pantalla: Option<usize>,
    },
    /// Retirar esa notificación. La lista vive en el shell, pero avisar a la
    /// aplicación de que se cerró es D-Bus, y eso lo hace el compositor.
    CerrarNotificacion(u32),
    /// Activar un botón ofrecido por una notificación.
    NotificacionAccion {
        id: u32,
        clave: String,
    },
    /// Vaciar la cola entera.
    BorrarNotificaciones,
    /// Echar la pantalla de bloqueo. La del propio compositor, no la de KDE.
    Bloquear,
    /// Terminar la sesión, o sea el compositor. Lo hace él: es su bucle.
    CerrarSesion,
    /// Cerrar las ventanas de esa aplicación. Quien las conoce es el
    /// compositor, que tiene el `Space`.
    Cerrar {
        app_id: String,
    },
    /// Ir a ese escritorio. La pide el indicador del panel al pulsar un punto;
    /// quien los gobierna es el compositor.
    Escritorio(usize),
    /// Abrir la vista general de escritorios desde el indicador del panel.
    VistaEscritorios,
    CrearEscritorio,
    EliminarEscritorio(usize),
    RenombrarEscritorio {
        indice: usize,
        nombre: String,
    },
    /// Acción de una actividad viva; el compositor la devuelve a la aplicación
    /// propietaria por la señal privada de BookOS.
    Actividad(actividad::Accion),
    /// Poner o quitar los efectos reducidos. El compositor la escribe en la
    /// configuración, como el resto de lo que se elige desde el shell.
    AlternarEfectos,
}

impl Accion {
    /// Las operaciones de administración actualizan la vista en el sitio; elegir
    /// un escritorio o lanzar algo sí completa la interacción y la cierra.
    fn conserva_emergente(&self) -> bool {
        matches!(
            self,
            Self::CrearEscritorio
                | Self::EliminarEscritorio(_)
                | Self::RenombrarEscritorio { .. }
                // Cerrar una notificación no cierra la tarjeta: se leen varias
                // seguidas, y que desapareciera al despachar la primera
                // obligaría a volver a abrirla cada vez.
                | Self::CerrarNotificacion(_)
                | Self::NotificacionAccion { .. }
                | Self::BorrarNotificaciones
                // Elegir un color es probar: la tarjeta se queda abierta para
                // ver el resultado y poder cambiar de idea.
                | Self::Apariencia { .. }
                | Self::Actividad(_)
                // Anclar arrastrando **no** cierra el launchpad: la mano está
                // ahí para poner varias, y cerrarse tras la primera obligaría a
                // reabrirlo por cada icono.
                | Self::Anclar { .. }
        )
    }
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
        // No es un widget del panel: es la tarjeta que abre el menú de BookOS.
        // Entra por aquí porque `Accion::Emergente` ya es el camino de «abre
        // esa otra tarjeta del shell» y no hacía falta un segundo.
        "apariencia" => Some(Emergente::apariencia),
        "apagar" => Some(Emergente::apagar),
        _ => None,
    }
}

/// Nombre interno de la tarjeta que corresponde a una zona del panel.
///
/// El widget y su emergente no siempre se llaman igual (`bateria`/`energia`,
/// `reloj`/`calendario`). El compositor necesita ambos nombres: el primero para
/// construirla después de una transición y el segundo para saber si el segundo
/// clic debe cerrarla.
fn nombre_emergente_de(nombre: &str) -> Option<&'static str> {
    match nombre {
        "reloj" => Some("calendario"),
        "volumen" => Some("sonido"),
        "brillo" => Some("brillo"),
        "bateria" => Some("energia"),
        "red" => Some("red"),
        "bluetooth" => Some("bluetooth"),
        "notificaciones" => Some("notificaciones"),
        "control" => Some("centro"),
        "apariencia" => Some("apariencia"),
        "apagar" => Some("apagar"),
        _ => None,
    }
}

/// El brillo de la pantalla ahora mismo, en tanto por ciento.
pub fn brillo_actual() -> Option<u8> {
    state::read_brightness()
}

pub mod retroiluminacion;

/// Nombre, nivel actual y máximo de la luz de teclado detectada.
pub fn brillo_teclado_actual() -> Option<(String, u32, u32)> {
    state::teclado_actual()
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

/// Variante para portadas que llegan por IPC como bytes PNG/JPEG.
pub fn decodificar_imagen(datos: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(datos).ok()?;
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
    /// Inicio y Fin: al principio y al final de lo que se esté viendo.
    Inicio,
    Fin,
    /// RePág y AvPág: una página entera, que en el launchpad es la unidad de
    /// verdad —las flechas se mueven de icono en icono—.
    PaginaArriba,
    PaginaAbajo,
    /// El tabulador. Lo usa el launchpad para el selector de color de una
    /// carpeta: es la única tecla libre ahí dentro, porque las letras buscan y
    /// las flechas recorren la rejilla.
    Tabulador,
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
/// Una barra de título con su buffer y lo último que se pintó en él.
struct Barra {
    estado: decoracion::Estado,
    canvas: Canvas,
}

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
    theme: Theme,

    /// **Tres renderizadores, no uno**, porque dentro de cada uno vive la caché
    /// de rasterizado de iced —los SVG ya parseados y los pixmaps ya pintados—
    /// y esa caché **se purga al final de cada `draw`**: `iced_tiny_skia` tira
    /// todo lo que no haya salido en el último dibujo (`Engine::trim`).
    ///
    /// Con uno solo, el panel y el dock se pisaban mutuamente: dibujar el panel
    /// tiraba los iconos del dock y viceversa, así que en cada fotograma se
    /// reparseaban y re-rasterizaban los SVG de los dos. Medido en release a
    /// 2881×1801 y escala 1,75: el panel solo cuesta 1,19 ms y el dock solo
    /// 1,39 ms, pero **alternándolos —que es lo que hace el compositor de
    /// verdad— costaban 7,61 ms**, casi tres veces la suma. Separándolos, 2,84.
    ///
    /// El tercero lo comparte todo lo demás —emergentes, conmutador, bloqueo,
    /// barras de título— y no pasa nada: de esas se dibuja **una por
    /// fotograma**, así que no hay a quién pisar. Van juntas y no cada una con
    /// el suyo porque nacen y mueren, y un renderizador por superficie efímera
    /// se llevaría la caché con él: el conmutador recrea su lienzo en cada
    /// Alt+Tab, y con caché fría cada apertura costaba 64 ms en vez de 15.
    ///
    /// Crear uno es barato —tres mapas vacíos— y el sistema de fuentes es un
    /// estático compartido que no se duplica.
    renderer_panel: Renderer,
    renderer_dock: Renderer,
    renderer: Renderer,

    panel: Canvas,
    widgets: widget::Panel,

    dock: Canvas,
    dock_items: Dock,
    /// La foto de perfil que dice la configuración, si la dice. El bloqueo la
    /// prefiere a las de siempre (`~/.face`, AccountsService).
    avatar: Option<String>,
    /// Disposición y elementos visibles del bloqueo. Se guarda aparte del
    /// `Config` porque el resto se consume al construir panel y dock.
    /// Lo que el usuario ha elegido. El valor **efectivo** puede ser otro: la
    /// batería baja fuerza los efectos reducidos sin tocar esta preferencia.
    efectos_config: config::Efectos,
    /// ¿El dock se queda a la vista con el launchpad abierto?
    launchpad_dock: bool,
    bloqueo_config: config::Bloqueo,
    actividades_config: config::Actividades,

    /// La pantalla de bloqueo, cuando está echada.
    ///
    /// Va con su propio `Canvas` y no dentro de `emergente` porque no es una
    /// emergente: no cuelga de nada, ocupa la pantalla entera y no se cierra
    /// pulsando fuera. Comparte el renderer con el resto del shell, que es lo
    /// que hace que el texto se dibuje: un `Renderer` recién creado mide el
    /// texto pero no llega a rasterizarlo hasta la siguiente vuelta, y el
    /// reloj salía en blanco.
    bloqueo: Option<(bloqueo::Bloqueo, Canvas)>,
    /// La capa de captura de pantalla, mientras está abierta. Ocupa la pantalla
    /// entera como el bloqueo, y por lo mismo: se queda con el ratón y el
    /// teclado mientras se elige qué capturar.
    captura: Option<(captura::Captura, Canvas)>,

    /// El aviso de volumen, brillo y demás, mientras dura.
    osd: Option<(osd::Osd, Canvas)>,
    /// El panel de diagnóstico, mientras esté puesto. Va con su propio lienzo
    /// por lo mismo que el aviso: es una superficie aparte, con su tamaño, que
    /// aparece y desaparece sin tocar al panel.
    diagnostico: Option<(diagnostico::Hud, Canvas)>,
    /// El conmutador de Alt+Tab, mientras el modificador siga pulsado.
    conmutador: Option<(conmutador::Conmutador, Canvas)>,

    /// «No molestar»: desde cuándo y hasta cuánto. Vive aquí y no en la
    /// tarjeta porque la tarjeta se destruye al cerrarla, y el silencio tiene
    /// que seguir puesto: silenciar y perder el silencio al cerrar la ventana
    /// donde lo pusiste es peor que no poder silenciar.
    silencio: Option<(std::time::Instant, Option<std::time::Duration>)>,

    /// El aviso de la última notificación, mientras dura.
    toast: Option<(toast::Toast, Canvas)>,

    /// La tarea viva publicada por una aplicación de sistema. Es distinta de
    /// `toast`: no caduca y sus controles devuelven acciones a quien la creó.
    actividad: Option<(actividad::Actividad, Canvas)>,
    /// Todas las tareas publicadas. La isla enseña una por prioridad, pero una
    /// grabación no debe borrar la música: al parar vuelve la que seguía viva.
    actividades: std::collections::HashMap<String, actividad::Estado>,

    /// Las notificaciones que han llegado. Las recibe el compositor por D-Bus
    /// y las deja aquí; el panel y su tarjeta las leen de un solo sitio.
    notificaciones: notificaciones::Registro,

    /// Las barras de título vivas, por ventana. La clave la pone el
    /// compositor, que es quien sabe qué ventana es cuál; aquí solo se guarda
    /// el buffer y lo último que se pintó en él, para no repintar una barra que
    /// no ha cambiado —hay una por ventana, y repintarlas todas en cada frame
    /// sería el trabajo del panel multiplicado por las ventanas abiertas.
    barras: std::collections::HashMap<u64, Barra>,

    /// La superficie emergente abierta, si hay alguna. Solo puede haber una:
    /// abrir el calendario con el menú desplegado cierra el menú, que es lo que
    /// hace cualquier barra de menús.
    emergente: Option<(Emergente, Canvas)>,

    /// Los iconos del escritorio y el lienzo con el que se pinta **cada** uno.
    ///
    /// El lienzo es uno solo y no uno por icono porque todas las celdas miden
    /// lo mismo: lo único que guarda es el tamaño, la escala y la caché de
    /// disposición de iced, y esa se reconstruye igual entre celdas —el árbol
    /// de widgets de todas es el mismo—. El buffer no está aquí: lo pone el
    /// compositor, que es quien tiene una superficie por icono.
    escritorio: escritorio::Escritorio,
    escritorio_canvas: Canvas,
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
        // Lo primero de todo: los widgets preguntan por los colores mientras se
        // construyen, y con el tema puesto después el primer frame saldría con
        // la paleta anterior.
        tema::aplicar(config.tema);
        tema::aplicar_modo(config.modo_tema);
        tema::aplicar_acento(config.acento);
        tema::aplicar_alto_contraste(config.alto_contraste);
        // Cuál de las familias instaladas está puesta, para que la tarjeta de
        // Apariencia pueda marcarla. Se mira la del tema que toca y, si esa
        // clave no está, el `fondo` común; sin ninguna no hay nada que marcar y
        // manda la que elija `fondos::por_defecto`.
        let puesto = if config.tema == tema::Tema::Claro {
            config.fondo_claro.as_deref()
        } else {
            config.fondo_oscuro.as_deref()
        }
        .or(config.fondo.as_deref());
        fondos::poner_elegida(
            puesto
                .map(std::path::Path::new)
                .and_then(fondos::familia_de)
                .or_else(|| {
                    fondos::por_defecto(config.tema == tema::Tema::Claro).map(|f| f.nombre)
                }),
        );
        tema::aplicar_efectos_reducidos(config.efectos == config::Efectos::Reducidos);
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
        let mut dock_items = Dock::from_config(&config.dock);
        dock_items.tamano(config.dock_tamano);
        let (dw, dh) = dock_items.size();
        Self {
            // Font::DEFAULT resuelve contra las fuentes del sistema vía fontdb.
            renderer_panel: Renderer::new(Font::DEFAULT, Pixels(13.0)),
            renderer_dock: Renderer::new(Font::DEFAULT, Pixels(13.0)),
            renderer: Renderer::new(Font::DEFAULT, Pixels(13.0)),
            theme: view::theme(),
            panel: Canvas::new(Size::new(width as f32, PANEL_HEIGHT as f32), scale),
            widgets,
            dock: Canvas::new(Size::new(dw, dh), scale),
            dock_items,
            avatar: config.avatar.clone(),
            efectos_config: config.efectos,
            launchpad_dock: config.launchpad_dock,
            bloqueo_config: config.bloqueo,
            actividades_config: config.actividades,
            emergente: None,
            bloqueo: None,
            captura: None,
            osd: None,
            diagnostico: None,
            conmutador: None,
            notificaciones: notificaciones::Registro::default(),
            silencio: None,
            toast: None,
            actividad: None,
            actividades: std::collections::HashMap::new(),
            barras: std::collections::HashMap::new(),
            // Sin alto todavía: lo sabe el compositor y llega por
            // `escritorio_pantalla` antes del primer frame.
            escritorio: escritorio::Escritorio::new(PANEL_HEIGHT as f32, (width as f32, 0.0)),
            escritorio_canvas: Canvas::new(
                Size::new(escritorio::CELDA.0, escritorio::CELDA.1),
                scale,
            ),
        }
    }

    pub fn resize(&mut self, width: u32, scale: f32) {
        if self.panel.size.width != width as f32 || self.panel.scale != scale {
            self.panel.size.width = width as f32;
            self.panel.scale = scale;
            self.panel.painted_once = false;
            self.dock.scale = scale;
            self.dock.painted_once = false;
            if let Some((_, canvas)) = self.actividad.as_mut() {
                canvas.scale = scale;
                canvas.painted_once = false;
            }
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

    // --- Iconos del escritorio ---------------------------------------------

    pub fn escritorio(&self) -> &escritorio::Escritorio {
        &self.escritorio
    }

    pub fn escritorio_mut(&mut self) -> &mut escritorio::Escritorio {
        &mut self.escritorio
    }

    /// Le dice al escritorio el tamaño **lógico** de la pantalla, que el shell
    /// no conoce: solo se le pasa el ancho al construirlo.
    pub fn escritorio_pantalla(&mut self, pantalla: (f32, f32), scale: f32) {
        self.escritorio.recolocar(PANEL_HEIGHT as f32, pantalla);
        if self.escritorio_canvas.scale != scale {
            self.escritorio_canvas =
                Canvas::new(Size::new(escritorio::CELDA.0, escritorio::CELDA.1), scale);
        }
    }

    /// Vuelve a mirar la carpeta del escritorio. `true` si la lista cambió.
    pub fn escritorio_releer(&mut self, pantalla: (f32, f32)) -> bool {
        self.escritorio.releer(PANEL_HEIGHT as f32, pantalla)
    }

    /// Tamaño en píxeles físicos del buffer de **una** celda. Son todas
    /// iguales, así que el compositor reserva el mismo para cada icono.
    pub fn escritorio_buffer_size(&self) -> (u32, u32) {
        self.escritorio_canvas.buffer_size()
    }

    pub fn escritorio_logical_size(&self) -> (f32, f32) {
        (
            self.escritorio_canvas.size.width,
            self.escritorio_canvas.size.height,
        )
    }

    /// Pinta la celda `i` en `buf`, del tamaño de [`Shell::escritorio_buffer_size`].
    pub fn draw_escritorio(&mut self, i: usize, buf: &mut [u8]) -> Vec<Damage> {
        let view = self.escritorio.ver(i);
        let Self {
            renderer,
            theme,
            escritorio_canvas,
            ..
        } = self;
        Self::paint(escritorio_canvas, renderer, theme, view, buf)
    }

    /// Relee los estados. Devuelve `true` si hay que repintar el panel.
    ///
    /// Esta es la puerta que mantiene el shell callado: si nada de lo que se
    /// enseña ha cambiado, nadie dibuja nada. Se llama tanto en el cambio de
    /// minuto como cuando el kernel avisa de un cambio de hardware, así que
    /// tiene que ser barata y no dar por hecho cada cuánto la llaman.
    pub fn refresh(&mut self) -> bool {
        let cambio = self.widgets.refrescar();
        let cambio_emergente = self.refresh_emergente();
        cambio || cambio_emergente || !self.panel.painted_once
    }

    pub fn panel_animando(&self) -> bool {
        self.widgets.animando()
    }

    pub fn panel_avanzar(&mut self) {
        self.widgets.avanzar();
    }

    /// Relee solo la tarjeta abierta; no consulta batería ni carpetas.
    pub fn refresh_emergente(&mut self) -> bool {
        let mut cambio_emergente = false;
        // La emergente abierta también relee lo suyo: la lista de redes cambia
        // mientras la tienes delante, y sin esto se quedaría con la foto del
        // instante en que se abrió.
        if let Some((e, canvas)) = self.emergente.as_mut() {
            if e.refrescar() {
                cambio_emergente = true;
                Self::ajustar(e, canvas);
                canvas.painted_once = false;
            }
        }
        cambio_emergente
    }

    // --- Notificaciones ----------------------------------------------------

    // --- Actividades vivas -------------------------------------------------

    pub fn publicar_actividad(&mut self, mut estado: actividad::Estado) {
        if !self.actividades_config.habilitadas {
            self.actividad = None;
            self.actividades.clear();
            return;
        }
        // La portada pesa cientos de KB. El Player solo la reenvía al cambiar
        // de canción y las actualizaciones de posición llegan vacías.
        if estado.portada.is_none() {
            estado.portada = self
                .actividades
                .get(&estado.app_id)
                .and_then(|anterior| anterior.portada.clone());
        }
        self.actividades.insert(estado.app_id.clone(), estado);
        self.sincronizar_actividad();
    }

    fn sincronizar_actividad(&mut self) {
        let estado = self
            .actividades
            .values()
            .filter(|e| {
                e.clase != actividad::Clase::Timer
                    || self.actividades_config.temporizador_siempre
                    || e.restante_ms <= 60_000
            })
            .max_by_key(|e| match e.clase {
                actividad::Clase::Recorder => 40,
                actividad::Clase::Timer if e.restante_ms < 0 => 30,
                actividad::Clase::Timer => 20,
                actividad::Clase::Player => 10,
            })
            .cloned();
        let Some(estado) = estado else {
            self.actividad = None;
            return;
        };
        let escala = self.panel.scale;
        match self.actividad.as_mut() {
            Some((actual, canvas)) if actual.app_id() == estado.app_id => {
                actual.actualizar(estado);
                let (w, h) = actual.size();
                let objetivo = Size::new(w, h);
                if canvas.size != objetivo {
                    *canvas = Canvas::new(objetivo, escala);
                } else {
                    canvas.painted_once = false;
                }
            }
            _ => {
                let actividad =
                    actividad::Actividad::nueva(estado, self.actividades_config.animaciones);
                let (w, h) = actividad.size();
                self.actividad = Some((actividad, Canvas::new(Size::new(w, h), escala)));
            }
        }
    }

    pub fn cerrar_actividad(&mut self, app_id: &str) -> bool {
        if self.actividades.remove(app_id).is_none() {
            return false;
        }
        self.sincronizar_actividad();
        true
    }

    pub fn aplicar_actividades_config(&mut self, config: config::Actividades) {
        self.actividades_config = config;
        if !config.habilitadas {
            self.actividades.clear();
            self.actividad = None;
            return;
        }
        if let Some((actividad, canvas)) = self.actividad.as_mut() {
            actividad.set_animaciones(config.animaciones);
            canvas.painted_once = false;
        }
        self.sincronizar_actividad();
    }

    pub fn actividad_app_id(&self) -> Option<&str> {
        self.actividad.as_ref().map(|(a, _)| a.app_id())
    }

    pub fn actividad_clase(&self) -> Option<actividad::Clase> {
        self.actividad.as_ref().map(|(a, _)| a.clase())
    }

    pub fn actividad_buffer_size(&self) -> Option<(u32, u32)> {
        self.actividad.as_ref().map(|(_, c)| c.buffer_size())
    }

    pub fn actividad_logical_size(&self) -> Option<(f32, f32)> {
        self.actividad
            .as_ref()
            .map(|(_, c)| (c.size.width, c.size.height))
    }

    pub fn actividad_needs_paint(&self) -> bool {
        self.actividad
            .as_ref()
            .is_some_and(|(a, c)| !c.painted_once || a.ondas_pendientes())
    }

    pub fn actividad_animando(&self) -> bool {
        self.actividad.as_ref().is_some_and(|(a, _)| a.animando())
    }

    pub fn actividad_entrada(&self) -> (f32, f32, f32) {
        self.actividad
            .as_ref()
            .map_or((0.0, 1.0, 0.0), |(a, _)| a.entrada())
    }

    pub fn actividad_puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let cambio = self
            .actividad
            .as_mut()
            .is_some_and(|(a, _)| a.puntero(punto));
        if cambio {
            if let Some((_, c)) = self.actividad.as_mut() {
                c.painted_once = false;
            }
        }
        cambio
    }

    pub fn actividad_pulsar(&mut self, x: f32, y: f32) -> Option<actividad::Accion> {
        let (accion, size) = {
            let (actividad, _) = self.actividad.as_mut()?;
            let accion = actividad.pulsar(x, y);
            (accion, actividad.size())
        };
        let escala = self.panel.scale;
        if let Some((_, canvas)) = self.actividad.as_mut() {
            let objetivo = Size::new(size.0, size.1);
            if canvas.size != objetivo {
                *canvas = Canvas::new(objetivo, escala);
            } else {
                canvas.painted_once = false;
            }
        }
        accion
    }

    pub fn abrir_actividad_previsualizacion(&mut self, app_id: &str) -> bool {
        let Some((actividad, canvas)) = self.actividad.as_mut() else {
            return false;
        };
        if actividad.app_id() != app_id || !actividad.abrir_previsualizacion() {
            return false;
        }
        let (w, h) = actividad.size();
        *canvas = Canvas::new(Size::new(w, h), self.panel.scale);
        true
    }

    pub fn alternar_actividad(&mut self) -> bool {
        let Some((actividad, canvas)) = self.actividad.as_mut() else {
            return false;
        };
        if !actividad.alternar_vista() {
            return false;
        }
        let (w, h) = actividad.size();
        *canvas = Canvas::new(Size::new(w, h), self.panel.scale);
        true
    }

    pub fn draw_actividad(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((actividad, canvas)) = self.actividad.as_mut() else {
            return Vec::new();
        };
        let vista = actividad.view();
        let damage = Self::paint(canvas, &mut self.renderer, &self.theme, vista, buf);
        actividad.marcar_ondas_pintadas();
        damage
    }

    /// Guarda una notificación recién llegada y saca su aviso. `true` si hay
    /// que repintar.
    ///
    /// Con «No molestar» puesto **no** se saca el aviso y la notificación se
    /// queda solo en la lista, que es justo lo que promete la fila de la
    /// tarjeta («las notificaciones solo se guardan aquí»). Las críticas sí
    /// salen: para eso son críticas.
    ///
    /// `caducidad` es el `expire_timeout` de D-Bus, tal cual llega.
    pub fn notificar(
        &mut self,
        notificacion: notificaciones::Notificacion,
        caducidad: i32,
    ) -> bool {
        if !self.notificaciones_silenciadas() || notificacion.critica {
            let aviso = toast::Toast::new(notificacion.clone(), caducidad);
            let (w, h) = aviso.size();
            let objetivo = Size::new(w, h);
            match self.toast.as_mut() {
                // El tamaño del aviso no depende del texto, así que el buffer
                // que ya está sirve: se reaprovecha en vez de tirarlo, igual
                // que hace el aviso de volumen al subirlo dos veces seguidas.
                Some((viejo, canvas)) if canvas.size == objetivo => {
                    *viejo = aviso;
                    canvas.painted_once = false;
                }
                _ => {
                    let canvas = Canvas::new(objetivo, self.panel.scale);
                    self.toast = Some((aviso, canvas));
                }
            }
        }
        self.notificaciones.añadir(notificacion);
        self.sincronizar_notificaciones();
        true
    }

    // --- El aviso de una notificación ---------------------------------------

    /// ¿Sigue el aviso a la vista? De paso lo retira si se le acabó el tiempo.
    pub fn toast_vivo(&mut self) -> bool {
        if self
            .toast
            .as_ref()
            .is_some_and(|(t, _)| t.queda().is_none())
        {
            self.toast = None;
        }
        self.toast.is_some()
    }

    /// Cuánto le queda, para programar **un** despertar en vez de repintar
    /// sesenta veces por segundo enseñando lo mismo.
    pub fn toast_queda(&self) -> Option<std::time::Duration> {
        self.toast.as_ref().and_then(|(t, _)| t.queda())
    }

    pub fn toast_alfa(&self) -> f32 {
        self.toast.as_ref().map_or(0.0, |(t, _)| t.alfa())
    }

    pub fn toast_escala(&self) -> f32 {
        self.toast.as_ref().map_or(1.0, |(t, _)| t.escala())
    }

    pub fn toast_animando(&self) -> bool {
        self.toast.as_ref().is_some_and(|(t, _)| t.animando())
    }

    pub fn toast_needs_paint(&self) -> bool {
        self.toast.as_ref().is_some_and(|(_, c)| !c.painted_once)
    }

    pub fn toast_id(&self) -> Option<u32> {
        self.toast.as_ref().map(|(t, _)| t.id())
    }

    pub fn toast_accion_relativa(&self, x: f32, y: f32) -> Option<String> {
        self.toast.as_ref().and_then(|(t, _)| t.accion_en(x, y))
    }

    pub fn toast_buffer_size(&self) -> Option<(u32, u32)> {
        self.toast.as_ref().map(|(_, c)| c.buffer_size())
    }

    pub fn toast_logical_size(&self) -> Option<(f32, f32)> {
        self.toast
            .as_ref()
            .map(|(_, c)| (c.size.width, c.size.height))
    }

    /// Manda el aviso a irse: es lo que hace pulsarlo. La notificación se queda
    /// en la lista —descartar el aviso no es haberla atendido—.
    pub fn descartar_toast(&mut self) -> bool {
        match self.toast.as_mut() {
            Some((t, _)) => {
                t.descartar();
                true
            }
            None => false,
        }
    }

    // --- Las barras de título ----------------------------------------------

    /// Prepara la barra de la ventana `id` para el ancho y el estado que se le
    /// pasan. Devuelve `true` si hay que volver a pintarla.
    ///
    /// El compositor llama a esto en cada frame, así que lo normal es que no
    /// haya nada que hacer: solo se marca para repintar cuando de verdad cambia
    /// algo que se ve —el ancho, el título, el foco o el botón señalado—.
    pub fn barra_preparar(&mut self, id: u64, ancho: f32, estado: decoracion::Estado) -> bool {
        let escala = self.panel.scale;
        match self.barras.get_mut(&id) {
            Some(barra) if barra.canvas.size.width == ancho && barra.canvas.scale == escala => {
                if barra.estado == estado {
                    return !barra.canvas.painted_once;
                }
                barra.estado = estado;
                barra.canvas.painted_once = false;
            }
            _ => {
                let canvas = Canvas::new(Size::new(ancho, decoracion::ALTO), escala);
                self.barras.insert(id, Barra { estado, canvas });
            }
        }
        true
    }

    pub fn barra_buffer_size(&self, id: u64) -> Option<(u32, u32)> {
        self.barras.get(&id).map(|b| b.canvas.buffer_size())
    }

    /// Tamaño **lógico** de la barra ya cuadrado a píxel entero, que es el que
    /// el compositor tiene que darle a Smithay para que la textura no se
    /// reescale.
    pub fn barra_logical_size(&self, id: u64) -> Option<(f32, f32)> {
        self.barras
            .get(&id)
            .map(|b| (b.canvas.size.width, b.canvas.size.height))
    }

    pub fn draw_barra(&mut self, id: u64, buf: &mut [u8]) -> Vec<Damage> {
        let Some(barra) = self.barras.get_mut(&id) else {
            return Vec::new();
        };
        let vista = decoracion::vista(&barra.estado, barra.canvas.size.width);
        Self::paint(
            &mut barra.canvas,
            &mut self.renderer,
            &self.theme,
            vista,
            buf,
        )
    }

    /// Tira las barras de las ventanas que ya no están. Sin esto, cada ventana
    /// cerrada dejaría su buffer —del ancho de la ventana— en el mapa para
    /// siempre.
    pub fn barras_retener(&mut self, vivas: &[u64]) {
        self.barras.retain(|id, _| vivas.contains(id));
    }

    pub fn draw_toast(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((t, canvas)) = self.toast.as_mut() else {
            return Vec::new();
        };
        let vista = t.view();
        Self::paint(canvas, &mut self.renderer, &self.theme, vista, buf)
    }

    /// Retira la del `id`, la haya cerrado quien la haya cerrado.
    pub fn cerrar_notificacion(&mut self, id: u32) -> bool {
        if !self.notificaciones.cerrar(id) {
            return false;
        }
        self.sincronizar_notificaciones();
        true
    }

    /// Vacía la cola y devuelve los identificadores, que hay que cerrar por
    /// D-Bus uno a uno.
    pub fn borrar_notificaciones(&mut self) -> Vec<u32> {
        let ids = self.notificaciones.vaciar();
        if !ids.is_empty() {
            self.sincronizar_notificaciones();
        }
        ids
    }

    /// Marca como leídas las notificaciones al abrir su historial.
    pub fn marcar_notificaciones_leidas(&mut self) -> bool {
        if !self.notificaciones.marcar_leidas() {
            return false;
        }
        self.sincronizar_notificaciones();
        true
    }

    pub fn notificaciones(&self) -> &[notificaciones::Notificacion] {
        self.notificaciones.lista()
    }

    /// ¿Está «No molestar» puesto ahora mismo?
    ///
    /// Se calcula en vez de guardarse porque **caduca solo**: un temporizador
    /// que despierte al shell a la hora en punto para apagar un booleano es
    /// justo el despertar de más que este compositor evita.
    pub fn poner_no_molestar(&mut self, enabled: bool) {
        self.silencio = enabled.then(|| (std::time::Instant::now(), None));
        if let Some((Emergente::Notificaciones(n), canvas)) = self.emergente.as_mut() {
            n.poner_silencio(self.silencio);
            n.actualizar(self.notificaciones.lista().to_vec());
            canvas.painted_once = false;
        }
    }

    pub fn notificaciones_silenciadas(&self) -> bool {
        match self.silencio {
            None => false,
            Some((_, None)) => true,
            Some((desde, Some(cuanto))) => desde.elapsed() < cuanto,
        }
    }

    /// Reparte la cola: el contador al widget del panel y la lista a la tarjeta
    /// si está abierta.
    fn sincronizar_notificaciones(&mut self) {
        let cuantas = self.notificaciones.cuantas();
        if self.widgets.notificaciones(cuantas) {
            self.panel.painted_once = false;
        }
        let lista = self.notificaciones.lista().to_vec();
        if let Some((Emergente::Notificaciones(n), canvas)) = self.emergente.as_mut() {
            n.actualizar(lista);
            canvas.painted_once = false;
        }
    }

    /// Cambia el tema y el acento en caliente, y manda repintarlo todo.
    ///
    /// Los colores son un global del proceso y lo ya dibujado no se entera:
    /// hay que invalidar **todas** las superficies, no solo la que se está
    /// mirando. El dock y la pantalla de bloqueo se quedaban con la paleta
    /// anterior hasta que algo más los tocara.
    pub fn aplicar_apariencia(&mut self, tema_nuevo: tema::Tema, acento: tema::Acento) {
        tema::aplicar(tema_nuevo);
        tema::aplicar_acento(acento);
        self.theme = view::theme();
        for canvas in [
            Some(&mut self.panel),
            Some(&mut self.dock),
            self.emergente.as_mut().map(|(_, c)| c),
            self.bloqueo.as_mut().map(|(_, c)| c),
            self.osd.as_mut().map(|(_, c)| c),
            self.conmutador.as_mut().map(|(_, c)| c),
            self.actividad.as_mut().map(|(_, c)| c),
        ]
        .into_iter()
        .flatten()
        {
            canvas.painted_once = false;
            // Y la caché del layout de iced con ellos: guarda los elementos ya
            // construidos, con el color de antes dentro.
            canvas.cache = None;
        }
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
        let mut subsistemas = self.widgets.subsistemas();
        // También los usan las tarjetas y las teclas, aunque el widget no esté fijado.
        subsistemas.extend(["backlight", "leds"]);
        subsistemas.sort_unstable();
        subsistemas.dedup();
        subsistemas
    }

    /// ¿Hace falta pintar el dock? Su contenido solo cambia al señalar un
    /// icono, así que la primera vez, tras un cambio de tamaño y en el hover.
    ///
    /// Mientras la placa del icono señalado entra o sale hay que repintarlo en
    /// cada fotograma: el dibujo cambia sin que llegue ningún evento.
    pub fn dock_needs_paint(&self) -> bool {
        !self.dock.painted_once || self.dock_items.animando()
    }

    /// ¿Se está moviendo algo dentro del dock?
    pub fn dock_animando(&self) -> bool {
        self.dock_items.animando()
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
        if widget == "notificaciones" {
            self.marcar_notificaciones_leidas();
        }
        let Some(constructor) = emergente_de(widget) else {
            return;
        };
        let mut emergente = constructor();
        // La tarjeta de notificaciones nace con la cola dentro: es lo único que
        // enseña, y construirla vacía dejaría un fotograma con «No hay
        // notificaciones» antes de repintarla.
        if let Emergente::Notificaciones(n) = &mut emergente {
            // El silencio lo guarda el shell: la tarjeta se destruye al
            // cerrarla y volvería a nacer sin él.
            *n = emergente::notificaciones_con(self.silencio);
            n.actualizar(self.notificaciones.lista().to_vec());
        }
        self.abrir(emergente);
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
    /// El tercer campo es el alto con el que hay que **colocarla** cuando no es
    /// el que mide: ver [`Emergente::alto_estable`].
    pub fn emergente_geometria(&self) -> Option<((i32, i32), Ancla, Option<i32>)> {
        let (e, canvas) = self.emergente.as_ref()?;
        Some((
            (canvas.size.width as i32, canvas.size.height as i32),
            e.ancla(),
            e.alto_estable().map(|h| h as i32),
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
        // El silencio se lo queda el shell en cuanto se toca: es lo único de la
        // tarjeta que tiene que sobrevivirla. Se recoge aquí, con `e` todavía a
        // mano, y se guarda al final: `self` está prestado hasta entonces.
        let silencio = match e {
            Emergente::Notificaciones(n) => Some(n.silencio()),
            _ => None,
        };
        // Una pulsación también puede cambiar estado sin producir una acción
        // (el segundo clic sobre un nombre entra en edición).
        canvas.painted_once = false;
        // Hay tarjetas que crecen con lo que se pulsa: «No molestar» despliega
        // sus cuatro duraciones. Sin esto el contenido nuevo se pintaba fuera
        // del buffer y desaparecía —el canvas se hizo al abrir y nadie lo
        // volvía a mirar.
        Self::ajustar(e, canvas);
        // Elegir algo del menú lo cierra, como cualquier menú. Pulsar en un
        // hueco no: ahí el usuario ha fallado la puntería, no ha decidido nada.
        if accion.as_ref().is_some_and(|a| !a.conserva_emergente()) {
            self.emergente = None;
        } else if accion.is_some() {
            canvas.painted_once = false;
        }
        if let Some(silencio) = silencio {
            self.silencio = silencio;
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

    /// Rehace el buffer si la emergente ha cambiado de tamaño.
    ///
    /// Hay tarjetas que crecen con lo que se hace dentro: «No molestar»
    /// despliega sus cuatro duraciones al pulsarla, y el buscador cambia de
    /// alto con cada tecla según cuántos resultados haya. Sin esto el contenido
    /// nuevo se pinta fuera del buffer y desaparece —el canvas se hizo al abrir
    /// y nadie lo volvía a mirar.
    fn ajustar(e: &Emergente, canvas: &mut Canvas) {
        let (w, h) = e.size();
        // Cuadrado a píxel entero antes de comparar: `Canvas::new` lo hace por
        // dentro, y sin cuadrarlo aquí el tamaño pedido nunca coincidiría con
        // el guardado y se tiraría la caché de iced en cada interacción.
        let objetivo = Size::new(
            a_pixel_entero(w, canvas.scale),
            a_pixel_entero(h, canvas.scale),
        );
        if canvas.size != objetivo {
            *canvas = Canvas::new(objetivo, canvas.scale);
        }
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
                Self::ajustar(e, canvas);
                canvas.painted_once = false;
                (true, None)
            }
            // Lanzar algo cierra la emergente, igual que elegirlo con el ratón.
            emergente::Tecla::Hacer(accion) => {
                if accion.conserva_emergente() {
                    canvas.painted_once = false;
                } else {
                    self.emergente = None;
                }
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
    /// ¿La emergente abierta quiere cristal esmerilado debajo?
    pub fn emergente_usa_cristal(&self) -> bool {
        self.emergente
            .as_ref()
            .is_some_and(|(e, _)| e.usa_cristal())
    }

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

    /// Rectángulos relativos al buffer reservados para las previsualizaciones.
    pub fn escritorios_miniaturas(&self) -> Vec<Rectangle> {
        self.emergente
            .as_ref()
            .map(|(e, _)| e.miniaturas_escritorios())
            .unwrap_or_default()
    }

    /// Actualiza la vista general tras crear, borrar, renombrar o cambiar.
    pub fn actualizar_vista_escritorios(&mut self, activo: usize, nombres: Vec<String>) -> bool {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return false;
        };
        let cambio = e.actualizar_escritorios(activo, nombres);
        if cambio {
            canvas.painted_once = false;
        }
        cambio
    }

    /// El cambio de vista del launchpad que el compositor todavía no ha
    /// animado, si lo hay. Se lo lleva quien pregunta.
    ///
    /// El launchpad no anima nada por su cuenta —rasterizar la rejilla cuesta
    /// 11 ms— y esto es cómo lo pide: el compositor guarda la superficie de
    /// antes, deja que se pinte la de después y cruza las dos en la GPU.
    pub fn launchpad_transicion(&mut self) -> Option<emergente::launchpad::Transicion> {
        match self.emergente.as_mut() {
            Some((Emergente::Launchpad(l), _)) => l.tomar_transicion(),
            _ => None,
        }
    }

    /// ¿El realce tiene que ir **por delante** del buffer de la emergente?
    ///
    /// Solo dentro de una carpeta del launchpad: su panel es opaco —es un
    /// diálogo— y detrás de él el realce no se ve. Por delante se cuela un 6 %
    /// de tinta sobre el icono señalado, que no se nota; detrás, no hay
    /// realce que valga.
    pub fn realce_delante(&self) -> bool {
        matches!(
            self.emergente.as_ref(),
            Some((Emergente::Launchpad(l), _)) if l.en_carpeta()
        )
    }

    /// Pinta el realce en `buf`, que debe medir lo que diga
    /// [`Shell::emergente_realce`].
    ///
    /// Es una superficie propia y diminuta: repintarla cuesta lo que una celda,
    /// no lo que la rejilla entera, y mover el ratón solo cambia su posición.
    pub fn draw_realce(&mut self, buf: &mut [u8], ancho: f32, alto: f32, marco: bool) {
        use iced_core::{Border, Length};
        use iced_widget::{Space, container};

        let vista: view::PanelElement<'_> = container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(
                    if marco {
                        Color {
                            a: 0.18,
                            ..tema::acento()
                        }
                    } else {
                        tema::hover()
                    }
                    .into(),
                ),
                border: Border {
                    radius: tema::R_POPOVER.into(),
                    width: if marco { 2.0 } else { 0.0 },
                    color: Color {
                        a: 0.7,
                        ..tema::acento()
                    },
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
        // El indicador de escritorios no abre ninguna tarjeta: el punto que se
        // pulsa es el escritorio al que se va, así que sale por otro camino
        // —una acción para el compositor— antes de mirar las emergentes.
        if let Some((nombre, x0, _)) = self
            .zonas_panel()
            .into_iter()
            .find(|(_, x0, x1)| x >= *x0 && x <= *x1)
        {
            if let Some(accion) = self.widgets.pulsar(nombre, x - x0) {
                return Some(accion);
            }
        }
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

    /// Nombre de la tarjeta que corresponde a una coordenada del panel.
    /// Permite al compositor encadenar la salida de una tarjeta con la entrada
    /// de otra sin construir la nueva encima de la textura anterior.
    pub fn objetivo_emergente_panel(&self, x: f32) -> Option<(&'static str, &'static str)> {
        if x <= view::ANCHO_LOGO {
            return Some(("menu", "menu"));
        }
        self.zonas_panel()
            .into_iter()
            .find(|(_, x0, x1)| x >= *x0 && x <= *x1)
            .and_then(|(widget, _, _)| {
                nombre_emergente_de(widget).map(|emergente| (widget, emergente))
            })
    }

    pub fn abrir_emergente_nombre(&mut self, nombre: &'static str) -> bool {
        let nueva = if nombre == "menu" {
            Some(Emergente::menu())
        } else {
            emergente_de(nombre).map(|f| f())
        };
        if let Some(nueva) = nueva {
            self.abrir(nueva);
            true
        } else {
            false
        }
    }

    /// Lo que el compositor sabe de los escritorios y el panel no puede saber.
    ///
    /// `true` si hay que repintar. Se llama en cada cambio de escritorio, no en
    /// cada frame: el widget compara y calla si no ha cambiado nada.
    pub fn escritorios(&mut self, activo: usize, cuantos: usize) -> bool {
        let cambio = self.widgets.escritorios(activo, cuantos);
        if cambio {
            self.panel.painted_once = false;
        }
        cambio
    }

    /// Dónde cae cada widget del panel, en lógicos: `(nombre, x0, x1)`.
    ///
    /// Es lo que convierte un clic en el panel en "has pulsado el volumen".
    pub fn zonas_panel(&self) -> Vec<(&'static str, f32, f32)> {
        self.widgets
            .zonas_derecha(self.panel.size.width, view::MARGEN_PANEL, view::HUECO)
    }

    /// El botón se ha soltado. `true` si hay que repintar.
    pub fn soltar(&mut self) -> (bool, Option<Accion>) {
        let Some((e, canvas)) = self.emergente.as_mut() else {
            return (false, None);
        };
        let (repintar, accion) = e.soltar();
        if repintar {
            canvas.painted_once = false;
        }
        // Lanzar algo cierra el launchpad, igual que hacía cuando la acción
        // salía de `pulsar`.
        if accion.as_ref().is_some_and(|a| !a.conserva_emergente()) {
            self.emergente = None;
        }
        (repintar, accion)
    }

    /// ¿Se enseña el dock con el launchpad abierto? Lo decide la configuración.
    pub fn dock_en_launchpad(&self) -> bool {
        self.launchpad_dock
    }

    /// Le dice al launchpad dónde está el dock, en coordenadas de pantalla.
    ///
    /// `None` mientras no se vea: sin esto, con el dock oculto se seguiría
    /// anclando al soltar un icono en la franja de abajo, donde no hay nada.
    pub fn launchpad_zona_dock(&mut self, zona: Option<(f32, f32, f32, f32)>) {
        if let Some((Emergente::Launchpad(l), _)) = self.emergente.as_mut() {
            l.poner_zona_dock(zona.map(|(x, y, w, h)| iced_core::Rectangle {
                x,
                y,
                width: w,
                height: h,
            }));
        }
    }

    /// Los primeros elementos visibles del launchpad. Para el autotest.
    pub fn launchpad_primeros(&self, n: usize) -> Vec<String> {
        match self.emergente.as_ref() {
            Some((Emergente::Launchpad(l), _)) => l.primeros(n),
            _ => Vec::new(),
        }
    }

    /// El icono que se arrastra por el launchpad: `(x, y, lado)` lógicos
    /// relativos a su esquina. `None` si no hay ninguno en el aire.
    pub fn launchpad_agarrado(&self) -> Option<(f32, f32, f32)> {
        match self.emergente.as_ref() {
            Some((Emergente::Launchpad(l), _)) => l.rect_agarrado().map(|r| (r.x, r.y, r.width)),
            _ => None,
        }
    }

    /// Pinta el icono agarrado. `buf` debe tener el tamaño que corresponde a
    /// `lado` con la escala del shell.
    pub fn draw_agarrado(&mut self, buf: &mut [u8], lado: f32) {
        let Some((Emergente::Launchpad(l), _)) = self.emergente.as_ref() else {
            return;
        };
        let Some(vista) = l.vista_agarrado() else {
            return;
        };
        let escala = self.panel.scale;
        let mut canvas = Canvas::new(Size::new(lado, lado), escala);
        Self::paint(&mut canvas, &mut self.renderer, &self.theme, vista, buf);
    }

    /// El centro de una celda del launchpad, en coordenadas de pantalla. Para
    /// el autotest.
    pub fn launchpad_celda(&self, i: usize) -> Option<(f32, f32)> {
        match self.emergente.as_ref() {
            Some((Emergente::Launchpad(l), _)) => l.centro_de_celda(i),
            _ => None,
        }
    }

    /// El chip del selector de color de la carpeta abierta y un tono de su
    /// tira, en coordenadas del launchpad. Para el autotest.
    pub fn launchpad_selector(&self, grados: f32) -> Option<((f32, f32), (f32, f32))> {
        match self.emergente.as_ref() {
            Some((Emergente::Launchpad(l), _)) => {
                Some((l.centro_selector(), l.centro_tono(grados)))
            }
            _ => None,
        }
    }

    /// Alterna el modo edición del launchpad, si es lo que está abierto.
    pub fn launchpad_editar(&mut self) -> bool {
        match self.emergente.as_mut() {
            Some((Emergente::Launchpad(l), canvas)) => {
                canvas.painted_once = false;
                l.alternar_edicion()
            }
            _ => false,
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

    /// La fecha larga que va bajo el reloj del bloqueo: «lunes, 17 de agosto».
    ///
    /// Aquí sí, al contrario que en el reloj del panel: la pantalla de bloqueo
    /// se mira al volver a un equipo que llevaba horas parado, y es el momento
    /// del día en que uno menos sabe qué día es.
    pub fn fecha_bloqueo() -> String {
        const DIAS: [&str; 7] = [
            "lunes",
            "martes",
            "miércoles",
            "jueves",
            "viernes",
            "sábado",
            "domingo",
        ];
        const MESES: [&str; 12] = [
            "enero",
            "febrero",
            "marzo",
            "abril",
            "mayo",
            "junio",
            "julio",
            "agosto",
            "septiembre",
            "octubre",
            "noviembre",
            "diciembre",
        ];
        let hoy = state::Fecha::hoy();
        format!(
            "{}, {} de {}",
            DIAS[hoy.dia_semana() as usize],
            hoy.dia,
            MESES[(hoy.mes - 1) as usize]
        )
    }

    /// Enseña un aviso: el icono, su nivel y, si no lo tiene, un texto.
    ///
    /// Sustituye al que hubiera: subir el volumen dos veces seguidas no apila
    /// dos tarjetas, reinicia la misma.
    /// Enseña el aviso. Si ya hay uno de la misma clase a la vista, lo
    /// reaprovecha: mueve su barra y le reinicia el tiempo en vez de apilar
    /// otra cápsula encima.
    pub fn mostrar_osd(&mut self, icono: &str, nivel: Option<u8>, texto: Option<String>) {
        if let Some((o, canvas)) = self.osd.as_mut() {
            // El ancho depende del texto, así que un aviso reaprovechado puede
            // pedir otro tamaño de buffer; si cambia, se hace uno nuevo.
            if o.actualizar(icono, nivel, texto.clone()) {
                let (w, h) = o.size();
                let objetivo = Size::new(w, h);
                if canvas.size != objetivo {
                    *canvas = Canvas::new(objetivo, canvas.scale);
                } else {
                    canvas.painted_once = false;
                }
                return;
            }
        }
        let osd = osd::Osd::new(icono, nivel, texto);
        let (w, h) = osd.size();
        let canvas = Canvas::new(Size::new(w, h), self.panel.scale);
        self.osd = Some((osd, canvas));
    }

    /// La escala con la que se compone el aviso: entra creciendo.
    pub fn osd_escala(&self) -> f32 {
        self.osd.as_ref().map_or(1.0, |(o, _)| o.escala())
    }

    /// ¿Se está moviendo algo del aviso? La desaparición no cuenta: es alfa, y
    /// va con un solo despertar programado.
    pub fn osd_animando(&self) -> bool {
        self.osd.as_ref().is_some_and(|(o, _)| o.animando())
    }

    /// ¿Hace falta repintar el buffer del aviso? Mientras la barra se mueve, sí.
    pub fn osd_needs_paint(&self) -> bool {
        self.osd
            .as_ref()
            .is_some_and(|(o, c)| !c.painted_once || o.animando())
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

    /// Cambia la preferencia de efectos y la aplica. Devuelve la elegida, que
    /// es lo que el compositor tiene que escribir en el fichero.
    ///
    /// Lo elegido y lo efectivo no son lo mismo: con la batería baja el
    /// escritorio ya está en reducidos, y volver a «completos» desde aquí
    /// guarda la preferencia aunque no se vea hasta que haya corriente.
    pub fn alternar_efectos(&mut self) -> config::Efectos {
        self.efectos_config = match self.efectos_config {
            config::Efectos::Completos => config::Efectos::Reducidos,
            config::Efectos::Reducidos => config::Efectos::Completos,
        };
        self.revisar_efectos();
        self.efectos_config
    }

    pub fn aplicar_efectos_config(&mut self, efectos: config::Efectos) -> bool {
        self.efectos_config = efectos;
        self.revisar_efectos()
    }

    /// Recalcula si tocan efectos reducidos y lo aplica. Devuelve si cambió,
    /// que es cuando hay que repintar.
    ///
    /// El umbral es el mismo 20 % con el que el widget de batería pide el perfil
    /// de ahorro: dos umbrales distintos para «va justo de batería» harían que
    /// el escritorio cambiara de aspecto en un momento y de perfil en otro.
    /// Leer la batería aquí cuesta 0,02 ms —`capacity` y `status`, sin
    /// `current_now`— y se hace solo cuando el panel ya se estaba refrescando.
    pub fn revisar_efectos(&self) -> bool {
        let bateria_baja =
            state::Battery::read(false).is_some_and(|b| !b.plugged && b.percent <= 20);
        tema::aplicar_efectos_reducidos(
            self.efectos_config == config::Efectos::Reducidos || bateria_baja,
        )
    }

    // --- Panel de diagnóstico -----------------------------------------------

    pub fn diagnostico_visible(&self) -> bool {
        self.diagnostico.is_some()
    }

    /// Pone o quita el panel. Devuelve si quedó puesto.
    pub fn alternar_diagnostico(&mut self) -> bool {
        match self.diagnostico.take() {
            Some(_) => false,
            None => {
                let hud = diagnostico::Hud::new();
                let (w, h) = hud.size();
                let canvas = Canvas::new(Size::new(w, h), self.panel.scale);
                self.diagnostico = Some((hud, canvas));
                true
            }
        }
    }

    /// Entrega la medida de la última ventana. Solo repinta si algo cambió: con
    /// el escritorio quieto, los números son los mismos y despertar para
    /// redibujar lo mismo es exactamente lo que este panel existe para detectar.
    pub fn diagnostico_datos(&mut self, datos: diagnostico::Datos) -> bool {
        let Some((hud, canvas)) = self.diagnostico.as_mut() else {
            return false;
        };
        if hud.datos == datos {
            return false;
        }
        // El número de salidas cambia el alto, y con él el buffer.
        let alto_antes = hud.size().1;
        hud.datos = datos;
        let (w, h) = hud.size();
        if h != alto_antes {
            *canvas = Canvas::new(Size::new(w, h), canvas.scale);
        } else {
            canvas.painted_once = false;
        }
        true
    }

    pub fn diagnostico_buffer_size(&self) -> Option<(u32, u32)> {
        self.diagnostico.as_ref().map(|(_, c)| c.buffer_size())
    }

    pub fn diagnostico_logical_size(&self) -> Option<(f32, f32)> {
        self.diagnostico.as_ref().map(|(h, _)| h.size())
    }

    pub fn diagnostico_needs_paint(&self) -> bool {
        self.diagnostico
            .as_ref()
            .is_some_and(|(_, c)| !c.painted_once)
    }

    pub fn draw_diagnostico(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((hud, canvas)) = self.diagnostico.as_mut() else {
            return Vec::new();
        };
        let vista = hud.view();
        Self::paint(canvas, &mut self.renderer, &self.theme, vista, buf)
    }

    // --- Conmutador de aplicaciones ----------------------------------------

    /// Abre el conmutador con las celdas por orden de uso reciente.
    ///
    /// La lista la arma el compositor: el shell no ve el foco ni las ventanas.
    /// Con menos de dos **celdas** no se abre —no hay nada que conmutar— y el
    /// compositor lo sabe porque esto devuelve `false`.
    pub fn abrir_conmutador(
        &mut self,
        modo: conmutador::Modo,
        apps: Vec<conmutador::Entrada>,
        pantalla: (f32, f32),
    ) -> bool {
        if apps.len() < 2 {
            return false;
        }
        let c = conmutador::Conmutador::new(modo, apps, pantalla);
        let (w, h) = c.size();
        let canvas = Canvas::new(Size::new(w, h), self.panel.scale);
        self.conmutador = Some((c, canvas));
        true
    }

    /// Mueve la selección. `1` adelante, `-1` atrás.
    pub fn conmutador_mover(&mut self, pasos: i32) {
        if let Some((c, canvas)) = self.conmutador.as_mut() {
            c.mover(pasos);
            canvas.painted_once = false;
        }
    }

    pub fn conmutador_elegir(&mut self, i: usize) -> bool {
        if let Some((c, canvas)) = self.conmutador.as_mut() {
            if c.elegir(i) {
                canvas.painted_once = false;
                return true;
            }
        }
        false
    }

    pub fn conmutador_en(&self, x: f32, y: f32) -> Option<usize> {
        self.conmutador.as_ref()?.0.en(x, y)
    }

    pub fn conmutador_miniaturas(&self) -> Vec<Rectangle> {
        self.conmutador
            .as_ref()
            .map(|(c, _)| c.miniaturas())
            .unwrap_or_default()
    }

    /// Cierra el conmutador y dice **qué celda** quedó elegida.
    ///
    /// Un índice y no un `app_id`: quien sabe a qué ventana lleva cada celda es
    /// el compositor, que las tiene.
    pub fn cerrar_conmutador(&mut self) -> Option<usize> {
        let (c, _) = self.conmutador.take()?;
        c.elegida()
    }

    /// Lo abandona sin elegir nada. Es lo que hace Escape.
    pub fn cancelar_conmutador(&mut self) -> bool {
        self.conmutador.take().is_some()
    }

    pub fn hay_conmutador(&self) -> bool {
        self.conmutador.is_some()
    }

    pub fn conmutador_buffer_size(&self) -> Option<(u32, u32)> {
        self.conmutador.as_ref().map(|(_, c)| c.buffer_size())
    }

    pub fn conmutador_logical_size(&self) -> Option<(f32, f32)> {
        self.conmutador.as_ref().map(|(c, _)| c.size())
    }

    pub fn conmutador_needs_paint(&self) -> bool {
        self.conmutador
            .as_ref()
            .is_some_and(|(c, canvas)| !canvas.painted_once || c.animando())
    }

    /// ¿Se está moviendo el recuadro del conmutador?
    pub fn conmutador_animando(&self) -> bool {
        self.conmutador.as_ref().is_some_and(|(c, _)| c.animando())
    }

    pub fn draw_conmutador(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((c, canvas)) = self.conmutador.as_mut() else {
            return Vec::new();
        };
        let vista = c.view();
        Self::paint(canvas, &mut self.renderer, &self.theme, vista, buf)
    }

    /// Echa el bloqueo, con la hora ya formateada.
    ///
    /// `pantalla` es el tamaño **lógico**: el bloqueo la ocupa entera.
    pub fn bloquear(&mut self, hora: String, fecha: String, pantalla: (f32, f32)) {
        let canvas = Canvas::new(Size::new(pantalla.0, pantalla.1), self.panel.scale);
        let bloqueo =
            bloqueo::Bloqueo::new(hora, fecha, self.avatar.as_deref(), self.bloqueo_config);
        self.bloqueo = Some((bloqueo, canvas));
    }

    /// Abre la capa de captura. `pantalla` es el tamaño **lógico**: la ocupa
    /// entera, igual que el bloqueo.
    pub fn abrir_captura(&mut self, pantalla: (f32, f32)) {
        let canvas = Canvas::new(Size::new(pantalla.0, pantalla.1), self.panel.scale);
        self.captura = Some((captura::Captura::new(pantalla), canvas));
    }

    pub fn cerrar_captura(&mut self) -> bool {
        self.captura.take().is_some()
    }

    pub fn hay_captura(&self) -> bool {
        self.captura.is_some()
    }

    pub fn captura_buffer_size(&self) -> Option<(u32, u32)> {
        self.captura.as_ref().map(|(_, c)| c.buffer_size())
    }

    /// ¿Se está moviendo el recuadro? Es lo que impide que el compositor se
    /// duerma mientras se arrastra.
    pub fn captura_animando(&self) -> bool {
        self.captura.as_ref().is_some_and(|(c, _)| c.animando())
    }

    /// Mueve el puntero sobre la capa. `true` si hay que repintar.
    pub fn captura_puntero(&mut self, x: f32, y: f32) -> bool {
        let Some((c, canvas)) = self.captura.as_mut() else {
            return false;
        };
        if !c.puntero(x, y) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    pub fn captura_pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let (c, canvas) = self.captura.as_mut()?;
        let accion = c.pulsar(x, y);
        canvas.painted_once = false;
        accion
    }

    pub fn captura_soltar(&mut self) -> Option<Accion> {
        let (c, canvas) = self.captura.as_mut()?;
        let accion = c.soltar();
        canvas.painted_once = false;
        accion
    }

    /// Una tecla para la capa de captura.
    pub fn captura_tecla(&mut self, tecla: TeclaPulsada) -> Tecla {
        let Some((c, canvas)) = self.captura.as_mut() else {
            return Tecla::Ignorada;
        };
        let resultado = c.tecla(tecla);
        canvas.painted_once = false;
        resultado
    }

    /// ¿Hay que repintar su buffer?
    pub fn captura_needs_paint(&self) -> bool {
        self.captura.as_ref().is_some_and(|(_, c)| !c.painted_once)
    }

    /// Pinta la capa de captura. Sale **transparente** donde no hay velo: el
    /// hueco del recuadro es justo eso, un agujero por el que se ve lo de
    /// debajo.
    pub fn draw_captura(&mut self, buf: &mut [u8]) -> Vec<Damage> {
        let Some((c, canvas)) = self.captura.as_mut() else {
            return Vec::new();
        };
        let vista = c.view();
        Self::paint(canvas, &mut self.renderer, &self.theme, vista, buf)
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
    /// ¿Se está moviendo algo del bloqueo? Es lo que mantiene al compositor
    /// dibujando mientras el menú de apagado entra o sale.
    pub fn bloqueo_animando(&self) -> bool {
        self.bloqueo.as_ref().is_some_and(|(b, _)| b.animando())
    }

    /// Un clic sobre la pantalla de bloqueo, en lógicos. Devuelve lo que haya
    /// que hacer —apagar, reiniciar, suspender— y si hay que repintar.
    pub fn bloqueo_pulsado(&mut self, x: f32, y: f32) -> (Option<bloqueo::Peticion>, bool) {
        let Some((bloqueo, canvas)) = self.bloqueo.as_mut() else {
            return (None, false);
        };
        let pantalla = (canvas.size.width, canvas.size.height);
        let menu_antes = bloqueo.menu;
        let (peticion, cambio_interno) = bloqueo.pulsar(x, y, pantalla);
        (
            peticion,
            cambio_interno || peticion.is_some() || bloqueo.menu != menu_antes,
        )
    }

    pub fn bloqueo_estado(&mut self, escritos: usize, estado: bloqueo::Estado) -> bool {
        let Some((b, canvas)) = self.bloqueo.as_mut() else {
            return false;
        };
        if !b.actualizar_estado(escritos, estado) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    pub fn bloqueo_confirmar_tecla(&mut self, tecla: TeclaPulsada) -> (bool, Option<bloqueo::Peticion>) {
        let Some((b, canvas)) = self.bloqueo.as_mut() else { return (false, None); };
        let resultado = b.confirmar_tecla(tecla);
        if resultado.0 { canvas.painted_once = false; }
        resultado
    }

    pub fn bloqueo_huella_mensaje(&mut self, mensaje: &'static str) {
        if let Some((b, canvas)) = self.bloqueo.as_mut() {
            b.huella_mensaje = mensaje;
            canvas.painted_once = false;
        }
    }

    pub fn bloqueo_caps_lock(&mut self, activo: bool) -> bool {
        let Some((b, canvas)) = self.bloqueo.as_mut() else {
            return false;
        };
        if !b.actualizar_caps_lock(activo) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    /// Entrega al bloqueo la lectura de MPRIS hecha fuera del hilo de dibujo.
    pub fn bloqueo_medio(&mut self, sonando: Option<medios::Sonando>) -> bool {
        let Some((b, canvas)) = self.bloqueo.as_mut() else {
            return false;
        };
        if !b.poner_medio(sonando) {
            return false;
        }
        canvas.painted_once = false;
        true
    }

    /// Sustituye la composición del bloqueo sin reiniciar la sesión.
    ///
    /// Siempre se conserva para el próximo bloqueo; si ya está visible se
    /// invalida también su canvas para que BookOS Settings funcione como un
    /// editor en vivo.
    pub fn aplicar_bloqueo_config(&mut self, config: ConfigBloqueo) -> bool {
        self.bloqueo_config = config;
        let Some((bloqueo, canvas)) = self.bloqueo.as_mut() else {
            return false;
        };
        bloqueo.aplicar_config(config);
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

    /// Qué aplicación hay en ese punto del dock: `(app_id, exec, icono)`.
    ///
    /// El launchpad no cuenta: no se saca del dock ni se ancla, así que quien
    /// pregunte por él recibe `None` y trata el clic como lo que es, abrir.
    pub fn dock_item_en(&self, x: f32, y: f32) -> Option<(String, String, String)> {
        let i = self.dock_items.item_en(x, y)?;
        let item = &self.dock_items.items()[i];
        (item.exec() != config::LAUNCHPAD).then(|| {
            (
                item.app_id().to_string(),
                item.exec().to_string(),
                item.icono_nombre().to_string(),
            )
        })
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

    pub fn menu_ventana(&mut self, opciones: Vec<(String, Accion)>) {
        self.abrir(Emergente::menu_ventana(opciones));
    }

    /// Ancla una aplicación al dock, o la desancla si ya estaba.
    ///
    /// Devuelve la lista de anclados que hay que guardar, para que el
    /// compositor la escriba en `panel.conf`: el shell no toca el disco.
    pub fn anclar(
        &mut self,
        app_id: &str,
        exec: &str,
        icono: &str,
        fijar: Option<bool>,
    ) -> Vec<String> {
        match fijar {
            None => self.dock_items.alternar_anclado(app_id, exec, icono),
            Some(true) => {
                self.dock_items.anclar(app_id, exec, icono);
            }
            Some(false) => {
                self.dock_items.desanclar(app_id);
            }
        }
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

    pub fn dock_tamano(&mut self, tamano: u32) {
        self.dock_items.tamano(tamano);
        let (w, h) = self.dock_items.size();
        self.dock = Canvas::new(Size::new(w, h), self.dock.scale);
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
            renderer_panel,
            theme,
            panel,
            ..
        } = self;
        Self::paint(panel, renderer_panel, theme, view, buf)
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
            renderer_dock,
            theme,
            dock,
            ..
        } = self;
        Self::paint(dock, renderer_dock, theme, view, buf)
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
            tracing::error!(
                w,
                h,
                len = buf.len(),
                "buffer del shell con tamaño incoherente"
            );
            return Vec::new();
        };

        let viewport = Viewport::with_physical_size(Size::new(w, h), canvas.scale);
        let logical = viewport.logical_size();

        let mut ui = UserInterface::build(
            view,
            logical,
            canvas.cache.take().unwrap_or_default(),
            renderer,
        );
        ui.draw(
            renderer,
            theme,
            &Style {
                text_color: view::TEXT(),
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
        renderer.draw(
            &mut pixmap,
            &mut mask,
            &viewport,
            &zonas,
            Color::TRANSPARENT,
        );

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
