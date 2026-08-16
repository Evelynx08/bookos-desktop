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
    /// La del aviso de volumen y brillo, mientras dura.
    osd: Option<Surface>,
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
    pub fn new(
        width: u32,
        height: u32,
        scale: f32,
        config: Option<bookos_shell::Config>,
    ) -> Self {
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
            realce: None,
            abierta_en: None,
            cerrando_en: None,
            realce_desde: None,
            screen: (width as i32, height as i32),
            panel_barra: EstadoBarra::new(),
            dock_barra: EstadoBarra::new(),
            bloqueo: None,
            osd: None,
            dead: false,
        };
        host.place_dock();
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
        // El dock cambia de sitio con el modo: pegado si es fijo, flotando si
        // esquiva.
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
    pub fn zonas_barras(&self) -> Vec<(Barra, smithay::utils::Rectangle<i32, smithay::utils::Logical>)> {
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
                    ((pw as f64 / escala).round() as i32, self.shell.panel_height()).into(),
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
                self.dock.buffer_alto() + bookos_shell::DOCK_MARGIN as f64 * self.shell.scale() as f64
            }
        };
        barra.fraccion() as f64 * recorrido
    }

    fn place_dock(&mut self) {
        let pegado = self.dock_barra.modo == Visibilidad::Siempre;
        if self.guard("dock pegado", |host| host.shell.dock_pegado(pegado)) == Some(true) {
            self.guard("pintar dock", |host| {
                let buffer = &mut host.dock.buffer;
                paint(buffer, |buf| host.shell.draw_dock(buf));
            });
        }
        let (dw, dh) = self.shell.dock_buffer_size();
        // Fijo, pegado al borde; esquivando ventanas, flotando sobre él. No es
        // un capricho: un dock que se aparta tiene que verse como algo que va
        // y viene, y uno que está siempre es parte del marco de la pantalla.
        let margin = if self.dock_barra.modo == Visibilidad::Siempre {
            0.0
        } else {
            bookos_shell::DOCK_MARGIN as f64 * self.shell.scale() as f64
        };
        let x = ((self.screen.0 - dw as i32) as f64 / 2.0).max(0.0);
        let y = (self.screen.1 - dh as i32) as f64 - margin;
        self.dock.location = (x, y.max(0.0)).into();
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

    /// Las zonas que llevan fondo esmerilado, en píxeles **físicos**: el panel
    /// y el dock.
    ///
    /// En físicos porque el desenfoque copia del framebuffer, que es físico, y
    /// convertir dos veces por frame para volver al mismo sitio solo añade
    /// oportunidades de equivocarse con el redondeo.
    pub fn zonas_cristal(
        &self,
    ) -> Vec<(smithay::utils::Rectangle<i32, smithay::utils::Physical>, f32, f32)> {
        use smithay::utils::Rectangle;
        if self.dead {
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
        let mut zonas = Vec::with_capacity(1);
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
        // Una barra apartada no recibe clics: si no, el borde de la pantalla se
        // tragaría pulsaciones destinadas a la ventana que hay debajo.
        if !self.a_la_vista(Barra::Panel) && !self.a_la_vista(Barra::Dock) && !self.shell.hay_emergente() {
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
        let Some(((w, h), ancla)) = self.shell.emergente_geometria() else {
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
        let rehacer = nueva || self.emergente.as_ref().is_some_and(|s| s.logical != (w, h).into());
        if rehacer {
            self.emergente = Some(Surface::new(buffer, (w, h)));
            self.colocar_emergente(ancla, (w, h));
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
                (centro - w as f64 / 2.0, self.shell.panel_height() as f64 + SEPARACION)
            }
            // Sobre el dock y centrada en el icono, sin salirse por los lados.
            Ancla::SobreElDock { x } => {
                let (dx, dy, _, _) = self.dock_rect();
                let ancho = self.screen.0 as f64 / escala;
                let izq = (dx + x as f64 - w as f64 / 2.0).clamp(SEPARACION, (ancho - w as f64 - SEPARACION).max(SEPARACION));
                (izq, dy - h as f64 - SEPARACION)
            }
            Ancla::Centrada => (
                (self.screen.0 as f64 / escala - w as f64) / 2.0,
                (self.screen.1 as f64 / escala - h as f64) / 2.0,
            ),
        };
        // Sin esto, una emergente anclada cerca del borde derecho se saldría de
        // la pantalla y se vería cortada.
        let max_x = (self.screen.0 as f64 / escala - w as f64).max(0.0);
        let max_y = (self.screen.1 as f64 / escala - h as f64).max(0.0);
        if let Some(surface) = self.emergente.as_mut() {
            surface.location = (x.clamp(0.0, max_x) * escala, y.clamp(0.0, max_y) * escala).into();
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
        self.cerrando_en = Some(std::time::Instant::now());
        true
    }

    /// Se llama cuando ya no hay animación en curso: suelta lo que quedaba.
    pub fn fin_animacion(&mut self) {
        self.realce_desde = None;
        self.recoger_cerrada();
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

    pub fn abrir_acerca(&mut self) {
        if self.dead {
            return;
        }
        self.guard("acerca", |host| {
            host.shell.abrir(bookos_shell::Emergente::acerca());
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
        let pantalla = (
            self.screen.0 as f32 / escala,
            self.screen.1 as f32 / escala,
        );
        self.guard("abrir launchpad", |host| {
            host.shell
                .abrir(bookos_shell::Emergente::launchpad(pantalla));
        });
        self.sincronizar_emergente();
    }

    /// Le dice al dock qué aplicaciones tienen ventana abierta.
    ///
    /// Los `app_id` los reúne el compositor, que es quien conoce el `Space`. El
    /// shell no lo mira por su cuenta: la frontera con Smithay está puesta a
    /// propósito para que cambiar de toolkit no obligue a tocar el compositor.
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
            self.dock = Surface::new(self.shell.dock_buffer_size(), self.shell.dock_logical_size());
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
            self.sincronizar_emergente();
        }
        // Consumido aunque no haya cambiado de página: el gesto es del
        // launchpad de principio a fin.
        true
    }

    /// Qué hacer al pulsar en un punto lógico. `None` si ahí no hay nada.
    pub fn pulsar(&mut self, x: f64, y: f64) -> Option<Accion> {
        if self.dead {
            return None;
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
                        .guard("pulsar emergente", |host| host.shell.emergente_pulsar(ex, ey))
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
        surface.location = (
            (self.screen.0 as f64 - w as f64) / 2.0,
            self.screen.1 as f64 - bookos_shell::osd::MARGEN_INFERIOR as f64 * escala,
        )
            .into();
        paint(&mut surface.buffer, |buf| self.shell.draw_osd(buf));
        self.osd = Some(surface);
    }

    /// Echa el bloqueo y prepara su superficie, del tamaño de la pantalla.
    pub fn bloquear(&mut self, hora: String, pantalla: (f32, f32)) {
        self.guard("bloquear", |host| {
            host.shell.bloquear(hora, pantalla);
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
        self.dock = Surface::new(self.shell.dock_buffer_size(), self.shell.dock_logical_size());
        self.place_dock();
        Some(lista)
    }

    /// El nombre de la emergente abierta. Para el autotest.
    pub fn emergente_nombre(&self) -> Option<&'static str> {
        (!self.dead).then(|| self.shell.emergente_nombre()).flatten()
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
    pub fn soltar(&mut self) {
        if self.dead {
            return;
        }
        if self.guard("soltar", |host| host.shell.soltar()) == Some(true) {
            self.sincronizar_emergente();
        }
    }

    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        if self.dead || (self.screen == (width as i32, height as i32) && self.shell.scale() == scale)
        {
            return;
        }
        self.screen = (width as i32, height as i32);
        self.guard("resize", |host| {
            let logical_width = (width as f32 / scale).round().max(1.0) as u32;
            host.shell.resize(logical_width, scale);
            host.panel =
                Surface::new(host.shell.panel_buffer_size(), host.shell.panel_logical_size());
            host.dock =
                Surface::new(host.shell.dock_buffer_size(), host.shell.dock_logical_size());
            host.place_dock();
        });
    }

    /// Relee los datos y repinta lo que haya cambiado. Devuelve `true` si hay
    /// algo nuevo en pantalla, es decir, si el compositor tiene que redibujar.
    pub fn refresh(&mut self) -> bool {
        if self.dead {
            return false;
        }
        self.guard("refresh", |host| {
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
        .unwrap_or(false)
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
    fn animacion(&self) -> (f32, f32) {
        use bookos_shell::tema;

        // Yéndose: la misma duración, pero con la curva suave y no con el
        // muelle. Un rebote al salir deja la emergente dando un respingo justo
        // cuando el usuario ya ha decidido que no la quiere.
        if let Some(desde) = self.cerrando_en {
            let t = tema::C_SUAVE.eval(tema::fraccion(desde.elapsed(), Self::SALIDA));
            return (1.0 - t, 1.0 + 0.05 * t);
        }

        let Some(desde) = self.abierta_en else {
            return (1.0, 1.0);
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
        (alfa, 1.05 - 0.05 * escala)
    }

    /// Lo que tarda una emergente en entrar y en irse: la duración de popover
    /// del sistema de diseño. La entrada y la salida ya no se separan —eran 280
    /// y 180— porque la tabla da un único valor para "aparición de popover" y
    /// dos duraciones distintas para el mismo gesto son justo lo que hace que un
    /// escritorio se note cosido a mano.
    const ENTRADA: std::time::Duration = bookos_shell::tema::D_POPOVER;
    const SALIDA: std::time::Duration = bookos_shell::tema::D_POPOVER;
    /// Lo que tarda el realce en deslizarse de una celda a otra. Es el token de
    /// hover del sistema: mover el resaltado con el cursor es un hover.
    const DESLIZ: std::time::Duration = bookos_shell::tema::D_HOVER;

    /// ¿Hay una animación en curso? Mientras la haya, el compositor tiene que
    /// seguir dibujando aunque no pase nada más.
    pub fn animando(&self) -> bool {
        if self.shell.emergente_animando() {
            return true;
        }
        // Las tres condiciones miran si la animación **sigue en marcha**, no si
        // existe. `cerrando_en.is_some()` a secas cerraba un ciclo: la salida no
        // terminaba nunca, `fin_animacion` —que es quien limpia— solo se llama
        // cuando esto devuelve false, y el compositor se quedaba dibujando a
        // toda velocidad para siempre después de cerrar cualquier emergente.
        // Eso era lo que se veía como que el escritorio iba a tirones, y no
        // paraba hasta reiniciar la sesión.
        self.abierta_en
            .is_some_and(|t| t.elapsed() < Self::ENTRADA)
            || self.cerrando_en.is_some_and(|t| t.elapsed() < Self::SALIDA)
            || self
                .realce_desde
                .is_some_and(|(_, t)| t.elapsed() < Self::DESLIZ)
    }

    /// El velo a pantalla completa que pide la emergente, si lo pide.
    pub fn velo(&self) -> Option<smithay::backend::renderer::element::solid::SolidColorRenderElement>
    {
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
        enum Papel {
            /// Se desvanece con su propio reloj.
            Aviso,
            /// Entra y sale con la animación de la emergente.
            Emergente,
            /// Igual, y además se desliza al cambiar de fila.
            Realce,
            /// Se aparta por su borde al esquivar ventanas.
            Barra(Barra),
        }

        let (alfa_emergente, zoom) = self.animacion();
        let alfa_osd = self.shell.osd_alfa();
        let superficies = self
            .osd
            .iter()
            .map(|s| (s, Papel::Aviso))
            .chain(self.emergente.iter().map(|s| (s, Papel::Emergente)))
            .chain(self.realce.iter().map(|s| (s, Papel::Realce)))
            // Con el launchpad abierto no se dibujan las barras: ocupa la
            // pantalla entera y el panel por encima delataba que lo de debajo
            // sigue ahí, además de tapar la primera fila de iconos.
            .chain(
                [
                    (&self.panel, Papel::Barra(Barra::Panel)),
                    (&self.dock, Papel::Barra(Barra::Dock)),
                ]
                .into_iter()
                .filter(|_| !self.shell.emergente_tapa_la_pantalla()),
            );

        superficies
            .filter_map(|(surface, papel)| {
                let (alfa, zoom) = match papel {
                    Papel::Aviso => (alfa_osd, 1.0),
                    Papel::Emergente | Papel::Realce => (alfa_emergente, zoom),
                    Papel::Barra(_) => (1.0, 1.0),
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
                    _ => surface.location,
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
