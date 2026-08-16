//! Las superficies que aparecen y desaparecen: el menú, el calendario y el
//! launchpad.
//!
//! Son la tercera clase de superficie del shell. El panel y el dock están
//! siempre y tienen tamaño fijo; estas nacen al pulsar algo, se colocan
//! respecto a quien las abrió y se van al pulsar fuera o al pulsar Esc.
//!
//! # Por qué un enum y no otro trait
//!
//! Son tres, se conocen todas en tiempo de compilación y cada una tiene una
//! forma distinta de posicionarse y de responder al teclado. Un trait aquí
//! sería una capa de indirección para tres casos cerrados; el `match` dice lo
//! que hay sin esconderlo. El trait está donde sí hace falta —los widgets del
//! panel, que sí se añaden desde fuera— y aquí no.
//!
//! # Anclaje
//!
//! Cada emergente dice **dónde** quiere salir con [`Ancla`], en píxeles
//! lógicos. El compositor la traduce a la pantalla; el shell no sabe dónde
//! empieza la pantalla ni cuánto mide, solo su relación con quien lo abrió.

mod acerca;
mod bluetooth;
mod calendario;
mod centro;
mod brillo;
mod energia;
mod lista;
mod menu_dock;
mod notificaciones;
mod red;
pub(crate) mod control;
mod launchpad;
mod sonido;
mod menu;

use crate::view::PanelElement;
use crate::Accion;

/// Dónde se coloca una superficie emergente.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ancla {
    /// Colgando del panel, con su borde izquierdo en esa `x` lógica. Es lo que
    /// quiere el menú: sale justo debajo del logo.
    BajoElPanel { x: f32 },
    /// Colgando del panel, centrada bajo el widget que la abre. Es lo que
    /// quieren los estados de la derecha: el emergente del volumen sale bajo
    /// su icono, no en un sitio fijo. El shell resuelve el nombre con las
    /// zonas del panel, que son las mismas que deciden a quién va el clic.
    BajoWidget(&'static str),
    /// Justo encima del dock, centrada en esa `x` **relativa al dock**. Es el
    /// menú del clic derecho: sale sobre el icono que lo abrió.
    SobreElDock { x: f32 },
    /// En el centro de la pantalla. El launchpad.
    Centrada,
}

/// Qué hacer con una tecla que llega a una emergente.
pub enum Tecla {
    /// La emergente se cierra.
    Cerrar,
    /// La ha consumido y hay que repintar.
    Consumida,
    /// No le interesa; que siga su camino.
    Ignorada,
    /// La ha consumido y además pide algo al compositor. Es Intro sobre una
    /// aplicación del launchpad: consumir la tecla y lanzar son lo mismo.
    Hacer(Accion),
}

pub use menu_dock::Objetivo;

pub enum Emergente {
    Menu(menu::Menu),
    Calendario(calendario::Calendario),
    Sonido(sonido::Sonido),
    Brillo(brillo::Brillo),
    Energia(energia::Energia),
    MenuDock(menu_dock::MenuDock),
    Acerca(acerca::Acerca),
    Centro(centro::Centro),
    Notificaciones(notificaciones::Notificaciones),
    Red(red::Red),
    Bluetooth(bluetooth::Bluetooth),
    Launchpad(launchpad::Launchpad),
}

impl Emergente {
    pub fn menu() -> Self {
        Self::Menu(menu::Menu::new())
    }

    pub fn calendario() -> Self {
        Self::Calendario(calendario::Calendario::new())
    }

    pub fn sonido() -> Self {
        Self::Sonido(sonido::Sonido::new())
    }

    pub fn brillo() -> Self {
        Self::Brillo(brillo::Brillo::new())
    }

    pub fn energia() -> Self {
        Self::Energia(energia::Energia::new())
    }

    pub fn acerca() -> Self {
        Self::Acerca(acerca::Acerca::new())
    }

    pub fn centro() -> Self {
        Self::Centro(centro::Centro::new())
    }

    pub fn notificaciones() -> Self {
        Self::Notificaciones(notificaciones::Notificaciones::new())
    }

    pub fn red() -> Self {
        Self::Red(red::Red::new())
    }

    pub fn bluetooth() -> Self {
        Self::Bluetooth(bluetooth::Bluetooth::new())
    }

    pub fn menu_dock(objetivo: menu_dock::Objetivo) -> Self {
        Self::MenuDock(menu_dock::MenuDock::new(objetivo))
    }

    /// `pantalla` es el tamaño **lógico** de la pantalla: el launchpad la
    /// ocupa entera.
    pub fn launchpad(pantalla: (f32, f32)) -> Self {
        Self::Launchpad(launchpad::Launchpad::new(pantalla))
    }

    /// Nombre para las trazas.
    pub fn nombre(&self) -> &'static str {
        match self {
            Self::Menu(_) => "menu",
            Self::Calendario(_) => "calendario",
            Self::Sonido(_) => "sonido",
            Self::Brillo(_) => "brillo",
            Self::Energia(_) => "energia",
            Self::MenuDock(_) => "menu-dock",
            Self::Acerca(_) => "acerca",
            Self::Centro(_) => "centro",
            Self::Notificaciones(_) => "notificaciones",
            Self::Red(_) => "red",
            Self::Bluetooth(_) => "bluetooth",
            Self::Launchpad(_) => "launchpad",
        }
    }

    /// Tamaño lógico.
    pub fn size(&self) -> (f32, f32) {
        match self {
            Self::Menu(m) => m.size(),
            Self::Calendario(c) => c.size(),
            Self::Sonido(s) => s.size(),
            Self::Brillo(b) => b.size(),
            Self::Energia(e) => e.size(),
            Self::MenuDock(m) => m.size(),
            Self::Acerca(a) => a.size(),
            Self::Centro(c) => c.size(),
            Self::Notificaciones(n) => n.size(),
            Self::Red(r) => r.size(),
            Self::Bluetooth(b) => b.size(),
            Self::Launchpad(l) => l.size(),
        }
    }

    pub fn ancla(&self) -> Ancla {
        match self {
            Self::Menu(m) => m.ancla(),
            Self::Calendario(c) => c.ancla(),
            Self::Sonido(s) => s.ancla(),
            Self::Brillo(b) => b.ancla(),
            Self::Energia(e) => e.ancla(),
            Self::MenuDock(m) => m.ancla(),
            Self::Acerca(a) => a.ancla(),
            Self::Centro(c) => c.ancla(),
            Self::Notificaciones(n) => n.ancla(),
            Self::Red(r) => r.ancla(),
            Self::Bluetooth(b) => b.ancla(),
            Self::Launchpad(l) => l.ancla(),
        }
    }

    /// Relee lo suyo. `true` si lo que enseña ha cambiado.
    ///
    /// Solo las tarjetas que dependen de algo de fuera —las redes a la vista,
    /// los dispositivos emparejados— y solo mientras están abiertas: cerrada,
    /// una emergente no existe, así que esto no es un sondeo en reposo.
    pub fn refrescar(&mut self) -> bool {
        match self {
            Self::Red(r) => r.refrescar(),
            Self::Bluetooth(b) => b.refrescar(),
            _ => false,
        }
    }

    /// ¿Tiene una animación propia en marcha dentro de su buffer?
    ///
    /// No es la de entrada y salida, que la lleva el compositor moviendo la
    /// superficie: es el contenido cambiando —la rejilla del launchpad
    /// pasando de página— y obliga a repintar el buffer en cada fotograma.
    pub fn animando(&self) -> bool {
        match self {
            Self::Launchpad(l) => l.animando(),
            _ => false,
        }
    }

    /// ¿Ocupa la pantalla entera?
    ///
    /// Solo el launchpad. Trae dos consecuencias: el fondo se desenfoca —sin
    /// eso el escritorio se lee a través del velo y compite con los iconos— y
    /// el panel no se dibuja, porque una superficie que tapa la pantalla no
    /// puede tener una barra por encima.
    pub fn tapa_la_pantalla(&self) -> bool {
        matches!(self, Self::Launchpad(_))
    }

    /// El velo a pantalla completa que va detrás, si la emergente lo quiere.
    /// Lo pinta el compositor con un color sólido, que en la GPU es gratis.
    pub fn velo(&self) -> Option<iced_core::Color> {
        match self {
            Self::Launchpad(l) => Some(l.velo()),
            Self::Acerca(a) => Some(a.velo()),
            _ => None,
        }
    }

    /// El realce de lo señalado: rectángulo **relativo a la emergente** y si
    /// lleva marco de acento. Va en su propia superficie para que mover el
    /// ratón no cueste un repintado.
    pub fn realce(&self) -> Option<(iced_core::Rectangle, bool)> {
        match self {
            Self::Launchpad(l) => l.realce(),
            _ => None,
        }
    }

    pub fn view(&self) -> PanelElement<'_> {
        match self {
            Self::Menu(m) => m.view(),
            Self::Calendario(c) => c.view(),
            Self::Sonido(s) => s.view(),
            Self::Brillo(b) => b.view(),
            Self::Energia(e) => e.view(),
            Self::MenuDock(m) => m.view(),
            Self::Acerca(a) => a.view(),
            Self::Centro(c) => c.view(),
            Self::Notificaciones(n) => n.view(),
            Self::Red(r) => r.view(),
            Self::Bluetooth(b) => b.view(),
            Self::Launchpad(l) => l.view(),
        }
    }

    /// ¿Dibuja lo señalado en una superficie aparte?
    ///
    /// Si es que sí, mover el ratón por encima no repinta su buffer.
    pub fn usa_realce(&self) -> bool {
        matches!(self, Self::Launchpad(_))
    }

    /// Mueve el puntero por encima, en coordenadas lógicas relativas a la
    /// esquina superior izquierda de la emergente. `None` = está fuera.
    /// Devuelve `true` si hay que repintar.
    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        match self {
            Self::Menu(m) => m.puntero(punto),
            Self::Calendario(c) => c.puntero(punto),
            Self::Sonido(s) => s.puntero(punto),
            Self::Brillo(b) => b.puntero(punto),
            Self::Energia(e) => e.puntero(punto),
            Self::MenuDock(m) => m.puntero(punto),
            Self::Acerca(a) => a.puntero(punto),
            Self::Centro(c) => c.puntero(punto),
            Self::Notificaciones(n) => n.puntero(punto),
            Self::Red(r) => r.puntero(punto),
            Self::Bluetooth(b) => b.puntero(punto),
            Self::Launchpad(l) => l.puntero(punto),
        }
    }

    /// Pulsación, en las mismas coordenadas. `None` si ahí no hay nada que
    /// hacer.
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        match self {
            Self::Menu(m) => m.pulsar(x, y),
            Self::Calendario(c) => c.pulsar(x, y),
            Self::Sonido(s) => s.pulsar(x, y),
            Self::Brillo(b) => b.pulsar(x, y),
            Self::Energia(e) => e.pulsar(x, y),
            Self::MenuDock(m) => m.pulsar(x, y),
            Self::Acerca(a) => a.pulsar(x, y),
            Self::Centro(c) => c.pulsar(x, y),
            Self::Notificaciones(n) => n.pulsar(x, y),
            Self::Red(r) => r.pulsar(x, y),
            Self::Bluetooth(b) => b.pulsar(x, y),
            Self::Launchpad(l) => l.pulsar(x, y),
        }
    }

    /// Desplazamiento de rueda o touchpad, en píxeles lógicos. `true` si ha
    /// cambiado algo y hay que repintar.
    pub fn desplazar(&mut self, dx: f32, dy: f32) -> bool {
        match self {
            Self::Launchpad(l) => l.desplazar(dx, dy),
            Self::Sonido(s) => s.desplazar(dx, dy),
            Self::Brillo(b) => b.desplazar(dx, dy),
            // El menú y el calendario caben enteros; no hay nada que desplazar.
            _ => false,
        }
    }

    /// El botón se ha soltado. `true` si hay que repintar.
    ///
    /// Solo le importa a lo que se arrastra: hasta el deslizador del volumen,
    /// ninguna emergente necesitaba saber cuándo acababa una pulsación.
    pub fn soltar(&mut self) -> bool {
        match self {
            Self::Sonido(s) => s.soltar(),
            Self::Brillo(b) => b.soltar(),
            Self::Centro(c) => c.soltar(),
            _ => false,
        }
    }

    /// ¿Tiene algo agarrado? Mientras lo tenga, el puntero le sigue llegando
    /// aunque se salga de la tarjeta: al llevar el volumen al máximo de un
    /// tirón, la mano se sale y el deslizador tiene que seguir.
    pub fn agarrado(&self) -> bool {
        match self {
            Self::Sonido(s) => s.agarrado(),
            Self::Brillo(b) => b.agarrado(),
            Self::Centro(c) => c.agarrado(),
            _ => false,
        }
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match self {
            Self::Menu(m) => m.tecla(tecla),
            Self::Calendario(c) => c.tecla(tecla),
            Self::Sonido(s) => s.tecla(tecla),
            Self::Brillo(b) => b.tecla(tecla),
            Self::Energia(e) => e.tecla(tecla),
            Self::MenuDock(m) => m.tecla(tecla),
            Self::Acerca(a) => a.tecla(tecla),
            Self::Centro(c) => c.tecla(tecla),
            Self::Notificaciones(n) => n.tecla(tecla),
            Self::Red(r) => r.tecla(tecla),
            Self::Bluetooth(b) => b.tecla(tecla),
            Self::Launchpad(l) => l.tecla(tecla),
        }
    }
}
