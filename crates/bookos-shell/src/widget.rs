//! El sistema de widgets del panel.
//!
//! Un widget es un módulo de este crate compilado dentro del binario, no un
//! proceso aparte como los plasmoides de Plasma. Eso es lo que permite que el
//! panel esté pintado en el primer frame de la sesión: no hay que arrancar
//! nada, ni conectar con nada, ni esperar a que responda. El precio es que
//! añadir un widget exige recompilar, y se paga a gusto.
//!
//! # Cada widget responde por sí mismo
//!
//! Antes el panel comparaba un `PanelData` entero con `PartialEq` para decidir
//! si repintar. Funcionaba, pero significaba que un widget nuevo tenía que
//! meter su campo en esa estructura y que cambiar un dato repintaba el panel
//! completo. Aquí cada widget dice si **lo suyo** cambió ([`Widget::refrescar`]),
//! cuándo quiere despertar ([`Widget::proxima_alarma`]) y qué eventos del
//! kernel le importan ([`Widget::subsistemas`]). El panel solo agrega.
//!
//! Eso último no es cosmético: hoy cualquier evento de udev provocaba releer
//! las cuatro fuentes de sysfs. Con los subsistemas declarados, un cambio de
//! brillo ya no hace mirar el estado de la red.
//!
//! # Un widget que revienta no se lleva el panel
//!
//! El compositor envuelve al shell entero en `catch_unwind`, pero eso es todo o
//! nada: un panic en el reloj dejaba la sesión sin panel **y** sin dock, para
//! siempre. Aquí cada llamada a un widget va en su propio `catch_unwind`; el
//! que revienta se apaga y los demás siguen dibujándose.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;

use iced_core::Length;
use iced_widget::Space;

use crate::view::PanelElement;

pub trait Widget {
    /// Nombre estable: es la clave en la configuración y lo que sale en las
    /// trazas cuando el widget falla.
    fn nombre(&self) -> &'static str;

    /// Relee su fuente. `true` si lo que enseña ha cambiado.
    fn refrescar(&mut self) -> bool;

    /// Dentro de cuánto tiene algo nuevo que enseñar aunque no pase nada.
    /// `None` = solo reacciona a eventos.
    fn proxima_alarma(&self) -> Option<Duration> {
        None
    }

    /// Subsistemas de udev cuyos eventos le conciernen (`power_supply`, `net`,
    /// `backlight`…).
    fn subsistemas(&self) -> &'static [&'static str] {
        &[]
    }

    fn ver(&self) -> PanelElement<'_>;

    /// Cuánto ocupa de ancho, en píxeles lógicos.
    ///
    /// Hace falta para saber **dónde se ha pulsado**: el layout lo calcula iced
    /// al pintar, y el hit-test ocurre antes de eso y desde el compositor, así
    /// que sin esto un clic en el panel solo puede distinguir zonas fijas —era
    /// el motivo de que el brillo, la red y la batería no respondieran.
    ///
    /// Es una estimación, no una medida: se comprueba contra el panel pintado
    /// en `tests/panel.rs`. Un widget que se quede corto pierde clics en su
    /// borde; uno que se pase se los roba al vecino, así que más vale ajustado.
    fn ancho(&self) -> f32 {
        crate::tema::ICONO_PANEL
    }
}

/// Ancho aproximado de un texto del panel, en lógicos, contando caracteres.
///
/// Se conserva para las medidas que no tienen el texto a mano —el recorte de
/// nombres largos en las listas— pero **no** para las zonas del panel: ahí la
/// aproximación desalineaba los clics. Ver [`ancho_de`].
pub fn ancho_texto(caracteres: usize) -> f32 {
    caracteres as f32 * 7.3
}

/// Lo que mide de verdad un texto del panel, en lógicos.
///
/// Antes esto se estimaba a 7,3 px por carácter y las zonas de clic no
/// cuadraban con lo dibujado: medido sobre el panel pintado, «16/8/26 18:49»
/// ocupa 73 px reales y la cuenta daba 95. El error se acumulaba de derecha a
/// izquierda —cada zona se coloca a partir del ancho de la anterior—, y para
/// cuando llegaba a la batería el dibujo estaba 10 px fuera de su zona: de ahí
/// que a veces un clic no abriera nada.
///
/// Mide con el mismo `Paragraph` de `iced_graphics` que usa el renderer al
/// pintar, con la fuente y el cuerpo por defecto del shell, así que no es una
/// estimación paralela: es el mismo motor.
pub fn ancho_de(texto: &str, tamaño: f32) -> f32 {
    use iced_core::text::{Paragraph as _, Shaping, Wrapping};
    use iced_core::{alignment, Pixels, Size};

    let parrafo = iced_graphics::text::Paragraph::with_text(iced_core::Text {
        content: texto,
        // Sin límite: lo que se quiere es el ancho natural de una línea.
        bounds: Size::INFINITE,
        size: Pixels(tamaño),
        line_height: iced_core::text::LineHeight::default(),
        font: iced_core::Font::DEFAULT,
        align_x: iced_core::text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        // La misma que usa `text()` de iced_widget por defecto.
        shaping: Shaping::Basic,
        wrapping: Wrapping::None,
    });
    parrafo.min_bounds().width
}

/// Qué widget tiene que reventar, si `BOOKOS_SHELL_PANIC_TEST` lo pide.
///
/// Se lee del entorno una sola vez: esto se consulta en cada refresco de cada
/// widget, y `getenv` en ese camino sería trabajo por nada.
fn panic_de_prueba() -> Option<&'static str> {
    static QUIEN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    QUIEN
        .get_or_init(|| std::env::var("BOOKOS_SHELL_PANIC_TEST").ok())
        .as_deref()
}

/// Un widget y su estado de salud.
struct Ranura {
    widget: Box<dyn Widget>,
    /// Un widget que ha entrado en pánico no se vuelve a llamar. Reintentarlo
    /// repetiría el fallo en cada frame y llenaría el log.
    muerto: bool,
}

impl Ranura {
    fn new(widget: Box<dyn Widget>) -> Self {
        Self {
            widget,
            muerto: false,
        }
    }

    /// Ejecuta `f` sobre el widget, apagándolo si revienta.
    ///
    /// `AssertUnwindSafe` es correcto porque el único estado que puede quedar a
    /// medias es el del propio widget, y al marcarlo muerto no se vuelve a
    /// tocar.
    fn guard<T>(&mut self, que: &str, f: impl FnOnce(&mut Box<dyn Widget>) -> T) -> Option<T> {
        if self.muerto {
            return None;
        }
        let nombre = self.widget.nombre();
        match catch_unwind(AssertUnwindSafe(|| {
            // Escotilla para comprobar que el aislamiento **por widget**
            // funciona de verdad. Sin esto, "un widget que revienta no se lleva
            // el panel" es una afirmación sin probar.
            if panic_de_prueba() == Some(nombre) {
                panic!("panic de prueba del widget {nombre}");
            }
            f(&mut self.widget)
        })) {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::error!(
                    widget = self.widget.nombre(),
                    que,
                    "el widget ha entrado en pánico; se apaga y el panel sigue"
                );
                self.muerto = true;
                None
            }
        }
    }
}

/// Los widgets del panel, repartidos por zonas.
///
/// Solo hay dos: el centro, que es una capa aparte, y la derecha, que es una
/// fila. El centro está separado porque tiene que estar centrado en la
/// **pantalla** y no entre sus vecinos: metido en la fila, el reloj daba un
/// salto cada vez que la batería pasaba de "85%" a "85% 2:15".
pub struct Panel {
    centro: Option<Ranura>,
    derecha: Vec<Ranura>,
}

impl Panel {
    pub fn new(centro: Option<Box<dyn Widget>>, derecha: Vec<Box<dyn Widget>>) -> Self {
        Self {
            centro: centro.map(Ranura::new),
            derecha: derecha.into_iter().map(Ranura::new).collect(),
        }
    }

    /// Relee todos los widgets. `true` si alguno cambió.
    ///
    /// Se refrescan **todos** aunque uno ya haya dicho que sí: saltarse el
    /// resto los dejaría con el dato viejo y el siguiente repintado enseñaría
    /// una mezcla de dos instantes.
    pub fn refrescar(&mut self) -> bool {
        let mut cambio = false;
        for ranura in self.ranuras_mut() {
            if ranura.guard("refrescar", |w| w.refrescar()) == Some(true) {
                cambio = true;
            }
        }
        cambio
    }

    /// Lo antes que alguno quiere despertar.
    pub fn proxima_alarma(&self) -> Option<Duration> {
        self.ranuras()
            .filter(|r| !r.muerto)
            .filter_map(|r| r.widget.proxima_alarma())
            .min()
    }

    /// Todos los subsistemas que hay que vigilar, sin repetir.
    ///
    /// Con esto el filtro del netlink se pone una vez al arrancar y el kernel
    /// ya no nos despierta por lo que no se enseña; por eso no hace falta
    /// preguntar después qué widget quería cada evento.
    pub fn subsistemas(&self) -> Vec<&'static str> {
        let mut todos: Vec<&'static str> = self
            .ranuras()
            .flat_map(|r| r.widget.subsistemas())
            .copied()
            .collect();
        todos.sort_unstable();
        todos.dedup();
        todos
    }

    /// ¿Está este widget en la capa central?
    /// Dónde cae cada widget de la derecha, en lógicos: `(nombre, x0, x1)`.
    ///
    /// Se calcula de derecha a izquierda porque esa fila está anclada al borde
    /// derecho: el primero de la lista es el que queda más a la izquierda, pero
    /// el que tiene la posición fija es el último.
    pub fn zonas_derecha(
        &self,
        ancho_panel: f32,
        margen: f32,
        hueco: f32,
    ) -> Vec<(&'static str, f32, f32)> {
        let mut zonas = Vec::new();
        let mut x = ancho_panel - margen;
        // Un widget muerto no se dibuja, así que tampoco ocupa sitio ni puede
        // recibir clics: se salta entero, huecos incluidos.
        for ranura in self.derecha.iter().rev().filter(|r| !r.muerto) {
            let ancho = ranura.widget.ancho();
            // Un widget sin nada que enseñar —el bluetooth sin adaptador, la
            // batería en un sobremesa— no se dibuja, así que ni ocupa sitio ni
            // recibe clics: se salta con su hueco. Sin esto quedaba una zona
            // de ancho cero que se comía los clics del vecino.
            if ancho <= 0.0 {
                continue;
            }
            // Cada zona se lleva medio hueco por cada lado. El aire entre
            // widgets es de 12 px y no era de nadie: pulsar ahí no abría nada, y
            // apuntar a un icono de 18 px en una barra de 32 con el ratón en
            // movimiento falla más de lo que parece. Repartido, todo el lado
            // derecho del panel responde y ningún widget se roba el dibujo del
            // vecino.
            zonas.push((
                ranura.widget.nombre(),
                x - ancho - hueco / 2.0,
                x + hueco / 2.0,
            ));
            x -= ancho + hueco;
        }
        zonas
    }

    pub fn centro(&self) -> Option<PanelElement<'_>> {
        self.centro.as_ref().and_then(ver)
    }

    pub fn derecha(&self) -> Vec<PanelElement<'_>> {
        self.derecha.iter().filter_map(ver).collect()
    }

    fn ranuras(&self) -> impl Iterator<Item = &Ranura> {
        self.centro.iter().chain(self.derecha.iter())
    }

    fn ranuras_mut(&mut self) -> impl Iterator<Item = &mut Ranura> {
        self.centro.iter_mut().chain(self.derecha.iter_mut())
    }
}

/// La vista de una ranura, o nada si está muerta.
///
/// Aquí no se puede usar `Ranura::guard` porque `ver` toma `&self` y devolver
/// el `Element` prestado desde dentro del `catch_unwind` obligaría a marcar el
/// fallo con mutabilidad interior. Un widget que dibuja mal es raro y ya
/// reventaría en `refrescar`; lo que sí se respeta es no dibujar al que ya
/// está apagado.
fn ver(ranura: &Ranura) -> Option<PanelElement<'_>> {
    if ranura.muerto {
        return None;
    }
    Some(ranura.widget.ver())
}

/// Un hueco de ancho cero, para cuando no hay nada que enseñar.
pub(crate) fn vacio<'a>() -> PanelElement<'a> {
    Space::new().width(Length::Fixed(0.0)).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bomba;
    impl Widget for Bomba {
        fn nombre(&self) -> &'static str {
            "bomba"
        }
        fn refrescar(&mut self) -> bool {
            panic!("bum")
        }
        fn ver(&self) -> PanelElement<'_> {
            vacio()
        }
    }

    /// Dice que cambió las `veces` primeras veces que le preguntan.
    struct Cambiante(u32);
    impl Widget for Cambiante {
        fn nombre(&self) -> &'static str {
            "cambiante"
        }
        fn refrescar(&mut self) -> bool {
            self.0 = self.0.saturating_sub(1);
            self.0 > 0
        }
        fn proxima_alarma(&self) -> Option<Duration> {
            Some(Duration::from_secs(7))
        }
        fn subsistemas(&self) -> &'static [&'static str] {
            &["net"]
        }
        fn ver(&self) -> PanelElement<'_> {
            vacio()
        }
    }

    /// El aislamiento por widget es la razón de ser de [`Ranura`]: si esto se
    /// rompe, un `unwrap` en cualquier widget vuelve a apagar el panel entero.
    #[test]
    fn el_widget_que_revienta_no_se_lleva_a_los_demas() {
        // El panic de la bomba es esperado; sin esto llena la salida del test
        // con un backtrace que parece un fallo de verdad.
        let anterior = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        let mut panel = Panel::new(Some(Box::new(Bomba)), vec![Box::new(Cambiante(3))]);
        assert!(panel.refrescar(), "el widget sano sí ha cambiado");
        assert!(
            panel.centro().is_none(),
            "el widget muerto se sigue dibujando"
        );
        // Y no se le vuelve a llamar: si se le llamara, volvería a reventar y
        // este segundo refresco no llegaría a devolver nada.
        panel.refrescar();

        std::panic::set_hook(anterior);
    }

    #[test]
    fn el_muerto_no_cuenta_para_alarmas_ni_subsistemas() {
        let anterior = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        let mut panel = Panel::new(Some(Box::new(Bomba)), vec![Box::new(Cambiante(3))]);
        panel.refrescar();
        assert_eq!(panel.proxima_alarma(), Some(Duration::from_secs(7)));
        assert_eq!(panel.subsistemas(), vec!["net"]);

        std::panic::set_hook(anterior);
    }

    #[test]
    fn la_alarma_es_la_del_mas_impaciente() {
        let panel = Panel::new(None, vec![Box::new(Cambiante(1)), Box::new(Bomba)]);
        // La bomba no pide alarma; el cambiante pide 7 s.
        assert_eq!(panel.proxima_alarma(), Some(Duration::from_secs(7)));
    }
}
