//! Puente entre el shell (widgets iced) y el compositor.
//!
//! El shell se dibuja en CPU sobre [`MemoryRenderBuffer`]s, que Smithay sube
//! como texturas y compone con damage tracking igual que cualquier ventana. No
//! hay superficie Wayland, ni cliente, ni proceso aparte: el panel y el dock
//! son elementos más de la escena.
//!
//! # Aislamiento de pánicos
//!
//! Meter el shell dentro del compositor cambia el modelo de fallos: un `unwrap`
//! desafortunado en un widget ya no mata a una applet, mata la sesión entera.
//! Por eso todo lo que entra en el shell pasa por [`ShellHost::guard`], que
//! atrapa el unwind. Si el shell revienta se marca como muerto y **el
//! compositor sigue con las ventanas vivas**: te quedas sin panel ni dock, que
//! es un fallo que se puede mirar y arreglar, no un cierre de sesión.

use std::panic::{catch_unwind, AssertUnwindSafe};

use bookos_shell::{Accion, Ancla, Damage, Shell, TeclaPulsada};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::{ImportMem, Renderer};
use smithay::utils::{Logical, Physical, Point, Rectangle, Size, Transform};

/// Cuál de las dos barras del escritorio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Barra {
    Panel,
    Dock,
}

/// Cómo se comporta una barra ante las ventanas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibilidad {
    /// Siempre a la vista, y las ventanas no le pisan el sitio: el área útil
    /// descuenta lo que ocupa.
    Siempre,
    /// Se aparta cuando una ventana llega a donde está, y vuelve cuando se va.
    /// Es el "esquivar ventanas" de KDE. En este modo **no reserva sitio**: las
    /// ventanas pueden ocupar la pantalla entera, que es justo lo que hace que
    /// haya algo que esquivar.
    Esquivando,
}

/// Lo que hay que recordar por barra.
struct EstadoBarra {
    modo: Visibilidad,
    /// Si hay una ventana donde ella está.
    escondida: bool,
    /// Si el cursor está pegado a su borde, pidiéndola. Gana sobre `escondida`:
    /// llevar el ratón al borde la trae aunque haya una ventana debajo.
    pedida: bool,
    /// Cuándo cambió por última vez, para animar la entrada y la salida.
    desde: std::time::Instant,
    /// Dónde estaba cuando empezó la transición actual.
    ///
    /// Sin esto, interrumpir una animación a medias la hacía **saltar**: sacar
    /// el ratón del borde mientras la barra aún estaba entrando la mandaba de
    /// golpe al otro extremo en vez de devolverla desde donde iba.
    desde_fraccion: f32,
}

impl EstadoBarra {
    fn new() -> Self {
        Self {
            modo: Visibilidad::Siempre,
            escondida: false,
            pedida: false,
            desde: std::time::Instant::now(),
            desde_fraccion: 0.0,
        }
    }

    /// Cuánto de fuera está, de 0 (a la vista) a 1 (escondida del todo).
    ///
    /// Va con la curva de entrada del sistema, la misma que usan las ventanas
    /// al aparecer: una barra que se aparta con otro movimiento distinto del
    /// resto del escritorio se nota en seguida.
    fn fraccion(&self) -> f32 {
        let t = bookos_shell::tema::fraccion(self.desde.elapsed(), OCULTAR);
        let avance = bookos_shell::tema::C_ENTRADA.eval(t);
        let destino = self.destino();
        self.desde_fraccion + (destino - self.desde_fraccion) * avance
    }

    /// Dónde tiene que acabar: 1 fuera, 0 a la vista. Pedida con el cursor
    /// vuelve aunque siga habiendo una ventana debajo.
    fn destino(&self) -> f32 {
        if self.escondida && !self.pedida {
            1.0
        } else {
            0.0
        }
    }

    /// Arranca una transición desde donde esté ahora mismo.
    fn arrancar(&mut self) {
        self.desde_fraccion = self.fraccion();
        self.desde = std::time::Instant::now();
    }

    fn animando(&self) -> bool {
        self.desde.elapsed() < OCULTAR && (self.fraccion() - self.destino()).abs() > 0.001
    }
}

/// Cuánto más se difumina el fondo bajo una emergente a pantalla completa.
///
/// Tres veces el paso del panel. Con 1 —el mismo cristal que la barra— el
/// escritorio seguía reconociéndose detrás de los iconos del launchpad, que es
/// justo lo que no debe pasar cuando lo que hay delante ocupa la pantalla
/// entera. El panel se queda en 1 porque encima suyo hay que leer texto y más
/// difuminado lo deja gris.
const FUERZA_PANTALLA: f32 = 3.0;

/// Hueco entre el panel y las tarjetas que cuelgan de él, en lógicos.
///
/// Ocho es lo que separa una tarjeta flotante de la barra sin que se lea como
/// desconectada de ella. Pegada, la emergente parecía parte del panel.
const SEPARACION: f64 = 8.0;

/// Franja del borde de la pantalla que trae de vuelta a una barra apartada,
/// en píxeles físicos.
///
/// Dos píxeles no bastan —hay que clavar el ratón en el borde— y cincuenta
/// hacen que la barra aparezca sola al pasar cerca. Cinco es lo que usa el
/// auto-ocultar de KDE.
const BORDE_SENSIBLE: f64 = 5.0;

/// Lo que tarda una barra en apartarse o volver.
///
/// Es la duración de tarjeta del sistema. Más lento se siente pesado cuando
/// arrastras una ventana contra el borde; más rápido parece un parpadeo.
const OCULTAR: std::time::Duration = bookos_shell::tema::D_TARJETA;

pub struct ShellHost {
    shell: Shell,
    panel: Surface,
    dock: Surface,
    /// La superficie emergente abierta (menú, calendario, launchpad). Se crea
    /// al abrirla y se destruye al cerrarla: a diferencia del panel y del dock,
    /// su tamaño lo decide su contenido y cambia de una a otra.
    emergente: Option<Surface>,
    /// Página anterior del Launchpad mientras cruza con la nueva. Ambos
    /// buffers están ya rasterizados; la GPU solo cambia posición y alfa.
    pagina_anterior: Option<(Surface, std::time::Instant, f32)>,
    /// El realce de lo señalado dentro de la emergente. Va aparte porque mover
    /// el ratón por el launchpad repintaría el buffer entero: medido, 23 ms
    /// cada celda que se cruza. Como superficie propia solo cambia su posición.
    realce: Option<Surface>,
    /// Cuándo se abrió la emergente, para la animación de entrada.
    abierta_en: Option<std::time::Instant>,
    /// La emergente se está yendo: sus superficies siguen vivas hasta que
    /// termine la animación de salida. Sin esto desaparecía de golpe, que es lo
    /// que hace que un escritorio parezca hecho a trozos.
    cerrando_en: Option<std::time::Instant>,
    /// Tarjeta solicitada mientras otra está saliendo. Se abre al terminar la
    /// salida para que nunca se mezclen dos contenidos en el mismo buffer.
    emergente_pendiente: Option<&'static str>,
    /// De dónde venía el realce y cuándo empezó a moverse, para que se deslice
    /// entre celdas en vez de saltar.
    realce_desde: Option<(Point<f64, Physical>, std::time::Instant)>,
    /// Tamaño de la pantalla en píxeles físicos, para colocar el dock.
    screen: (i32, i32),
    /// Si el panel está a la vista o esquivando ventanas, y por dónde va su
    /// animación.
    panel_barra: EstadoBarra,
    dock_barra: EstadoBarra,
    /// La superficie del bloqueo, cuando está echado. Ocupa la pantalla entera
    /// y va por delante de todo lo demás.
    bloqueo: Option<Surface>,
    /// La capa de captura de pantalla, mientras se elige qué capturar. Ocupa la
    /// pantalla entera, como el bloqueo. Ver [`bookos_shell::captura`].
    captura: Option<Surface>,
    /// La del aviso de volumen y brillo, mientras dura.
    osd: Option<Surface>,
    /// El panel de diagnóstico, mientras esté puesto.
    diagnostico: Option<Surface>,
    /// El aviso de la última notificación, mientras dura.
    toast: Option<Surface>,
    /// Isla superior de música, temporizador o grabación.
    actividad: Option<Surface>,
    /// Se oculta mientras la ventana que la publicó tiene el foco.
    actividad_visible: bool,
    /// Las barras de título, por ventana. Van aquí y no en el shell porque lo
    /// que se guarda es la textura ya subida; el dibujo lo tiene él.
    barras: std::collections::HashMap<u64, Surface>,
    /// La del conmutador de Alt+Tab, mientras el modificador siga pulsado.
    conmutador: Option<Surface>,
    /// Una superficie por icono del escritorio. Van detrás de las ventanas, así
    /// que se emiten aparte del resto del shell — ver
    /// [`ShellHost::elementos_escritorio`].
    escritorio: Vec<Surface>,
    /// La banda elástica mientras se arrastra sobre el escritorio, en píxeles
    /// **lógicos** y ya normalizada: `(x, y, ancho, alto)`.
    banda: Option<(f64, f64, f64, f64)>,
    /// Cuando el shell hace panic se desactiva para siempre. Reintentar cada
    /// frame solo llenaría el log y repetiría el fallo.
    dead: bool,
}

struct Surface {
    buffer: MemoryRenderBuffer,
    /// Posición en píxeles **físicos**: es lo que espera el elemento de render.
    location: Point<f64, Physical>,
    /// Tamaño en píxeles **lógicos**: a qué tamaño se dibuja en el escritorio.
    ///
    /// El buffer se rasteriza a resolución física para que se vea nítido, pero
    /// `MemoryRenderBuffer` solo admite escalas de buffer enteras, así que con
    /// escala fraccional (1,75 en este portátil) no puede deducir el tamaño
    /// lógico él solo. Se le pasan las dos cosas al elemento:
    ///
    /// - `src`  = el buffer **entero**, en sus propias coordenadas.
    /// - `size` = este tamaño lógico.
    ///
    /// Pasar solo `size` no vale: `src` toma por defecto el valor de `size`, y
    /// entonces recorta la esquina superior izquierda del buffer y la estira
    /// por 1,75 — que es exactamente el shell gigante y cortado que salía.
    logical: Size<i32, Logical>,
    /// El buffer completo, para usarlo como `src`.
    src: Rectangle<f64, Logical>,
    /// Solo para el realce: si está dibujado con marco de acento. Guardarlo
    /// evita repintarlo cuando lo único que cambia es dónde está.
    marco: bool,
}

impl Surface {
    /// Alto del buffer en físicos, que es cuánto tiene que recorrer la barra
    /// para desaparecer por su borde.
    fn buffer_alto(&self) -> f64 {
        self.src.size.h
    }

    fn new(size: (u32, u32), logical: (i32, i32)) -> Self {
        Self {
            // Argb8888, o sea bytes B,G,R,A en little-endian: es lo que hay de
            // verdad en el buffer al salir de iced. Aquí ponía Abgr8888 por el
            // razonamiento de que tiny-skia escribe R,G,B,A, y es falso —
            // medido, el píxel del acento sale [250,143,89,255] y el color es
            // (89,143,250). Se veía en pantalla: el nombre naranja en vez de
            // azul y el zorro de Firefox azul en vez de naranja.
            buffer: MemoryRenderBuffer::new(
                Fourcc::Argb8888,
                (size.0 as i32, size.1 as i32),
                1,
                Transform::Normal,
                None,
            ),
            location: (0.0, 0.0).into(),
            logical: logical.into(),
            src: Rectangle::from_size((size.0 as f64, size.1 as f64).into()),
            marco: false,
        }
    }
}

impl ShellHost {
    /// `width`/`height` son píxeles **físicos** de la pantalla; el shell se
    /// dimensiona en lógicos dividiendo por la escala.
    pub fn new(width: u32, height: u32, scale: f32, config: Option<bookos_shell::Config>) -> Self {
        let logical_width = (width as f32 / scale).round().max(1.0) as u32;
        // Sin configuración se lee del disco, que es lo que hacía siempre: los
        // tests construyen el shell sin pasar nada.
        let shell = match config {
            Some(config) => Shell::con_config(logical_width, scale, config),
            None => Shell::new(logical_width, scale),
        };
        let mut host = Self {
            panel: Surface::new(shell.panel_buffer_size(), shell.panel_logical_size()),
            dock: Surface::new(shell.dock_buffer_size(), shell.dock_logical_size()),
            shell,
            emergente: None,
            pagina_anterior: None,
            realce: None,
            abierta_en: None,
            cerrando_en: None,
            emergente_pendiente: None,
            realce_desde: None,
            screen: (width as i32, height as i32),
            panel_barra: EstadoBarra::new(),
            dock_barra: EstadoBarra::new(),
            bloqueo: None,
            captura: None,
            osd: None,
            diagnostico: None,
            toast: None,
            actividad: None,
            actividad_visible: true,
            conmutador: None,
            barras: std::collections::HashMap::new(),
            escritorio: Vec::new(),
            banda: None,
            dead: false,
        };
        host.place_dock();
        host.escritorio_recolocar();
        host
    }

    /// El dock va centrado abajo. Se recalcula al cambiar el tamaño de pantalla.
    /// Pone el modo de una barra y devuelve el que queda.
    pub fn alternar_visibilidad(&mut self, cual: Barra) -> Visibilidad {
        let barra = self.barra_mut(cual);
        barra.modo = match barra.modo {
            Visibilidad::Siempre => Visibilidad::Esquivando,
            Visibilidad::Esquivando => Visibilidad::Siempre,
        };
        // Al volver a "siempre visible" se sale del escondite de inmediato: si
        // no, la barra se quedaría fuera hasta que alguien moviera una ventana.
        if barra.modo == Visibilidad::Siempre && barra.escondida {
            barra.arrancar();
            barra.escondida = false;
        }
        let modo = barra.modo;
        // El modo cambia cuándo se aparta, no su lenguaje visual: el dock se
        // mantiene flotante en ambos casos.
        if cual == Barra::Dock {
            self.place_dock();
        }
        modo
    }

    pub fn visibilidad(&self, cual: Barra) -> Visibilidad {
        self.barra(cual).modo
    }

    /// Le dice a una barra si el cursor la está pidiendo: pegado a su borde de
    /// la pantalla. Devuelve `true` si eso cambia lo que se ve.
    ///
    /// Es lo que hace usable el modo esquivar: la barra se aparta cuando
    /// estorba, pero vuelve en cuanto llevas el ratón al borde, sin tener que
    /// mover la ventana de sitio.
    pub fn reclamada(&mut self, cual: Barra, cursor: (f64, f64)) -> bool {
        let alto = self.screen.1 as f64;
        // Hasta dónde llega la barra desde su borde, en físicos: lo que hay que
        // recorrer con el ratón para llegar a sus iconos.
        let limite = match cual {
            Barra::Panel => self.panel.src.size.h,
            Barra::Dock => alto - self.dock.location.y,
        }
        .max(BORDE_SENSIBLE);
        let barra = self.barra_mut(cual);
        if barra.modo != Visibilidad::Esquivando {
            return false;
        }
        // Dos zonas distintas, y esa es la clave de que se pueda usar:
        //
        // - Para **traerla**, la franja del borde de la pantalla: cuando está
        //   apartada, su sitio ya no está bajo el cursor.
        // - Para **mantenerla**, la barra entera. Sin esto era inservible: la
        //   traías al borde, subías el ratón para pulsar un icono y al salir de
        //   los cinco píxeles se volvía a esconder antes de llegar.
        let zona = if barra.pedida { limite } else { BORDE_SENSIBLE };
        let pedida = match cual {
            Barra::Panel => cursor.1 <= zona,
            Barra::Dock => cursor.1 >= alto - zona,
        };
        if barra.pedida == pedida {
            return false;
        }
        barra.arrancar();
        barra.pedida = pedida;
        true
    }

    /// Le dice a una barra si ahora mismo hay una ventana donde ella está.
    ///
    /// Devuelve `true` si eso cambia lo que se ve, o sea si hay que repintar.
    /// En modo "siempre visible" no hace nada: la ventana pasa por debajo.
    pub fn estorbada(&mut self, cual: Barra, hay_ventana: bool) -> bool {
        let barra = self.barra_mut(cual);
        if barra.modo != Visibilidad::Esquivando || barra.escondida == hay_ventana {
            return false;
        }
        // Con el cursor en el borde el destino no cambia —sigue a la vista—, así
        // que arrancar la transición no la hace parpadear: acaba donde estaba.
        barra.arrancar();
        barra.escondida = hay_ventana;
        true
    }

    /// ¿Sigue moviéndose alguna barra? Mientras sí, hay que repintar.
    pub fn barras_animando(&self) -> bool {
        [&self.panel_barra, &self.dock_barra]
            .iter()
            .any(|b| b.animando())
    }

    fn barra(&self, cual: Barra) -> &EstadoBarra {
        match cual {
            Barra::Panel => &self.panel_barra,
            Barra::Dock => &self.dock_barra,
        }
    }

    fn barra_mut(&mut self, cual: Barra) -> &mut EstadoBarra {
        match cual {
            Barra::Panel => &mut self.panel_barra,
            Barra::Dock => &mut self.dock_barra,
        }
    }

    /// El sitio que ocupa cada barra, en **lógicos**, para saber si una ventana
    /// llega hasta ella. Solo las que esquivan: las fijas no tienen que
    /// enterarse de nada.
    pub fn zonas_barras(
        &self,
    ) -> Vec<(
        Barra,
        smithay::utils::Rectangle<i32, smithay::utils::Logical>,
    )> {
        use smithay::utils::Rectangle;
        if self.dead {
            return Vec::new();
        }
        let escala = self.shell.scale() as f64;
        let mut zonas = Vec::new();
        if self.panel_barra.modo == Visibilidad::Esquivando {
            let (pw, _) = self.shell.panel_buffer_size();
            zonas.push((
                Barra::Panel,
                Rectangle::new(
                    (0, 0).into(),
                    (
                        (pw as f64 / escala).round() as i32,
                        self.shell.panel_height(),
                    )
                        .into(),
                ),
            ));
        }
        if self.dock_barra.modo == Visibilidad::Esquivando {
            let (x, y, w, h) = self.dock_rect();
            zonas.push((
                Barra::Dock,
                Rectangle::new(
                    (x.round() as i32, y.round() as i32).into(),
                    (w.round() as i32, h.round() as i32).into(),
                ),
            ));
        }
        zonas
    }

    /// ¿Está esta barra donde se puede pulsar? Se cuenta como visible mientras
    /// no se haya apartado más de la mitad: durante la animación sigue
    /// respondiendo, que es menos raro que perder el clic a media transición.
    pub fn a_la_vista(&self, cual: Barra) -> bool {
        self.barra(cual).fraccion() < 0.5
    }

    /// Cuánto está desplazada una barra fuera de su sitio, en físicos.
    ///
    /// Positivo es "hacia fuera de la pantalla": el panel se va hacia arriba y
    /// el dock hacia abajo, cada uno por su borde.
    fn desplazamiento(&self, cual: Barra) -> f64 {
        let barra = self.barra(cual);
        let recorrido = match cual {
            Barra::Panel => self.panel.buffer_alto(),
            Barra::Dock => {
                self.dock.buffer_alto()
                    + bookos_shell::DOCK_MARGIN as f64 * self.shell.scale() as f64
            }
        };
        barra.fraccion() as f64 * recorrido
    }

    fn place_dock(&mut self) {
        // Siempre flotante. Antes el modo fijo lo pegaba al canto inferior,
        // eliminaba sus esquinas de abajo y parecía una barra del sistema en
        // vez de un dock. La visibilidad solo decide si puede apartarse.
        if self.guard("dock flotante", |host| host.shell.dock_pegado(false)) == Some(true) {
            self.guard("pintar dock", |host| {
                let buffer = &mut host.dock.buffer;
                paint(buffer, |buf| host.shell.draw_dock(buf));
            });
        }
        let (dw, dh) = self.shell.dock_buffer_size();
        let margin = bookos_shell::DOCK_MARGIN as f64 * self.shell.scale() as f64;
        let x = ((self.screen.0 - dw as i32) as f64 / 2.0).max(0.0);
        let y = (self.screen.1 - dh as i32) as f64 - margin;
        self.dock.location = (x, y.max(0.0)).into();
    }

    // --- Iconos del escritorio ---------------------------------------------

    /// Rehace la rejilla del escritorio y repinta todas sus celdas.
    ///
    /// Se llama al arrancar y al cambiar la pantalla de tamaño o de escala, no
    /// en cada frame: rasterizar una celda cuesta lo suyo y aquí se rasterizan
    /// todas.
    pub fn escritorio_recolocar(&mut self) {
        if self.dead {
            return;
        }
        let escala = self.shell.scale() as f64;
        let logica = (
            self.screen.0 as f32 / escala as f32,
            self.screen.1 as f32 / escala as f32,
        );
        let escala_f32 = self.shell.scale();
        self.guard("recolocar el escritorio", |host| {
            host.shell.escritorio_pantalla(logica, escala_f32);
            let cuantos = host.shell.escritorio().cuantos();
            let buffer = host.shell.escritorio_buffer_size();
            let (lw, lh) = host.shell.escritorio_logical_size();
            host.escritorio = (0..cuantos)
                .map(|_| Surface::new(buffer, (lw as i32, lh as i32)))
                .collect();
            for i in 0..cuantos {
                host.pintar_celda(i);
            }
            host.escritorio_colocar();
        });
    }

    /// Pone cada superficie donde diga la rejilla. Es lo único que hace falta
    /// mientras se arrastra: los buffers ya están pintados y solo cambian de
    /// sitio, que es trabajo de la GPU.
    fn escritorio_colocar(&mut self) {
        let escala = self.shell.scale() as f64;
        for i in 0..self.escritorio.len() {
            let Some((x, y, _, _)) = self.shell.escritorio().rect(i) else {
                continue;
            };
            self.escritorio[i].location = (x as f64 * escala, y as f64 * escala).into();
        }
    }

    fn pintar_celda(&mut self, i: usize) {
        let Self {
            escritorio, shell, ..
        } = self;
        if let Some(surface) = escritorio.get_mut(i) {
            paint(&mut surface.buffer, |buf| shell.draw_escritorio(i, buf));
        }
    }

    /// Cambia la selección y repinta **solo** las celdas que cambiaron de
    /// estado. Es lo que hace que barrer con la banda no cueste la pantalla
    /// entera: por cada movimiento del ratón se rasterizan las una o dos celdas
    /// que el borde acaba de cruzar.
    fn escritorio_cambiar(
        &mut self,
        f: impl FnOnce(&mut bookos_shell::escritorio::Escritorio) -> bool,
    ) -> bool {
        if self.dead {
            return false;
        }
        self.guard("selección del escritorio", |host| {
            let antes = host.shell.escritorio().seleccion();
            if !f(host.shell.escritorio_mut()) {
                return false;
            }
            let despues = host.shell.escritorio().seleccion();
            for i in 0..despues.len() {
                if antes.get(i) != despues.get(i) {
                    host.pintar_celda(i);
                }
            }
            true
        })
        .unwrap_or(false)
    }

    /// Qué icono hay en un punto lógico.
    pub fn escritorio_en(&self, x: f64, y: f64) -> Option<usize> {
        if self.dead {
            return None;
        }
        self.shell.escritorio().en(x as f32, y as f32)
    }

    pub fn escritorio_seleccionar(&mut self, i: Option<usize>, aditivo: bool) -> bool {
        self.escritorio_cambiar(|e| e.seleccionar(i, aditivo))
    }

    pub fn escritorio_seleccion(&self) -> Vec<bool> {
        if self.dead {
            return Vec::new();
        }
        self.shell.escritorio().seleccion()
    }

    /// Pone la banda elástica —en lógicos y ya normalizada— y marca lo que
    /// cruza. `previa` es la selección de antes de empezar a arrastrar.
    ///
    /// No dice si hay que repintar porque la respuesta es siempre que sí: la
    /// banda cambia de tamaño con cada movimiento, y es parte de la escena
    /// aunque no toque ningún icono.
    pub fn escritorio_banda(&mut self, rect: (f64, f64, f64, f64), previa: &[bool]) {
        self.banda = Some(rect);
        let rect = (rect.0 as f32, rect.1 as f32, rect.2 as f32, rect.3 as f32);
        self.escritorio_cambiar(|e| e.banda(rect, previa));
    }

    pub fn escritorio_quitar_banda(&mut self) -> bool {
        self.banda.take().is_some()
    }

    /// Corre los iconos seleccionados mientras se arrastran.
    pub fn escritorio_arrastrar(&mut self, delta: (f64, f64)) {
        if self.dead {
            return;
        }
        self.guard("arrastrar iconos", |host| {
            host.shell
                .escritorio_mut()
                .arrastrar((delta.0 as f32, delta.1 as f32));
            host.escritorio_colocar();
        });
    }

    pub fn escritorio_arrastrando(&self) -> bool {
        !self.dead && self.shell.escritorio().arrastrando()
    }

    /// Deja los iconos en su celda nueva y guarda las posiciones.
    pub fn escritorio_soltar(&mut self) -> bool {
        if self.dead {
            return false;
        }
        self.guard("soltar iconos", |host| {
            let movidos = host.shell.escritorio_mut().soltar();
            host.escritorio_colocar();
            movidos
        })
        .unwrap_or(false)
    }

    pub fn escritorio_abrir(&self, i: usize) -> Option<Accion> {
        if self.dead {
            return None;
        }
        self.shell.escritorio().abrir(i)
    }

    /// Los iconos del escritorio, para dibujarlos **detrás de las ventanas** y
    /// delante del fondo. Por eso no salen por [`ShellHost::elements`], que es
    /// lo que va por encima de todo.
    pub fn elementos_escritorio<R>(
        &self,
        renderer: &mut R,
    ) -> Vec<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        if self.dead {
            return Vec::new();
        }
        self.escritorio
            .iter()
            .filter_map(|surface| {
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    surface.location,
                    &surface.buffer,
                    None,
                    Some(surface.src),
                    Some(surface.logical),
                    Kind::Unspecified,
                )
                .inspect_err(|_| tracing::warn!("no se pudo subir un icono del escritorio"))
                .ok()
            })
            .collect()
    }

    /// La banda elástica: relleno y cuatro filos.
    ///
    /// Cinco rectángulos de color y no un buffer propio, por lo mismo que el
    /// velo del launchpad: la banda cambia de tamaño en **cada** movimiento del
    /// ratón, y rasterizarla en CPU sería repintar un buffer de medio millón de
    /// píxeles por evento. Así solo cambia la geometría que se le pasa a la GPU.
    pub fn banda_elementos(
        &self,
    ) -> Vec<smithay::backend::renderer::element::solid::SolidColorRenderElement> {
        use smithay::backend::renderer::element::solid::SolidColorRenderElement;
        use smithay::backend::renderer::element::Id;

        let Some((x, y, w, h)) = self.banda else {
            return Vec::new();
        };
        if self.dead || w < 1.0 || h < 1.0 {
            return Vec::new();
        }
        // Los identificadores tienen que ser los mismos en cada frame: uno
        // nuevo por frame le diría al damage tracker que cambia toda la
        // pantalla, no solo donde estuvo la banda.
        static IDS: std::sync::OnceLock<[Id; 5]> = std::sync::OnceLock::new();
        let ids = IDS.get_or_init(|| std::array::from_fn(|_| Id::new()));

        let escala = self.shell.scale() as f64;
        let acento = bookos_shell::tema::acento();
        let fisico = |v: f64| (v * escala).round() as i32;
        let (x, y, w, h) = (fisico(x), fisico(y), fisico(w), fisico(h));
        // Un píxel lógico de filo: a escala 2 son dos físicos, que es lo que
        // hace que el borde se vea igual de fino en cualquier pantalla.
        let filo = escala.round().max(1.0) as i32;
        let rectangulos = [
            (Rectangle::new((x, y).into(), (w, h).into()), 0.14),
            (Rectangle::new((x, y).into(), (w, filo).into()), 0.9),
            (
                Rectangle::new((x, y + h - filo).into(), (w, filo).into()),
                0.9,
            ),
            (Rectangle::new((x, y).into(), (filo, h).into()), 0.9),
            (
                Rectangle::new((x + w - filo, y).into(), (filo, h).into()),
                0.9,
            ),
        ];
        ids.iter()
            .zip(rectangulos)
            .map(|(id, (rect, alfa))| {
                SolidColorRenderElement::new(
                    id.clone(),
                    rect,
                    0,
                    // Premultiplicado, que es lo que espera el renderer.
                    [
                        acento.r * alfa,
                        acento.g * alfa,
                        acento.b * alfa,
                        alfa,
                    ],
                    Kind::Unspecified,
                )
            })
            .collect()
    }

    /// Cuánto falta para que algún widget tenga algo nuevo que enseñar.
    /// `None` = ninguno necesita despertarse solo.
    pub fn next_tick(&self) -> Option<std::time::Duration> {
        if self.dead {
            return None;
        }
        self.shell.next_tick()
    }

    /// Subsistemas de udev que hay que vigilar, según lo que pidan los widgets.
    pub fn subsistemas(&self) -> Vec<&'static str> {
        if self.dead {
            return Vec::new();
        }
        self.shell.subsistemas()
    }

    /// Alto del panel: lo que hay que descontarle al área utilizable por las
    /// ventanas.
    pub fn panel_height(&self) -> i32 {
        if self.dead {
            0
        } else {
            self.shell.panel_height()
        }
    }

    /// El rectángulo lógico de la pantalla principal, que es donde vive **todo**
    /// el shell: panel, dock, emergentes, bloqueo, OSD y avisos.
    ///
    /// Su origen es (0,0) porque `pantallas::normalizar` lleva ahí el de la
    /// principal, y por eso una pantalla colocada a la izquierda o arriba usa
    /// coordenadas negativas. Quien pregunte por un punto tiene que acotarlo
    /// contra esto antes de creerse nada: los puntos que llegan son del
    /// escritorio entero, no de este monitor.
    pub fn area_principal(&self) -> (f64, f64) {
        let escala = self.shell.scale() as f64;
        (self.screen.0 as f64 / escala, self.screen.1 as f64 / escala)
    }

    /// La escala de la pantalla principal. **No** es la de la salida bajo el
    /// puntero: lo que se mide contra las superficies del shell tiene que ir en
    /// la escala en la que están dibujadas, y están dibujadas en la principal.
    pub fn escala_principal(&self) -> f64 {
        self.shell.scale() as f64
    }

    /// ¿Llega el escritorio hasta este punto lógico **global**? Es
    /// [`Self::en_la_principal`] con nombre de lo que pregunta quien la usa: el
    /// escritorio ocupa la principal entera, iconos y hueco vacío incluidos.
    pub fn escritorio_alcanza(&self, x: f64, y: f64) -> bool {
        !self.dead && self.en_la_principal(x, y)
    }

    /// ¿Cae este punto lógico **global** dentro de la pantalla principal?
    fn en_la_principal(&self, x: f64, y: f64) -> bool {
        let (w, h) = self.area_principal();
        x >= 0.0 && y >= 0.0 && x < w && y < h
    }

    /// El dock en coordenadas **lógicas**: `(x, y, ancho, alto)`.
    ///
    /// `Surface::location` está en físicos porque es lo que quiere el elemento
    /// de render, pero el hit-testing del puntero ocurre en lógicos. La
    /// división se hace aquí, una vez, y no en cada consulta.
    pub fn dock_rect(&self) -> (f64, f64, f64, f64) {
        let scale = self.shell.scale() as f64;
        let (w, h) = self.shell.dock_logical_size();
        (
            self.dock.location.x / scale,
            self.dock.location.y / scale,
            w as f64,
            h as f64,
        )
    }

    /// El rectángulo del icono del dock de una aplicación, en lógicos.
    ///
    /// Es el destino del minimizar: la ventana se encoge hacia **su** icono, no
    /// hacia el centro del dock. Si la aplicación no tiene icono ahí —no está
    /// anclada ni abierta— devuelve `None` y quien llame elegirá otro sitio.
    pub fn icono_dock(&self, app_id: &str) -> Option<(f64, f64, f64, f64)> {
        let i = self
            .shell
            .dock()
            .items()
            .iter()
            .position(|item| bookos_shell::mismo_programa(item.app_id(), app_id))?;
        let (dx, dy, _, dh) = self.dock_rect();
        let centro = self.shell.dock().centro_de(i) as f64;
        // El lado del icono sale del alto del dock menos su margen: pedirle al
        // shell la constante sería exponer una medida de dibujo por un dato que
        // aquí solo sirve para saber a qué tamaño se encoge la ventana.
        let lado = (dh - 20.0).max(24.0);
        Some((dx + centro - lado / 2.0, dy + (dh - lado) / 2.0, lado, lado))
    }

    /// Las zonas que llevan fondo esmerilado, en píxeles **físicos**: el panel
    /// y el dock.
    ///
    /// En físicos porque el desenfoque copia del framebuffer, que es físico, y
    /// convertir dos veces por frame para volver al mismo sitio solo añade
    /// oportunidades de equivocarse con el redondeo.
    pub fn zonas_cristal(
        &self,
    ) -> Vec<(
        smithay::utils::Rectangle<i32, smithay::utils::Physical>,
        f32,
        f32,
    )> {
        use smithay::utils::Rectangle;
        if self.dead {
            return Vec::new();
        }
        // Con el bloqueo echado no hay cristal: el panel y el dock no se
        // dibujan, pero su fondo esmerilado sí seguía saliendo, y lo que se veía
        // era la silueta redondeada del dock flotando sobre la pantalla de
        // bloqueo sin nada dentro.
        if self.bloqueo.is_some() {
            return Vec::new();
        }
        let escala = self.shell.scale();
        // El launchpad desenfoca la pantalla entera. Va el primero de la lista
        // —o sea, el más al fondo— para que el cristal del panel y el del dock
        // sigan viéndose encima de él.
        if self.shell.emergente_tapa_la_pantalla() {
            return vec![(
                Rectangle::new((0, 0).into(), (self.screen.0, self.screen.1).into()),
                0.0,
                FUERZA_PANTALLA,
            )];
        }
        // **El panel no lleva cristal.** Lo tuvo, y sobre un fondo con degradado
        // como el de serie el desenfoque no aporta nada: la franja es de 32 px
        // y lo que hay detrás ya es liso, así que solo se notaba en que los
        // bordes del wallpaper se emborronaban al pasar bajo la barra.
        //
        // El dock sí lo conserva: flota sobre las ventanas, y ahí el desenfoque
        // es lo que lo separa de lo que tenga debajo.
        let mut zonas = Vec::with_capacity(2);
        // El buscador flota en mitad de la pantalla y ahí debajo hay ventanas:
        // sin cristal, su fondo translúcido deja leer lo que tape. Va antes que
        // el dock para quedar por detrás si llegan a solaparse.
        if self.shell.emergente_usa_cristal() {
            if let Some(s) = self.emergente.as_ref() {
                let (bw, bh) = (s.src.size.w.round() as i32, s.src.size.h.round() as i32);
                zonas.push((
                    Rectangle::new(
                        (s.location.x.round() as i32, s.location.y.round() as i32).into(),
                        (bw, bh).into(),
                    ),
                    // El HIG reserva 6 px de blur para overlays normales. El
                    // radio de la tarjeta y el radio del blur son tokens
                    // distintos: mezclarlos hacía el buscador demasiado
                    // lavado, especialmente a escala 2x.
                    6.0 * escala,
                    1.0,
                ));
            }
        }
        // La actividad tiene un fondo opaco. Aplicarle aquí una zona de blur
        // rectangular dejaba ver el wallpaper azul en sus esquinas
        // transparentes, por fuera del radio de la tarjeta, como si tuviese un
        // contorno azul. Se volverá a activar cuando el blur admita una máscara
        // redondeada con exactamente la misma geometría de la actividad.
        // El cristal se mueve con su barra. Sin esto, al apartarse el dock se
        // iba el buffer con los iconos y quedaba flotando el rectángulo
        // esmerilado en su sitio — se veía en pantalla.
        let (dw, dh) = self.shell.dock_buffer_size();
        let dock_y = (self.dock.location.y + self.desplazamiento(Barra::Dock)).round() as i32;
        if dock_y < self.screen.1 {
            zonas.push((
                Rectangle::new(
                    (self.dock.location.x.round() as i32, dock_y).into(),
                    (dw as i32, dh as i32).into(),
                ),
                bookos_shell::tema::R_TARJETA * escala,
                1.0,
            ));
        }
        zonas
    }

    /// ¿Cae el punto lógico dentro de algo del shell? Si es así el clic es
    /// nuestro y no debe llegar a la ventana de debajo.
    ///
    /// El panel ocupa toda la franja de arriba y el dock flota abajo; ambos se
    /// dibujan **por encima** de las ventanas (ver `backend::escena`), así que
    /// tienen que ganar el hit-test.
    pub fn contiene(&self, x: f64, y: f64) -> bool {
        if self.dead {
            return false;
        }
        // Lo primero, y no un detalle: el shell **solo existe en la principal**,
        // pero el punto que llega es del escritorio entero. Sin acotar aquí, en
        // una pantalla colocada a la izquierda —x negativa— la franja `y < 32`
        // caía dentro del panel y se tragaba los clics, y peor: con cualquier
        // emergente abierta se devolvía `true` para **toda** la pantalla
        // secundaria, así que el ratón dejaba de funcionar ahí hasta cerrarla.
        // El síntoma era «el ratón se vuelve raro en el monitor de al lado».
        if !self.en_la_principal(x, y) {
            return false;
        }
        // El aviso de una notificación se dibuja encima de las ventanas, así
        // que también gana el hit-test mientras está: si no, pulsarlo escribiría
        // en lo que tenga debajo.
        if Self::dentro(self.toast_rect(), x, y).is_some() {
            return true;
        }
        if Self::dentro(self.actividad_rect(), x, y).is_some() {
            return true;
        }
        // Una barra apartada no recibe clics: si no, el borde de la pantalla se
        // tragaría pulsaciones destinadas a la ventana que hay debajo.
        if !self.a_la_vista(Barra::Panel)
            && !self.a_la_vista(Barra::Dock)
            && !self.shell.hay_emergente()
        {
            return false;
        }
        if self.a_la_vista(Barra::Panel) && y < self.shell.panel_height() as f64 {
            return true;
        }
        // Con una emergente abierta, el clic es del shell **aunque caiga
        // fuera** de ella: pulsar fuera la cierra, y ese clic no debe llegar
        // además a la ventana de debajo. Es lo que hace cualquier menú.
        if self.shell.hay_emergente() {
            return true;
        }
        let (dx, dy, dw, dh) = self.dock_rect();
        x >= dx && x < dx + dw && y >= dy && y < dy + dh
    }

    fn dentro(rect: Option<(f64, f64, f64, f64)>, x: f64, y: f64) -> Option<(f32, f32)> {
        let (rx, ry, rw, rh) = rect?;
        (x >= rx && x < rx + rw && y >= ry && y < ry + rh)
            .then(|| ((x - rx) as f32, (y - ry) as f32))
    }

    /// Mueve el puntero sobre el shell.
    ///
    /// Fuera del dock se le pasa `None`, para que el icono señalado se apague
    /// al salir. Repintar solo ocurre si el icono señalado **cambia**: mover el
    /// ratón dentro del mismo icono, o por el resto de la pantalla, no cuesta
    /// nada más que esta comparación.
    pub fn puntero(&mut self, x: f64, y: f64) {
        if self.dead {
            return;
        }

        // `actividad_rect` devuelve solo la tarjeta, sin el aire transparente
        // reservado para su sombra. La lógica de Actividad, igual que la de
        // pulsación, trabaja en coordenadas del buffer completo; se repone el
        // margen antes de entregarle el punto. Sin esto el hover del volumen
        // quedaba desplazado 26 px arriba y a la izquierda.
        let margen_actividad = bookos_shell::actividad::MARGEN_SOMBRA;
        let punto_actividad = Self::dentro(self.actividad_rect(), x, y)
            .map(|(px, py)| (px + margen_actividad, py + margen_actividad));
        let repintar_actividad = self
            .guard("puntero actividad", |host| {
                host.shell.actividad_puntero(punto_actividad)
            })
            .unwrap_or(false);
        if repintar_actividad {
            self.pintar_actividad();
        }

        if self.shell.hay_emergente() {
            // Con algo agarrado —el deslizador del volumen— el puntero le sigue
            // llegando aunque se salga de la tarjeta: al llevar el volumen al
            // máximo de un tirón la mano se sale, y ahí el deslizador tiene que
            // seguir hasta que se suelte el botón.
            let punto = match (self.emergente_rect(), self.shell.emergente_agarrada()) {
                (Some(r), true) => Some(((x - r.0) as f32, (y - r.1) as f32)),
                (rect, _) => Self::dentro(rect, x, y),
            };
            let repintar = self
                .guard("puntero emergente", |host| {
                    host.shell.emergente_puntero(punto)
                })
                .unwrap_or(false);
            if repintar {
                self.sincronizar_emergente();
            }
        }

        let punto = Self::dentro(Some(self.dock_rect()), x, y);
        let repintar = self
            .guard("puntero", |host| host.shell.dock_hover(punto))
            .unwrap_or(false);
        if repintar {
            self.guard("pintar dock", |host| {
                let buffer = &mut host.dock.buffer;
                paint(buffer, |buf| host.shell.draw_dock(buf));
            });
        }
    }

    // --- Superficies emergentes --------------------------------------------

    /// Crea (o rehace) la superficie de la emergente abierta y la pinta.
    ///
    /// Se llama después de cualquier cosa que pueda abrir, cerrar o cambiar una
    /// emergente. Si no hay ninguna, suelta la superficie: un buffer de un
    /// launchpad a pantalla completa son varios megas que no tiene sentido
    /// guardar mientras está cerrado.
    fn sincronizar_emergente(&mut self) {
        let Some(((w, h), ancla, alto_colocacion)) = self.shell.emergente_geometria() else {
            // Cerrándose: las superficies se quedan hasta que acabe la salida.
            if self.cerrando_en.is_none() {
                self.emergente = None;
                self.realce = None;
                self.abierta_en = None;
            }
            return;
        };
        // Abrir algo nuevo cancela la salida de lo anterior.
        self.cerrando_en = None;
        let Some(buffer) = self.shell.emergente_buffer_size() else {
            self.emergente = None;
            return;
        };

        // La superficie se rehace solo cuando cambia el tamaño: rehacerla en
        // cada repintado tiraría la textura que el compositor ya tiene subida.
        // Cambia al abrir otra distinta y también cuando una crece sobre la
        // marcha —«No molestar» despliega sus duraciones—, y esos dos casos no
        // son lo mismo: al crecer **no** se rearranca la animación de entrada,
        // que haría que la tarjeta volviera a aparecer de golpe con cada clic.
        let nueva = self.emergente.is_none();
        let rehacer = nueva
            || self
                .emergente
                .as_ref()
                .is_some_and(|s| s.logical != (w, h).into());
        if rehacer {
            self.emergente = Some(Surface::new(buffer, (w, h)));
            // Se coloca por el alto **estable** cuando la emergente tiene uno:
            // el buscador cambia de alto con cada tecla, y usar el de ahora lo
            // hacía saltar media fila mientras escribes.
            self.colocar_emergente(ancla, (w, alto_colocacion.unwrap_or(h)));
            if nueva {
                self.abierta_en = Some(std::time::Instant::now());
            }
        }

        if self.shell.emergente_needs_paint() {
            self.guard("pintar emergente", |host| {
                let Some(surface) = host.emergente.as_mut() else {
                    return;
                };
                let buffer = &mut surface.buffer;
                paint(buffer, |buf| host.shell.draw_emergente(buf));
            });
        }

        self.sincronizar_realce();
    }

    /// Coloca —y pinta si hace falta— el realce de lo señalado.
    fn sincronizar_realce(&mut self) {
        let Some((rx, ry, rw, rh, marco)) = self.shell.emergente_realce() else {
            self.realce = None;
            return;
        };
        let Some(origen) = self.emergente.as_ref().map(|s| s.location) else {
            self.realce = None;
            return;
        };
        let escala = self.shell.scale();
        let fisico = (
            (rw * escala).round().max(1.0) as u32,
            (rh * escala).round().max(1.0) as u32,
        );

        // Solo se vuelve a pintar si cambia de tamaño o de estilo; recorrer la
        // rejilla con el ratón es mover esta superficie y nada más.
        let rehacer = self
            .realce
            .as_ref()
            .is_none_or(|s| s.logical != (rw as i32, rh as i32).into() || s.marco != marco);
        if rehacer {
            let mut surface = Surface::new(fisico, (rw as i32, rh as i32));
            surface.marco = marco;
            self.realce = Some(surface);
            self.guard("pintar realce", |host| {
                let Some(surface) = host.realce.as_mut() else {
                    return;
                };
                let buffer = &mut surface.buffer;
                paint(buffer, |buf| {
                    host.shell.draw_realce(buf, rw, rh, marco);
                    Vec::new()
                });
            });
        }

        let destino = Point::<f64, Physical>::from((
            origen.x + (rx * escala) as f64,
            origen.y + (ry * escala) as f64,
        ));
        if let Some(surface) = self.realce.as_mut() {
            if surface.location != destino {
                // Se guarda de dónde viene para interpolar. Deslizar el realce
                // es gratis —solo cambia una posición, no se repinta nada— y es
                // la diferencia entre que el resaltado siga al cursor o dé
                // saltos de celda en celda.
                let salida = if rehacer { destino } else { surface.location };
                self.realce_desde = Some((salida, std::time::Instant::now()));
                surface.location = destino;
            }
        }
    }

    /// Dónde está el realce **ahora mismo**, contando el deslizamiento.
    fn realce_location(&self, destino: Point<f64, Physical>) -> Point<f64, Physical> {
        let Some((desde, t0)) = self.realce_desde else {
            return destino;
        };
        use bookos_shell::tema;
        // Curva de hover del sistema, la misma que un cambio de fondo al pasar
        // el cursor: es lo que este deslizamiento es.
        let s = tema::C_SUAVE.eval(tema::fraccion(t0.elapsed(), Self::DESLIZ)) as f64;
        (
            desde.x + (destino.x - desde.x) * s,
            desde.y + (destino.y - desde.y) * s,
        )
            .into()
    }

    /// Traduce el anclaje lógico a la posición física de la superficie.
    fn colocar_emergente(&mut self, ancla: Ancla, (w, h): (i32, i32)) {
        let escala = self.shell.scale() as f64;
        let (x, y) = match ancla {
            // Separadas del panel, no pegadas: son tarjetas que flotan sobre
            // el escritorio, y con el borde tocando la barra se leían como una
            // prolongación de ella en vez de como algo aparte.
            Ancla::BajoElPanel { x } => (x as f64, self.shell.panel_height() as f64 + SEPARACION),
            // Centrada bajo el widget que la abrió. Si ese widget no está en el
            // panel —lo han quitado de la configuración— cae al centro, que es
            // mejor que no salir o salir en la esquina.
            Ancla::BajoWidget(nombre) => {
                let centro = self
                    .shell
                    .zonas_panel()
                    .into_iter()
                    .find(|(n, _, _)| *n == nombre)
                    .map(|(_, x0, x1)| ((x0 + x1) / 2.0) as f64)
                    .unwrap_or(self.screen.0 as f64 / escala / 2.0);
                (
                    centro - w as f64 / 2.0,
                    self.shell.panel_height() as f64 + SEPARACION,
                )
            }
            // Sobre el dock y centrada en el icono, sin salirse por los lados.
            Ancla::SobreElDock { x } => {
                let (dx, dy, _, _) = self.dock_rect();
                let ancho = self.screen.0 as f64 / escala;
                let izq = (dx + x as f64 - w as f64 / 2.0)
                    .clamp(SEPARACION, (ancho - w as f64 - SEPARACION).max(SEPARACION));
                (izq, dy - h as f64 - SEPARACION)
            }
            // Centrada en el **área útil**, no en la pantalla: el panel come
            // 32 px arriba y el dock unos 78 abajo, así que centrar contra el
            // rectángulo entero deja la tarjeta por debajo del centro que se
            // percibe. Con algo más alto que el área útil —el launchpad— la
            // cuenta sale negativa y el recorte de abajo la deja en su sitio.
            Ancla::Centrada => {
                let arriba = self.shell.panel_height() as f64;
                let (_, dock_y, _, _) = self.dock_rect();
                let util = (dock_y - SEPARACION - arriba).max(1.0);
                (
                    (self.screen.0 as f64 / escala - w as f64) / 2.0,
                    arriba + (util - h as f64) / 2.0,
                )
            }
            Ancla::Arriba => (0.0, 0.0),
        };
        // Sin esto, una emergente anclada cerca del borde derecho se saldría de
        // la pantalla y se vería cortada.
        //
        // El recorte de las tarjetas que flotan deja el mismo aire que hay
        // contra el panel, no cero: el calendario cuelga del reloj, que está en
        // la esquina, y pegado al borde exacto se leía como cortado —su sombra
        // cae fuera del buffer y desaparece justo en el lado que toca—. Con
        // margen se ve entero y flotando, que es lo que es.
        //
        // Las que **no** flotan no lo llevan: el launchpad y la franja de
        // escritorios se pegan al borde a propósito, y darles 8 px los
        // descolocaría.
        let margen = match ancla {
            Ancla::Arriba => 0.0,
            _ if self.shell.emergente_tapa_la_pantalla() => 0.0,
            _ => SEPARACION,
        };
        let ancho = self.screen.0 as f64 / escala;
        let alto = self.screen.1 as f64 / escala;
        // Si con margen no cabe, se recorta sin él antes que dejar el mínimo
        // por encima del máximo, que en `clamp` es un pánico.
        let hueco = |disponible: f64, tamano: f64| {
            let max = disponible - tamano - margen;
            if max >= margen { (margen, max) } else { (0.0, (disponible - tamano).max(0.0)) }
        };
        let (min_x, max_x) = hueco(ancho, w as f64);
        let (min_y, max_y) = hueco(alto, h as f64);
        if let Some(surface) = self.emergente.as_mut() {
            surface.location =
                (x.clamp(min_x, max_x) * escala, y.clamp(min_y, max_y) * escala).into();
        }
    }

    /// El rectángulo lógico de la emergente abierta.
    fn emergente_rect(&self) -> Option<(f64, f64, f64, f64)> {
        let surface = self.emergente.as_ref()?;
        let escala = self.shell.scale() as f64;
        Some((
            surface.location.x / escala,
            surface.location.y / escala,
            surface.logical.w as f64,
            surface.logical.h as f64,
        ))
    }

    /// Cierra la emergente que hubiera. `true` si había alguna.
    pub fn cerrar_emergente(&mut self) -> bool {
        if self.dead || !self.shell.hay_emergente() {
            return false;
        }
        self.shell.cerrar_emergente();
        // Las superficies **no** se sueltan aquí: se quedan mientras dura la
        // salida. El realce sí, porque señalar algo que ya se está yendo no
        // significa nada; sin esto, además, se quedaba su recuadro gris
        // flotando en el escritorio para siempre.
        self.realce = None;
        self.realce_desde = None;
        self.pagina_anterior = None;
        self.cerrando_en = Some(std::time::Instant::now());
        true
    }

    /// Se llama cuando ya no hay animación en curso: suelta lo que quedaba.
    pub fn fin_animacion(&mut self) {
        self.realce_desde = None;
        if self
            .pagina_anterior
            .as_ref()
            .is_some_and(|(_, t, _)| t.elapsed() >= bookos_shell::tema::D_PAGINA)
        {
            self.pagina_anterior = None;
        }
        self.recoger_cerrada();
        if self.emergente.is_none() {
            if let Some(nombre) = self.emergente_pendiente.take() {
                self.guard("abrir siguiente tarjeta", |host| {
                    host.shell.abrir_emergente_nombre(nombre)
                });
                self.sincronizar_emergente();
            }
        }
    }

    /// Suelta lo que quede de una emergente que ya terminó de irse.
    fn recoger_cerrada(&mut self) {
        if self
            .cerrando_en
            .is_some_and(|t| t.elapsed() >= Self::SALIDA)
        {
            self.emergente = None;
            self.abierta_en = None;
            self.cerrando_en = None;
        }
    }

    /// Una tecla para la emergente abierta. `true` si la ha consumido, en cuyo
    /// caso **no** debe llegar al cliente con el foco.
    pub fn tecla(&mut self, tecla: TeclaPulsada) -> (bool, Option<Accion>) {
        if self.dead || !self.shell.hay_emergente() {
            return (false, None);
        }
        let (consumida, accion) = self
            .guard("tecla", |host| host.shell.emergente_tecla(tecla))
            .unwrap_or((false, None));
        self.sincronizar_emergente();
        (consumida, accion)
    }

    /// Abre el launchpad, o lo cierra si ya lo estaba.
    /// Abre «Acerca de este PC». Cierra lo que hubiera: se llega desde el menú
    /// del logo, que es una emergente él mismo.
    /// Abre la tarjeta que le corresponde a un widget del panel, por su nombre.
    ///
    /// Lo pide el centro de control: sus tarjetas de Wi-Fi y Bluetooth abren
    /// las listas que ya existen en vez de dibujarlas otra vez dentro.
    pub fn abrir_de_widget(&mut self, widget: &str) {
        if self.dead {
            return;
        }
        self.guard("abrir tarjeta", |host| {
            host.shell.abrir_de_widget(widget);
        });
        self.sincronizar_emergente();
    }

    /// Abre el permiso de compartir pantalla que pide el portal.
    ///
    /// Cierra lo que hubiera abierto: es un diálogo y tiene que verse.
    pub fn abrir_compartir(
        &mut self,
        sesion: u32,
        app: String,
        pantallas: Vec<bookos_shell::PantallaCompartible>,
    ) {
        if self.dead {
            return;
        }
        self.guard("permiso de compartir pantalla", |host| {
            host.shell
                .abrir(bookos_shell::Emergente::compartir(sesion, app, pantallas));
        });
        self.sincronizar_emergente();
    }

    pub fn abrir_acerca(&mut self) {
        if self.dead {
            return;
        }
        self.guard("acerca", |host| {
            host.shell.abrir(bookos_shell::Emergente::acerca());
        });
        self.sincronizar_emergente();
    }

    pub fn alternar_proyeccion(&mut self, conectadas: usize) {
        if self.dead {
            return;
        }
        if self.shell.emergente_nombre() == Some("proyeccion") {
            self.cerrar_emergente();
            return;
        }
        self.guard("selector de proyección", |host| {
            host.shell
                .abrir(bookos_shell::Emergente::proyeccion(conectadas));
        });
        self.sincronizar_emergente();
    }

    pub fn alternar_launchpad(&mut self) {
        if self.dead {
            return;
        }
        if self.shell.hay_emergente() {
            self.cerrar_emergente();
            return;
        }
        // Leer los `.desktop` puede tardar unos milisegundos, así que va dentro
        // del guard: un fichero raro no puede tumbar la sesión.
        let escala = self.shell.scale();
        let pantalla = (self.screen.0 as f32 / escala, self.screen.1 as f32 / escala);
        self.guard("abrir launchpad", |host| {
            host.shell
                .abrir(bookos_shell::Emergente::launchpad(pantalla));
        });
        self.sincronizar_emergente();
    }

    /// Abre o cierra el buscador. Es Meta+Espacio.
    pub fn alternar_buscador(&mut self) {
        if self.dead {
            return;
        }
        // Con cualquier otra emergente abierta, esto la cierra: solo puede
        // haber una, y pedir el buscador con el menú desplegado significa que
        // quieres el buscador.
        if self.shell.hay_emergente() {
            let era_buscador = self.shell.emergente_nombre() == Some("buscador");
            self.cerrar_emergente();
            if era_buscador {
                return;
            }
        }
        // Leer los `.desktop` puede tardar unos milisegundos y va dentro del
        // guard, igual que el launchpad: un fichero raro no tumba la sesión.
        let escala = self.shell.scale();
        let pantalla = (self.screen.0 as f32 / escala, self.screen.1 as f32 / escala);
        self.guard("abrir buscador", |host| {
            host.shell
                .abrir(bookos_shell::Emergente::buscador_en(pantalla));
        });
        self.sincronizar_emergente();
    }

    /// Abre o cierra la vista general de escritorios.
    pub fn alternar_vista_escritorios(&mut self, activo: usize, nombres: Vec<String>) {
        if self.dead {
            return;
        }
        if self.shell.emergente_nombre() == Some("escritorios") {
            self.cerrar_emergente();
            return;
        }
        let escala = self.shell.scale();
        let pantalla = (self.screen.0 as f32 / escala, self.screen.1 as f32 / escala);
        self.guard("abrir vista de escritorios", |host| {
            host.shell.abrir(bookos_shell::Emergente::escritorios(
                pantalla, activo, nombres,
            ));
        });
        self.sincronizar_emergente();
    }

    pub fn actualizar_vista_escritorios(&mut self, activo: usize, nombres: Vec<String>) {
        if self.dead {
            return;
        }
        self.guard("actualizar vista de escritorios", |host| {
            host.shell.actualizar_vista_escritorios(activo, nombres);
        });
        self.sincronizar_emergente();
    }

    pub fn vista_escritorios_abierta(&self) -> bool {
        !self.dead && self.shell.emergente_nombre() == Some("escritorios")
    }

    /// Huecos globales lógicos en los que la escena monta cada escritorio.
    pub fn escritorios_miniaturas(&self) -> Vec<Rectangle<i32, Logical>> {
        let Some(surface) = self.emergente.as_ref() else {
            return Vec::new();
        };
        let escala = self.shell.scale() as f64;
        let origen_x = surface.location.x / escala;
        let origen_y = surface.location.y / escala;
        self.shell
            .escritorios_miniaturas()
            .into_iter()
            .map(|r| {
                Rectangle::new(
                    (
                        (origen_x + r.x as f64).round() as i32,
                        (origen_y + r.y as f64).round() as i32,
                    )
                        .into(),
                    (
                        r.width.round().max(1.0) as i32,
                        r.height.round().max(1.0) as i32,
                    )
                        .into(),
                )
            })
            .collect()
    }

    /// Le dice al dock qué aplicaciones tienen ventana abierta.
    ///
    /// Los `app_id` los reúne el compositor, que es quien conoce el `Space`. El
    /// shell no lo mira por su cuenta: la frontera con Smithay está puesta a
    /// propósito para que cambiar de toolkit no obligue a tocar el compositor.
    /// Le pasa al panel en qué escritorio está la sesión.
    ///
    /// El panel puede cambiar de ancho —el indicador aparece en cuanto se sabe
    /// cuántos escritorios hay—, así que se repinta por el mismo camino que
    /// cualquier otro cambio de sus widgets.
    pub fn escritorios(&mut self, activo: usize, cuantos: usize) {
        if self.dead {
            return;
        }
        let repintar = self
            .guard("escritorios", |host| {
                host.shell.escritorios(activo, cuantos)
            })
            .unwrap_or(false);
        if repintar {
            // La superficie no se rehace: el panel ocupa el ancho de la
            // pantalla y eso no cambia porque aparezca un widget. Lo que cambia
            // es su contenido y el reparto de las zonas de clic, que sale del
            // ancho de cada widget en el siguiente `zonas_panel`.
            self.guard("pintar panel", |host| {
                let buffer = &mut host.panel.buffer;
                paint(buffer, |buf| host.shell.draw_panel(buf));
            });
        }
    }

    pub fn ventanas(&mut self, app_ids: &[String]) -> bool {
        if self.dead {
            return false;
        }
        let repintar = self
            .guard("ventanas", |host| host.shell.dock_ventanas(app_ids))
            .unwrap_or(false);
        if repintar {
            // Recolocar antes de pintar: el dock pudo cambiar de ancho al
            // aparecer el icono de una aplicación abierta sin lanzador.
            self.dock = Surface::new(
                self.shell.dock_buffer_size(),
                self.shell.dock_logical_size(),
            );
            self.place_dock();
            self.guard("pintar dock", |host| {
                let buffer = &mut host.dock.buffer;
                paint(buffer, |buf| host.shell.draw_dock(buf));
            });
        }
        repintar
    }

    /// Desplazamiento de rueda o touchpad sobre el shell.
    ///
    /// Devuelve `true` si lo ha consumido, en cuyo caso **no** debe llegar al
    /// cliente: desplazarse por el launchpad no puede además hacer scroll en la
    /// ventana que hay debajo.
    pub fn desplazar(&mut self, dx: f64, dy: f64) -> bool {
        if self.dead || !self.shell.hay_emergente() {
            return false;
        }
        let repintar = self
            .guard("desplazar", |host| {
                host.shell.emergente_desplazar(dx as f32, dy as f32)
            })
            .unwrap_or(false);
        if repintar {
            let delta = if dx.abs() >= dy.abs() { dx } else { dy };
            let hacia = if delta >= 0.0 { 1.0 } else { -1.0 };
            if self.shell.emergente_nombre() == Some("launchpad") {
                if let Some(anterior) = self.emergente.take() {
                    self.pagina_anterior = Some((anterior, std::time::Instant::now(), hacia));
                }
            }
            self.sincronizar_emergente();
            // La superficie nueva es otra página de la misma emergente: no
            // debe sumar el zoom de apertura al deslizamiento lateral.
            self.abierta_en = None;
        }
        // Consumido aunque no haya cambiado de página: el gesto es del
        // launchpad de principio a fin.
        true
    }

    /// Botón derecho dentro del Launchpad: entra o sale de edición.
    pub fn editar_launchpad(&mut self) -> bool {
        if self.dead {
            return false;
        }
        let cambio = self
            .guard("editar launchpad", |host| host.shell.launchpad_editar())
            .unwrap_or(false);
        if cambio {
            self.sincronizar_emergente();
        }
        cambio
    }

    /// Qué hacer al pulsar en un punto lógico. `None` si ahí no hay nada.
    pub fn pulsar(&mut self, x: f64, y: f64) -> Option<Accion> {
        if self.dead {
            return None;
        }

        // El aviso de una notificación gana a todo lo que tenga debajo: está
        // encima de las ventanas, y un clic que lo atravesara escribiría en una
        // aplicación que el usuario ni siquiera está mirando.
        if self.toast_pulsado(x, y) {
            return None;
        }

        if let Some((ax, ay)) = Self::dentro(self.actividad_rect(), x, y) {
            let margen = bookos_shell::actividad::MARGEN_SOMBRA;
            let accion = self
                .guard("pulsar actividad", |host| {
                    host.shell.actividad_pulsar(ax + margen, ay + margen)
                })
                .flatten()
                .map(Accion::Actividad);
            self.pintar_actividad();
            return accion;
        }

        // El panel gana al «pulsar fuera cierra» de la emergente. Con una
        // tarjeta abierta, un clic en otro widget caía fuera de ella y se
        // gastaba solo en cerrarla: había que pulsar dos veces para cambiar de
        // tarjeta, que es justo lo que hacía que abrir los widgets «costara».
        // Una barra de menús pasa de un menú al siguiente con un clic, y
        // `panel_pulsado` ya sabe alternar cuando se repite el mismo widget.
        // Salvo cuando lo que hay abierto tapa la pantalla entera: el
        // launchpad se dibuja **encima** del panel, y ahí arriba lo que hay es
        // launchpad, no widgets.
        if y < self.shell.panel_height() as f64 && !self.shell.emergente_tapa_la_pantalla() {
            let objetivo = self.shell.objetivo_emergente_panel(x as f32);
            let abierta = self.shell.emergente_nombre();
            if let (Some(abierta), Some((widget, nombre_objetivo))) = (abierta, objetivo) {
                if abierta == nombre_objetivo {
                    // Segundo clic en el mismo widget: cerrar con la animación
                    // del host, no destruyendo el buffer de golpe en el shell.
                    self.cerrar_emergente();
                    return None;
                }
                self.emergente_pendiente = Some(widget);
                self.cerrar_emergente();
                return None;
            }
            let accion = self
                .guard("pulsar panel", |host| {
                    host.shell.panel_pulsado(x as f32, y as f32)
                })
                .flatten();
            self.sincronizar_emergente();
            return accion;
        }

        if self.shell.hay_emergente() {
            match Self::dentro(self.emergente_rect(), x, y) {
                Some((ex, ey)) => {
                    let accion = self
                        .guard("pulsar emergente", |host| {
                            host.shell.emergente_pulsar(ex, ey)
                        })
                        .flatten();
                    self.sincronizar_emergente();
                    return accion;
                }
                // Fuera de la emergente: se cierra y el clic muere ahí. Que
                // además activase lo que hubiera debajo sería una sorpresa
                // desagradable — cerrar un menú no es pulsar el escritorio.
                None => {
                    self.cerrar_emergente();
                    return None;
                }
            }
        }

        let (ex, ey) = Self::dentro(Some(self.dock_rect()), x, y)?;
        self.guard("pulsar", |host| host.shell.dock_pulsar(ex, ey))
            .flatten()
    }

    /// Enseña el aviso de volumen, brillo y demás, centrado sobre el dock.
    pub fn mostrar_osd(&mut self, icono: &str, nivel: Option<u8>, texto: Option<String>) {
        self.guard("osd", |host| host.shell.mostrar_osd(icono, nivel, texto));
        self.pintar_osd();
    }

    /// Aplica el tema y el acento elegidos en Apariencia, y repinta el shell.
    ///
    /// Devuelve `false` si el shell está caído: entonces no hay nada que
    /// guardar tampoco, porque el usuario no ha podido pulsar nada.
    pub fn aplicar_apariencia(
        &mut self,
        tema: bookos_shell::tema::Tema,
        acento: bookos_shell::tema::Acento,
    ) -> bool {
        if self.dead {
            return false;
        }
        let hecho = self
            .guard("apariencia", |host| {
                host.shell.aplicar_apariencia(tema, acento);
                let buffer = &mut host.panel.buffer;
                paint(buffer, |buf| host.shell.draw_panel(buf));
                let buffer = &mut host.dock.buffer;
                paint(buffer, |buf| host.shell.draw_dock(buf));
                // Y los iconos del escritorio, que llevan el acento en la
                // pastilla del nombre y en el realce de lo seleccionado.
                for i in 0..host.escritorio.len() {
                    host.pintar_celda(i);
                }
            })
            .is_some();
        // La emergente va aparte: su superficie se rehace, no se repinta en un
        // buffer que ya está puesto.
        if hecho {
            self.sincronizar_emergente();
        }
        hecho
    }

    /// Recarga la composición del bloqueo guardada por BookOS Settings.
    pub fn aplicar_bloqueo_config(&mut self, config: bookos_shell::ConfigBloqueo) -> bool {
        if self.dead {
            return false;
        }
        let visible = self
            .guard("ajustes del bloqueo", |host| {
                host.shell.aplicar_bloqueo_config(config)
            })
            .unwrap_or(false);
        if visible {
            self.guard("pintar bloqueo", |host| {
                let Some(surface) = host.bloqueo.as_mut() else {
                    return;
                };
                let buffer = &mut surface.buffer;
                paint(buffer, |buf| host.shell.draw_bloqueo(buf));
            });
        }
        true
    }

    pub fn aplicar_actividades_config(&mut self, config: bookos_shell::ConfigActividades) -> bool {
        if self.dead {
            return false;
        }
        self.guard("ajustes de actividades", |host| {
            host.shell.aplicar_actividades_config(config)
        });
        self.pintar_actividad();
        true
    }

    /// Repinta la emergente si su contenido se está moviendo.
    ///
    /// Lo llama el compositor antes de componer cada frame: la transición de
    /// página del launchpad ocurre **dentro** del buffer, así que nadie más lo
    /// marcaría sucio.
    pub fn animar_emergente(&mut self) {
        if self.dead || !self.shell.emergente_animando() {
            return;
        }
        self.sincronizar_emergente();
    }

    // --- Actividades vivas -------------------------------------------------

    pub fn publicar_actividad(&mut self, estado: bookos_shell::actividad::Estado) {
        self.guard("publicar actividad", |host| {
            host.shell.publicar_actividad(estado)
        });
        self.actividad_visible = true;
        self.pintar_actividad();
    }

    pub fn cerrar_actividad(&mut self, app_id: &str) -> bool {
        let cerrada = self.guard("cerrar actividad", |host| {
            host.shell.cerrar_actividad(app_id)
        }) == Some(true);
        if cerrada {
            self.pintar_actividad();
        }
        cerrada
    }

    pub fn abrir_actividad_previsualizacion(&mut self, app_id: &str) -> bool {
        let abierta = self.guard("abrir actividad de previsualización", |host| {
            host.shell.abrir_actividad_previsualizacion(app_id)
        }) == Some(true);
        if abierta {
            self.pintar_actividad();
        }
        abierta
    }

    pub fn actividad_app_id(&self) -> Option<&str> {
        self.shell.actividad_app_id()
    }

    pub fn actividad_clase(&self) -> Option<bookos_shell::actividad::Clase> {
        self.shell.actividad_clase()
    }

    pub fn actividad_visible(&mut self, visible: bool) -> bool {
        if self.actividad_visible == visible {
            return false;
        }
        self.actividad_visible = visible;
        true
    }

    fn pintar_actividad(&mut self) {
        let Some((w, h)) = self.shell.actividad_buffer_size() else {
            self.actividad = None;
            return;
        };
        let Some((lw, lh)) = self.shell.actividad_logical_size() else {
            return;
        };
        let rehacer = self
            .actividad
            .as_ref()
            .is_none_or(|s| s.src.size.w != w as f64 || s.src.size.h != h as f64);
        if rehacer {
            self.actividad = Some(Surface::new((w, h), (lw as i32, lh as i32)));
        }
        let escala = self.shell.scale() as f64;
        let arriba =
            self.shell.panel_height() as f64 + bookos_shell::actividad::SEPARACION_PANEL as f64;
        if let Some(s) = self.actividad.as_mut() {
            s.location = (
                (self.screen.0 as f64 - w as f64) / 2.0,
                (arriba - bookos_shell::actividad::MARGEN_SOMBRA as f64) * escala,
            )
                .into();
            let shell = &mut self.shell;
            paint(&mut s.buffer, |buf| shell.draw_actividad(buf));
        }
    }

    pub fn animar_actividad(&mut self) {
        if self.dead || !self.actividad_visible || !self.shell.actividad_needs_paint() {
            return;
        }
        self.pintar_actividad();
    }

    fn actividad_rect(&self) -> Option<(f64, f64, f64, f64)> {
        if !self.actividad_visible {
            return None;
        }
        let s = self.actividad.as_ref()?;
        let escala = self.shell.scale() as f64;
        let sombra = bookos_shell::actividad::MARGEN_SOMBRA as f64;
        Some((
            s.location.x / escala + sombra,
            s.location.y / escala + sombra,
            s.logical.w as f64 - sombra * 2.0,
            s.logical.h as f64 - sombra * 2.0,
        ))
    }

    // --- Notificaciones ----------------------------------------------------

    /// Guarda una notificación y repinta lo que la enseña.
    pub fn notificar(
        &mut self,
        notificacion: bookos_shell::notificaciones::Notificacion,
        caducidad: i32,
    ) -> bool {
        if self.dead {
            return false;
        }
        let repintar = self
            .guard("notificar", |host| {
                host.shell.notificar(notificacion, caducidad)
            })
            .unwrap_or(false);
        self.pintar_toast();
        self.tras_notificar(repintar)
    }

    /// Crea (o rehace) la superficie del aviso y la pinta, arriba a la derecha.
    fn pintar_toast(&mut self) {
        let Some((w, h)) = self.shell.toast_buffer_size() else {
            self.toast = None;
            return;
        };
        let Some((lw, lh)) = self.shell.toast_logical_size() else {
            return;
        };
        let escala = self.shell.scale() as f64;
        let mut surface = Surface::new((w, h), (lw as i32, lh as i32));
        // Pegado a la esquina, bajo el panel. Los márgenes se cuentan desde la
        // **tarjeta**, no desde el buffer, que lleva su hueco para la sombra.
        let margen = (bookos_shell::toast::MARGEN_LATERAL - bookos_shell::toast::MARGEN_SOMBRA)
            as f64
            * escala;
        let alto_panel = self.shell.panel_height() as f64 * escala;
        let arriba = (bookos_shell::toast::MARGEN_SUPERIOR - bookos_shell::toast::MARGEN_SOMBRA)
            as f64
            * escala;
        surface.location = (
            self.screen.0 as f64 - w as f64 - margen,
            alto_panel + arriba,
        )
            .into();
        paint(&mut surface.buffer, |buf| self.shell.draw_toast(buf));
        self.toast = Some(surface);
    }

    // --- Las barras de título de las ventanas -------------------------------

    /// El elemento de la barra de la ventana `id`, ya pintada si hacía falta.
    ///
    /// `origen` es la esquina superior izquierda de la barra en píxeles
    /// **físicos**, y `ancho` su anchura en lógicos —la de la ventana—. Devuelve
    /// `None` si el shell está muerto o la textura no se pudo subir: una sesión
    /// sin barras es un fallo que se mira, quedarse sin ventanas no.
    pub fn barra_ventana<R>(
        &mut self,
        renderer: &mut R,
        id: u64,
        ancho: i32,
        estado: bookos_shell::decoracion::Estado,
        origen: Point<f64, Physical>,
        alfa: f32,
    ) -> Option<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        if self.dead || ancho <= 0 {
            return None;
        }
        let repintar = self.guard("preparar barra de título", |host| {
            host.shell.barra_preparar(id, ancho as f32, estado)
        })?;
        let (w, h) = self.shell.barra_buffer_size(id)?;
        let (lw, lh) = self.shell.barra_logical_size(id)?;
        // El buffer se rehace solo cuando cambia de tamaño: redimensionar una
        // ventana es lo único que lo obliga, y mientras el ancho no cambie el
        // repintado va sobre el que ya está subido.
        let surface = match self.barras.get(&id) {
            Some(s) if s.src.size.w == w as f64 && s.src.size.h == h as f64 => {
                self.barras.get_mut(&id)?
            }
            _ => self
                .barras
                .entry(id)
                .insert_entry(Surface::new((w, h), (lw as i32, lh as i32)))
                .into_mut(),
        };
        surface.location = origen;
        if repintar {
            let shell = &mut self.shell;
            paint(&mut surface.buffer, |buf| shell.draw_barra(id, buf));
        }
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            surface.location,
            &surface.buffer,
            Some(alfa),
            Some(surface.src),
            Some(surface.logical),
            Kind::Unspecified,
        )
        .inspect_err(|_| tracing::warn!("no se pudo subir la barra de título"))
        .ok()
    }

    /// Tira las barras de las ventanas que ya no existen.
    pub fn barras_retener(&mut self, vivas: &[u64]) {
        if self.barras.len() == vivas.len() {
            return;
        }
        self.barras.retain(|id, _| vivas.contains(id));
        self.guard("purgar barras de título", |host| {
            host.shell.barras_retener(vivas)
        });
    }

    /// ¿Sigue el aviso a la vista? Devuelve además si **acaba** de retirarse,
    /// porque entonces hace falta un frame más: el que lo quita de la pantalla.
    pub fn toast_vivo(&mut self) -> (bool, bool) {
        let vivo = self.guard("toast vivo", |host| host.shell.toast_vivo()) == Some(true);
        let retirado = !vivo && self.toast.take().is_some();
        (vivo, retirado)
    }

    pub fn toast_queda(&self) -> Option<std::time::Duration> {
        self.shell.toast_queda()
    }

    /// Repinta el aviso mientras entra.
    pub fn animar_toast(&mut self) {
        if self.dead || !self.shell.toast_needs_paint() {
            return;
        }
        self.guard("animar toast", |host| host.pintar_toast());
    }

    /// El rectángulo de la **tarjeta** del aviso, en físicos: el buffer lleva
    /// el margen de la sombra, que no se pulsa.
    fn toast_rect(&self) -> Option<(f64, f64, f64, f64)> {
        let s = self.toast.as_ref()?;
        let escala = self.shell.scale() as f64;
        let sombra = bookos_shell::toast::MARGEN_SOMBRA as f64 * escala;
        Some((
            s.location.x + sombra,
            s.location.y + sombra,
            s.logical.w as f64 * escala - sombra * 2.0,
            s.logical.h as f64 * escala - sombra * 2.0,
        ))
    }

    /// ¿Se ha pulsado el aviso? Entonces se va, y el clic no llega a la ventana
    /// de debajo: pulsar un aviso es despedirlo, no escribir en lo que tape.
    pub fn toast_pulsado(&mut self, x: f64, y: f64) -> bool {
        if Self::dentro(self.toast_rect(), x, y).is_none() {
            return false;
        }
        let descartado =
            self.guard("descartar toast", |host| host.shell.descartar_toast()) == Some(true);
        if descartado {
            self.pintar_toast();
        }
        descartado
    }

    /// Retira una notificación por su identificador.
    pub fn cerrar_notificacion(&mut self, id: u32) -> bool {
        if self.dead {
            return false;
        }
        let repintar = self
            .guard("cerrar notificación", |host| {
                host.shell.cerrar_notificacion(id)
            })
            .unwrap_or(false);
        self.tras_notificar(repintar)
    }

    /// Vacía la cola y devuelve a quién hay que avisar por D-Bus.
    pub fn borrar_notificaciones(&mut self) -> Vec<u32> {
        if self.dead {
            return Vec::new();
        }
        let ids = self
            .guard("borrar notificaciones", |host| {
                host.shell.borrar_notificaciones()
            })
            .unwrap_or_default();
        self.tras_notificar(!ids.is_empty());
        ids
    }

    /// Repinta el panel y la tarjeta tras tocar la cola. La chapa del contador
    /// cambia el **ancho** del widget, así que hay que rehacer las zonas de
    /// clic del panel además de su dibujo.
    fn tras_notificar(&mut self, repintar: bool) -> bool {
        if !repintar {
            return false;
        }
        self.guard("pintar panel", |host| {
            let buffer = &mut host.panel.buffer;
            paint(buffer, |buf| host.shell.draw_panel(buf));
        });
        if self.shell.emergente_nombre() == Some("notificaciones") {
            self.sincronizar_emergente();
        }
        true
    }

    /// Repinta el conmutador mientras su recuadro se mueve de celda a celda.
    pub fn animar_conmutador(&mut self) {
        if self.dead || !self.shell.conmutador_animando() {
            return;
        }
        self.guard("animar conmutador", |host| host.pintar_conmutador());
    }

    /// Repinta el dock mientras la placa de un icono entra o sale.
    ///
    /// Va aparte de `refresh` porque el dock no tiene datos que releer: lo que
    /// cambia es solo su dibujo, y `refresh` se llama en el tick del panel —una
    /// vez por minuto— que no es el ritmo de una animación de 120 ms.
    pub fn animar_dock(&mut self) {
        if self.dead || !self.shell.dock_animando() {
            return;
        }
        self.guard("animar dock", |host| {
            let buffer = &mut host.dock.buffer;
            paint(buffer, |buf| host.shell.draw_dock(buf));
        });
    }

    /// ¿Sigue el aviso a la vista? De paso lo retira si se le acabó el tiempo.
    ///
    /// Devuelve además si **acaba** de retirarlo, porque entonces hace falta un
    /// frame más: el que lo quita de la pantalla.
    pub fn osd_vivo(&mut self) -> (bool, bool) {
        let vivo = self.guard("osd vivo", |host| host.shell.osd_vivo()) == Some(true);
        let retirado = !vivo && self.osd.take().is_some();
        (vivo, retirado)
    }

    /// Cuánto falta para que se vaya, para programar un solo despertar en vez
    /// de repintar sesenta veces por segundo enseñando algo que no cambia.
    pub fn osd_queda(&self) -> Option<std::time::Duration> {
        self.shell.osd_queda()
    }

    /// Repinta el aviso mientras su barra se mueve.
    pub fn animar_osd(&mut self) {
        if self.dead || !self.shell.osd_needs_paint() || self.osd.is_none() {
            return;
        }
        self.guard("animar osd", |host| host.pintar_osd());
    }

    fn pintar_osd(&mut self) {
        let Some((w, h)) = self.shell.osd_buffer_size() else {
            self.osd = None;
            return;
        };
        let Some((lw, lh)) = self.shell.osd_logical_size() else {
            return;
        };
        let mut surface = Surface::new((w, h), (lw as i32, lh as i32));
        // Centrado y por encima del dock: taparlo al subir el volumen es justo
        // lo que molesta del OSD de otros escritorios.
        let escala = self.shell.scale() as f64;
        // El buffer empieza más arriba que la cápsula justo lo que ocupa el
        // margen de la sombra; la cuenta la hace el shell, que es donde están
        // las dos medidas.
        let margen = bookos_shell::osd::MARGEN_BUFFER_INFERIOR as f64;
        surface.location = (
            (self.screen.0 as f64 - w as f64) / 2.0,
            self.screen.1 as f64 - margen * escala,
        )
            .into();
        paint(&mut surface.buffer, |buf| self.shell.draw_osd(buf));
        self.osd = Some(surface);
    }

    /// Cambia la preferencia de efectos y devuelve la elegida, para guardarla.
    pub fn alternar_efectos(&mut self) -> bookos_shell::Efectos {
        self.shell.alternar_efectos()
    }

    /// Recalcula el modo de efectos —lo elegido y lo que impone la batería— y
    /// devuelve si cambió. No pasa por `guard`: no dibuja nada, solo lee sysfs
    /// y mueve un booleano.
    pub fn revisar_efectos(&self) -> bool {
        !self.dead && self.shell.revisar_efectos()
    }

    // --- Panel de diagnóstico -----------------------------------------------

    /// Pone o quita el panel. Devuelve si quedó puesto, que es lo que el
    /// compositor necesita para programar o soltar su despertar por segundo.
    pub fn alternar_diagnostico(&mut self) -> bool {
        let puesto = self.guard("diagnóstico", |host| host.shell.alternar_diagnostico())
            == Some(true);
        if puesto {
            self.pintar_diagnostico();
        } else {
            self.diagnostico = None;
        }
        puesto
    }

    pub fn diagnostico_visible(&self) -> bool {
        self.diagnostico.is_some()
    }

    /// Entrega la medida de la última ventana. Devuelve si hay que repintar.
    pub fn diagnostico_datos(&mut self, datos: bookos_shell::diagnostico::Datos) -> bool {
        if self.dead || self.diagnostico.is_none() {
            return false;
        }
        let cambio = self.guard("datos del diagnóstico", |host| {
            host.shell.diagnostico_datos(datos)
        }) == Some(true);
        if cambio {
            self.pintar_diagnostico();
        }
        cambio
    }

    fn pintar_diagnostico(&mut self) {
        let Some((w, h)) = self.shell.diagnostico_buffer_size() else {
            self.diagnostico = None;
            return;
        };
        let Some((lw, lh)) = self.shell.diagnostico_logical_size() else {
            return;
        };
        let mut surface = Surface::new((w, h), (lw as i32, lh as i32));
        // Arriba a la derecha, justo debajo del panel: es la esquina que menos
        // tapa mientras se prueba algo, y no se solapa con el OSD ni con los
        // avisos, que salen abajo y arriba a la derecha respectivamente. El
        // toast se le puede poner encima; se acepta, es una herramienta.
        let escala = self.shell.scale() as f64;
        let margen = 12.0 * escala;
        surface.location = (
            self.screen.0 as f64 - w as f64 - margen,
            bookos_shell::PANEL_HEIGHT as f64 * escala + margen,
        )
            .into();
        paint(&mut surface.buffer, |buf| self.shell.draw_diagnostico(buf));
        self.diagnostico = Some(surface);
    }

    // --- Conmutador de aplicaciones ----------------------------------------

    pub fn hay_conmutador(&self) -> bool {
        !self.dead && self.shell.hay_conmutador()
    }

    /// Abre el conmutador. `false` si no había nada que conmutar.
    pub fn abrir_conmutador(
        &mut self,
        modo: bookos_shell::conmutador::Modo,
        apps: Vec<bookos_shell::conmutador::Entrada>,
    ) -> bool {
        if self.dead {
            // Se distingue del `false` normal: un shell muerto tras un panic no
            // vuelve, y confundirlo con "no hay celdas" manda a buscar el fallo
            // al sitio equivocado.
            tracing::warn!("el shell está muerto: no hay conmutador");
            return false;
        }
        let pantalla = (
            self.screen.0 as f32 / self.shell.scale(),
            self.screen.1 as f32 / self.shell.scale(),
        );
        let abierto = self
            .guard("abrir conmutador", |host| {
                host.shell.abrir_conmutador(modo, apps, pantalla)
            })
            .unwrap_or(false);
        if abierto {
            self.pintar_conmutador();
        }
        abierto
    }

    pub fn conmutador_mover(&mut self, pasos: i32) {
        self.guard("mover conmutador", |host| {
            host.shell.conmutador_mover(pasos)
        });
        self.pintar_conmutador();
    }

    pub fn conmutador_elegir(&mut self, i: usize) -> bool {
        let cambio = self
            .guard("elegir en conmutador", |host| {
                host.shell.conmutador_elegir(i)
            })
            .unwrap_or(false);
        if cambio {
            self.pintar_conmutador();
        }
        cambio
    }

    /// Celda bajo un punto de pantalla lógico.
    pub fn conmutador_en(&self, x: f64, y: f64) -> Option<usize> {
        let surface = self.conmutador.as_ref()?;
        let escala = self.shell.scale() as f64;
        let local_x = x - surface.location.x / escala;
        let local_y = y - surface.location.y / escala;
        self.shell.conmutador_en(local_x as f32, local_y as f32)
    }

    /// Huecos globales lógicos en los que se componen las ventanas vivas.
    pub fn conmutador_miniaturas(&self) -> Vec<Rectangle<i32, Logical>> {
        let Some(surface) = self.conmutador.as_ref() else {
            return Vec::new();
        };
        let escala = self.shell.scale() as f64;
        let origen_x = surface.location.x / escala;
        let origen_y = surface.location.y / escala;
        self.shell
            .conmutador_miniaturas()
            .into_iter()
            .map(|r| {
                Rectangle::new(
                    (
                        (origen_x + r.x as f64).round() as i32,
                        (origen_y + r.y as f64).round() as i32,
                    )
                        .into(),
                    (
                        r.width.round().max(1.0) as i32,
                        r.height.round().max(1.0) as i32,
                    )
                        .into(),
                )
            })
            .collect()
    }

    /// El chrome modal del selector, separado del resto del shell para que se
    /// siga dibujando encima de una ventana a pantalla completa.
    pub fn conmutador_element<R>(
        &self,
        renderer: &mut R,
    ) -> Option<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        let surface = self.conmutador.as_ref()?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            surface.location,
            &surface.buffer,
            Some(1.0),
            Some(surface.src),
            Some(surface.logical),
            Kind::Unspecified,
        )
        .ok()
    }

    /// Cierra el conmutador y devuelve **qué celda** quedó elegida. La ventana
    /// a la que corresponde la sabe el compositor, no el shell.
    pub fn cerrar_conmutador(&mut self) -> Option<usize> {
        let elegida = self
            .guard("cerrar conmutador", |host| host.shell.cerrar_conmutador())
            .flatten();
        self.conmutador = None;
        elegida
    }

    pub fn cancelar_conmutador(&mut self) -> bool {
        let habia = self
            .guard("cancelar conmutador", |host| {
                host.shell.cancelar_conmutador()
            })
            .unwrap_or(false);
        self.conmutador = None;
        habia
    }

    /// Rehace su superficie y la centra en la pantalla.
    ///
    /// Se rehace en cada paso y no solo al abrir porque el tamaño no cambia pero
    /// el contenido sí —la celda realzada y el nombre de debajo—, y centrar de
    /// nuevo cuesta dos restas.
    fn pintar_conmutador(&mut self) {
        let Some((w, h)) = self.shell.conmutador_buffer_size() else {
            self.conmutador = None;
            return;
        };
        let Some((lw, lh)) = self.shell.conmutador_logical_size() else {
            return;
        };
        let mut surface = Surface::new((w, h), (lw as i32, lh as i32));
        // Centrado en los dos ejes, como el de macOS: es una tarjeta modal, no
        // un aviso que se asoma por un borde.
        surface.location = (
            (self.screen.0 as f64 - w as f64) / 2.0,
            (self.screen.1 as f64 - h as f64) / 2.0,
        )
            .into();
        paint(&mut surface.buffer, |buf| self.shell.draw_conmutador(buf));
        self.conmutador = Some(surface);
    }

    /// Echa el bloqueo y prepara su superficie, del tamaño de la pantalla.
    pub fn bloquear(&mut self, hora: String, fecha: String, pantalla: (f32, f32)) {
        self.guard("bloquear", |host| {
            host.shell.bloquear(hora, fecha, pantalla);
        });
        // La superficie se crea aquí y no en `refresh` porque el bloqueo tiene
        // que estar pintado en el primer frame: una pantalla de bloqueo que
        // tarda es una pantalla de bloqueo que enseña el escritorio.
        if let Some((w, h)) = self.shell.bloqueo_buffer_size() {
            let logico = (
                (w as f32 / self.shell.scale()).round() as i32,
                (h as f32 / self.shell.scale()).round() as i32,
            );
            let mut surface = Surface::new((w, h), logico);
            paint(&mut surface.buffer, |buf| self.shell.draw_bloqueo(buf));
            self.bloqueo = Some(surface);
        }
    }

    /// Abre la capa de captura de pantalla y la deja pintada.
    ///
    /// La superficie se crea aquí y no en `refresh` por lo mismo que la del
    /// bloqueo: tiene que estar puesta en el primer fotograma, o al pulsar Impr
    /// se vería el escritorio sin velo durante un frame.
    pub fn abrir_captura(&mut self) {
        if self.dead {
            return;
        }
        let escala = self.shell.scale();
        let pantalla = (
            self.screen.0 as f32 / escala,
            self.screen.1 as f32 / escala,
        );
        self.guard("abrir captura", |host| {
            host.shell.abrir_captura(pantalla);
        });
        if let Some((w, h)) = self.shell.captura_buffer_size() {
            let logico = (
                (w as f32 / escala).round() as i32,
                (h as f32 / escala).round() as i32,
            );
            let mut surface = Surface::new((w, h), logico);
            paint(&mut surface.buffer, |buf| self.shell.draw_captura(buf));
            self.captura = Some(surface);
        }
    }

    pub fn cerrar_captura(&mut self) -> bool {
        let habia = self
            .guard("cerrar captura", |host| host.shell.cerrar_captura())
            .unwrap_or(false);
        self.captura = None;
        habia
    }

    pub fn hay_captura(&self) -> bool {
        !self.dead && self.captura.is_some()
    }

    pub fn captura_animando(&self) -> bool {
        !self.dead && self.shell.captura_animando()
    }

    /// Repinta la capa si lo pide. Devuelve `true` si se repintó.
    fn repintar_captura(&mut self) -> bool {
        if !self.shell.captura_needs_paint() {
            return false;
        }
        self.guard("pintar captura", |host| {
            let Some(surface) = host.captura.as_mut() else {
                return;
            };
            let buffer = &mut surface.buffer;
            paint(buffer, |buf| host.shell.draw_captura(buf));
        });
        true
    }

    /// Mueve el puntero sobre la capa, en lógicos de la pantalla.
    pub fn captura_puntero(&mut self, x: f64, y: f64) -> bool {
        if !self.hay_captura() {
            return false;
        }
        let movido = self
            .guard("puntero de la captura", |host| {
                host.shell.captura_puntero(x as f32, y as f32)
            })
            .unwrap_or(false);
        if movido {
            self.repintar_captura();
        }
        movido
    }

    pub fn captura_pulsar(&mut self, x: f64, y: f64) -> Option<bookos_shell::Accion> {
        if !self.hay_captura() {
            return None;
        }
        let accion = self
            .guard("clic en la captura", |host| {
                host.shell.captura_pulsar(x as f32, y as f32)
            })
            .flatten();
        self.repintar_captura();
        accion
    }

    pub fn captura_soltar(&mut self) -> Option<bookos_shell::Accion> {
        if !self.hay_captura() {
            return None;
        }
        let accion = self
            .guard("soltar en la captura", |host| host.shell.captura_soltar())
            .flatten();
        self.repintar_captura();
        accion
    }

    /// Una tecla para la capa. Devuelve `(consumida, acción)`.
    pub fn captura_tecla(
        &mut self,
        tecla: bookos_shell::TeclaPulsada,
    ) -> (bool, Option<bookos_shell::Accion>) {
        if !self.hay_captura() {
            return (false, None);
        }
        let resultado = self
            .guard("tecla de la captura", |host| host.shell.captura_tecla(tecla))
            .unwrap_or(bookos_shell::Tecla::Ignorada);
        self.repintar_captura();
        match resultado {
            bookos_shell::Tecla::Cerrar => {
                self.cerrar_captura();
                (true, None)
            }
            bookos_shell::Tecla::Hacer(accion) => (true, Some(accion)),
            bookos_shell::Tecla::Consumida => (true, None),
            bookos_shell::Tecla::Ignorada => (false, None),
        }
    }

    /// Repinta el bloqueo mientras su menú se mueve. Devuelve `true` si sigue
    /// habiendo algo que animar, que es lo que impide que el bucle se duerma a
    /// mitad de la transición.
    pub fn animar_bloqueo(&mut self) -> bool {
        if self.dead || !self.shell.bloqueo_animando() {
            return false;
        }
        self.guard("pintar bloqueo", |host| {
            let Some(surface) = host.bloqueo.as_mut() else {
                return;
            };
            let buffer = &mut surface.buffer;
            paint(buffer, |buf| host.shell.draw_bloqueo(buf));
        });
        true
    }

    /// Un clic sobre el bloqueo, en coordenadas lógicas de la pantalla.
    pub fn bloqueo_pulsado(&mut self, x: f64, y: f64) -> Option<bookos_shell::bloqueo::Peticion> {
        let (peticion, repintar) = self
            .guard("clic en el bloqueo", |host| {
                host.shell.bloqueo_pulsado(x as f32, y as f32)
            })
            .unwrap_or((None, false));
        if repintar {
            self.guard("pintar bloqueo", |host| {
                let Some(surface) = host.bloqueo.as_mut() else {
                    return;
                };
                let buffer = &mut surface.buffer;
                paint(buffer, |buf| host.shell.draw_bloqueo(buf));
            });
        }
        peticion
    }

    /// Actualiza los puntos y el estado del bloqueo, y lo repinta.
    pub fn bloqueo_estado(&mut self, escritos: usize, estado: bookos_shell::bloqueo::Estado) {
        if self.dead {
            return;
        }
        let cambio = self
            .guard("estado del bloqueo", |host| {
                host.shell.bloqueo_estado(escritos, estado)
            })
            .unwrap_or(false);
        if !cambio {
            return;
        }
        self.guard("pintar bloqueo", |host| {
            let Some(surface) = host.bloqueo.as_mut() else {
                return;
            };
            let buffer = &mut surface.buffer;
            paint(buffer, |buf| host.shell.draw_bloqueo(buf));
        });
    }

    /// Incorpora la lectura multimedia que llegó en segundo plano y repinta la
    /// capa del bloqueo. Si ya se desbloqueó, el shell la descarta.
    pub fn bloqueo_medio(&mut self, sonando: Option<bookos_shell::medios::Sonando>) {
        if self.dead {
            return;
        }
        let cambio = self
            .guard("medios del bloqueo", |host| {
                host.shell.bloqueo_medio(sonando)
            })
            .unwrap_or(false);
        if !cambio {
            return;
        }
        self.guard("pintar bloqueo", |host| {
            let Some(surface) = host.bloqueo.as_mut() else {
                return;
            };
            let buffer = &mut surface.buffer;
            paint(buffer, |buf| host.shell.draw_bloqueo(buf));
        });
    }

    pub fn desbloquear(&mut self) {
        self.shell.desbloquear();
        self.bloqueo = None;
    }

    pub fn esta_bloqueado(&self) -> bool {
        !self.dead && self.shell.esta_bloqueado()
    }

    /// ¿Cae el punto dentro del dock?
    pub fn en_el_dock(&self, x: f64, y: f64) -> bool {
        !self.dead
            && self.a_la_vista(Barra::Dock)
            && Self::dentro(Some(self.dock_rect()), x, y).is_some()
    }

    /// Abre el menú contextual del icono que haya en ese punto del dock.
    /// `true` si se abrió alguno.
    pub fn menu_dock_en(&mut self, x: f64, y: f64) -> bool {
        let Some((rx, ry)) = Self::dentro(Some(self.dock_rect()), x, y) else {
            return false;
        };
        if self.guard("menú del dock", |host| host.shell.dock_menu(rx, ry)) != Some(true) {
            return false;
        }
        self.sincronizar_emergente();
        true
    }

    /// Ancla o desancla una aplicación. Devuelve la lista que hay que guardar.
    pub fn anclar(&mut self, app_id: &str, exec: &str, icono: &str) -> Option<Vec<String>> {
        let lista = self.guard("anclar", |host| host.shell.anclar(app_id, exec, icono))?;
        // El dock cambia de tamaño al ganar o perder un icono, así que hay que
        // recolocarlo y volver a pintarlo entero.
        self.dock = Surface::new(
            self.shell.dock_buffer_size(),
            self.shell.dock_logical_size(),
        );
        self.place_dock();
        Some(lista)
    }

    /// El nombre de la emergente abierta. Para el autotest.
    pub fn emergente_nombre(&self) -> Option<&'static str> {
        (!self.dead)
            .then(|| self.shell.emergente_nombre())
            .flatten()
    }

    /// ¿Hay alguna superficie emergente abierta? Para el autotest.
    pub fn hay_emergente(&self) -> bool {
        !self.dead && self.shell.hay_emergente()
    }

    /// Dónde cae un widget del panel, en lógicos. Para el autotest.
    pub fn zona_widget(&self, nombre: &str) -> Option<(f64, f64)> {
        self.shell
            .zonas_panel()
            .into_iter()
            .find(|(n, _, _)| *n == nombre)
            .map(|(_, x0, x1)| (x0 as f64, x1 as f64))
    }

    /// El botón se ha soltado: lo que estuviera agarrado deja de estarlo.
    /// Clic derecho sobre el launchpad: entra o sale del modo edición, que es
    /// donde cada icono lleva su ✕. `true` si lo ha atendido.
    pub fn launchpad_menu(&mut self) -> bool {
        if self.dead || self.shell.emergente_nombre() != Some("launchpad") {
            return false;
        }
        let hecho =
            self.guard("editar launchpad", |host| host.shell.launchpad_editar()) == Some(true);
        if hecho {
            self.sincronizar_emergente();
        }
        hecho
    }

    /// El botón se ha soltado. Devuelve lo que haya que hacer: soltar un icono
    /// del launchpad sin arrastrarlo es lanzarlo.
    pub fn soltar(&mut self) -> Option<Accion> {
        if self.dead {
            return None;
        }
        let (repintar, accion) = self
            .guard("soltar", |host| host.shell.soltar())
            .unwrap_or((false, None));
        if repintar || accion.is_some() {
            self.sincronizar_emergente();
        }
        accion
    }

    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        if self.dead
            || (self.screen == (width as i32, height as i32) && self.shell.scale() == scale)
        {
            return;
        }
        self.screen = (width as i32, height as i32);
        self.guard("resize", |host| {
            let logical_width = (width as f32 / scale).round().max(1.0) as u32;
            host.shell.resize(logical_width, scale);
            host.panel = Surface::new(
                host.shell.panel_buffer_size(),
                host.shell.panel_logical_size(),
            );
            host.dock = Surface::new(
                host.shell.dock_buffer_size(),
                host.shell.dock_logical_size(),
            );
            host.place_dock();
        });
        self.escritorio_recolocar();
    }

    /// Relee los datos y repinta lo que haya cambiado. Devuelve `true` si hay
    /// algo nuevo en pantalla, es decir, si el compositor tiene que redibujar.
    pub fn refresh(&mut self) -> bool {
        if self.dead {
            return false;
        }
        let dirty = self.guard("refresh", |host| {
            let mut dirty = false;
            if host.shell.refresh() {
                let buffer = &mut host.panel.buffer;
                paint(buffer, |buf| host.shell.draw_panel(buf));
                dirty = true;
            }
            // El dock es estático: se pinta la primera vez y no se vuelve a
            // tocar hasta que cambie el tamaño de la pantalla.
            if host.shell.dock_needs_paint() {
                let buffer = &mut host.dock.buffer;
                paint(buffer, |buf| host.shell.draw_dock(buf));
                dirty = true;
            }
            dirty
        })
        .unwrap_or(false);

        // La carpeta del escritorio se mira en este mismo latido, que es el que
        // ya despierta al cambiar el minuto: así un fichero nuevo aparece sin
        // abrir un inotify ni programar un temporizador propio. El precio es
        // hasta un minuto de retraso, y es el trato de siempre aquí.
        //
        // Mientras se arrastra no se toca: releer rehace las superficies y el
        // icono se quedaría clavado a media cuesta.
        let escala = self.shell.scale();
        let logica = (
            self.screen.0 as f32 / escala,
            self.screen.1 as f32 / escala,
        );
        let cambio = !self.escritorio_arrastrando()
            && self
                .guard("releer el escritorio", |host| {
                    host.shell.escritorio_releer(logica)
                })
                .unwrap_or(false);
        if cambio {
            self.escritorio_recolocar();
        }
        dirty || cambio
    }

    /// Cuánto ha avanzado la animación de entrada, de 0 a 1.
    ///
    /// Las duraciones y las curvas salen de la tabla de movimiento del sistema
    /// de diseño (§2.7), no de aquí: una emergente **es** un popover, y el
    /// sistema le asigna 180 ms con muelle. Antes esto eran 220 y 280 ms con
    /// potencias copiadas del plasmoide de Plasma, que no rebotan; el muelle es
    /// lo que hace que se lea como que aparece y no como que se despliega.
    ///
    /// La animación **no repinta nada**: el buffer está dibujado y lo único que
    /// cambia de un fotograma a otro son el alfa y el tamaño con que el
    /// compositor lo compone, que es trabajo de la GPU.
    fn animacion(&self) -> (f32, f32, f32) {
        use bookos_shell::tema;

        // Yéndose: la misma duración, pero con la curva suave y no con el
        // muelle. Un rebote al salir deja la emergente dando un respingo justo
        // cuando el usuario ya ha decidido que no la quiere.
        if let Some(desde) = self.cerrando_en {
            let t = tema::C_SUAVE.eval(tema::fraccion(desde.elapsed(), Self::SALIDA));
            // Se recoge hacia el panel: poca escala y más desplazamiento. La
            // salida corta evita que cambiar entre varios widgets se sienta
            // como esperar dos animaciones completas.
            return (1.0 - t, 1.0 - 0.025 * t, -18.0 * t);
        }

        let Some(desde) = self.abierta_en else {
            return (1.0, 1.0, 0.0);
        };
        let t = desde.elapsed();
        // El alfa va con la curva sin rebote aunque comparta duración: un muelle
        // aquí lo llevaría por encima de 1 y habría que recortarlo, gastando la
        // parte interesante de la curva en nada.
        let alfa = tema::C_ENTRADA.eval(tema::fraccion(t, Self::ENTRADA));
        let escala = tema::C_MUELLE_POPOVER.eval(tema::fraccion(t, Self::ENTRADA));
        // Entra encogiendo desde 1,05, que es el zoom hacia dentro del
        // launchpad. Con el muelle, la escala se pasa un poco por debajo de 1 y
        // vuelve, en vez de frenar y quedarse.
        // Nace ligeramente por encima y baja hasta su anclaje, como los
        // popovers del panel de Plasma, conservando el muelle del HIG.
        (alfa, 0.97 + 0.03 * escala, -16.0 * (1.0 - escala))
    }

    /// Lo que tarda una emergente en entrar y en irse: la duración de popover
    /// del sistema de diseño. La entrada y la salida ya no se separan —eran 280
    /// y 180— porque la tabla da un único valor para "aparición de popover" y
    /// dos duraciones distintas para el mismo gesto son justo lo que hace que un
    /// escritorio se note cosido a mano.
    const ENTRADA: std::time::Duration = bookos_shell::tema::D_POPOVER;
    const SALIDA: std::time::Duration = std::time::Duration::from_millis(105);
    /// Lo que tarda el realce en deslizarse de una celda a otra. Es el token de
    /// hover del sistema: mover el resaltado con el cursor es un hover.
    const DESLIZ: std::time::Duration = bookos_shell::tema::D_HOVER;

    /// ¿Hay una animación en curso? Mientras la haya, el compositor tiene que
    /// seguir dibujando aunque no pase nada más.
    pub fn animando(&self) -> bool {
        if self.shell.emergente_animando()
            || self.shell.dock_animando()
            || self.shell.conmutador_animando()
            || self.shell.osd_animando()
            || self.shell.toast_animando()
            || (self.actividad_visible && self.shell.actividad_animando())
        {
            return true;
        }
        // Las tres condiciones miran si la animación **sigue en marcha**, no si
        // existe. `cerrando_en.is_some()` a secas cerraba un ciclo: la salida no
        // terminaba nunca, `fin_animacion` —que es quien limpia— solo se llama
        // cuando esto devuelve false, y el compositor se quedaba dibujando a
        // toda velocidad para siempre después de cerrar cualquier emergente.
        // Eso era lo que se veía como que el escritorio iba a tirones, y no
        // paraba hasta reiniciar la sesión.
        self.abierta_en.is_some_and(|t| t.elapsed() < Self::ENTRADA)
            || self.cerrando_en.is_some_and(|t| t.elapsed() < Self::SALIDA)
            || self
                .pagina_anterior
                .as_ref()
                .is_some_and(|(_, t, _)| t.elapsed() < bookos_shell::tema::D_PAGINA)
            || self
                .realce_desde
                .is_some_and(|(_, t)| t.elapsed() < Self::DESLIZ)
    }

    /// El velo a pantalla completa que pide la emergente, si lo pide.
    pub fn velo(
        &self,
    ) -> Option<smithay::backend::renderer::element::solid::SolidColorRenderElement> {
        use smithay::backend::renderer::element::solid::SolidColorRenderElement;
        use smithay::backend::renderer::element::{Id, Kind};

        if self.dead {
            return None;
        }
        // El identificador tiene que ser **el mismo** en cada frame: el damage
        // tracker compara elementos por id, y uno nuevo cada vez le diría que
        // toda la pantalla ha cambiado.
        static ID: std::sync::OnceLock<Id> = std::sync::OnceLock::new();

        let [r, g, b, a] = self.shell.emergente_velo()?;
        let a = a * self.animacion().0;
        Some(SolidColorRenderElement::new(
            ID.get_or_init(Id::new).clone(),
            Rectangle::new((0, 0).into(), (self.screen.0, self.screen.1).into()),
            0,
            // Premultiplicado, que es lo que espera el renderer.
            [r * a, g * a, b * a, a],
            Kind::Unspecified,
        ))
    }

    /// Los elementos que se le pasan a `render_output` como `custom_elements`.
    pub fn elements<R>(&self, renderer: &mut R) -> Vec<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        if self.dead {
            return Vec::new();
        }
        // Orden de delante hacia atrás: el aviso por encima de todo, luego la
        // emergente, su realce justo detrás —para que quede bajo los iconos,
        // que van en el buffer de la emergente— y al fondo el panel y el dock.
        //
        // Cada superficie lleva su papel escrito y no se deduce de la posición
        // en la lista. Antes se decidía por índice y el aviso, que va el
        // primero, se llevaba el alfa y el zoom de la emergente mientras la
        // emergente se quedaba sin animar.
        #[derive(Clone, Copy)]
        enum Papel {
            /// Se desvanece con su propio reloj.
            Aviso,
            /// Entra y sale con la animación de la emergente.
            Emergente,
            /// Página del Launchpad que sale por el lado contrario.
            PaginaAnterior(std::time::Instant, f32),
            /// El aviso de una notificación, arriba a la derecha.
            Toast,
            /// Actividad viva que emerge del borde superior.
            Actividad,
            /// Igual, y además se desliza al cambiar de fila.
            Realce,
            /// Se aparta por su borde al esquivar ventanas.
            Barra(Barra),
            /// Quieto donde está, opaco: el panel de diagnóstico. No se anima a
            /// propósito — una herramienta de medida que entra con un muelle
            /// falsearía los primeros fotogramas de lo que se está midiendo.
            Fijo,
        }

        let (alfa_emergente, zoom, y_emergente) = self.animacion();
        let pagina = self.pagina_anterior.as_ref().map(|(_, t0, hacia)| {
            let t = bookos_shell::tema::C_SUAVE.eval(bookos_shell::tema::fraccion(
                t0.elapsed(),
                bookos_shell::tema::D_PAGINA,
            ));
            (t, *hacia)
        });
        let alfa_osd = self.shell.osd_alfa();
        let escala_osd = self.shell.osd_escala();
        // Con el bloqueo echado **solo** se dibuja el bloqueo, y el aviso de
        // volumen o brillo por encima: el panel y el dock delatarían lo que hay
        // detrás —y el dock, además, deja pulsar iconos—. Esta superficie
        // faltaba en la lista: se creaba y se pintaba, pero no llegaba a la
        // escena, así que Meta+L escondía las ventanas y no enseñaba nada.
        let bloqueado = self.bloqueo.is_some();
        let superficies = self
            .bloqueo
            .iter()
            .map(|s| (s, Papel::Barra(Barra::Panel)))
            // La capa de captura va por delante de todo lo demás menos el
            // bloqueo: tapa el escritorio entero con su velo, y el panel o el
            // dock por encima saldrían en la foto que se está encuadrando.
            .chain(
                self.captura
                    .iter()
                    .map(|s| (s, Papel::Fijo))
                    .filter(|_| !bloqueado),
            )
            .chain(self.osd.iter().map(|s| (s, Papel::Aviso)))
            // Con el bloqueo echado tampoco sale el diagnóstico: la regla es
            // que ahí solo se dibuja el bloqueo, y una excepción «porque esta
            // no enseña nada» es como se erosionan las reglas de ese tipo.
            .chain(
                self.diagnostico
                    .iter()
                    .map(|s| (s, Papel::Fijo))
                    .filter(|_| !bloqueado),
            )
            .chain(self.toast.iter().map(|s| (s, Papel::Toast)))
            .chain(
                self.actividad
                    .iter()
                    .map(|s| (s, Papel::Actividad))
                    .filter(|_| !bloqueado && self.actividad_visible),
            )
            .chain(
                self.pagina_anterior
                    .iter()
                    .map(|(s, t, hacia)| (s, Papel::PaginaAnterior(*t, *hacia)))
                    .filter(|_| !bloqueado),
            )
            .chain(
                self.emergente
                    .iter()
                    .map(|s| (s, Papel::Emergente))
                    .filter(|_| !bloqueado),
            )
            .chain(
                self.realce
                    .iter()
                    .map(|s| (s, Papel::Realce))
                    .filter(|_| !bloqueado && pagina.is_none()),
            )
            // Con una vista a pantalla completa no se dibujan las barras: el
            // panel por encima delataría que lo de debajo sigue ahí.
            .chain(
                [
                    (&self.panel, Papel::Barra(Barra::Panel)),
                    (&self.dock, Papel::Barra(Barra::Dock)),
                ]
                .into_iter()
                .filter(|_| {
                    !bloqueado
                        && self.captura.is_none()
                        && !self.shell.emergente_tapa_la_pantalla()
                }),
            );

        superficies
            .filter_map(|(surface, papel)| {
                let (alfa, zoom, desplazamiento_x, desplazamiento_y) = match papel {
                    Papel::Aviso => (alfa_osd, escala_osd, 0.0, 0.0),
                    Papel::Toast => (self.shell.toast_alfa(), self.shell.toast_escala(), 0.0, 0.0),
                    Papel::Emergente => match pagina {
                        Some((t, hacia)) => (t, 1.0, hacia * 140.0 * (1.0 - t), 0.0),
                        None => (alfa_emergente, zoom, 0.0, y_emergente),
                    },
                    Papel::PaginaAnterior(t0, hacia) => {
                        let t = bookos_shell::tema::C_SUAVE.eval(bookos_shell::tema::fraccion(
                            t0.elapsed(),
                            bookos_shell::tema::D_PAGINA,
                        ));
                        (1.0 - t, 1.0, -hacia * 140.0 * t, 0.0)
                    }
                    Papel::Realce => (alfa_emergente, zoom, 0.0, y_emergente),
                    Papel::Actividad => {
                        let (a, z, y) = self.shell.actividad_entrada();
                        (a, z, 0.0, y)
                    }
                    Papel::Barra(_) | Papel::Fijo => (1.0, 1.0, 0.0, 0.0),
                };
                // El zoom crece desde el centro, así que hay que compensar la
                // posición: si no, la superficie se agranda hacia la derecha y
                // hacia abajo y parece que se desliza en diagonal.
                let escala = self.shell.scale() as f64;
                let ancho = surface.logical.w as f64 * escala;
                let alto = surface.logical.h as f64 * escala;
                let base = match papel {
                    Papel::Realce => self.realce_location(surface.location),
                    Papel::Barra(cual) => {
                        // El panel se aparta hacia arriba y el dock hacia abajo.
                        let signo = match cual {
                            Barra::Panel => -1.0,
                            Barra::Dock => 1.0,
                        };
                        Point::from((
                            surface.location.x,
                            surface.location.y + signo * self.desplazamiento(cual),
                        ))
                    }
                    _ => Point::from((
                        surface.location.x + desplazamiento_x as f64 * escala,
                        surface.location.y + desplazamiento_y as f64 * escala,
                    )),
                };
                let location = Point::<f64, Physical>::from((
                    base.x - ancho * (zoom as f64 - 1.0) / 2.0,
                    base.y - alto * (zoom as f64 - 1.0) / 2.0,
                ));
                let logical = Size::<i32, Logical>::from((
                    (surface.logical.w as f32 * zoom) as i32,
                    (surface.logical.h as f32 * zoom) as i32,
                ));
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    location,
                    &surface.buffer,
                    Some(alfa),
                    Some(surface.src),
                    Some(logical),
                    Kind::Unspecified,
                )
                .inspect_err(|_| tracing::warn!("no se pudo subir una textura del shell"))
                .ok()
            })
            .collect()
    }

    /// Ejecuta `f` atrapando cualquier panic del shell.
    ///
    /// `AssertUnwindSafe` es correcto aquí porque el único estado que puede
    /// quedar a medias es el del propio shell, y al marcarlo muerto no se
    /// vuelve a tocar.
    fn guard<T>(&mut self, que: &str, f: impl FnOnce(&mut Self) -> T) -> Option<T> {
        match catch_unwind(AssertUnwindSafe(|| f(self))) {
            Ok(value) => Some(value),
            Err(_) => {
                tracing::error!(
                    que,
                    "el shell ha entrado en panic; se desactiva y la sesión \
                     continúa sin panel ni dock"
                );
                self.dead = true;
                None
            }
        }
    }
}

/// Vuelca un dibujo del shell en su `MemoryRenderBuffer`, traduciendo los
/// rectángulos dañados de iced a los de Smithay.
fn paint(buffer: &mut MemoryRenderBuffer, draw: impl FnOnce(&mut [u8]) -> Vec<Damage>) {
    let mut ctx = buffer.render();
    let result = ctx.draw(|slice| {
        let damage = draw(slice);
        Ok::<_, std::convert::Infallible>(
            damage
                .into_iter()
                .map(|r| Rectangle::new((r.x, r.y).into(), (r.width, r.height).into()))
                .collect(),
        )
    });
    // El error es Infallible, pero dejarlo explícito documenta que aquí no se
    // traga nada en silencio.
    if let Err(err) = result {
        match err {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El buscador **no se mueve** mientras escribes.
    ///
    /// Su alto cambia con cada tecla —cada resultado que entra o sale es media
    /// fila— y antes se recolocaba en cada cambio de tamaño, así que el campo
    /// de texto subía y bajaba mientras tecleabas y el rectángulo de cristal de
    /// detrás saltaba con él. Ahora se coloca por su alto máximo. Se comprueba
    /// aquí y no en el shell porque quien coloca es el compositor.
    #[test]
    fn el_buscador_no_salta_al_escribir() {
        use bookos_shell::TeclaPulsada;

        let mut host = ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));
        host.alternar_buscador();
        assert_eq!(host.shell.emergente_nombre(), Some("buscador"));
        let (_, y0, _, alto0) = host.emergente_rect().expect("el buscador tiene superficie");

        // «sh» encuentra al menos el ejecutable del PATH, que existe en
        // cualquier Linux: la lista aparece y la tarjeta crece.
        for c in "sh".chars() {
            host.tecla(TeclaPulsada::Caracter(c));
        }
        let (_, y1, _, alto1) = host.emergente_rect().expect("sigue abierto");
        assert!(alto1 > alto0, "la lista no apareció: el test no prueba nada");
        assert_eq!(y1, y0, "la tarjeta se movió al escribir");

        // Y al borrar tampoco vuelve a moverse.
        host.tecla(TeclaPulsada::Retroceso);
        let (_, y2, _, _) = host.emergente_rect().expect("sigue abierto");
        assert_eq!(y2, y0, "la tarjeta se movió al borrar");
    }

    /// Una tarjeta que cuelga del panel no toca el borde de la pantalla, y la
    /// franja de escritorios sigue pegada a él.
    ///
    /// El calendario cuelga del reloj, que está en la esquina superior derecha,
    /// así que su sitio natural cae fuera de la pantalla. Antes se recortaba a
    /// cero y quedaba pegado al borde exacto: su sombra cae fuera del buffer,
    /// desaparecía justo por ese lado y la tarjeta se leía como cortada. Pero
    /// el arreglo no puede ser dar margen a todo: la franja de escritorios se
    /// ancla al borde de arriba a propósito y ahí 8 px la descolocan.
    #[test]
    fn el_calendario_no_toca_el_borde_y_la_franja_si() {
        let mut host = ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));

        host.abrir_de_widget("reloj");
        assert_eq!(host.shell.emergente_nombre(), Some("calendario"));
        let (x, y, w, h) = host.emergente_rect().expect("el calendario tiene superficie");
        assert!(x >= SEPARACION, "pegado al borde izquierdo: x={x}");
        assert!(y >= SEPARACION, "pegado al borde de arriba: y={y}");
        assert!(
            x + w <= 1920.0 - SEPARACION,
            "pegado al borde derecho: x+w={} de 1920",
            x + w
        );
        assert!(
            y + h <= 1080.0 - SEPARACION,
            "pegado al borde de abajo: y+h={} de 1080",
            y + h
        );

        host.cerrar_emergente();
        host.alternar_vista_escritorios(0, vec!["Uno".into(), "Dos".into()]);
        assert_eq!(host.shell.emergente_nombre(), Some("escritorios"));
        let (x, y, _, _) = host.emergente_rect().expect("la franja tiene superficie");
        assert_eq!((x, y), (0.0, 0.0), "la franja se ha descolocado del borde");
    }

    /// El shell vive solo en la pantalla principal y no puede quedarse con los
    /// clics de las demás.
    ///
    /// Sin esto, con una pantalla colocada **a la izquierda** —que en el
    /// escritorio normalizado tiene `x` negativa— pasaban dos cosas: la franja
    /// de arriba caía dentro del panel de la principal y se tragaba los clics,
    /// y con cualquier emergente abierta el shell reclamaba la pantalla
    /// secundaria **entera**, así que el ratón dejaba de funcionar ahí hasta
    /// cerrarla. Es un caso que no se ve sin dos monitores enchufados, de ahí
    /// el test.
    #[test]
    fn el_shell_no_reclama_los_clics_de_otra_pantalla() {
        // 1920×1080 a escala 1: el área lógica de la principal es (0,0)-(1920,1080).
        let mut host = ShellHost::new(1920, 1080, 1.0, Some(bookos_shell::Config::default()));
        assert_eq!(host.area_principal(), (1920.0, 1080.0));

        // Dentro de la principal, la franja del panel es suya.
        assert!(host.a_la_vista(Barra::Panel), "el panel arranca a la vista");
        assert!(host.contiene(960.0, 4.0), "la franja del panel es del shell");

        // La misma altura, pero en la pantalla de la izquierda.
        assert!(
            !host.contiene(-400.0, 4.0),
            "x negativa es otro monitor: el clic es del cliente que haya ahí"
        );
        // Y a la derecha, pasado el borde.
        assert!(!host.contiene(2400.0, 4.0), "pasado el ancho tampoco es suyo");

        // Con una emergente abierta el shell se queda con todo lo de la
        // principal —pulsar fuera la cierra— pero **solo** con lo de la
        // principal.
        host.abrir_acerca();
        assert!(host.shell.hay_emergente(), "no se abrió la emergente");
        assert!(
            host.contiene(960.0, 600.0),
            "con una emergente, el clic en la principal la cierra"
        );
        assert!(
            !host.contiene(-400.0, 600.0),
            "una emergente en la principal no puede tragarse el ratón del monitor de al lado"
        );

        // Y lo mismo para el escritorio: el hueco vacío de otra pantalla no
        // arranca una banda elástica aquí.
        assert!(host.escritorio_alcanza(960.0, 600.0));
        assert!(!host.escritorio_alcanza(-400.0, 600.0));
    }
}
