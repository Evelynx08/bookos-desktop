//! El launchpad: la rejilla de aplicaciones a pantalla completa.
//!
//! Es la versión propia del plasmoide `bookos-launchpad`: rejilla de **6×5**,
//! barra de búsqueda en píldora arriba del todo, puntos de página abajo, y la
//! misma puntuación de búsqueda (exacto, prefijo, prefijo de palabra,
//! iniciales, substring, subsecuencia difusa).
//!
//! Lo que **no** está, y por qué:
//!
//! - **Carpetas y arrastre.** Son la mitad del plasmoide y todo su estado
//!   persistente (`foldersJson`, `orderJson`). Sin un formato de configuración
//!   propio no hay dónde guardarlo, y el JSON de Plasma necesitaría un
//!   analizador que hoy sería una dependencia nueva.
//! # Lo que cuesta
//!
//! Medido en release a 2074x1549: dibujarlo entero **11 ms**, y **32 ms** la
//! primera vez que aparecen iconos que no estaban —resvg tiene que rasterizar
//! los SVG nuevos, y la caché de iced solo guarda los del último fotograma—.
//! Recorrerlo con el ratón cuesta **cero**: el realce va en su propia
//! superficie. Filtrar por texto, 0,09 ms; lo caro nunca es la búsqueda.
//!
//! - **El fondo de pantalla borroso.** El plasmoide saca el wallpaper de la
//!   configuración de plasmashell y lo desenfoca llamando a `magick` por un
//!   `DataSource` de tipo "executable". Aquí no hay plasmashell de quien
//!   heredarlo, y lanzar ImageMagick desde el compositor para pintar un fondo
//!   es exactamente lo que este proceso no debe hacer. Se pinta el velo opaco,
//!   que es lo que el propio plasmoide usa cuando no hay wallpaper.

use std::collections::HashMap;

use iced_core::alignment::Horizontal;
use iced_core::{Border, Color, Length};
use iced_widget::{column, container, image as iced_image, row, svg, text, Space};

use crate::apps::{self, App};
use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;
use crate::Accion;

use super::{Ancla, Tecla};

/// La rejilla del plasmoide.
const COLUMNAS: usize = 6;
const FILAS: usize = 5;
const POR_PAGINA: usize = COLUMNAS * FILAS;

/// La unidad de rejilla de Plasma, de la que salen todos los espaciados del
/// plasmoide. Con la fuente por defecto son 18 px.
const UNIDAD: f32 = 18.0;
/// Alto de la barra de búsqueda: 3 unidades, como el plasmoide.
const BUSQUEDA: f32 = UNIDAD * 3.0;
/// Ancho de la barra: 20 unidades.
const BUSQUEDA_ANCHO: f32 = UNIDAD * 20.0;
/// Separación entre la barra de búsqueda y la rejilla.
const HUECO: f32 = UNIDAD * 2.0;
/// Alto de la fila de puntos de página, con su margen.
const PUNTOS: f32 = UNIDAD * 3.0;
/// Alto reservado a la etiqueta bajo cada icono: 2 unidades.
const ETIQUETA: f32 = UNIDAD * 2.0;
/// Qué fracción de la pantalla ocupa la rejilla. Es el 0,72 del plasmoide: el
/// resto es aire, y ese aire es la mitad del aspecto.
const FRACCION: f32 = 0.72;

pub struct Launchpad {
    /// Tamaño **lógico** de la pantalla: el launchpad la ocupa entera y la
    /// rejilla se dimensiona a partir de ahí, como el plasmoide.
    pantalla: (f32, f32),
    apps: Vec<App>,
    /// Índices en `apps`, ya filtrados y ordenados por la búsqueda.
    visibles: Vec<usize>,
    consulta: String,
    pagina: usize,
    /// Posición **dentro de `visibles`** de lo que está seleccionado.
    seleccion: Option<usize>,
    /// Desplazamiento acumulado del touchpad desde el último cambio de página.
    acumulado: f32,
    /// Cuándo se cambió de página por última vez, para no encadenar saltos.
    ultimo_paso: Option<std::time::Instant>,
    /// Transición de página en curso: cuándo empezó y hacia dónde se fue
    /// (`1` a la siguiente, `-1` a la anterior).
    transicion: Option<(std::time::Instant, f32)>,
    /// Iconos ya resueltos, por índice en `apps`. El `None` de dentro es "se
    /// buscó y no está", que no es lo mismo que "todavía no se ha buscado" —sin
    /// esa distinción se repetiría la búsqueda fallida en cada repintado, que
    /// es justo la cara.
    iconos: HashMap<usize, Option<Icono>>,
}

impl Launchpad {
    pub fn new(pantalla: (f32, f32)) -> Self {
        let apps = apps::leer();
        tracing::debug!(n = apps.len(), "aplicaciones leídas");
        let visibles = (0..apps.len()).collect();
        let mut lp = Self {
            pantalla,
            apps,
            visibles,
            consulta: String::new(),
            pagina: 0,
            seleccion: None,
            acumulado: 0.0,
            ultimo_paso: None,
            transicion: None,
            iconos: HashMap::new(),
        };
        lp.preparar_pagina();
        lp
    }

    /// Resuelve los iconos de la página que se está viendo.
    ///
    /// Solo los treinta de la página, y una vez cada uno. Resolver los de todas
    /// las aplicaciones instaladas al abrir serían varios cientos de miles de
    /// `stat` —la búsqueda de freedesktop recorre directorios por temas por
    /// tamaños por categorías— y el launchpad tardaría en salir justo lo que no
    /// puede tardar.
    fn preparar_pagina(&mut self) {
        let desde = self.pagina * POR_PAGINA;
        for i in desde..(desde + POR_PAGINA).min(self.visibles.len()) {
            let app = self.visibles[i];
            self.iconos
                .entry(app)
                .or_insert_with(|| icono::cargar(&self.apps[app].icono));
        }
    }

    /// El **buffer** es solo el bloque de contenido, no la pantalla.
    ///
    /// Medido: rasterizar a pantalla completa cuesta 52 ms y señalar una celda
    /// otros 23, porque iced borra la máscara de recorte entera por cada capa y
    /// cada zona dañada — o sea que el coste va con el tamaño del buffer y no
    /// con lo que cambia, y el damage parcial no lo salva. El velo que ocupa la
    /// pantalla lo pinta el compositor con un color sólido, que en la GPU no
    /// cuesta nada, y aquí solo se rasteriza lo que tiene contenido.
    pub fn size(&self) -> (f32, f32) {
        (
            self.celda_ancho() * COLUMNAS as f32,
            BUSQUEDA + HUECO + self.celda_alto() * FILAS as f32 + PUNTOS,
        )
    }

    /// El velo que va detrás, a pantalla completa. Lo pinta el compositor.
    ///
    /// **Se aparta del sistema de diseño a sabiendas.** §2.6 pide un scrim al
    /// 45-55 % con desenfoque de 6-12 px, y aquí va al 82 % sin desenfoque: el
    /// blur necesita un shader contra el `GlesRenderer` y todavía no existe.
    /// Sin él, un velo al 45 % deja el escritorio legible por debajo y las
    /// etiquetas del launchpad se pelean con lo que hay detrás. Cuando haya
    /// blur, esto baja a 0,45 y no antes: bajarlo ahora sería cumplir el token
    /// y romper lo que el token existe para conseguir.
    pub fn velo(&self) -> Color {
        // 0,45, que es lo que pide el sistema de diseño. Estuvo en 0,82 todo el
        // tiempo que el fondo salió nítido: sin desenfoque, un velo al 45 % no
        // separaba los iconos de lo que hubiera detrás. Ahora el compositor
        // desenfoca la pantalla entera bajo el launchpad y el velo vuelve a su
        // valor; con 0,82 encima, el desenfoque no se veía.
        Color { a: 0.45, ..tema::BG }
    }

    /// El realce de la celda señalada: su rectángulo lógico **relativo al
    /// buffer** y si va con marco de acento.
    ///
    /// Se devuelve en vez de dibujarlo porque mover el ratón por la rejilla
    /// repintaría el buffer entero. Como superficie aparte, señalar otra celda
    /// es mover un rectángulo de 200 px: gratis.
    pub fn realce(&self) -> Option<(iced_core::Rectangle, bool)> {
        let rect = self.rect_celda(self.seleccion?)?;
        Some((rect, !self.consulta.is_empty()))
    }

    /// Tamaño lógico del realce: siempre el de una celda.
    pub fn realce_size(&self) -> (f32, f32) {
        (self.celda_ancho() - UNIDAD, self.celda_alto() - UNIDAD)
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    /// El área que ocupa la rejilla, y su esquina. Es el 72 % de la pantalla
    /// con un mínimo de 640×480, centrado.
    fn area(&self) -> (f32, f32) {
        (
            (self.pantalla.0 * FRACCION).max(640.0),
            (self.pantalla.1 * FRACCION).max(480.0),
        )
    }

    fn celda_ancho(&self) -> f32 {
        (self.area().0 / COLUMNAS as f32).max(80.0)
    }

    fn celda_alto(&self) -> f32 {
        (self.area().1 / FILAS as f32).max(80.0)
    }

    /// El icono, tan grande como quepa en su celda dejando sitio a la etiqueta.
    fn icono_px(&self) -> f32 {
        (self.celda_ancho() - UNIDAD * 3.0)
            .min(self.celda_alto() - ETIQUETA - UNIDAD * 2.0)
            .max(48.0)
    }

    /// La rejilla empieza en el borde del buffer: el centrado en la pantalla lo
    /// hace el anclaje, no un margen dibujado.
    fn margen_x(&self) -> f32 {
        0.0
    }

    fn margen_y(&self) -> f32 {
        0.0
    }

    fn paginas(&self) -> usize {
        self.visibles.len().div_ceil(POR_PAGINA).max(1)
    }

    /// Rehace la lista visible tras cambiar la búsqueda.
    fn filtrar(&mut self) {
        let mut con_puntos: Vec<(u32, usize)> = self
            .apps
            .iter()
            .enumerate()
            .filter_map(|(i, app)| apps::puntuar(app, &self.consulta).map(|p| (p, i)))
            .collect();
        // A igual puntuación, alfabético: `apps` ya viene ordenado, así que
        // basta con que la ordenación sea estable.
        con_puntos.sort_by_key(|(p, _)| *p);
        self.visibles = con_puntos.into_iter().map(|(_, i)| i).collect();
        self.pagina = 0;
        self.preparar_pagina();
        // Con búsqueda activa se preselecciona el primero: escribir y pulsar
        // Intro tiene que lanzar lo que se está viendo arriba a la izquierda.
        self.seleccion = (!self.consulta.is_empty() && !self.visibles.is_empty()).then_some(0);
    }

    /// Qué celda de la página cae en un punto lógico.
    fn celda_en(&self, x: f32, y: f32) -> Option<usize> {
        let x0 = self.margen_x();
        let y0 = self.margen_y() + BUSQUEDA + HUECO;
        if x < x0 || y < y0 {
            return None;
        }
        let col = ((x - x0) / self.celda_ancho()) as usize;
        let fila = ((y - y0) / self.celda_alto()) as usize;
        if col >= COLUMNAS || fila >= FILAS {
            return None;
        }
        let en_pagina = fila * COLUMNAS + col;
        let indice = self.pagina * POR_PAGINA + en_pagina;
        (indice < self.visibles.len()).then_some(indice)
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let seleccion = punto.and_then(|(x, y)| self.celda_en(x, y));
        // Sin puntero encima se conserva la selección del teclado: si el ratón
        // saliendo del área la borrase, escribir y mover el ratón sin querer
        // dejaría a Intro sin destino.
        if seleccion.is_none() || self.seleccion == seleccion {
            return false;
        }
        self.seleccion = seleccion;
        true
    }

    /// El rectángulo lógico de una celda de la página actual.
    fn rect_celda(&self, indice: usize) -> Option<iced_core::Rectangle> {
        let en_pagina = indice.checked_sub(self.pagina * POR_PAGINA)?;
        if en_pagina >= POR_PAGINA {
            return None;
        }
        let (fila, col) = (en_pagina / COLUMNAS, en_pagina % COLUMNAS);
        Some(iced_core::Rectangle {
            x: self.margen_x() + col as f32 * self.celda_ancho(),
            y: self.margen_y() + BUSQUEDA + HUECO + fila as f32 * self.celda_alto(),
            width: self.celda_ancho(),
            height: self.celda_alto(),
        })
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let i = self.celda_en(x, y)?;
        let app = &self.apps[*self.visibles.get(i)?];
        Some(Accion::Lanzar(app.exec.clone()))
    }

    /// Cuánto se desliza la rejilla al entrar una página, en lógicos.
    ///
    /// La rejilla ocupa el 72 % de la pantalla, así que el margen que hay para
    /// deslizarse sin recortar es el 14 % de cada lado: en una pantalla de
    /// 1646 lógicos son 230 px. 110 se queda holgadamente dentro y ya se lee
    /// como un cambio de página; con los 48 de antes el movimiento apenas se
    /// notaba.
    const DESLIZ: f32 = 110.0;

    /// ¿Sigue entrando la página nueva?
    pub fn animando(&self) -> bool {
        self.transicion
            .is_some_and(|(t0, _)| t0.elapsed() < tema::D_PAGINA)
    }

    /// Avance de la transición, de 0 a 1. Uno cuando no hay ninguna.
    fn avance(&self) -> f32 {
        match self.transicion {
            Some((t0, _)) => tema::C_ENTRADA.eval(tema::fraccion(t0.elapsed(), tema::D_PAGINA)),
            None => 1.0,
        }
    }

    /// Desplazamiento del touchpad o de la rueda, en píxeles lógicos.
    ///
    /// Devuelve `true` si ha cambiado de página. El umbral y la espera son los
    /// del plasmoide: 20 px horizontales y 180 ms entre cambios. Sin la espera,
    /// un gesto normal de touchpad —que manda decenas de eventos— saltaría
    /// cinco páginas de una pasada.
    pub fn desplazar(&mut self, dx: f32, dy: f32) -> bool {
        const UMBRAL: f32 = 20.0;
        const ESPERA: std::time::Duration = std::time::Duration::from_millis(180);

        // El eje vertical también pasa de página: en un touchpad el gesto
        // natural sobre una rejilla es el horizontal, pero con rueda de ratón
        // solo hay vertical y quedarse quieto sería raro.
        let delta = if dx.abs() >= dy.abs() { dx } else { dy };
        if self
            .ultimo_paso
            .is_some_and(|t| t.elapsed() < ESPERA)
        {
            // Dentro de la espera se sigue tragando el gesto para que el
            // acumulador no arrastre el impulso de la página anterior.
            self.acumulado = 0.0;
            return false;
        }
        self.acumulado += delta;
        if self.acumulado.abs() < UMBRAL {
            return false;
        }
        let hacia = if self.acumulado > 0.0 { 1 } else { -1 };
        self.acumulado = 0.0;
        let destino = self.pagina as isize + hacia;
        if destino < 0 || destino >= self.paginas() as isize {
            return false;
        }
        self.pagina = destino as usize;
        self.preparar_pagina();
        let ahora = std::time::Instant::now();
        self.ultimo_paso = Some(ahora);
        self.transicion = Some((ahora, hacia as f32));
        true
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        match tecla {
            // Esc limpia la búsqueda antes de cerrar, como el plasmoide: si has
            // escrito algo, lo primero que quieres deshacer es eso.
            T::Escape if !self.consulta.is_empty() => {
                self.consulta.clear();
                self.filtrar();
                Tecla::Consumida
            }
            T::Escape => Tecla::Cerrar,
            T::Caracter(c) => {
                self.consulta.push(c);
                self.filtrar();
                Tecla::Consumida
            }
            T::Retroceso => {
                self.consulta.pop();
                self.filtrar();
                Tecla::Consumida
            }
            T::Izquierda => self.mover(-1),
            T::Derecha => self.mover(1),
            T::Arriba => self.mover(-(COLUMNAS as isize)),
            T::Abajo => self.mover(COLUMNAS as isize),
            T::Intro => match self.seleccionado() {
                Some(accion) => Tecla::Hacer(accion),
                None => Tecla::Consumida,
            },
        }
    }

    /// La acción de lo que esté seleccionado, para Intro.
    pub fn seleccionado(&self) -> Option<Accion> {
        let i = self.seleccion?;
        let app = &self.apps[*self.visibles.get(i)?];
        Some(Accion::Lanzar(app.exec.clone()))
    }

    fn mover(&mut self, paso: isize) -> Tecla {
        if self.visibles.is_empty() {
            return Tecla::Consumida;
        }
        let actual = self.seleccion.unwrap_or(0) as isize;
        let nuevo = (actual + paso).clamp(0, self.visibles.len() as isize - 1) as usize;
        self.seleccion = Some(nuevo);
        // Salirse de la página la pasa: la selección manda sobre la página, no
        // al revés.
        self.pagina = nuevo / POR_PAGINA;
        self.preparar_pagina();
        Tecla::Consumida
    }

    pub fn view(&self) -> PanelElement<'_> {
        let texto_busqueda = if self.consulta.is_empty() {
            text("Buscar aplicaciones…").size(15).color(tema::TEXTO2)
        } else {
            text(self.consulta.clone()).size(15).color(tema::TEXTO)
        };
        // Píldora: radio la mitad del alto. Con foco el borde pasa a acento,
        // que aquí es siempre porque el campo no se puede desenfocar.
        let busqueda = container(
            container(texto_busqueda)
                .padding([0, 16])
                .height(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(Length::Fixed(BUSQUEDA_ANCHO))
        .height(Length::Fixed(BUSQUEDA))
        .style(|_theme| container::Style {
            background: Some(Color { a: 0.12, ..Color::WHITE }.into()),
            border: Border {
                radius: tema::R_PILL.into(),
                width: 2.0,
                color: Color { a: 0.7, ..tema::ACENTO },
            },
            ..Default::default()
        });

        let mut rejilla = column![];
        for fila in 0..FILAS {
            let mut linea = row![];
            for col in 0..COLUMNAS {
                let en_pagina = fila * COLUMNAS + col;
                linea = linea.push(self.celda_view(self.pagina * POR_PAGINA + en_pagina));
            }
            rejilla = rejilla.push(linea);
        }

        // La página que entra viene del lado hacia el que se ha pasado y se va
        // colocando. Solo se dibuja la nueva: pintar además la que se va
        // obligaría a tener resueltos los iconos de las dos páginas, que es
        // justo el trabajo que `preparar_pagina` evita.
        let rejilla: PanelElement<'_> = match self.transicion {
            Some((_, hacia)) if self.animando() => {
                let resto = 1.0 - self.avance();
                let corrimiento = Self::DESLIZ * resto;
                let padding = if hacia > 0.0 {
                    iced_core::Padding::ZERO.left(corrimiento)
                } else {
                    iced_core::Padding::ZERO.right(corrimiento)
                };
                container(rejilla).padding(padding).into()
            }
            _ => rejilla.into(),
        };

        let contenido = column![
            container(busqueda)
                .width(Length::Fill)
                .align_x(Horizontal::Center),
            Space::new().height(Length::Fixed(HUECO)),
            rejilla,
            self.puntos_view(),
        ]
        .align_x(Horizontal::Center);

        // Sin fondo: el velo lo pone el compositor a pantalla completa. Este
        // buffer es transparente salvo donde hay algo dibujado.
        container(contenido)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn celda_view(&self, indice: usize) -> PanelElement<'_> {
        let (cw, ch) = (self.celda_ancho(), self.celda_alto());
        let hueco = || -> PanelElement<'_> {
            Space::new()
                .width(Length::Fixed(cw))
                .height(Length::Fixed(ch))
                .into()
        };
        let Some(app) = self.visibles.get(indice).map(|i| &self.apps[*i]) else {
            return hueco();
        };
        let px = self.icono_px();
        // La página que entra lo hace apareciendo, no solo deslizándose. Empieza
        // en 0,1 y no en 0: a cero, el primer fotograma es una pantalla vacía y
        // parece que el launchpad ha parpadeado.
        let alfa = 0.1 + 0.9 * self.avance();

        let indice_app = self.visibles[indice];
        let baldosa: PanelElement<'_> = match self.iconos.get(&indice_app).and_then(Option::as_ref) {
            Some(Icono::Svg(handle)) => svg(handle.clone())
                .width(Length::Fixed(px))
                .height(Length::Fixed(px))
                .opacity(alfa)
                .into(),
            Some(Icono::Raster(handle)) => iced_image(handle.clone())
                .width(Length::Fixed(px))
                .height(Length::Fixed(px))
                .opacity(alfa)
                .into(),
            // Baldosa con la inicial, igual que en el dock: un icono que falta
            // no puede dejar un hueco en la rejilla.
            None => {
                let inicial = app
                    .nombre
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default();
                container(text(inicial).size(px * 0.42).color(tema::TEXTO))
                    .width(Length::Fixed(px))
                    .height(Length::Fixed(px))
                    .center_x(Length::Fixed(px))
                    .center_y(Length::Fixed(px))
                    .style(move |_theme| container::Style {
                        background: Some(Color { a: 0.25, ..tema::ACENTO }.into()),
                        border: Border {
                            // Círculo, no baldosa redondeada: el sistema de
                            // diseño define los iconos de aplicación como un
                            // círculo de color con un glifo blanco encima
                            // (§3.1), así que el respaldo de uno que falta
                            // tiene que tener la misma silueta que sus vecinos.
                            radius: tema::R_PILL.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .into()
            }
        };

        let etiqueta = container(
            text(app.nombre.clone())
                .size(UNIDAD * 0.75)
                .color(Color { a: alfa, ..tema::TEXTO })
                .align_x(Horizontal::Center),
        )
        .width(Length::Fixed(cw - UNIDAD))
        .height(Length::Fixed(ETIQUETA))
        .center_x(Length::Fixed(cw - UNIDAD));

        let interior = column![baldosa, Space::new().height(Length::Fixed(6.0)), etiqueta]
            .align_x(Horizontal::Center);

        // El realce de la celda señalada **no** se dibuja aquí: va en su propia
        // superficie, encima del velo y debajo de esta. Ver `realce`.
        container(interior)
            .width(Length::Fixed(cw))
            .height(Length::Fixed(ch))
            .center_x(Length::Fixed(cw))
            .center_y(Length::Fixed(ch))
            .into()
    }

    /// Los puntos de página: píldora de 20×8 el activo, círculo de 8 los demás.
    fn puntos_view(&self) -> PanelElement<'_> {
        let paginas = self.paginas();
        if paginas <= 1 {
            return Space::new().height(Length::Fixed(PUNTOS)).into();
        }
        let mut fila = row![].spacing(8);
        for i in 0..paginas {
            let activo = i == self.pagina;
            let ancho = if activo { 20.0 } else { 8.0 };
            fila = fila.push(
                container(
                    container(Space::new().width(Length::Fixed(ancho)).height(Length::Fixed(8.0)))
                        .style(move |_theme| container::Style {
                            background: Some(
                                if activo {
                                    tema::ACENTO
                                } else {
                                    Color { a: 0.35, ..Color::WHITE }
                                }
                                .into(),
                            ),
                            border: Border {
                                radius: tema::R_PILL.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                )
                // Ranura de ancho fijo: sin ella, la fila entera se recoloca al
                // cambiar de página porque el activo mide 12 px más.
                .width(Length::Fixed(20.0))
                .align_x(Horizontal::Center),
            );
        }
        container(fila)
            .width(Length::Fill)
            .height(Length::Fixed(PUNTOS))
            .center_x(Length::Fill)
            .center_y(Length::Fixed(PUNTOS))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn con_apps(nombres: &[&str]) -> Launchpad {
        let apps: Vec<App> = nombres
            .iter()
            .map(|n| crate::apps::App {
                nombre: n.to_string(),
                exec: n.to_lowercase(),
                icono: n.to_lowercase(),
                normalizado: n.to_lowercase(),
            })
            .collect();
        let visibles = (0..apps.len()).collect();
        Launchpad {
            // Una pantalla cualquiera: los tests miran la lógica de búsqueda y
            // de páginas, que no depende del tamaño.
            pantalla: (1646.0, 1029.0),
            apps,
            visibles,
            consulta: String::new(),
            pagina: 0,
            seleccion: None,
            acumulado: 0.0,
            ultimo_paso: None,
            transicion: None,
            iconos: HashMap::new(),
        }
    }

    #[test]
    fn escribir_filtra_y_preselecciona() {
        use crate::TeclaPulsada as T;
        let mut l = con_apps(&["Firefox", "Kate", "Konsole"]);
        assert_eq!(l.visibles.len(), 3);

        l.tecla(T::Caracter('k'));
        assert_eq!(l.visibles.len(), 2, "kate y konsole");
        assert_eq!(l.seleccion, Some(0), "con búsqueda, Intro tiene destino");

        l.tecla(T::Caracter('o'));
        assert_eq!(l.visibles.len(), 1, "solo konsole");
        assert_eq!(l.seleccionado(), Some(Accion::Lanzar("konsole".into())));

        // Retroceso deshace.
        l.tecla(T::Retroceso);
        assert_eq!(l.visibles.len(), 2);
    }

    #[test]
    fn escape_limpia_la_busqueda_antes_de_cerrar() {
        use crate::TeclaPulsada as T;
        let mut l = con_apps(&["Firefox"]);
        l.tecla(T::Caracter('f'));
        assert!(matches!(l.tecla(T::Escape), Tecla::Consumida));
        assert!(l.consulta.is_empty());
        // Ya sin búsqueda, el segundo Escape sí cierra.
        assert!(matches!(l.tecla(T::Escape), Tecla::Cerrar));
    }

    #[test]
    fn la_seleccion_arrastra_la_pagina() {
        use crate::TeclaPulsada as T;
        let nombres: Vec<String> = (0..45).map(|i| format!("App{i:02}")).collect();
        let refs: Vec<&str> = nombres.iter().map(String::as_str).collect();
        let mut l = con_apps(&refs);
        assert_eq!(l.paginas(), 2, "45 apps a 30 por página");

        // Bajar cinco filas desde el principio se sale de la primera página.
        for _ in 0..5 {
            l.tecla(T::Abajo);
        }
        assert_eq!(l.seleccion, Some(30));
        assert_eq!(l.pagina, 1, "la selección arrastra la página");
    }

    #[test]
    fn el_gesto_pasa_de_pagina_con_umbral_y_espera() {
        let nombres: Vec<String> = (0..45).map(|i| format!("App{i:02}")).collect();
        let refs: Vec<&str> = nombres.iter().map(String::as_str).collect();
        let mut l = con_apps(&refs);
        assert_eq!(l.paginas(), 2);

        // Por debajo del umbral no pasa nada, pero se acumula.
        assert!(!l.desplazar(8.0, 0.0));
        assert!(!l.desplazar(8.0, 0.0));
        assert_eq!(l.pagina, 0);
        // Al superar los 20 px acumulados, cambia.
        assert!(l.desplazar(8.0, 0.0));
        assert_eq!(l.pagina, 1);

        // Y justo después no encadena otro salto: un gesto de touchpad manda
        // decenas de eventos y sin la espera se recorrerían cinco páginas.
        assert!(!l.desplazar(40.0, 0.0));
        assert_eq!(l.pagina, 1);
    }

    #[test]
    fn el_gesto_no_se_sale_por_los_extremos() {
        let mut l = con_apps(&["A", "B"]);
        // Una sola página: no hay a dónde ir ni hacia adelante ni hacia atrás.
        assert!(!l.desplazar(100.0, 0.0));
        assert!(!l.desplazar(-100.0, 0.0));
        assert_eq!(l.pagina, 0);
    }

    #[test]
    fn moverse_no_se_sale_de_la_lista() {
        use crate::TeclaPulsada as T;
        let mut l = con_apps(&["A", "B", "C"]);
        for _ in 0..10 {
            l.tecla(T::Derecha);
        }
        assert_eq!(l.seleccion, Some(2), "no pasa del último");
        for _ in 0..10 {
            l.tecla(T::Izquierda);
        }
        assert_eq!(l.seleccion, Some(0), "ni del primero");
    }
}
