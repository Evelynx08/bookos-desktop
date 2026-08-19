//! El launchpad: la rejilla de aplicaciones a pantalla completa.
//!
//! Es la versión propia del plasmoide `bookos-launchpad`: rejilla de **6×5**,
//! barra de búsqueda en píldora arriba del todo, puntos de página abajo, y la
//! misma puntuación de búsqueda (exacto, prefijo, prefijo de palabra,
//! iniciales, substring, subsecuencia difusa).
//!
//! # Carpetas
//!
//! Una carpeta es un nombre y una lista de aplicaciones, y se guarda en
//! `~/.config/bookos/launchpad.conf` —una línea por carpeta— en vez de en el
//! `foldersJson` del plasmoide: leer JSON aquí sería una dependencia nueva para
//! guardar dos listas de nombres.
//!
//! **Las carpetas van primero y las aplicaciones sueltas después**, cada grupo
//! por orden alfabético. El plasmoide guarda además el orden manual completo
//! (`orderJson`); aquí no, y por eso el sitio de cada icono es siempre
//! deducible de lo que hay instalado: nada de un fichero que decide dónde va
//! cada cosa y que se desincroniza en cuanto instalas algo.
//!
//! Buscando **no hay carpetas**: se busca entre todas las aplicaciones, estén
//! dentro de una o no. Quien escribe «kate» quiere Kate, no que le recuerden
//! dónde lo guardó.
//!
//! Lo que **no** está, y por qué:
//!
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
/// Lado de cada punto de color de la fila que sale dentro de una carpeta, y su
/// paso. Diez colores en una fila que no puede robarle alto a la rejilla.
const COLOR_PUNTO: f32 = 18.0;
const COLOR_PASO: f32 = 28.0;
/// Lo que ocupa la fila de colores con su aire.
const COLORES_ALTO: f32 = 30.0;
/// Lado de la ✕ que quita una aplicación, en la esquina de su icono.
const CIERRE: f32 = 22.0;

/// Cuántas miniaturas caben en el icono de una carpeta: la rejilla de 3×3 de
/// siempre. La décima aplicación existe, pero en la tapa no se ve.
const MINIATURAS: usize = 9;

/// Lo que ocupa una celda de la rejilla: una aplicación o una carpeta.
///
/// Los dos son índices —a `apps` y a `carpetas`— y no datos: la celda se
/// reconstruye en cada `view` y copiar aquí el nombre y el icono sería
/// duplicar lo que ya está en su sitio.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Item {
    App(usize),
    Carpeta(usize),
}

/// Un icono que se está arrastrando por la rejilla.
///
/// Guarda **dónde empezó** y no qué se agarró: la lista visible puede
/// reordenarse mientras tanto —crear una carpeta la reordena— y un índice
/// guardado apuntaría a otra cosa. Lo que se agarró se resuelve al soltar.
#[derive(Clone, Copy)]
struct Arrastre {
    /// Posición en `visibles` de lo que se agarró.
    origen: usize,
    inicio: (f32, f32),
    actual: (f32, f32),
    /// Si ya se ha movido lo bastante como para ser un arrastre y no un clic.
    movido: bool,
}

/// Cuánto hay que mover el ratón para que deje de ser un clic, en lógicos.
///
/// Doce y no dos: pulsar un icono con el ratón siempre lo mueve un poco, y con
/// un umbral pequeño lanzar una aplicación se convertía en arrastrarla.
const UMBRAL_ARRASTRE: f32 = 12.0;

/// Un grupo de aplicaciones con su nombre.
#[derive(Clone, Debug)]
pub struct Carpeta {
    pub nombre: String,
    /// Su color, de la paleta de acentos. `None` = el gris de siempre.
    ///
    /// Se guarda el [`tema::Acento`] y no un `#rrggbb` por lo mismo que el
    /// acento del sistema: un color libre se sale del contraste que el resto
    /// de la paleta da por hecho, y aquí además tiene que leerse contra el velo
    /// del launchpad.
    pub color: Option<tema::Acento>,
    /// Los `exec` de sus aplicaciones, en el orden en que se metieron.
    ///
    /// Se guardan por `exec` y no por índice en `apps` porque el índice cambia
    /// al instalar o quitar programas, y esto sobrevive entre sesiones.
    pub apps: Vec<String>,
}

pub struct Launchpad {
    /// Tamaño **lógico** de la pantalla: el launchpad la ocupa entera y la
    /// rejilla se dimensiona a partir de ahí, como el plasmoide.
    pantalla: (f32, f32),
    apps: Vec<App>,
    /// Las carpetas, tal y como están en el disco.
    carpetas: Vec<Carpeta>,
    /// Los `exec` de las aplicaciones que el usuario ha quitado de la rejilla.
    ///
    /// **Quitadas, no desinstaladas.** El escritorio no borra programas del
    /// sistema: eso es del gestor de paquetes, necesita permisos que aquí no
    /// hay, y una ✕ que desinstala de verdad es la clase de botón que se pulsa
    /// sin querer una vez y se lamenta el resto de la tarde. Se vuelven a ver
    /// borrando su nombre de `launchpad.conf`.
    ocultas: Vec<String>,
    /// El modo de edición: con él, cada icono lleva su ✕ para quitarlo. Se
    /// entra con el botón derecho, como el menú contextual del dock.
    editando: bool,
    /// La carpeta abierta, si se está viendo una por dentro.
    dentro: Option<usize>,
    /// El nombre que se está escribiendo, si se está renombrando la carpeta
    /// abierta. Una carpeta nace llamándose «Carpeta» y hay que poder
    /// cambiarlo sin editar el fichero a mano.
    renombrando: Option<String>,
    /// Lo que se enseña ahora mismo, ya filtrado y ordenado.
    visibles: Vec<Item>,
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
    /// El icono que se está arrastrando, si hay alguno.
    arrastre: Option<Arrastre>,
    /// Iconos ya resueltos, por índice en `apps`. El `None` de dentro es "se
    /// buscó y no está", que no es lo mismo que "todavía no se ha buscado" —sin
    /// esa distinción se repetiría la búsqueda fallida en cada repintado, que
    /// es justo la cara.
    iconos: HashMap<usize, Option<Icono>>,
}

impl Launchpad {
    pub fn new(pantalla: (f32, f32)) -> Self {
        let apps = apps::leer();
        let guardado = crate::config::cargar_launchpad();
        let carpetas: Vec<Carpeta> = guardado
            .carpetas
            .into_iter()
            .map(|(nombre, color, apps)| Carpeta {
                nombre,
                color: color.as_deref().and_then(tema::Acento::desde_nombre),
                apps,
            })
            .collect();
        let ocultas = guardado.ocultas;
        tracing::debug!(
            n = apps.len(),
            carpetas = carpetas.len(),
            "aplicaciones leídas"
        );
        let mut lp = Self {
            pantalla,
            apps,
            carpetas,
            ocultas,
            editando: false,
            dentro: None,
            renombrando: None,
            visibles: Vec::new(),
            consulta: String::new(),
            pagina: 0,
            seleccion: None,
            acumulado: 0.0,
            ultimo_paso: None,
            transicion: None,
            arrastre: None,
            iconos: HashMap::new(),
        };
        lp.rehacer_visibles();
        lp.preparar_pagina();
        lp
    }

    /// Rehace la lista de lo que se enseña, según dónde se esté.
    ///
    /// Es el único sitio donde se decide qué va en la rejilla, y por eso no hay
    /// dos ordenaciones que puedan discrepar: dentro de una carpeta, lo suyo;
    /// buscando, aplicaciones sueltas puntuadas; y en la vista normal, las
    /// carpetas primero y detrás lo que no está en ninguna.
    fn rehacer_visibles(&mut self) {
        if let Some(c) = self.dentro {
            self.visibles = self.apps_de(c).into_iter().map(Item::App).collect();
            return;
        }
        if !self.consulta.is_empty() {
            return;
        }
        let en_carpetas: std::collections::HashSet<&str> = self
            .carpetas
            .iter()
            .flat_map(|c| c.apps.iter().map(String::as_str))
            .collect();
        let ocultas: std::collections::HashSet<&str> =
            self.ocultas.iter().map(String::as_str).collect();
        let mut visibles: Vec<Item> = (0..self.carpetas.len()).map(Item::Carpeta).collect();
        visibles.extend(
            self.apps
                .iter()
                .enumerate()
                .filter(|(_, app)| {
                    !en_carpetas.contains(app.exec.as_str()) && !ocultas.contains(app.exec.as_str())
                })
                .map(|(i, _)| Item::App(i)),
        );
        self.visibles = visibles;
    }

    /// Los índices de las aplicaciones de una carpeta, en su orden.
    ///
    /// Las que ya no estén instaladas simplemente no salen: desinstalar algo no
    /// puede dejar un hueco que al pulsarlo no haga nada.
    fn apps_de(&self, carpeta: usize) -> Vec<usize> {
        let Some(c) = self.carpetas.get(carpeta) else {
            return Vec::new();
        };
        c.apps
            .iter()
            .filter_map(|exec| self.apps.iter().position(|a| &a.exec == exec))
            .collect()
    }

    /// ¿Se está viendo una carpeta por dentro?
    pub fn en_carpeta(&self) -> bool {
        self.dentro.is_some()
    }

    /// Sale de la carpeta abierta. `true` si había alguna.
    pub fn salir_de_carpeta(&mut self) -> bool {
        if self.dentro.take().is_none() {
            return false;
        }
        self.renombrando = None;
        self.consulta.clear();
        self.pagina = 0;
        self.seleccion = None;
        self.rehacer_visibles();
        self.preparar_pagina();
        true
    }

    /// Dónde empieza la fila de colores de la carpeta abierta.
    ///
    /// Va **entre** el título y la rejilla y le come su alto al hueco que ya
    /// había: sin eso, la rejilla de dentro de una carpeta tendría una fila
    /// menos que la de fuera y los iconos bailarían al entrar.
    fn rect_colores(&self) -> iced_core::Rectangle {
        let ancho = COLOR_PASO * tema::Acento::TODOS.len() as f32;
        iced_core::Rectangle {
            x: (self.size().0 - ancho) / 2.0,
            y: self.margen_y() + BUSQUEDA + (HUECO - COLORES_ALTO) / 2.0,
            width: ancho,
            height: COLORES_ALTO,
        }
    }

    /// Qué color de la fila cae en un punto. El último es «sin color».
    fn color_en(&self, x: f32, y: f32) -> Option<Option<tema::Acento>> {
        let r = self.rect_colores();
        if !r.contains(iced_core::Point::new(x, y)) {
            return None;
        }
        let i = ((x - r.x) / COLOR_PASO) as usize;
        // El hueco entre puntos no es de nadie.
        if x - r.x - i as f32 * COLOR_PASO > COLOR_PUNTO {
            return None;
        }
        tema::Acento::TODOS.get(i).map(|a| Some(*a))
    }

    /// El rectángulo de la ✕ de la celda `i`, si el modo edición está puesto.
    fn cierre_de(&self, i: usize) -> Option<iced_core::Rectangle> {
        if !self.editando {
            return None;
        }
        let celda = self.rect_celda(i)?;
        // Arriba a la izquierda del icono, como en iOS: es la esquina que no
        // tapa ni el dibujo ni la etiqueta.
        let px = self.icono_px();
        Some(iced_core::Rectangle {
            x: celda.x + (celda.width - px) / 2.0 - CIERRE / 3.0,
            y: celda.y + (celda.height - ETIQUETA - px) / 2.0 - CIERRE / 3.0,
            width: CIERRE,
            height: CIERRE,
        })
    }

    /// Alterna el modo edición: con él salen las ✕. Lo abre el botón derecho.
    pub fn alternar_edicion(&mut self) -> bool {
        self.editando = !self.editando;
        true
    }

    /// Quita una aplicación de la rejilla. **No la desinstala**: ver
    /// [`Launchpad::ocultas`].
    fn quitar(&mut self, app: usize) {
        let exec = self.apps[app].exec.clone();
        // Si estaba en una carpeta, sale de ella además de ocultarse: si no,
        // seguiría dentro y volvería a aparecer al deshacerla.
        for c in &mut self.carpetas {
            c.apps.retain(|a| a != &exec);
        }
        if !self.ocultas.contains(&exec) {
            self.ocultas.push(exec);
        }
        self.tras_cambiar_carpetas();
    }

    /// El rectángulo del título de la carpeta abierta, que es donde se pulsa
    /// para renombrarla.
    fn rect_titulo(&self) -> iced_core::Rectangle {
        // Centrado arriba, del ancho de la barra de búsqueda: es el sitio que
        // ocupa el buscador cuando no estás dentro de una carpeta.
        iced_core::Rectangle {
            x: (self.size().0 - BUSQUEDA_ANCHO) / 2.0,
            y: self.margen_y(),
            width: BUSQUEDA_ANCHO,
            height: BUSQUEDA,
        }
    }

    /// Empieza a renombrar la carpeta abierta.
    fn renombrar(&mut self) -> bool {
        let Some(c) = self.dentro else {
            return false;
        };
        self.renombrando = Some(self.carpetas[c].nombre.clone());
        true
    }

    /// Guarda el nombre escrito. Un nombre vacío deja el que había: una carpeta
    /// sin rótulo no se distingue de otra.
    fn confirmar_nombre(&mut self) {
        let (Some(nombre), Some(c)) = (self.renombrando.take(), self.dentro) else {
            return;
        };
        let nombre = nombre.trim();
        if nombre.is_empty() || self.carpetas[c].nombre == nombre {
            return;
        }
        self.carpetas[c].nombre = nombre.to_string();
        self.tras_cambiar_carpetas();
    }

    /// Escribe el estado entero en `launchpad.conf`.
    fn guardar(&self) {
        let datos = crate::config::Launchpad {
            carpetas: self
                .carpetas
                .iter()
                .map(|c| {
                    (
                        c.nombre.clone(),
                        c.color.map(|a| a.nombre().to_string()),
                        c.apps.clone(),
                    )
                })
                .collect(),
            ocultas: self.ocultas.clone(),
        };
        if let Err(err) = crate::config::guardar_launchpad(&datos) {
            tracing::warn!("no se pudo guardar el launchpad: {err}");
        }
    }

    /// Entra en una carpeta.
    fn abrir_carpeta(&mut self, carpeta: usize) {
        self.dentro = Some(carpeta);
        self.pagina = 0;
        self.seleccion = None;
        self.rehacer_visibles();
        self.preparar_pagina();
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
        let hasta = (desde + POR_PAGINA).min(self.visibles.len());
        // Los de la página, y además los de dentro de las carpetas que se ven:
        // el icono de una carpeta **son** las miniaturas de lo que guarda, así
        // que sin ellos se dibujaría una caja vacía.
        let mut pendientes: Vec<usize> = Vec::new();
        for item in &self.visibles[desde..hasta] {
            match *item {
                Item::App(i) => pendientes.push(i),
                Item::Carpeta(c) => pendientes.extend(self.apps_de(c).into_iter().take(MINIATURAS)),
            }
        }
        for app in pendientes {
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
        Color {
            a: 0.45,
            ..tema::bg()
        }
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
            // Lo quitado no sale tampoco buscando: si saliera, la ✕ no habría
            // servido de nada.
            .filter(|(_, app)| !self.ocultas.contains(&app.exec))
            .filter_map(|(i, app)| apps::puntuar(app, &self.consulta).map(|p| (p, i)))
            .collect();
        // A igual puntuación, alfabético: `apps` ya viene ordenado, así que
        // basta con que la ordenación sea estable.
        con_puntos.sort_by_key(|(p, _)| *p);
        self.visibles = con_puntos.into_iter().map(|(_, i)| Item::App(i)).collect();
        // Sin nada escrito se vuelve a la rejilla de siempre —carpetas
        // incluidas—, que es lo que `rehacer_visibles` sabe montar.
        if self.consulta.is_empty() {
            self.rehacer_visibles();
        }
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
        if let (Some(arrastre), Some((x, y))) = (self.arrastre.as_mut(), punto) {
            arrastre.actual = (x, y);
            let (dx, dy) = (x - arrastre.inicio.0, y - arrastre.inicio.1);
            if !arrastre.movido && dx.hypot(dy) > UMBRAL_ARRASTRE {
                arrastre.movido = true;
            }
            // Mientras se arrastra, lo señalado es **el destino**: es lo que
            // dice sobre qué se va a soltar. El realce va en su propia
            // superficie, así que moverlo no repinta la rejilla.
            let destino = self.celda_en(x, y);
            if destino.is_some() && destino != self.seleccion {
                self.seleccion = destino;
                return true;
            }
            return false;
        }
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

    /// Pulsar **no lanza nada**: agarra. Lo que hace el icono se decide al
    /// soltar, porque hasta entonces no se sabe si esto era un clic o el
    /// principio de un arrastre.
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        // La ✕ manda sobre la celda: está encima de su icono, y si ganara el
        // icono no habría forma de pulsarla.
        if self.editando {
            let cerrar = (0..self.visibles.len())
                .find(|i| {
                    self.cierre_de(*i)
                        .is_some_and(|r| r.contains(iced_core::Point::new(x, y)))
                })
                .and_then(|i| match self.visibles.get(i) {
                    Some(Item::App(a)) => Some(*a),
                    // Una carpeta no se quita con la ✕: se deshace sacando lo
                    // que tiene dentro, y así no se pierde de vista nada sin
                    // querer.
                    _ => None,
                });
            if let Some(app) = cerrar {
                self.quitar(app);
                return None;
            }
        }
        // Dentro de una carpeta se puede elegir su color en la fila de puntos.
        if let (Some(c), Some(color)) = (self.dentro, self.color_en(x, y)) {
            {
                // Volver a pulsar el color que ya tiene lo quita: es la forma
                // de devolverla al gris sin un botón de «ninguno».
                self.carpetas[c].color = if self.carpetas[c].color == color {
                    None
                } else {
                    color
                };
                self.guardar();
                return None;
            }
        }
        // Dentro de una carpeta, pulsar su nombre lo pone a editar. Pulsar en
        // cualquier otro sitio confirma lo escrito, como cualquier campo.
        if self.dentro.is_some() {
            let en_titulo = self.rect_titulo().contains(iced_core::Point::new(x, y));
            if self.renombrando.is_some() && !en_titulo {
                self.confirmar_nombre();
            } else if en_titulo {
                self.renombrar();
                return None;
            }
        }
        let i = self.celda_en(x, y)?;
        self.seleccion = Some(i);
        self.arrastre = Some(Arrastre {
            origen: i,
            inicio: (x, y),
            actual: (x, y),
            movido: false,
        });
        None
    }

    /// ¿Hay un icono agarrado? Mientras lo haya, el puntero le sigue llegando
    /// aunque se salga de la rejilla: soltar fuera es lo que saca una
    /// aplicación de su carpeta.
    pub fn agarrado(&self) -> bool {
        self.arrastre.is_some()
    }

    /// Se ha soltado el botón. Devuelve si hay que repintar y qué hacer.
    pub fn soltar(&mut self) -> (bool, Option<Accion>) {
        let Some(arrastre) = self.arrastre.take() else {
            return (false, None);
        };
        if !arrastre.movido {
            // No se movió: era un clic de toda la vida.
            return (true, self.activar(arrastre.origen));
        }
        let (x, y) = arrastre.actual;
        let destino = self.celda_en(x, y);
        (self.soltar_en(arrastre.origen, destino), None)
    }

    /// Qué pasa al soltar lo de `origen` sobre `destino`.
    ///
    /// Las tres cosas que se pueden hacer con una carpeta, y ninguna más:
    /// crearla juntando dos aplicaciones, meter otra dentro, y sacar una
    /// soltándola fuera de la rejilla. **Reordenar no está**: el sitio de cada
    /// icono lo decide el orden alfabético, así que no hay dónde guardar «este
    /// va antes que aquel» ni forma de que sobreviva a instalar algo nuevo.
    fn soltar_en(&mut self, origen: usize, destino: Option<usize>) -> bool {
        let Some(&item_origen) = self.visibles.get(origen) else {
            return false;
        };
        let Item::App(app) = item_origen else {
            // Arrastrar una carpeta entera no hace nada: no hay carpetas dentro
            // de carpetas, y moverla de sitio no tendría dónde guardarse.
            return true;
        };
        let exec = self.apps[app].exec.clone();

        // Fuera de la rejilla, estando dentro de una carpeta: se saca de ella.
        let Some(destino) = destino else {
            if let Some(c) = self.dentro {
                self.sacar_de_carpeta(c, &exec);
            }
            return true;
        };
        if destino == origen {
            return true;
        }
        match self.visibles.get(destino).copied() {
            // Sobre otra aplicación: nace una carpeta con las dos.
            Some(Item::App(otra)) if self.dentro.is_none() => {
                let con = self.apps[otra].exec.clone();
                self.crear_carpeta(&con, &exec);
            }
            // Sobre una carpeta: se mete dentro.
            Some(Item::Carpeta(c)) => self.meter_en_carpeta(c, &exec),
            _ => {}
        }
        true
    }

    /// Crea una carpeta con dos aplicaciones. La primera da el nombre inicial.
    fn crear_carpeta(&mut self, primera: &str, segunda: &str) {
        // «Carpeta» a secas, como macOS cuando no puede deducir una categoría:
        // adivinarla de las categorías del `.desktop` acierta poco —media
        // distribución declara `Utility`— y un nombre equivocado es peor que
        // uno neutro que se cambia en dos segundos.
        self.carpetas.push(Carpeta {
            nombre: "Carpeta".into(),
            color: None,
            apps: vec![primera.to_string(), segunda.to_string()],
        });
        self.tras_cambiar_carpetas();
    }

    fn meter_en_carpeta(&mut self, carpeta: usize, exec: &str) {
        // Antes se saca de donde estuviera: una aplicación está en una carpeta
        // o en ninguna, nunca en dos.
        for c in &mut self.carpetas {
            c.apps.retain(|a| a != exec);
        }
        if let Some(c) = self.carpetas.get_mut(carpeta) {
            c.apps.push(exec.to_string());
        }
        self.tras_cambiar_carpetas();
    }

    fn sacar_de_carpeta(&mut self, carpeta: usize, exec: &str) {
        if let Some(c) = self.carpetas.get_mut(carpeta) {
            c.apps.retain(|a| a != exec);
        }
        self.tras_cambiar_carpetas();
    }

    /// Limpia, guarda y rehace la vista. Se llama tras **cualquier** cambio en
    /// las carpetas, que es lo que garantiza que el disco y lo que se ve digan
    /// lo mismo.
    fn tras_cambiar_carpetas(&mut self) {
        // Una carpeta con una sola aplicación no es una carpeta: se deshace
        // sola, como en macOS al sacar la penúltima.
        let dentro_de = self.dentro.and_then(|c| self.carpetas.get(c)).cloned();
        self.carpetas.retain(|c| c.apps.len() > 1);
        // Si la que estaba abierta ha desaparecido, se sale; si sigue, se
        // recoloca por nombre, que es lo único estable tras el `retain`.
        self.dentro = dentro_de.and_then(|antigua| {
            self.carpetas
                .iter()
                .position(|c| c.nombre == antigua.nombre && c.apps.len() > 1)
        });
        self.guardar();
        self.seleccion = None;
        self.rehacer_visibles();
        self.pagina = self.pagina.min(self.paginas() - 1);
        self.preparar_pagina();
    }

    /// Lo que hace pulsar la celda `i`: lanzar la aplicación, o entrar en la
    /// carpeta —que no es una acción para el compositor, sino un cambio de
    /// vista aquí dentro—.
    fn activar(&mut self, i: usize) -> Option<Accion> {
        match *self.visibles.get(i)? {
            Item::App(a) => Some(Accion::Lanzar(self.apps[a].exec.clone())),
            Item::Carpeta(c) => {
                self.abrir_carpeta(c);
                None
            }
        }
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
        if self.ultimo_paso.is_some_and(|t| t.elapsed() < ESPERA) {
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
        // Renombrando, el teclado es del campo: escribir no busca y Esc no
        // cierra nada, solo deja el nombre como estaba.
        if let Some(nombre) = self.renombrando.as_mut() {
            return match tecla {
                T::Caracter(c) => {
                    nombre.push(c);
                    Tecla::Consumida
                }
                T::Retroceso => {
                    nombre.pop();
                    Tecla::Consumida
                }
                T::Intro => {
                    self.confirmar_nombre();
                    Tecla::Consumida
                }
                T::Escape => {
                    self.renombrando = None;
                    Tecla::Consumida
                }
                _ => Tecla::Consumida,
            };
        }
        match tecla {
            // Esc limpia la búsqueda antes de cerrar, como el plasmoide: si has
            // escrito algo, lo primero que quieres deshacer es eso.
            T::Escape if !self.consulta.is_empty() => {
                self.consulta.clear();
                self.filtrar();
                Tecla::Consumida
            }
            // Esc deshace lo último que hayas hecho, en orden: primero el modo
            // edición, luego la carpeta abierta, y solo entonces cierra.
            T::Escape if self.editando => {
                self.editando = false;
                Tecla::Consumida
            }
            T::Escape if self.salir_de_carpeta() => Tecla::Consumida,
            T::Escape => Tecla::Cerrar,
            T::Caracter(c) => {
                // Escribir busca en todas las aplicaciones, también estando
                // dentro de una carpeta: quien escribe «kate» quiere Kate, no
                // que le recuerden en qué carpeta lo guardó.
                self.dentro = None;
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
    pub fn seleccionado(&mut self) -> Option<Accion> {
        let i = self.seleccion?;
        self.activar(i)
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
        // Dentro de una carpeta, arriba va su nombre y no el buscador: es lo
        // que dice dónde estás, y buscar desde dentro sale igualmente porque
        // escribir te saca a la rejilla completa.
        if let Some(c) = self.dentro {
            return self.view_carpeta(c);
        }
        let texto_busqueda = if self.consulta.is_empty() {
            text("Buscar aplicaciones…").size(15).color(tema::TEXTO2)
        } else {
            text(self.consulta.clone()).size(15).color(tema::texto())
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
            background: Some(
                Color {
                    a: 0.12,
                    ..tema::tinta()
                }
                .into(),
            ),
            border: Border {
                radius: tema::R_PILL.into(),
                width: 2.0,
                color: Color {
                    a: 0.7,
                    ..tema::acento()
                },
            },
            ..Default::default()
        });

        let contenido = column![
            container(busqueda)
                .width(Length::Fill)
                .align_x(Horizontal::Center),
            Space::new().height(Length::Fixed(HUECO)),
            self.rejilla_view(),
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
        let Some(item) = self.visibles.get(indice).copied() else {
            return hueco();
        };
        let px = self.icono_px();
        // La página que entra lo hace apareciendo, no solo deslizándose. Empieza
        // en 0,1 y no en 0: a cero, el primer fotograma es una pantalla vacía y
        // parece que el launchpad ha parpadeado.
        let alfa = 0.1 + 0.9 * self.avance();

        // La carpeta tiene su propia baldosa —la rejilla de miniaturas— y su
        // propio rótulo; el resto de la celda es idéntico.
        let (indice_app, nombre) = match item {
            Item::App(a) => (a, self.apps[a].nombre.clone()),
            Item::Carpeta(c) => {
                let alfa = 0.1 + 0.9 * self.avance();
                let interior = column![
                    self.tapa_carpeta(c, px, alfa),
                    Space::new().height(Length::Fixed(6.0)),
                    self.rotulo(&self.carpetas[c].nombre, cw, alfa),
                ]
                .align_x(Horizontal::Center);
                return container(interior)
                    .width(Length::Fixed(cw))
                    .height(Length::Fixed(ch))
                    .center_x(Length::Fixed(cw))
                    .center_y(Length::Fixed(ch))
                    .into();
            }
        };
        let app = &self.apps[indice_app];
        let _ = &nombre;
        let baldosa: PanelElement<'_> = match self.iconos.get(&indice_app).and_then(Option::as_ref)
        {
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
                container(text(inicial).size(px * 0.42).color(tema::texto()))
                    .width(Length::Fixed(px))
                    .height(Length::Fixed(px))
                    .center_x(Length::Fixed(px))
                    .center_y(Length::Fixed(px))
                    .style(move |_theme| container::Style {
                        background: Some(
                            Color {
                                a: 0.25,
                                ..tema::acento()
                            }
                            .into(),
                        ),
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

        let etiqueta = self.rotulo(&app.nombre, cw, alfa);

        // En modo edición, la ✕ va **encima** del icono, en su esquina: apilada
        // y no en la columna, que si no empujaría la etiqueta.
        let baldosa: PanelElement<'_> = if self.editando {
            iced_widget::stack![baldosa, self.cierre_view(px)].into()
        } else {
            baldosa
        };
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

    /// La rejilla de la página actual, con su deslizamiento si está entrando.
    fn rejilla_view(&self) -> PanelElement<'_> {
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
        match self.transicion {
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
        }
    }

    /// La vista de dentro de una carpeta: su nombre arriba y su rejilla.
    fn view_carpeta(&self, carpeta: usize) -> PanelElement<'_> {
        // Editando se enseña el cursor con el mismo carácter que la vista de
        // escritorios, para que renombrar se vea igual en los dos sitios.
        let (nombre, pista) = match &self.renombrando {
            Some(escrito) => (format!("{escrito}│"), "Intro para guardar"),
            // La pista es corta a propósito: en dos líneas se comía el alto de
            // la barra y empujaba la rejilla. Que el nombre se pulse para
            // cambiarlo se descubre pulsándolo, que es donde se busca.
            None => (self.carpetas[carpeta].nombre.clone(), "Esc para volver"),
        };
        let titulo = container(
            row![
                text(nombre).size(20).color(tema::texto()),
                Space::new().width(Length::Fixed(12.0)),
                // Sin botón de cerrar: se sale con Esc o pulsando fuera, igual
                // que se cierra el launchpad entero. Un botón más aquí sería el
                // único de toda la pantalla.
                text(pista).size(12).color(tema::TEXTO2),
            ]
            .align_y(iced_core::alignment::Vertical::Center),
        )
        .width(Length::Fixed(BUSQUEDA_ANCHO))
        .height(Length::Fixed(BUSQUEDA))
        .center_y(Length::Fixed(BUSQUEDA));

        let contenido = column![
            container(titulo)
                .width(Length::Fill)
                .align_x(Horizontal::Center),
            // La fila de colores le come su alto al hueco que ya había: la
            // rejilla de dentro tiene que empezar donde la de fuera.
            container(self.colores_view(carpeta))
                .width(Length::Fill)
                .height(Length::Fixed(HUECO))
                .center_x(Length::Fill)
                .center_y(Length::Fixed(HUECO)),
            self.rejilla_view(),
            self.puntos_view(),
        ]
        .align_x(Horizontal::Center);
        container(contenido)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Los diez colores de la paleta, para teñir la carpeta abierta.
    ///
    /// Los mismos que el acento del sistema y en el mismo orden: dos paletas
    /// distintas en el mismo escritorio se notan aunque no se sepa decir por
    /// qué. El que ya está puesto lleva un anillo, y volver a pulsarlo devuelve
    /// la carpeta al gris.
    fn colores_view(&self, carpeta: usize) -> PanelElement<'_> {
        let puesto = self.carpetas[carpeta].color;
        let mut fila = row![].spacing(COLOR_PASO - COLOR_PUNTO);
        for acento in tema::Acento::TODOS {
            let color = acento.color();
            let elegido = puesto == Some(acento);
            fila = fila.push(
                container(
                    container(Space::new())
                        .width(Length::Fixed(COLOR_PUNTO))
                        .height(Length::Fixed(COLOR_PUNTO))
                        .style(move |_theme| container::Style {
                            background: Some(color.into()),
                            border: Border {
                                radius: tema::R_PILL.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                )
                .width(Length::Fixed(COLOR_PUNTO))
                .height(Length::Fixed(COLORES_ALTO))
                .center_y(Length::Fixed(COLORES_ALTO))
                .style(move |_theme| container::Style {
                    border: Border {
                        radius: tema::R_PILL.into(),
                        width: if elegido { 2.0 } else { 0.0 },
                        color: if elegido {
                            tema::texto()
                        } else {
                            Color::TRANSPARENT
                        },
                    },
                    ..Default::default()
                }),
            );
        }
        fila.into()
    }

    /// La ✕ que quita una aplicación de la rejilla, en la esquina de su icono.
    fn cierre_view<'a>(&self, px: f32) -> PanelElement<'a> {
        let aspa: PanelElement<'a> = match crate::icono::propio("cerrar") {
            Some(ic) => {
                crate::icono::ver_teñido_propio(&ic, CIERRE * 0.5, tema::tinta_sobre(tema::rojo()))
            }
            None => Space::new().into(),
        };
        // El círculo es **rojo** y no gris: quitar algo de la rejilla es lo
        // único destructivo que se puede hacer aquí, y el sistema de diseño
        // reserva el rojo para eso.
        let boton = container(aspa)
            .width(Length::Fixed(CIERRE))
            .height(Length::Fixed(CIERRE))
            .center_x(Length::Fixed(CIERRE))
            .center_y(Length::Fixed(CIERRE))
            .style(|_theme| container::Style {
                background: Some(tema::rojo().into()),
                border: Border {
                    radius: tema::R_PILL.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        // Se coloca en la esquina superior izquierda del icono, saliéndose un
        // tercio: es donde la pone `cierre_de` para el hit-test.
        container(boton)
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .padding(iced_core::Padding::ZERO)
            .align_x(Horizontal::Left)
            .align_y(iced_core::alignment::Vertical::Top)
            .into()
    }

    /// El rótulo bajo un icono, que es el mismo para aplicaciones y carpetas.
    fn rotulo<'a>(&self, nombre: &str, cw: f32, alfa: f32) -> PanelElement<'a> {
        container(
            text(nombre.to_string())
                .size(UNIDAD * 0.75)
                .color(Color {
                    a: alfa,
                    ..tema::texto()
                })
                .align_x(Horizontal::Center),
        )
        .width(Length::Fixed(cw - UNIDAD))
        .height(Length::Fixed(ETIQUETA))
        .center_x(Length::Fixed(cw - UNIDAD))
        .into()
    }

    /// La tapa de una carpeta: sus nueve primeras aplicaciones en miniatura,
    /// dentro de una caja redondeada.
    ///
    /// Nueve y en rejilla de tres porque es lo que se reconoce como carpeta de
    /// un vistazo. Las miniaturas son los mismos iconos ya resueltos —no hay
    /// que cargar nada más— a un tercio del tamaño.
    fn tapa_carpeta<'a>(&'a self, carpeta: usize, px: f32, alfa: f32) -> PanelElement<'a> {
        let dentro = self.apps_de(carpeta);
        // Con color, la caja se tiñe de él y le sale un filo del mismo tono:
        // el relleno solo ya se pierde contra el velo, y el filo es lo que la
        // hace reconocible de una página a otra.
        let (fondo_tapa, borde_tapa) = match self.carpetas[carpeta].color {
            Some(acento) => (
                tema::alfa(acento.color(), 0.28 * alfa),
                tema::alfa(acento.color(), alfa),
            ),
            None => (tema::alfa(tema::tinta(), 0.18 * alfa), Color::TRANSPARENT),
        };
        let hueco = px * 0.06;
        let mini = (px - hueco * 4.0) / 3.0;
        let mut rejilla = column![].spacing(hueco);
        for fila in 0..3 {
            let mut linea = row![].spacing(hueco);
            for col in 0..3 {
                let i = fila * 3 + col;
                let celda: PanelElement<'a> = match dentro
                    .get(i)
                    .and_then(|a| self.iconos.get(a))
                    .and_then(Option::as_ref)
                {
                    Some(Icono::Svg(handle)) => svg(handle.clone())
                        .width(Length::Fixed(mini))
                        .height(Length::Fixed(mini))
                        .opacity(alfa)
                        .into(),
                    Some(Icono::Raster(handle)) => iced_image(handle.clone())
                        .width(Length::Fixed(mini))
                        .height(Length::Fixed(mini))
                        .opacity(alfa)
                        .into(),
                    // El hueco se dibuja igual: una carpeta con dos
                    // aplicaciones tiene que verse como una carpeta, no como
                    // dos iconos pequeños sueltos.
                    None => Space::new()
                        .width(Length::Fixed(mini))
                        .height(Length::Fixed(mini))
                        .into(),
                };
                linea = linea.push(celda);
            }
            rejilla = rejilla.push(linea);
        }
        container(rejilla)
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .center_x(Length::Fixed(px))
            .center_y(Length::Fixed(px))
            .style(move |_theme| container::Style {
                background: Some(fondo_tapa.into()),
                border: Border {
                    // Caja redondeada, no círculo: es lo que la distingue de un
                    // icono de aplicación, que el sistema de diseño define como
                    // círculo (§3.1).
                    radius: (px * 0.22).into(),
                    width: borde_tapa.a,
                    color: borde_tapa,
                },
                ..Default::default()
            })
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
                    container(
                        Space::new()
                            .width(Length::Fixed(ancho))
                            .height(Length::Fixed(8.0)),
                    )
                    .style(move |_theme| container::Style {
                        background: Some(
                            if activo {
                                tema::acento()
                            } else {
                                Color {
                                    a: 0.35,
                                    ..tema::tinta()
                                }
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
        con_apps_y_carpetas(nombres, Vec::new())
    }

    /// Un launchpad de mentira, sin tocar el disco: las aplicaciones se dan a
    /// mano y las carpetas también, que si no dependerían de lo que tenga el
    /// usuario en `~/.config`.
    fn con_apps_y_carpetas(nombres: &[&str], carpetas: Vec<Carpeta>) -> Launchpad {
        let apps: Vec<App> = nombres
            .iter()
            .map(|n| crate::apps::App {
                nombre: n.to_string(),
                exec: n.to_lowercase(),
                icono: n.to_lowercase(),
                normalizado: n.to_lowercase(),
            })
            .collect();
        let mut l = Launchpad {
            // Una pantalla cualquiera: los tests miran la lógica de búsqueda y
            // de páginas, que no depende del tamaño.
            pantalla: (1646.0, 1029.0),
            apps,
            carpetas,
            ocultas: Vec::new(),
            editando: false,
            dentro: None,
            renombrando: None,
            visibles: Vec::new(),
            consulta: String::new(),
            pagina: 0,
            seleccion: None,
            acumulado: 0.0,
            ultimo_paso: None,
            transicion: None,
            arrastre: None,
            iconos: HashMap::new(),
        };
        l.rehacer_visibles();
        l
    }

    /// Manda la configuración a un directorio de usar y tirar.
    ///
    /// Crear o deshacer una carpeta **escribe en el disco**, y sin esto los
    /// tests se llevarían por delante el `launchpad.conf` de quien compile. Se
    /// hace una sola vez por proceso: `set_var` es del proceso entero y los
    /// tests corren en hilos a la vez.
    fn sin_tocar_el_disco() {
        static UNA_VEZ: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        UNA_VEZ.get_or_init(|| {
            let dir = std::env::temp_dir().join("bookos-tests-launchpad");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("directorio de pruebas");
            // SAFETY: se hace una vez, antes de que ningún test lea la
            // configuración, y ningún otro hilo toca esta variable.
            unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        });
    }

    /// Arrastrar una aplicación sobre otra crea una carpeta con las dos, y
    /// las dos dejan de estar sueltas en la rejilla.
    #[test]
    fn arrastrar_una_sobre_otra_crea_carpeta() {
        sin_tocar_el_disco();
        let mut l = con_apps(&["Firefox", "Kate", "Konsole"]);
        assert_eq!(l.visibles.len(), 3);

        // Se agarra la primera y se suelta sobre la segunda.
        let origen = l.rect_celda(0).expect("la celda 0 se ve");
        let destino = l.rect_celda(1).expect("la celda 1 se ve");
        l.pulsar(origen.x + 10.0, origen.y + 10.0);
        l.puntero(Some((destino.x + 10.0, destino.y + 10.0)));
        let (repintar, accion) = l.soltar();
        assert!(repintar && accion.is_none(), "arrastrar no lanza nada");

        assert_eq!(l.carpetas.len(), 1);
        assert_eq!(l.carpetas[0].apps, ["kate", "firefox"]);
        // Una carpeta y la aplicación que quedó fuera.
        assert_eq!(l.visibles.len(), 2);
        assert_eq!(l.visibles[0], Item::Carpeta(0), "las carpetas van primero");
    }

    /// Pulsar sin mover **sí** lanza: el arrastre no puede robarle el clic a
    /// quien solo quería abrir algo.
    #[test]
    fn pulsar_sin_mover_lanza() {
        sin_tocar_el_disco();
        let mut l = con_apps(&["Firefox"]);
        let celda = l.rect_celda(0).unwrap();
        assert_eq!(
            l.pulsar(celda.x + 10.0, celda.y + 10.0),
            None,
            "pulsar agarra"
        );
        // Un temblor de dos píxeles sigue siendo un clic.
        l.puntero(Some((celda.x + 12.0, celda.y + 11.0)));
        let (_, accion) = l.soltar();
        assert_eq!(accion, Some(Accion::Lanzar("firefox".into())));
    }

    /// Y dentro de una carpeta, soltar fuera de la rejilla saca la aplicación;
    /// al quedarse con una sola, la carpeta se deshace.
    #[test]
    fn sacar_de_la_carpeta_la_deshace_al_quedarse_sola() {
        sin_tocar_el_disco();
        let carpeta = Carpeta {
            nombre: "Utilidades".into(),
            color: None,
            apps: vec!["kate".into(), "konsole".into(), "firefox".into()],
        };
        let mut l = con_apps_y_carpetas(&["Firefox", "Kate", "Konsole"], vec![carpeta]);
        assert_eq!(l.visibles, [Item::Carpeta(0)], "las tres están dentro");

        // Se entra en la carpeta y se saca una arrastrándola arriba del todo,
        // fuera de la rejilla.
        l.abrir_carpeta(0);
        assert!(l.en_carpeta());
        assert_eq!(l.visibles.len(), 3);
        let celda = l.rect_celda(0).unwrap();
        l.pulsar(celda.x + 10.0, celda.y + 10.0);
        l.puntero(Some((celda.x + 10.0, 2.0)));
        l.soltar();
        assert_eq!(l.carpetas[0].apps.len(), 2, "sacó una");

        // Y al sacar la penúltima, la carpeta desaparece: una sola aplicación
        // no es una carpeta.
        let celda = l.rect_celda(0).unwrap();
        l.pulsar(celda.x + 10.0, celda.y + 10.0);
        l.puntero(Some((celda.x + 10.0, 2.0)));
        l.soltar();
        assert!(l.carpetas.is_empty(), "la carpeta tenía que deshacerse");
        assert!(!l.en_carpeta(), "y sacarnos de ella");
        assert_eq!(l.visibles.len(), 3, "las tres vuelven a estar sueltas");
    }

    /// El color se elige de la misma paleta que el acento del sistema, y
    /// volver a pulsar el que ya está puesto devuelve la carpeta al gris.
    #[test]
    fn la_carpeta_se_tiñe_y_se_destiñe() {
        sin_tocar_el_disco();
        let carpeta = Carpeta {
            nombre: "Utilidades".into(),
            color: None,
            apps: vec!["kate".into(), "konsole".into()],
        };
        let mut l = con_apps_y_carpetas(&["Firefox", "Kate", "Konsole"], vec![carpeta]);
        l.abrir_carpeta(0);

        // El tercer punto de la fila es el tercer acento de la tabla.
        let fila = l.rect_colores();
        let x = fila.x + COLOR_PASO * 2.0 + 2.0;
        l.pulsar(x, fila.y + fila.height / 2.0);
        assert_eq!(l.carpetas[0].color, Some(tema::Acento::TODOS[2]));

        // Otra vez el mismo: vuelve al gris.
        l.pulsar(x, fila.y + fila.height / 2.0);
        assert_eq!(l.carpetas[0].color, None);

        // Y la calle entre dos puntos no elige nada.
        l.pulsar(fila.x + COLOR_PUNTO + 3.0, fila.y + fila.height / 2.0);
        assert_eq!(l.carpetas[0].color, None);
    }

    /// La ✕ quita la aplicación de la rejilla —y de su carpeta— pero **no la
    /// desinstala**: sigue en el sistema y vuelve borrándola del fichero.
    #[test]
    fn la_equis_quita_de_la_rejilla_sin_desinstalar() {
        sin_tocar_el_disco();
        let mut l = con_apps(&["Firefox", "Kate", "Konsole"]);
        assert_eq!(l.visibles.len(), 3);
        // Sin modo edición no hay ✕ que pulsar.
        assert!(l.cierre_de(0).is_none());

        l.alternar_edicion();
        let cierre = l.cierre_de(0).expect("con edición sí hay ✕");
        l.pulsar(cierre.x + 2.0, cierre.y + 2.0);
        assert_eq!(l.ocultas.len(), 1, "quitó una");
        assert_eq!(l.visibles.len(), 2);
        // Sigue instalada: lo que cambia es lo que se enseña.
        assert_eq!(l.apps.len(), 3);

        // Y tampoco sale buscándola, que si no la ✕ no habría servido de nada.
        l.tecla(crate::TeclaPulsada::Caracter(
            l.apps[0]
                .nombre
                .chars()
                .next()
                .unwrap()
                .to_ascii_lowercase(),
        ));
        assert!(
            !l.visibles.contains(&Item::App(0)),
            "la quitada vuelve a salir al buscar"
        );
    }

    /// Una carpeta nace llamándose «Carpeta»: hay que poder cambiarlo sin
    /// editar el fichero a mano.
    #[test]
    fn la_carpeta_se_renombra_desde_dentro() {
        use crate::TeclaPulsada as T;
        sin_tocar_el_disco();
        let carpeta = Carpeta {
            nombre: "Carpeta".into(),
            color: None,
            apps: vec!["kate".into(), "konsole".into()],
        };
        let mut l = con_apps_y_carpetas(&["Firefox", "Kate", "Konsole"], vec![carpeta]);
        l.abrir_carpeta(0);

        // Pulsar el nombre lo pone a editar, y entonces el teclado es suyo:
        // escribir no busca.
        let titulo = l.rect_titulo();
        l.pulsar(titulo.x + 10.0, titulo.y + 10.0);
        assert!(l.renombrando.is_some());
        for c in "Trabajo".chars() {
            l.tecla(T::Caracter(c));
        }
        assert!(l.consulta.is_empty(), "escribir el nombre no puede buscar");
        assert!(matches!(l.tecla(T::Intro), Tecla::Consumida));
        assert_eq!(l.carpetas[0].nombre, "CarpetaTrabajo");
        assert!(l.en_carpeta(), "renombrar no saca de la carpeta");

        // Y Esc mientras se edita cancela sin tocar el nombre.
        l.pulsar(titulo.x + 10.0, titulo.y + 10.0);
        l.tecla(T::Retroceso);
        l.tecla(T::Escape);
        assert_eq!(l.carpetas[0].nombre, "CarpetaTrabajo");
        assert!(l.en_carpeta(), "el Esc de la edición no sale de la carpeta");
    }

    /// Buscando no hay carpetas: se busca entre todas las aplicaciones, estén
    /// dentro de una o no.
    #[test]
    fn la_busqueda_atraviesa_las_carpetas() {
        sin_tocar_el_disco();
        let carpeta = Carpeta {
            nombre: "Utilidades".into(),
            color: None,
            apps: vec!["kate".into(), "konsole".into()],
        };
        let mut l = con_apps_y_carpetas(&["Firefox", "Kate", "Konsole"], vec![carpeta]);
        assert_eq!(l.visibles.len(), 2, "la carpeta y Firefox");

        l.tecla(crate::TeclaPulsada::Caracter('k'));
        assert_eq!(l.visibles.len(), 2, "kate y konsole, sin la carpeta");
        assert!(l.visibles.iter().all(|i| matches!(i, Item::App(_))));

        // Y al borrar lo escrito vuelve la rejilla con su carpeta.
        l.tecla(crate::TeclaPulsada::Retroceso);
        assert_eq!(l.visibles[0], Item::Carpeta(0));
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
