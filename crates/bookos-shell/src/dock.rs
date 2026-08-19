//! El dock: rectángulo redondeado con los lanzadores, abajo y centrado.
//!
//! La geometría sale del prototipo `bookos/prototypes/DockMagnify.qml`, para
//! que el dock en Rust sea el mismo dock que ya habías calibrado en QML: icono
//! de 50, hueco de 14, padding de 9 y esquinas de 22 (redondeado, no cápsula).
//!
//! El puntero ya llega: se señala el icono de debajo, se pulsa y se lanza, y un
//! punto marca las aplicaciones que tienen ventana.
//!
//! **La lente de aumento sigue sin estar**, y ya no es por falta de puntero.
//! `DockMagnify.qml` la define con `magnifyScale = 1.5` e `influence = 110 px`,
//! o sea que el icono señalado crece hasta 75 px y empuja a sus vecinos: el
//! buffer tendría que reservar ese crecimiento por arriba y por los lados, y el
//! fondo redondeado redimensionarse con él, que es rehacer la geometría que
//! ahora está calibrada contra el prototipo. Además el prototipo hace la
//! entrada y la salida suaves (`lensStrength`), y aquí eso significa un
//! temporizador repintando por fotograma — justo lo que este compositor evita.

use std::path::PathBuf;

use iced_core::{Border, Color, Length};
use iced_widget::{column, container, image as iced_image, row, svg, text, Space};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{PanelElement, ACENTO, TEXT};

pub const ICON: f32 = 50.0;
pub const GAP: f32 = 14.0;
pub const PAD: f32 = 9.0;
/// Separación entre el dock y el borde inferior de la pantalla.
pub const MARGIN: f32 = 12.0;

/// El `card` del sistema de diseño, translúcido: el dock flota sobre el
/// escritorio y opaco se vería pegado.
///
/// 0,30 por lo mismo que el panel: debajo va el cristal esmerilado que dibuja
/// el compositor, y con 0,78 encima apenas se distinguía del dock opaco.
fn fondo() -> Color {
    Color {
        a: 0.30,
        ..tema::card()
    }
}

/// El mismo fondo cuando el dock está pegado al borde. Más cuerpo que el
/// flotante: apoyado en el borde deja de leerse como algo que va y viene y pasa
/// a ser parte del marco de la pantalla, y con 0,30 se veía como si estuviese
/// despegado y transparente a la vez.
fn fondo_pegado() -> Color {
    Color {
        a: 0.55,
        ..tema::card()
    }
}

/// Alto de la banda del indicador de ventana abierta, bajo el icono.
const PUNTO: f32 = 7.0;

/// Un lanzador del dock.
pub struct DockItem {
    /// Lo que se ejecuta al pulsarlo.
    pub exec: String,
    pub label: String,
    /// Con qué `app_id` de Wayland se corresponde, para saber si ya está
    /// abierto. Ver [`Dock::set_abiertas`].
    app_id: String,
    icon: Option<Icono>,
    /// Con qué nombre se pidió el icono. Hace falta para volver a escribir la
    /// lista en la configuración al anclar o desanclar algo: sin él habría que
    /// adivinarlo del `exec`, y no siempre coinciden.
    icono_nombre: String,
    /// Tiene al menos una ventana en el escritorio.
    abierta: bool,
    /// Está fijada por el usuario. Los que no lo están salen en el dock solo
    /// mientras tienen ventana —como en macOS— y desaparecen al cerrarla; son
    /// también los que se pueden fijar con el clic derecho, porque hasta que
    /// una aplicación no se abre no hay dónde pulsar para anclarla.
    anclada: bool,
}

impl DockItem {
    /// Busca el icono en los temas del sistema. Si no aparece, el item se
    /// dibuja igual con una baldosa y su inicial: un icono que falta no puede
    /// dejar un hueco en el dock.
    pub fn new(exec: &str, label: &str, icon_name: &str, app_id: &str) -> Self {
        let icon = icono::cargar(icon_name);
        Self {
            exec: exec.to_string(),
            label: label.to_string(),
            app_id: app_id.to_string(),
            icono_nombre: icon_name.to_string(),
            anclada: true,
            icon,
            abierta: false,
        }
    }

    /// ¿Es este `app_id` de Wayland el de este lanzador?
    fn es_suya(&self, app_id: &str) -> bool {
        crate::mismo_programa(&self.app_id, app_id)
    }

    pub fn exec(&self) -> &str {
        &self.exec
    }

    pub fn icono_nombre(&self) -> &str {
        &self.icono_nombre
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// ¿Tiene ya alguna ventana abierta? Lo fija [`Dock::set_abiertas`].
    pub fn anclada(&self) -> bool {
        self.anclada
    }

    pub fn abierta(&self) -> bool {
        self.abierta
    }

    pub fn has_icon(&self) -> bool {
        self.icon.is_some()
    }
}

pub struct Dock {
    items: Vec<DockItem>,
    /// Índice del icono bajo el puntero, si lo hay.
    /// El icono señalado, con su placa entrando y saliendo.
    hover: tema::Realce,
    /// Apoyado en el borde inferior. Lo decide el compositor, que es quien sabe
    /// si el dock está fijo o esquivando ventanas.
    pegado: bool,
}

impl Dock {
    /// Ancla una aplicación o la desancla si ya estaba.
    ///
    /// El launchpad no se toca: es parte del escritorio, no un lanzador que se
    /// quita. Y lo anclado se añade al final, que es donde la mano espera
    /// encontrar lo último que puso.
    pub fn alternar_anclado(&mut self, app_id: &str, exec: &str, icono: &str) {
        if let Some(i) = self.indice_de(app_id) {
            // El launchpad no es una aplicación: quitarlo dejaría el dock sin
            // la única forma de abrir lo que no está en él.
            if self.items[i].exec == crate::config::LAUNCHPAD {
                return;
            }
            if self.items[i].anclada {
                // Desanclar algo que está abierto no lo borra del dock: la
                // ventana sigue ahí y su icono tiene que seguir enseñándola.
                if self.items[i].abierta {
                    self.items[i].anclada = false;
                } else {
                    self.items.remove(i);
                    self.hover.señalar(None);
                }
            } else {
                self.items[i].anclada = true;
            }
            return;
        }
        self.items.push(DockItem::new(exec, app_id, icono, app_id));
    }

    pub fn esta_anclada(&self, app_id: &str) -> bool {
        self.indice_de(app_id)
            .is_some_and(|i| self.items[i].anclada)
    }

    fn indice_de(&self, app_id: &str) -> Option<usize> {
        self.items
            .iter()
            .position(|i| crate::mismo_programa(i.app_id(), app_id) || i.app_id() == app_id)
    }

    /// La lista tal y como se escribe en `panel.conf`, para poder guardarla.
    pub fn como_configuracion(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|i| i.anclada)
            .map(|i| format!("{}:{}:{}:{}", i.exec, i.label, i.icono_nombre(), i.app_id()))
            .collect()
    }

    /// Los lanzadores que diga la configuración.
    pub fn from_config(specs: &[crate::config::Lanzador]) -> Self {
        let items = specs
            .iter()
            .map(|l| DockItem::new(&l.exec, &l.etiqueta, &l.icono, &l.app_id))
            .collect();
        Self {
            items,
            hover: tema::Realce::nuevo(),
            pegado: true,
        }
    }

    /// Marca qué lanzadores tienen ventana abierta, a partir de los `app_id`
    /// que hay en el escritorio. Devuelve `true` si cambió algo.
    ///
    /// Recibe los `app_id` y no una lista de índices para que el compositor no
    /// tenga que saber cómo está montado el dock: él sabe qué ventanas hay, el
    /// dock sabe a quién corresponden.
    pub fn set_abiertas(&mut self, app_ids: &[String]) -> bool {
        let mut cambio = false;
        for item in &mut self.items {
            let abierta = app_ids.iter().any(|id| item.es_suya(id));
            if item.abierta != abierta {
                item.abierta = abierta;
                cambio = true;
            }
        }
        // Lo que está abierto y no tiene lanzador entra en el dock mientras
        // dure la ventana. Es lo que hace macOS y lo que hace falta para poder
        // fijar una aplicación: sin esto, el clic derecho no tendría dónde
        // caer para algo que no está ya en el dock.
        for id in app_ids {
            if self.indice_de(id).is_some() {
                continue;
            }
            let app = crate::apps::por_app_id(id);
            // Sin `.desktop` se tira del propio `app_id`: como etiqueta, su
            // último componente —`org.gnome.Calculator` enseña "Calculator" y
            // no una "O" de inicial—, y como icono el id entero, que es el
            // nombre con el que la mayoría de temas lo tienen.
            let respaldo = id.rsplit('.').next().unwrap_or(id);
            let (exec, label, icono) = match &app {
                Some(a) => (a.exec.as_str(), a.nombre.as_str(), a.icono.as_str()),
                None => (id.as_str(), respaldo, id.as_str()),
            };
            let mut item = DockItem::new(exec, label, icono, id);
            item.anclada = false;
            item.abierta = true;
            self.items.push(item);
            cambio = true;
        }
        // Y se van al cerrarse.
        let antes = self.items.len();
        self.items.retain(|i| i.anclada || i.abierta);
        if self.items.len() != antes {
            self.hover.señalar(None);
            cambio = true;
        }
        cambio
    }

    pub fn items(&self) -> &[DockItem] {
        &self.items
    }

    /// Qué icono cae bajo un punto **lógico relativo a la esquina superior
    /// izquierda del dock**.
    ///
    /// El hit-test es aritmético y no pasa por iced a propósito. Los slots son
    /// de tamaño fijo y están en fila, así que el índice sale de una resta y
    /// una división; hacerlo con `UserInterface::update` obligaría a cambiar el
    /// tipo de mensaje de todo el crate, a mantener un `Clipboard` que no
    /// usamos y a sincronizar la caché entre `update` y `draw`. Cuando haya un
    /// widget con estado interno de verdad —un deslizador de volumen— entonces
    /// sí hará falta el camino de iced.
    pub fn item_en(&self, x: f32, y: f32) -> Option<usize> {
        // La banda del indicador cuenta como parte del icono: pulsar el punto
        // de "abierta" es pulsar la aplicación, no un hueco muerto.
        if y < PAD || y > PAD + ICON + PUNTO {
            return None;
        }
        let rel = x - PAD;
        if rel < 0.0 {
            return None;
        }
        let paso = ICON + GAP;
        let i = (rel / paso) as usize;
        // El hueco entre dos iconos no es de nadie: pulsar ahí no debe lanzar
        // el de la izquierda.
        if rel - i as f32 * paso > ICON {
            return None;
        }
        (i < self.items.len()).then_some(i)
    }

    /// Marca qué icono está señalado. Devuelve `true` si cambió, que es la
    /// señal de que hay que repintar.
    pub fn set_hover(&mut self, hover: Option<usize>) -> bool {
        self.hover.señalar(hover)
    }

    /// ¿Sigue moviéndose la placa de algún icono? Mientras sí, el dock hay que
    /// repintarlo aunque no llegue ningún evento: es la única superficie del
    /// shell que se anima sin que nadie la toque después del último movimiento
    /// del ratón.
    pub fn animando(&self) -> bool {
        self.hover.animando()
    }

    /// El lanzador que hay bajo `(x, y)`, en las mismas coordenadas que
    /// [`Dock::item_en`].
    ///
    /// Devuelve el item entero y no solo su `exec` porque pulsar un programa ya
    /// abierto no es lanzarlo otra vez: hay que traer su ventana al frente, y
    /// para eso quien decide necesita el `app_id`.
    pub fn pulsado(&self, x: f32, y: f32) -> Option<&DockItem> {
        self.item_en(x, y).map(|i| &self.items[i])
    }

    /// El centro **horizontal** del icono `i`, relativo al dock. Lo necesita el
    /// menú contextual para salir justo encima de él.
    pub fn centro_de(&self, i: usize) -> f32 {
        PAD + i as f32 * (ICON + GAP) + ICON / 2.0
    }

    /// Tamaño lógico del dock, derivado del número de items.
    ///
    /// El alto lleva la banda del indicador de ventana abierta además del
    /// icono: el punto no cabe en el padding sin comerse el borde redondeado.
    pub fn size(&self) -> (f32, f32) {
        let n = self.items.len().max(1) as f32;
        (
            PAD * 2.0 + n * ICON + (n - 1.0) * GAP,
            PAD * 2.0 + ICON + PUNTO,
        )
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut fila = row![].spacing(GAP);
        for (i, item) in self.items.iter().enumerate() {
            fila = fila.push(icon_view(item, self.hover.intensidad(i)));
        }

        // Pegado al borde solo se redondea por arriba: unas esquinas curvas
        // contra el canto de la pantalla dejan dos triángulos de escritorio
        // asomando debajo, que es justo lo que delata que la barra no está
        // realmente apoyada.
        let (fondo, radio) = if self.pegado {
            (
                fondo_pegado(),
                iced_core::border::Radius::default()
                    .top_left(tema::R_TARJETA)
                    .top_right(tema::R_TARJETA),
            )
        } else {
            (fondo(), tema::R_TARJETA.into())
        };
        container(fila)
            .padding(PAD)
            .style(move |_theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: radio,
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// Dice si el dock está apoyado en el borde de la pantalla. `true` si eso
    /// cambia el dibujo.
    pub fn set_pegado(&mut self, pegado: bool) -> bool {
        if self.pegado == pegado {
            return false;
        }
        self.pegado = pegado;
        true
    }
}

/// El icono, con una placa detrás cuando está señalado.
///
/// Todavía no es la lente de `DockMagnify.qml`: magnificar de verdad exige que
/// el icono se salga de su slot, y el buffer del dock hoy mide exactamente
/// `PAD*2 + ICON` de alto, así que lo ampliado quedaría cortado. Reservar ese
/// hueco cambia la geometría ya calibrada y va aparte.
fn icon_view(item: &DockItem, señalado: f32) -> PanelElement<'_> {
    let contenido: PanelElement<'_> = match &item.icon {
        Some(Icono::Svg(handle)) => svg(handle.clone())
            .width(Length::Fixed(ICON))
            .height(Length::Fixed(ICON))
            .into(),
        Some(Icono::Raster(handle)) => iced_image(handle.clone())
            .width(Length::Fixed(ICON))
            .height(Length::Fixed(ICON))
            .into(),
        None => {
            // Baldosa con la inicial. Fea a propósito: se ve que falta el icono.
            let inicial = item
                .label
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            container(
                container(text(inicial).size(22).color(TEXT()))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .width(Length::Fixed(ICON))
            .height(Length::Fixed(ICON))
            .style(|_theme| container::Style {
                background: Some(
                    Color {
                        a: 0.25,
                        ..ACENTO()
                    }
                    .into(),
                ),
                border: Border {
                    radius: tema::R_BOTON.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
        }
    };

    // La placa se desvanece en vez de encenderse de golpe: el dock es lo que
    // más se recorre con el ratón y el parpadeo de seis placas al cruzarlo era
    // lo más aparatoso que le quedaba al shell. Por debajo del 1 % de alfa no
    // se dibuja el contenedor, para no meter una capa por nada.
    let icono: PanelElement<'_> = if señalado > 0.01 {
        container(contenido)
            .style(move |_theme| container::Style {
                background: Some(tema::alfa(TEXT(), 0.18 * señalado).into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    } else {
        contenido
    };

    column![icono, punto(item.abierta)].into()
}

/// El indicador de "esta aplicación tiene ventana", bajo el icono.
///
/// Siempre ocupa su sitio, esté encendido o no: si el hueco apareciera y
/// desapareciera, el dock entero cambiaría de alto al abrir una ventana.
fn punto<'a>(encendida: bool) -> PanelElement<'a> {
    let color = if encendida {
        Color { a: 0.85, ..TEXT() }
    } else {
        Color::TRANSPARENT
    };
    container(
        container(
            Space::new()
                .width(Length::Fixed(5.0))
                .height(Length::Fixed(5.0)),
        )
        .style(move |_theme| container::Style {
            background: Some(color.into()),
            border: Border {
                radius: tema::R_PILL.into(),
                ..Default::default()
            },
            ..Default::default()
        }),
    )
    .width(Length::Fixed(ICON))
    .height(Length::Fixed(PUNTO))
    .center_x(Length::Fixed(ICON))
    .into()
}

/// Ayuda para depurar la resolución de iconos desde fuera del crate.
pub fn icon_path(name: &str) -> Option<PathBuf> {
    icono::resolve_icon(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un dock de tres items sin tocar el disco: `DockItem::new` buscaría
    /// iconos por medio sistema de ficheros y el hit-test no depende de eso.
    fn dock(n: usize) -> Dock {
        let items = (0..n)
            .map(|i| DockItem {
                exec: format!("prog{i}"),
                label: format!("P{i}"),
                app_id: format!("org.bookos.prog{i}"),
                icono_nombre: String::new(),
                icon: None,
                anclada: true,
                abierta: false,
            })
            .collect();
        Dock {
            items,
            hover: tema::Realce::nuevo(),
            pegado: true,
        }
    }

    #[test]
    fn el_centro_de_cada_icono_es_suyo() {
        let d = dock(3);
        let centro = |i: usize| PAD + i as f32 * (ICON + GAP) + ICON / 2.0;
        assert_eq!(d.item_en(centro(0), PAD + ICON / 2.0), Some(0));
        assert_eq!(d.item_en(centro(1), PAD + ICON / 2.0), Some(1));
        assert_eq!(d.item_en(centro(2), PAD + ICON / 2.0), Some(2));
    }

    #[test]
    fn el_hueco_entre_iconos_no_es_de_nadie() {
        let d = dock(3);
        // Justo en medio del GAP que separa el primero del segundo.
        let x = PAD + ICON + GAP / 2.0;
        assert_eq!(d.item_en(x, PAD + ICON / 2.0), None);
    }

    #[test]
    fn fuera_del_dock_no_hay_item() {
        let d = dock(3);
        let y = PAD + ICON / 2.0;
        // El padding de la izquierda y la franja de arriba.
        assert_eq!(d.item_en(PAD / 2.0, y), None);
        assert_eq!(d.item_en(PAD + ICON / 2.0, PAD / 2.0), None);
        // Abajo, el icono llega hasta el final de la banda del indicador: ese
        // punto es de la aplicación, no un hueco muerto.
        assert_eq!(
            d.item_en(PAD + ICON / 2.0, PAD + ICON + PUNTO / 2.0),
            Some(0)
        );
        assert_eq!(d.item_en(PAD + ICON / 2.0, PAD + ICON + PUNTO + 1.0), None);
        // Y más allá del último icono, dentro del ancho por el padding derecho.
        let (ancho, _) = d.size();
        assert_eq!(d.item_en(ancho - PAD / 2.0, y), None);
    }

    /// La correspondencia entre el lanzador y la ventana es heurística, así que
    /// conviene fijar exactamente qué casa y qué no.
    #[test]
    fn la_ventana_se_reconoce_por_el_sufijo_del_app_id() {
        let item = DockItem {
            exec: "konsole".into(),
            label: "Terminal".into(),
            app_id: "konsole".into(),
            icono_nombre: "konsole".into(),
            icon: None,
            anclada: true,
            abierta: false,
        };
        assert!(item.es_suya("konsole"));
        assert!(item.es_suya("org.kde.konsole"));
        assert!(item.es_suya("org.kde.Konsole"), "el caso no debe importar");
        // Parecerse no basta: sin esto, "konsole-profile" contaría como abierta.
        assert!(!item.es_suya("org.kde.konsole-profile"));
        assert!(!item.es_suya("konsole.parte"));
    }

    #[test]
    fn marcar_las_mismas_ventanas_no_obliga_a_repintar() {
        let mut d = dock(2);
        assert!(d.set_abiertas(&["org.bookos.prog0".to_string()]));
        assert!(!d.set_abiertas(&["org.bookos.prog0".to_string()]));
        assert!(d.set_abiertas(&[]));
    }

    #[test]
    fn señalar_dos_veces_el_mismo_icono_no_obliga_a_repintar() {
        let mut d = dock(3);
        assert!(d.set_hover(Some(1)));
        assert!(!d.set_hover(Some(1)));
        assert!(d.set_hover(None));
    }
}

#[cfg(test)]
mod anclado {
    use super::*;

    /// El ciclo completo: una ventana sin lanzador entra en el dock, se fija
    /// con el clic derecho, sobrevive al cierre de la ventana y se puede
    /// quitar. Es lo que el usuario pidió como "pin".
    #[test]
    fn una_ventana_se_fija_y_sobrevive_al_cierre() {
        let mut dock = Dock::from_config(&crate::Config::default().dock);
        let antes = dock.items().len();

        dock.set_abiertas(&["org.bookos.prueba".into()]);
        assert_eq!(dock.items().len(), antes + 1, "la ventana no entró al dock");
        assert!(!dock.esta_anclada("org.bookos.prueba"));
        assert!(
            !dock
                .como_configuracion()
                .iter()
                .any(|l| l.contains("prueba")),
            "lo que no está fijado no se guarda"
        );

        dock.alternar_anclado("org.bookos.prueba", "prueba", "prueba");
        assert!(dock.esta_anclada("org.bookos.prueba"));

        // Se cierra la ventana: al estar fijada, se queda.
        dock.set_abiertas(&[]);
        assert!(dock.esta_anclada("org.bookos.prueba"), "se fue al cerrarse");
        assert!(dock
            .como_configuracion()
            .iter()
            .any(|l| l.contains("prueba")));

        dock.alternar_anclado("org.bookos.prueba", "prueba", "prueba");
        assert_eq!(dock.items().len(), antes, "no volvió a su estado inicial");
    }

    /// Una ventana sin fijar se va del dock al cerrarse.
    #[test]
    fn una_ventana_sin_fijar_se_va_al_cerrarse() {
        let mut dock = Dock::from_config(&crate::Config::default().dock);
        let antes = dock.items().len();
        dock.set_abiertas(&["org.bookos.prueba".into()]);
        dock.set_abiertas(&[]);
        assert_eq!(dock.items().len(), antes);
    }

    /// El launchpad no se puede quitar: es la única forma de abrir lo que no
    /// está en el dock.
    #[test]
    fn el_launchpad_no_se_quita() {
        let mut dock = Dock::from_config(&crate::Config::default().dock);
        let antes = dock.items().len();
        dock.alternar_anclado(
            crate::config::LAUNCHPAD,
            crate::config::LAUNCHPAD,
            "launchpad",
        );
        assert_eq!(dock.items().len(), antes);
    }
}
