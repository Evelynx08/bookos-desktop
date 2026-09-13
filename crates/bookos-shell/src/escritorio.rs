//! Los iconos del escritorio: lo que hay en la carpeta Escritorio, dibujado
//! sobre el fondo y por debajo de las ventanas.
//!
//! ## Una superficie por icono, y no una del tamaño de la pantalla
//!
//! Lo natural sería una sola capa a pantalla completa con toda la rejilla
//! dentro. No vale: rasterizar un buffer de 2881×1801 con iced cuesta 23 ms
//! medidos (ver el comentario de `Shell::paint`), y la selección con banda
//! elástica cambia el aspecto de las celdas **en cada movimiento del ratón**.
//! Serían 40 ms por evento de puntero.
//!
//! Por eso cada icono es su propia superficie de 96×96 lógicos: al barrer con
//! la banda solo se repintan las celdas que **cambian** de estado —una o dos
//! por movimiento— y el resto de la escena la compone la GPU. La banda en sí no
//! se rasteriza aquí: son cinco rectángulos de color que dibuja el compositor.
//!
//! ## Qué hay dentro de la carpeta
//!
//! Se lee una vez al arrancar y se vuelve a mirar en el latido del panel —el
//! que ya despierta al cambiar el minuto—, así que guardar un fichero en el
//! escritorio se ve sin abrir ningún descriptor nuevo ni programar un
//! temporizador propio. El precio es hasta un minuto de retraso, que es el
//! trato que este escritorio hace siempre a cambio de no despertar de más.

use std::path::{Path, PathBuf};

use iced_core::alignment::Horizontal;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, text};

use crate::icono::Icono;
use crate::tema;
use crate::view::PanelElement;

/// Lado de una celda de la rejilla, en píxeles lógicos.
///
/// 96 es el ancho que deja caber dos renglones de nombre de 11 px sin que
/// «Documentos comprimidos.tar.gz» se coma a su vecino, y el alto que cabe
/// entero —icono, aire y etiqueta— sin recortar la segunda línea.
pub const CELDA: (f32, f32) = (96.0, 96.0);
/// Lado del icono dentro de la celda.
pub const ICONO: f32 = 48.0;
/// Aire entre el borde de la pantalla y la primera columna, y entre el panel y
/// la primera fila.
const MARGEN_X: f32 = 12.0;
const MARGEN_Y: f32 = 8.0;
/// Lo que se reserva abajo para que la última fila no quede debajo del dock.
///
/// El dock flota a 78 lógicos del canto inferior contando su margen; 96 deja
/// además el aire de una celda entre el nombre y el cristal.
const RESERVA_DOCK: f32 = 96.0;

/// Cuerpo del nombre. Once es lo que deja caber «informe-trimestral.pdf» en
/// dos renglones dentro de 80 px útiles.
const TAMANO_NOMBRE: f32 = 11.0;

/// Un icono del escritorio.
pub struct Elemento {
    pub nombre: String,
    pub ruta: PathBuf,
    /// Qué ejecutar al abrirlo: el `Exec` de su `.desktop`, o `xdg-open`.
    orden: String,
    icono: Option<Icono>,
    /// Columna y fila que ocupa. Es lo que se guarda en disco, no los píxeles:
    /// así una posición sobrevive a cambiar de resolución.
    celda: (i32, i32),
    seleccionado: bool,
}

pub struct Escritorio {
    elementos: Vec<Elemento>,
    /// La zona de la rejilla en lógicos: `(x, y, ancho, alto)`. Empieza bajo el
    /// panel y acaba antes del dock.
    zona: (f32, f32, f32, f32),
    filas: i32,
    columnas: i32,
    /// Cuánto se han corrido los iconos seleccionados mientras se arrastran.
    /// Va aparte de sus celdas porque durante el arrastre no han cambiado de
    /// sitio todavía: solo se dibujan corridos.
    arrastre: Option<(f32, f32)>,
}

impl Escritorio {
    /// `alto_panel` y `pantalla` van en píxeles lógicos.
    pub fn new(alto_panel: f32, pantalla: (f32, f32)) -> Self {
        let mut escritorio = Self {
            elementos: leer(),
            zona: (0.0, 0.0, 0.0, 0.0),
            filas: 1,
            columnas: 1,
            arrastre: None,
        };
        escritorio.recolocar(alto_panel, pantalla);
        escritorio
    }

    /// Rehace la rejilla para un tamaño de pantalla. Conserva la columna y la
    /// fila de cada icono mientras quepan; lo que se sale se recoloca al primer
    /// hueco libre, que es mejor que dejarlo fuera de la pantalla.
    pub fn recolocar(&mut self, alto_panel: f32, (ancho, alto): (f32, f32)) {
        self.zona = (
            MARGEN_X,
            alto_panel + MARGEN_Y,
            (ancho - MARGEN_X * 2.0).max(CELDA.0),
            (alto - alto_panel - MARGEN_Y - RESERVA_DOCK).max(CELDA.1),
        );
        self.columnas = (self.zona.2 / CELDA.0).floor().max(1.0) as i32;
        self.filas = (self.zona.3 / CELDA.1).floor().max(1.0) as i32;
        self.asignar_celdas();
    }

    /// Reparte las celdas: primero las guardadas que siguen cabiendo, y el
    /// resto al primer hueco libre, llenando por columnas de arriba abajo.
    fn asignar_celdas(&mut self) {
        let guardadas = posiciones_guardadas();
        let mut ocupadas: Vec<(i32, i32)> = Vec::new();
        // Dos vueltas: primero se apalabran las posiciones guardadas, para que
        // un icono nuevo no le quite el sitio a uno que el usuario colocó.
        for elemento in &mut self.elementos {
            elemento.celda = (-1, -1);
            if let Some(&celda) = guardadas
                .iter()
                .find(|(nombre, _)| *nombre == elemento.nombre)
                .map(|(_, celda)| celda)
            {
                if celda.0 < self.columnas && celda.1 < self.filas && !ocupadas.contains(&celda) {
                    elemento.celda = celda;
                    ocupadas.push(celda);
                }
            }
        }
        for elemento in &mut self.elementos {
            if elemento.celda != (-1, -1) {
                continue;
            }
            elemento.celda = primer_hueco(&ocupadas, self.columnas, self.filas);
            ocupadas.push(elemento.celda);
        }
    }

    /// Vuelve a mirar la carpeta. `true` si algo cambió y hay que repintar.
    ///
    /// Compara nombres y no marcas de tiempo a propósito: lo que se ve en el
    /// escritorio es la lista, y `mtime` del directorio cambia también cuando
    /// se toca un fichero que ya estaba.
    pub fn releer(&mut self, alto_panel: f32, pantalla: (f32, f32)) -> bool {
        let nuevos = leer();
        let iguales = nuevos.len() == self.elementos.len()
            && nuevos
                .iter()
                .zip(&self.elementos)
                .all(|(a, b)| a.ruta == b.ruta);
        if iguales {
            return false;
        }
        // La selección se pierde al releer. Es correcto: lo seleccionado puede
        // ser justo lo que acaba de desaparecer, y arrastrar de una selección
        // fantasma borraría o movería otra cosa.
        self.elementos = nuevos;
        self.recolocar(alto_panel, pantalla);
        true
    }

    pub fn cuantos(&self) -> usize {
        self.elementos.len()
    }

    pub fn seleccionado(&self, i: usize) -> bool {
        self.elementos.get(i).is_some_and(|e| e.seleccionado)
    }

    pub fn hay_seleccion(&self) -> bool {
        self.elementos.iter().any(|e| e.seleccionado)
    }

    /// El rectángulo de una celda en lógicos, ya con el desplazamiento del
    /// arrastre si el icono va en él.
    pub fn rect(&self, i: usize) -> Option<(f32, f32, f32, f32)> {
        let elemento = self.elementos.get(i)?;
        let (dx, dy) = match self.arrastre {
            Some(delta) if elemento.seleccionado => delta,
            _ => (0.0, 0.0),
        };
        Some((
            self.zona.0 + elemento.celda.0 as f32 * CELDA.0 + dx,
            self.zona.1 + elemento.celda.1 as f32 * CELDA.1 + dy,
            CELDA.0,
            CELDA.1,
        ))
    }

    /// Qué icono hay en un punto lógico. Se recorre al revés para que, si dos
    /// se solapan durante un arrastre, gane el que se dibuja encima.
    pub fn en(&self, x: f32, y: f32) -> Option<usize> {
        (0..self.elementos.len()).rev().find(|&i| {
            self.rect(i)
                .is_some_and(|(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh)
        })
    }

    /// Selecciona uno, o limpia la selección con `None`. Con `aditivo` (Ctrl)
    /// alterna ese solo y deja el resto como estaba.
    ///
    /// Devuelve `true` si cambió algo, o sea si hay que repintar.
    pub fn seleccionar(&mut self, i: Option<usize>, aditivo: bool) -> bool {
        let mut cambio = false;
        match (i, aditivo) {
            (Some(i), true) => {
                if let Some(elemento) = self.elementos.get_mut(i) {
                    elemento.seleccionado = !elemento.seleccionado;
                    cambio = true;
                }
            }
            (elegido, _) => {
                for (n, elemento) in self.elementos.iter_mut().enumerate() {
                    let quiero = elegido == Some(n);
                    cambio |= elemento.seleccionado != quiero;
                    elemento.seleccionado = quiero;
                }
            }
        }
        cambio
    }

    /// Marca lo que toca la banda elástica. `rect` va en lógicos y ya
    /// normalizado; `previa` es la selección de antes de empezar el gesto, que
    /// se conserva —es lo que hace Ctrl+arrastrar— y va vacía sin Ctrl.
    pub fn banda(&mut self, rect: (f32, f32, f32, f32), previa: &[bool]) -> bool {
        let (bx, by, bw, bh) = rect;
        let mut cambio = false;
        for (n, elemento) in self.elementos.iter_mut().enumerate() {
            let ex = self.zona.0 + elemento.celda.0 as f32 * CELDA.0;
            let ey = self.zona.1 + elemento.celda.1 as f32 * CELDA.1;
            let toca = bx < ex + CELDA.0 && bx + bw > ex && by < ey + CELDA.1 && by + bh > ey;
            // Lo que ya estaba seleccionado antes de empezar la banda se
            // conserva: sin esto, Ctrl+banda deseleccionaría lo anterior.
            let quiero = toca || previa.get(n).copied().unwrap_or(false);
            cambio |= elemento.seleccionado != quiero;
            elemento.seleccionado = quiero;
        }
        cambio
    }

    /// La selección de ahora mismo, para poder sumarle una banda encima.
    pub fn seleccion(&self) -> Vec<bool> {
        self.elementos.iter().map(|e| e.seleccionado).collect()
    }

    /// Corre los iconos seleccionados mientras dura el arrastre.
    pub fn arrastrar(&mut self, delta: (f32, f32)) {
        self.arrastre = Some(delta);
    }

    pub fn arrastrando(&self) -> bool {
        self.arrastre.is_some()
    }

    /// Deja los iconos arrastrados en la celda que les toque y guarda las
    /// posiciones. `true` si alguno cambió de sitio.
    pub fn soltar(&mut self) -> bool {
        let Some((dx, dy)) = self.arrastre.take() else {
            return false;
        };
        let paso = ((dx / CELDA.0).round() as i32, (dy / CELDA.1).round() as i32);
        if paso == (0, 0) {
            return false;
        }
        // Las celdas de los que **no** se mueven son las que hay que respetar;
        // entre los arrastrados se resuelven los choques por orden.
        let mut ocupadas: Vec<(i32, i32)> = self
            .elementos
            .iter()
            .filter(|e| !e.seleccionado)
            .map(|e| e.celda)
            .collect();
        for elemento in self.elementos.iter_mut().filter(|e| e.seleccionado) {
            let destino = (
                (elemento.celda.0 + paso.0).clamp(0, self.columnas - 1),
                (elemento.celda.1 + paso.1).clamp(0, self.filas - 1),
            );
            elemento.celda = if ocupadas.contains(&destino) {
                primer_hueco(&ocupadas, self.columnas, self.filas)
            } else {
                destino
            };
            ocupadas.push(elemento.celda);
        }
        self.guardar_posiciones();
        true
    }

    fn guardar_posiciones(&self) {
        let posiciones: Vec<(&str, (i32, i32))> = self
            .elementos
            .iter()
            .map(|e| (e.nombre.as_str(), e.celda))
            .collect();
        if let Err(err) = crate::config::guardar_escritorio(&posiciones) {
            tracing::warn!("no se pudieron guardar las posiciones del escritorio: {err}");
        }
    }

    /// Qué hacer al abrir un icono: su `Exec` si es un lanzador, y si no,
    /// `xdg-open`, que es quien sabe con qué se abre cada cosa.
    pub fn abrir(&self, i: usize) -> Option<crate::Accion> {
        let elemento = self.elementos.get(i)?;
        Some(crate::Accion::Lanzar(elemento.orden.clone()))
    }

    /// La celda `i` dibujada. Ocupa el buffer entero: es lo que hace que
    /// repintar una selección cueste una celda y no la pantalla.
    pub fn ver(&self, i: usize) -> PanelElement<'_> {
        let Some(elemento) = self.elementos.get(i) else {
            return Space::new().into();
        };
        let dibujo: PanelElement<'_> = match &elemento.icono {
            Some(ic) => crate::icono::ver(ic, ICONO, ICONO),
            None => Space::new().width(Length::Fixed(ICONO)).into(),
        };
        // El nombre va partido en **dos renglones exactos**, hechos aquí y con
        // el ajuste de iced apagado. Dejarle partir a él no vale: no sabe
        // cuándo parar, y «captura de pantalla 2026-08-20.png» salía en tres
        // líneas que se comían la celda de debajo — se vio en la
        // previsualización.
        //
        // El ancho útil son los 96 de la celda menos los 4+4 de su relleno y
        // los 4+4 de la pastilla: con 80 el nombre se salía por los dos lados.
        let ancho_texto = CELDA.0 - 24.0;
        let nombre = dos_lineas(&elemento.nombre, ancho_texto, TAMANO_NOMBRE);
        // Blanco sobre pastilla oscura, y no los colores del tema: aquí debajo
        // no hay una tarjeta sino el fondo de pantalla, que puede ser
        // cualquier cosa. El texto negro del tema claro sobre un fondo de noche
        // no se lee, y el sistema de diseño no tiene un token para «encima de
        // lo que sea».
        let (pastilla, tinta) = if elemento.seleccionado {
            (tema::acento(), tema::sobre_acento())
        } else {
            (
                Color {
                    a: 0.45,
                    ..Color::BLACK
                },
                Color::WHITE,
            )
        };
        let etiqueta = container(
            text(nombre)
                .size(TAMANO_NOMBRE)
                .color(tinta)
                .align_x(Horizontal::Center)
                .wrapping(iced_core::text::Wrapping::None)
                .shaping(iced_core::text::Shaping::Advanced),
        )
        .padding([1, 4])
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(pastilla.into()),
            border: Border {
                radius: tema::R_CHIP.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let celda = column![dibujo, etiqueta]
            .spacing(4)
            .align_x(Horizontal::Center);

        container(celda)
            .width(Length::Fixed(CELDA.0))
            .height(Length::Fixed(CELDA.1))
            .padding([6, 4])
            .align_x(Horizontal::Center)
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: elemento
                    .seleccionado
                    .then(|| tema::alfa(tema::acento(), 0.22).into()),
                border: Border {
                    radius: tema::R_BOTON.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }
}

/// Parte un nombre en como mucho dos renglones que quepan en `ancho`.
///
/// El sobrante se remata con puntos suspensivos. Se hace aquí y no con el
/// ajuste de línea de iced porque iced no sabe parar a las dos líneas.
fn dos_lineas(nombre: &str, ancho: f32, tamaño: f32) -> String {
    let (primera, resto) = cortar(nombre, ancho, tamaño);
    if resto.is_empty() {
        return primera;
    }
    let (segunda, sobra) = cortar(&resto, ancho, tamaño);
    if sobra.is_empty() {
        return format!("{primera}\n{segunda}");
    }
    // Los puntos ocupan sitio: la segunda línea se corta dejándoselo.
    let hueco = ancho - crate::widget::ancho_de("…", tamaño);
    let (segunda, _) = cortar(&resto, hueco.max(0.0), tamaño);
    format!("{primera}\n{}…", segunda.trim_end())
}

/// Corta `texto` por donde quepa en `ancho`: devuelve el trozo y lo que queda.
///
/// La primera tijera se pone con el ancho medio por letra y se corrige midiendo
/// de verdad. Medir carácter a carácter sería un `Paragraph` por letra —treinta
/// por nombre— y esto se paga en cada repintado de celda, o sea en cada celda
/// que cruza la banda elástica.
fn cortar(texto: &str, ancho: f32, tamaño: f32) -> (String, String) {
    if crate::widget::ancho_de(texto, tamaño) <= ancho {
        return (texto.to_string(), String::new());
    }
    let letras: Vec<char> = texto.chars().collect();
    let medio = crate::widget::ancho_de(texto, tamaño) / letras.len() as f32;
    let mut caben = (ancho / medio).floor().max(1.0) as usize;
    caben = caben.min(letras.len());
    while caben > 1 {
        let linea: String = letras[..caben].iter().collect();
        if crate::widget::ancho_de(&linea, tamaño) <= ancho {
            break;
        }
        caben -= 1;
    }
    // Se prefiere partir por un separador, si cae en la segunda mitad de lo que
    // cabe: cortar «informe-trimestral.pdf» por en medio de «trimes|tral» se
    // lee peor que dejar el guion al final del renglón.
    let corte = letras[..caben]
        .iter()
        .rposition(|c| matches!(c, ' ' | '-' | '_'))
        .map(|i| i + 1)
        .filter(|i| i * 2 >= caben)
        .unwrap_or(caben);
    (
        letras[..corte]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string(),
        letras[corte..].iter().collect(),
    )
}

/// El primer hueco libre llenando por columnas, de arriba abajo.
///
/// Si no queda ninguno se apila en la última: perder un icono porque la
/// pantalla es pequeña es peor que solaparlo.
fn primer_hueco(ocupadas: &[(i32, i32)], columnas: i32, filas: i32) -> (i32, i32) {
    (0..columnas)
        .flat_map(|col| (0..filas).map(move |fila| (col, fila)))
        .find(|celda| !ocupadas.contains(celda))
        .unwrap_or((columnas - 1, filas - 1))
}

/// Lo que hay en la carpeta Escritorio: carpetas primero y luego ficheros, cada
/// grupo por nombre.
///
/// Se ignoran los ocultos, como cualquier gestor de archivos, y los que no se
/// pueden leer: un `.desktop` roto es un icono menos, no un escritorio sin
/// iconos.
fn leer() -> Vec<Elemento> {
    let Some(carpeta) = carpeta_escritorio() else {
        return Vec::new();
    };
    let Ok(entradas) = std::fs::read_dir(&carpeta) else {
        return Vec::new();
    };
    let mut elementos: Vec<Elemento> = entradas
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| Elemento::leer(&e.path()))
        .collect();
    elementos.sort_by(|a, b| {
        let carpeta_a = a.ruta.is_dir();
        let carpeta_b = b.ruta.is_dir();
        carpeta_b
            .cmp(&carpeta_a)
            .then_with(|| a.nombre.to_lowercase().cmp(&b.nombre.to_lowercase()))
    });
    elementos
}

impl Elemento {
    fn leer(ruta: &Path) -> Option<Self> {
        let nombre_fichero = ruta.file_name()?.to_string_lossy().into_owned();
        // Un `.desktop` en el escritorio es un lanzador, no un fichero de
        // texto: enseña su `Name` y su icono, y al abrirlo se ejecuta su
        // `Exec`. Es lo único que distingue un escritorio de un listado.
        if ruta.extension().is_some_and(|e| e == "desktop") {
            if let Some(app) = crate::apps::leer_una(ruta) {
                return Some(Self {
                    nombre: app.nombre,
                    ruta: ruta.to_path_buf(),
                    orden: app.exec,
                    icono: crate::icono::cargar(&app.icono),
                    celda: (-1, -1),
                    seleccionado: false,
                });
            }
        }
        Some(Self {
            nombre: nombre_fichero,
            ruta: ruta.to_path_buf(),
            orden: format!("xdg-open {}", entrecomillar(ruta)),
            icono: crate::icono::cargar(nombre_icono(ruta)),
            celda: (-1, -1),
            seleccionado: false,
        })
    }
}

/// El icono del tema que le toca a un fichero.
///
/// No se resuelve el tipo MIME de verdad —eso es leer y cruzar
/// `shared-mime-info`, unos megas de XML— sino la extensión, que acierta en lo
/// que la gente tiene de verdad en el escritorio. Lo que no se reconoce cae en
/// el genérico, que es exactamente lo que enseñaría un MIME desconocido.
fn nombre_icono(ruta: &Path) -> &'static str {
    if ruta.is_dir() {
        return "folder";
    }
    let ext = ruta
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "svg" | "avif" => "image-x-generic",
        "mp4" | "mkv" | "webm" | "avi" | "mov" => "video-x-generic",
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" => "audio-x-generic",
        "pdf" => "application-pdf",
        "zip" | "tar" | "gz" | "xz" | "zst" | "bz2" | "7z" | "rar" => "package-x-generic",
        "txt" | "md" | "log" | "conf" | "toml" | "json" | "rs" | "py" | "sh" | "c" | "h"
        | "cpp" | "qml" => "text-x-generic",
        _ => "text-x-generic",
    }
}

/// Entrecomilla una ruta para `sh -c`: es lo que ejecuta el compositor, y un
/// nombre con espacios o con `;` no puede acabar siendo dos órdenes.
fn entrecomillar(ruta: &Path) -> String {
    format!("'{}'", ruta.to_string_lossy().replace('\'', r"'\''"))
}

/// Dónde está la carpeta del escritorio.
///
/// Primero lo que diga `user-dirs.dirs`, que es donde lo escriben `xdg-user-dirs`
/// y los entornos de escritorio; si no está, los dos nombres de siempre. Sin
/// esto, en una sesión en castellano se leería `~/Desktop` —que no existe— y el
/// escritorio saldría vacío.
fn carpeta_escritorio() -> Option<PathBuf> {
    // La escotilla es para la previsualización y para las pruebas: apuntar a
    // una carpeta de mentira es la única forma de mirar la rejilla sin
    // ensuciar el escritorio de verdad de quien ejecuta el ejemplo.
    if let Some(dir) = std::env::var_os("BOOKOS_ESCRITORIO") {
        return Some(PathBuf::from(dir));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if let Some(dir) = leer_user_dirs(&home) {
        if dir.is_dir() {
            return Some(dir);
        }
    }
    [home.join("Escritorio"), home.join("Desktop")]
        .into_iter()
        .find(|d| d.is_dir())
}

fn leer_user_dirs(home: &Path) -> Option<PathBuf> {
    let ruta = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) => PathBuf::from(dir).join("user-dirs.dirs"),
        None => home.join(".config/user-dirs.dirs"),
    };
    let texto = std::fs::read_to_string(ruta).ok()?;
    let valor = texto
        .lines()
        .map(str::trim)
        .find_map(|linea| linea.strip_prefix("XDG_DESKTOP_DIR="))?
        .trim_matches('"');
    // El fichero escribe la ruta como `"$HOME/Escritorio"`.
    Some(match valor.strip_prefix("$HOME/") {
        Some(resto) => home.join(resto),
        None => PathBuf::from(valor),
    })
}

/// Las posiciones que el usuario dejó puestas arrastrando.
fn posiciones_guardadas() -> Vec<(String, (i32, i32))> {
    crate::config::cargar_escritorio()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escritorio(cuantos: usize, pantalla: (f32, f32)) -> Escritorio {
        let mut e = Escritorio {
            elementos: (0..cuantos)
                .map(|n| Elemento {
                    nombre: format!("fichero {n}"),
                    ruta: PathBuf::from(format!("/tmp/fichero {n}")),
                    orden: String::new(),
                    icono: None,
                    celda: (-1, -1),
                    seleccionado: false,
                })
                .collect(),
            zona: (0.0, 0.0, 0.0, 0.0),
            filas: 1,
            columnas: 1,
            arrastre: None,
        };
        e.zona = (
            MARGEN_X,
            32.0 + MARGEN_Y,
            pantalla.0 - MARGEN_X * 2.0,
            pantalla.1 - 32.0 - MARGEN_Y - RESERVA_DOCK,
        );
        e.columnas = (e.zona.2 / CELDA.0).floor().max(1.0) as i32;
        e.filas = (e.zona.3 / CELDA.1).floor().max(1.0) as i32;
        // Sin leer del disco: `asignar_celdas` consultaría las posiciones
        // guardadas del usuario que ejecute los tests.
        let mut ocupadas = Vec::new();
        for elemento in &mut e.elementos {
            elemento.celda = primer_hueco(&ocupadas, e.columnas, e.filas);
            ocupadas.push(elemento.celda);
        }
        e
    }

    /// La rejilla se llena por columnas, de arriba abajo: el segundo icono va
    /// **debajo** del primero, no a su derecha.
    #[test]
    fn la_rejilla_se_llena_por_columnas() {
        let e = escritorio(3, (1440.0, 900.0));
        let (x0, y0, _, _) = e.rect(0).expect("hay tres iconos");
        let (x1, y1, _, _) = e.rect(1).expect("hay tres iconos");
        assert_eq!(x0, x1);
        assert_eq!(y1 - y0, CELDA.1);
    }

    /// La banda elástica marca lo que toca y desmarca lo que deja de tocar.
    #[test]
    fn la_banda_marca_lo_que_cruza() {
        let mut e = escritorio(3, (1440.0, 900.0));
        let (x, y, w, h) = e.rect(1).expect("hay tres iconos");
        let previa = vec![false; 3];
        assert!(e.banda((x + 2.0, y + 2.0, w - 4.0, h - 4.0), &previa));
        assert!(!e.seleccionado(0) && e.seleccionado(1) && !e.seleccionado(2));
        // Una banda que ya no toca nada deja la selección vacía.
        assert!(e.banda((0.0, 0.0, 1.0, 1.0), &previa));
        assert!(!e.hay_seleccion());
    }

    /// Arrastrar mueve la selección de celda, y lo que estaba en el destino no
    /// se pisa: el que llega busca hueco.
    #[test]
    fn al_soltar_el_icono_cae_en_una_celda_libre() {
        let mut e = escritorio(2, (1440.0, 900.0));
        e.seleccionar(Some(1), false);
        // Justo una celda hacia arriba, o sea encima del icono 0.
        e.arrastrar((0.0, -CELDA.1));
        assert!(e.soltar());
        assert_ne!(e.elementos[0].celda, e.elementos[1].celda);
    }

    /// Un nombre con comillas o espacios no puede convertirse en dos órdenes al
    /// pasar por `sh -c`.
    #[test]
    fn la_ruta_va_entrecomillada() {
        let orden = entrecomillar(Path::new("/tmp/a b'; rm -rf ~"));
        assert_eq!(orden, r"'/tmp/a b'\''; rm -rf ~'");
    }
}
