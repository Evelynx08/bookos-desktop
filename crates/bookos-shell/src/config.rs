//! Qué widgets lleva el panel y en qué orden.
//!
//! El fichero es `~/.config/bookos/panel.conf` y tiene esta pinta:
//!
//! ```text
//! # Los widgets del panel, por zona.
//! centro = reloj
//! derecha = escritorios, red, brillo, bateria
//! escala = 1.75
//! tema = oscuro
//! acento = azul
//! avatar = ~/Imágenes/yo.png
//! bloqueo_animaciones = si
//! bloqueo_fecha = si
//! bloqueo_medios = si
//! bloqueo_reloj_y = 0.08
//! bloqueo_acceso_y = 0.36
//! bloqueo_medios_y = 0.68
//! bloqueo_reloj_tamano = 120
//! bloqueo_avatar_tamano = 112
//! actividades = si
//! actividades_animaciones = si
//! temporizador_siempre_visible = no
//! escritorios = 2
//! nombres_escritorios = Escritorio 1, Escritorio 2
//! dock = konsole:Terminal:utilities-terminal, firefox:Navegador:firefox
//!
//! # Cursor y entrada. Las velocidades van en la escala de libinput: [-1, 1].
//! fondo = /usr/share/wallpapers/BookOS/blue_dark.png
//! cursor = 24
//! velocidad_touchpad = 0.3
//! toque_para_clic = si
//! scroll_natural = si
//! ```
//!
//! **Por qué no TOML.** Leer dos listas de nombres no justifica meter `toml`,
//! que arrastra `serde` entero en un crate que hoy no tiene ninguna
//! dependencia de serialización. El formato de arriba se analiza en veinte
//! líneas y no puede fallar de formas interesantes.
//!
//! Si el fichero no existe se usan los valores de [`Config::default`], así que
//! el panel se sigue pintando en el primer frame sin tocar el disco. Y si
//! existe pero tiene una errata, la línea mala se ignora y se avisa: quedarse
//! sin panel por una coma es peor que quedarse sin un widget.

use std::path::PathBuf;

pub struct Config {
    pub centro: Option<String>,
    pub derecha: Vec<String>,
    pub dock: Vec<Lanzador>,
    /// Escala de la pantalla. `None` = la que deduzca el compositor del tamaño
    /// físico del panel.
    ///
    /// Vive aquí aunque el shell no la use porque es lo que el usuario entiende
    /// por "la configuración del escritorio", y tener dos ficheros —uno para el
    /// panel y otro para la pantalla— es la clase de reparto que solo tiene
    /// sentido para quien escribió el código.
    pub escala: Option<f64>,
    /// Tamaño **lógico** del cursor. El tema elige luego qué imagen de las que
    /// trae se acerca más a ese tamaño por la escala de la pantalla.
    pub cursor: u32,
    pub entrada: Entrada,
    /// Imagen del fondo del escritorio. `None` = la que se encuentre.
    pub fondo: Option<String>,
    /// Distribución de teclado (`es`, `us`, `fr`…). `None` = la del sistema.
    pub teclado: Option<String>,
    /// Cuántos escritorios virtuales hay.
    ///
    /// Dos de serie y un máximo de cinco, que es lo que cabe en la vista
    /// general sin convertir las previsualizaciones en sellos.
    pub escritorios: usize,
    /// Nombres editables de los escritorios, en el mismo orden.
    pub nombres_escritorios: Vec<String>,
    /// Claro u oscuro. Lo aplica quien crea el shell, porque el tema es del
    /// proceso entero y no de una superficie.
    pub tema: crate::tema::Tema,
    /// El color de acento, de la tabla cerrada de [`crate::tema::Acento`].
    pub acento: crate::tema::Acento,
    /// Foto de perfil para el bloqueo. `None` = la del sistema (`~/.face` o
    /// AccountsService), y si tampoco hay, las iniciales.
    pub avatar: Option<String>,
    /// Composición visual del bloqueo. Vive en la configuración compartida
    /// para que BookOS Settings pueda editarla sin conocer el código de iced.
    pub bloqueo: Bloqueo,
    /// Isla de tareas vivas; estas claves quedan preparadas para BookOS
    /// Settings y permiten desactivar movimiento sin apagar la función.
    pub actividades: Actividades,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actividades {
    pub habilitadas: bool,
    pub animaciones: bool,
    pub temporizador_siempre: bool,
}

impl Default for Actividades {
    fn default() -> Self {
        Self { habilitadas: true, animaciones: true, temporizador_siempre: false }
    }
}

/// Opciones de la pantalla de bloqueo.
///
/// Las posiciones son fracciones del alto lógico de la salida. De ese modo un
/// valor guardado sirve igual en 1920×1080, 2880×1800 y con escala fraccional:
/// la densidad cambia los píxeles físicos, no la composición que ve el usuario.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bloqueo {
    pub animaciones: bool,
    pub fecha: bool,
    pub medios: bool,
    pub reloj_y: f32,
    pub acceso_y: f32,
    pub medios_y: f32,
    pub reloj_tamano: f32,
    pub avatar_tamano: f32,
}

impl Default for Bloqueo {
    fn default() -> Self {
        Self {
            animaciones: true,
            fecha: true,
            medios: true,
            // La referencia: reloj en el primer tercio, identidad en el centro
            // y la tarjeta de lo que suena por debajo del acceso.
            reloj_y: 0.08,
            acceso_y: 0.36,
            medios_y: 0.68,
            reloj_tamano: 144.0,
            avatar_tamano: 132.0,
        }
    }
}

/// Lo que se le pide a libinput sobre los dispositivos de entrada.
///
/// Son los cuatro ajustes que se tocan de verdad en un portátil. El resto de
/// libinput se queda en sus valores por defecto: exponerlo entero sería un
/// panel de control, no un fichero de configuración.
pub struct Entrada {
    /// Velocidad del touchpad, en la escala de libinput: [-1, 1], 0 = normal.
    pub velocidad_touchpad: f64,
    /// Velocidad del ratón y del trackpoint, misma escala.
    pub velocidad_raton: f64,
    pub toque_para_clic: bool,
    pub scroll_natural: bool,
}

impl Default for Entrada {
    fn default() -> Self {
        Self {
            // 0,3 y no 0: con la velocidad de serie de libinput hay que dar
            // tres pasadas al touchpad para cruzar una pantalla de 2880 px.
            // Es el mismo valor que este escritorio tiene puesto en KDE
            // (`PointerAcceleration=0.300` en `kcminputrc`), o sea el que ya
            // estaba calibrado a mano contra esta máquina.
            velocidad_touchpad: 0.3,
            // El ratón se queda en el de serie: tiene su propia resolución y
            // acelerarlo por defecto lo vuelve nervioso.
            velocidad_raton: 0.0,
            toque_para_clic: true,
            scroll_natural: true,
        }
    }
}

/// Cuántos escritorios virtuales caben como mucho.
///
/// Cinco es lo que entra en la vista general sin convertir las
/// previsualizaciones en sellos, y es también hasta dónde llegan los iconos
/// numerados del aviso. El compositor tiene el mismo tope y por eso este valor
/// no puede subir aquí sin subir allí.
pub const MAXIMO_ESCRITORIOS: usize = 5;

/// El `exec` reservado que abre el launchpad en vez de lanzar un programa.
///
/// Es una palabra y no un binario porque el launchpad no es un programa: vive
/// dentro del shell. Quien quiera quitarlo del dock solo tiene que dejarlo
/// fuera de la lista `dock` de su configuración.
pub const LAUNCHPAD: &str = "launchpad";

/// Un lanzador del dock, tal como sale de la configuración.
pub struct Lanzador {
    pub exec: String,
    pub etiqueta: String,
    pub icono: String,
    /// El `app_id` de Wayland con el que se reconoce su ventana.
    pub app_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // El centro va vacío y el reloj el último de la derecha, como la
            // referencia del panel y como el propio plasmoide. La capa central
            // sigue existiendo para quien quiera poner ahí algo —`centro =
            // reloj` la recupera—, pero ya no es lo de serie.
            centro: None,
            // El orden del diseño, de izquierda a derecha: lo que cambia solo
            // —batería, conexiones— antes que lo que se toca, y el reloj el
            // último, pegado a la esquina.
            derecha: vec![
                // El primero de la fila: es el único que dice *dónde estás* y
                // no *qué tienes*, así que va aparte de los estados, pegado al
                // borde izquierdo del grupo.
                "escritorios".into(),
                "bateria".into(),
                "bluetooth".into(),
                "red".into(),
                "volumen".into(),
                "brillo".into(),
                "notificaciones".into(),
                "control".into(),
                "reloj".into(),
            ],
            dock: [
                // El primero, como en el dock de macOS: es el cajón de todo lo
                // demás y conviene que esté donde siempre.
                (LAUNCHPAD, "Aplicaciones", "launchpad"),
                ("konsole", "Terminal", "utilities-terminal"),
                ("dolphin", "Archivos", "system-file-manager"),
                ("firefox", "Navegador", "firefox"),
                // Los ajustes son los de BookOS, no los de Plasma: abrir el
                // panel de otro escritorio desde este es enseñar opciones que
                // no gobiernan lo que se está usando.
                ("bookos-settings", "Ajustes", "bookos-settings"),
                ("kate", "Editor", "accessories-text-editor"),
            ]
            .into_iter()
            .map(|(e, l, i)| Lanzador {
                exec: e.to_string(),
                etiqueta: l.to_string(),
                icono: i.to_string(),
                app_id: e.to_string(),
            })
            .collect(),
            escala: None,
            // 24 lógicos: el de KDE y el de GNOME, y el que asumen los temas
            // al elegir imagen. Pedir 24 a escala 1,75 da los 42 px que el
            // tema tiene dibujados de verdad, sin inventar píxeles.
            cursor: 24,
            entrada: Entrada::default(),
            fondo: None,
            teclado: None,
            escritorios: 2,
            nombres_escritorios: vec!["Escritorio 1".into(), "Escritorio 2".into()],
            tema: crate::tema::Tema::Oscuro,
            acento: crate::tema::Acento::Azul,
            avatar: None,
            bloqueo: Bloqueo::default(),
            actividades: Actividades::default(),
        }
    }
}

impl Config {
    pub fn cargar() -> Self {
        let Some(ruta) = ruta() else {
            return Self::default();
        };
        let Ok(texto) = std::fs::read_to_string(&ruta) else {
            return Self::default();
        };
        Self::desde_texto(&texto, &ruta)
    }

    /// El análisis, separado de la lectura para poder probarlo sin tocar el
    /// disco: es donde están todas las decisiones que pueden salir mal.
    fn desde_texto(texto: &str, ruta: &std::path::Path) -> Self {
        let mut config = Self::default();
        for (n, linea) in texto.lines().enumerate() {
            let linea = linea.trim();
            if linea.is_empty() || linea.starts_with('#') {
                continue;
            }
            let Some((clave, valor)) = linea.split_once('=') else {
                tracing::warn!(?ruta, linea = n + 1, "línea sin '=', se ignora");
                continue;
            };
            let items = || {
                valor
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            };
            match clave.trim() {
                // "centro = " a secas es la forma de quitar el reloj.
                "centro" => config.centro = items().into_iter().next(),
                "derecha" => config.derecha = items(),
                "dock" => config.dock = items().iter().filter_map(|s| lanzador(s)).collect(),
                // Un valor absurdo se descarta en vez de aplicarse: una escala
                // de 0 deja la pantalla en 0x0 píxeles lógicos y el escritorio
                // no vuelve a arrancar hasta editar el fichero a ciegas.
                "escritorios" => match valor.trim().parse::<usize>() {
                    Ok(v) => config.escritorios = v.clamp(1, MAXIMO_ESCRITORIOS),
                    Err(_) => tracing::warn!(valor, "«escritorios» no es un número"),
                },
                "nombres_escritorios" => config.nombres_escritorios = items(),
                // Sin comprobar que exista: el fichero puede llegar después que
                // la configuración —un montaje de red, por ejemplo— y descartar
                // la ruta aquí obligaría a editar el fichero otra vez.
                "avatar" => config.avatar = Some(valor.trim().to_string()),
                "bloqueo_animaciones" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.bloqueo.animaciones = v;
                    }
                }
                "actividades" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.actividades.habilitadas = v;
                    }
                }
                "actividades_animaciones" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.actividades.animaciones = v;
                    }
                }
                "temporizador_siempre_visible" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.actividades.temporizador_siempre = v;
                    }
                }
                "bloqueo_fecha" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.bloqueo.fecha = v;
                    }
                }
                "bloqueo_medios" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.bloqueo.medios = v;
                    }
                }
                "bloqueo_reloj_y" => {
                    if let Some(v) = decimal(valor, 0.02..=0.40, ruta, n + 1) {
                        config.bloqueo.reloj_y = v as f32;
                    }
                }
                "bloqueo_acceso_y" => {
                    if let Some(v) = decimal(valor, 0.18..=0.72, ruta, n + 1) {
                        config.bloqueo.acceso_y = v as f32;
                    }
                }
                "bloqueo_medios_y" => {
                    if let Some(v) = decimal(valor, 0.42..=0.88, ruta, n + 1) {
                        config.bloqueo.medios_y = v as f32;
                    }
                }
                "bloqueo_reloj_tamano" => {
                    if let Some(v) = decimal(valor, 72.0..=220.0, ruta, n + 1) {
                        config.bloqueo.reloj_tamano = v as f32;
                    }
                }
                "bloqueo_avatar_tamano" => {
                    if let Some(v) = decimal(valor, 64.0..=220.0, ruta, n + 1) {
                        config.bloqueo.avatar_tamano = v as f32;
                    }
                }
                // Cualquier otra cosa se queda en oscuro y se avisa: un tema
                // mal escrito no puede dejar el escritorio a medio pintar.
                "tema" => match valor.trim() {
                    "claro" => config.tema = crate::tema::Tema::Claro,
                    "oscuro" => config.tema = crate::tema::Tema::Oscuro,
                    otro => tracing::warn!(otro, "«tema» solo entiende claro u oscuro"),
                },
                // Un nombre que no está en la tabla se ignora en vez de
                // dejar el escritorio con un acento a medias.
                "acento" => match crate::tema::Acento::desde_nombre(valor) {
                    Some(a) => config.acento = a,
                    None => tracing::warn!(valor = valor.trim(), "acento desconocido"),
                },
                "escala" => match valor.trim().replace(',', ".").parse::<f64>() {
                    Ok(v) if (0.5..=4.0).contains(&v) => config.escala = Some(v),
                    _ => tracing::warn!(
                        ?ruta,
                        linea = n + 1,
                        valor = valor.trim(),
                        "escala fuera de [0,5 - 4]; se ignora"
                    ),
                },
                // El límite de arriba no es un capricho: un cursor enorme deja
                // de caber en el plano de hardware del cursor y pasa a
                // componerse en la GPU, con lo que mover el ratón vuelve a
                // costar un frame entero.
                "cursor" => match valor.trim().parse::<u32>() {
                    Ok(v) if (8..=128).contains(&v) => config.cursor = v,
                    _ => tracing::warn!(
                        ?ruta,
                        linea = n + 1,
                        valor = valor.trim(),
                        "tamaño de cursor fuera de [8 - 128]; se ignora"
                    ),
                },
                "fondo" => config.fondo = Some(valor.trim().to_string()),
                "teclado" => config.teclado = Some(valor.trim().to_string()),
                "velocidad_touchpad" => {
                    if let Some(v) = velocidad(valor, ruta, n + 1) {
                        config.entrada.velocidad_touchpad = v;
                    }
                }
                "velocidad_raton" => {
                    if let Some(v) = velocidad(valor, ruta, n + 1) {
                        config.entrada.velocidad_raton = v;
                    }
                }
                "toque_para_clic" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.entrada.toque_para_clic = v;
                    }
                }
                "scroll_natural" => {
                    if let Some(v) = booleano(valor, ruta, n + 1) {
                        config.entrada.scroll_natural = v;
                    }
                }
                otro => tracing::warn!(?ruta, linea = n + 1, clave = otro, "clave desconocida"),
            }
        }
        config.nombres_escritorios.truncate(config.escritorios);
        while config.nombres_escritorios.len() < config.escritorios {
            config.nombres_escritorios.push(format!(
                "Escritorio {}",
                config.nombres_escritorios.len() + 1
            ));
        }
        config
    }
}

/// Una velocidad de puntero en la escala de libinput. Fuera de [-1, 1]
/// libinput rechaza el valor y el dispositivo se queda como estaba, así que
/// más vale avisar aquí que dejar que falle en silencio al aplicarlo.
fn velocidad(valor: &str, ruta: &std::path::Path, linea: usize) -> Option<f64> {
    match valor.trim().replace(',', ".").parse::<f64>() {
        Ok(v) if (-1.0..=1.0).contains(&v) => Some(v),
        _ => {
            tracing::warn!(
                ?ruta,
                linea,
                valor = valor.trim(),
                "velocidad fuera de [-1 - 1]; se ignora"
            );
            None
        }
    }
}

fn decimal(
    valor: &str,
    rango: std::ops::RangeInclusive<f64>,
    ruta: &std::path::Path,
    linea: usize,
) -> Option<f64> {
    match valor.trim().replace(',', ".").parse::<f64>() {
        Ok(v) if rango.contains(&v) => Some(v),
        _ => {
            tracing::warn!(
                ?ruta,
                linea,
                valor = valor.trim(),
                minimo = *rango.start(),
                maximo = *rango.end(),
                "valor fuera de rango; se ignora"
            );
            None
        }
    }
}

fn booleano(valor: &str, ruta: &std::path::Path, linea: usize) -> Option<bool> {
    match valor.trim() {
        "si" | "sí" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        otro => {
            tracing::warn!(?ruta, linea, valor = otro, "se esperaba sí/no; se ignora");
            None
        }
    }
}

/// `exec:etiqueta:icono:app_id`, y todo menos `exec` se puede omitir.
///
/// Lo que falta se deduce del ejecutable, que es lo que acierta la mayoría de
/// las veces: `firefox` a secas ya da el icono y el `app_id` correctos.
fn lanzador(spec: &str) -> Option<Lanzador> {
    let mut campos = spec.split(':').map(str::trim);
    let exec = campos.next().filter(|s| !s.is_empty())?;
    let o_exec = |c: Option<&str>| c.filter(|s| !s.is_empty()).unwrap_or(exec).to_string();
    Some(Lanzador {
        etiqueta: o_exec(campos.next()),
        icono: o_exec(campos.next()),
        app_id: o_exec(campos.next()),
        exec: exec.to_string(),
    })
}

/// Reescribe la clave `dock` del fichero, dejando lo demás como está.
pub fn guardar_dock(anclados: &[String]) -> std::io::Result<()> {
    escribir_claves(&[("dock", anclados.join(", "))])
}

/// Persiste la cantidad y los nombres sin reescribir el resto del fichero.
pub fn guardar_escritorios(nombres: &[String]) -> std::io::Result<()> {
    // La coma separa los nombres en el fichero, así que un nombre con coma
    // partiría la lista en dos escritorios al releerla.
    let nombres: Vec<_> = nombres
        .iter()
        .map(|n| n.replace([',', '\n', '\r'], " "))
        .collect();
    escribir_claves(&[
        ("escritorios", nombres.len().to_string()),
        ("nombres_escritorios", nombres.join(", ")),
    ])
}

/// Persiste el tema y el acento que se acaban de elegir en Apariencia.
pub fn guardar_apariencia(
    tema: crate::tema::Tema,
    acento: crate::tema::Acento,
) -> std::io::Result<()> {
    let tema = match tema {
        crate::tema::Tema::Claro => "claro",
        crate::tema::Tema::Oscuro => "oscuro",
    };
    escribir_claves(&[
        ("tema", tema.to_string()),
        ("acento", acento.nombre().to_string()),
    ])
}

/// Reescribe esas claves del fichero y deja lo demás como está.
///
/// A mano y no serializando la `Config` entera porque el fichero es del
/// usuario: lleva sus comentarios y su orden, y volcarlo desde el código los
/// borraría. La clave que ya estaba se sustituye en su sitio; la que no,
/// se añade al final.
fn escribir_claves(valores: &[(&str, String)]) -> std::io::Result<()> {
    let Some(ruta) = ruta() else {
        return Ok(());
    };
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let anterior = std::fs::read_to_string(&ruta).unwrap_or_default();
    let mut vistos = vec![false; valores.len()];
    let mut salida = String::new();
    for linea in anterior.lines() {
        let clave = linea.split_once('=').map(|(k, _)| k.trim());
        if let Some(i) = valores.iter().position(|(k, _)| Some(*k) == clave) {
            // Repetida en el fichero: la primera se sustituye y las demás se
            // caen, que es lo que hace la lectura —se queda con la última— al
            // revés, pero deja el fichero sin claves duplicadas.
            if !vistos[i] {
                salida.push_str(&format!("{} = {}", valores[i].0, valores[i].1));
                vistos[i] = true;
            } else {
                continue;
            }
        } else {
            salida.push_str(linea);
        }
        salida.push('\n');
    }
    for (i, (clave, valor)) in valores.iter().enumerate() {
        if !vistos[i] {
            salida.push_str(&format!("{clave} = {valor}\n"));
        }
    }
    std::fs::write(ruta, salida)
}

/// El launchpad guardado, en `~/.config/bookos/launchpad.conf`.
///
/// Un fichero aparte y no una clave de `panel.conf` porque esto **lo escribe el
/// escritorio**, no el usuario: cada vez que se arrastra un icono se reescribe
/// entero, y mezclarlo con la configuración escrita a mano acabaría pisando los
/// comentarios de alguien.
///
/// Una carpeta por línea, `nombre[:color] = exec1, exec2, …`, más la lista de
/// las aplicaciones que se han quitado de la rejilla:
///
/// ```text
/// Utilidades:verde = konsole, kate, kcalc
/// Internet = firefox, thunderbird
/// ocultas = xterm, gnome-tetravex
/// ```
///
/// El color es uno de la tabla de [`crate::tema::Acento`]; sin él, la carpeta
/// va en gris. [`CLAVE_OCULTAS`] es una clave reservada: una carpeta no puede
/// llamarse así.
///
/// Las aplicaciones se identifican por su `exec` porque es lo que ya usa el
/// dock en `panel.conf`: dos formas de nombrar la misma aplicación en el mismo
/// escritorio se separan a la primera.
pub const CLAVE_OCULTAS: &str = "ocultas";

/// Lo que hay en `launchpad.conf`.
#[derive(Default)]
pub struct Launchpad {
    /// Nombre, color —si lo tiene— y aplicaciones de cada carpeta.
    pub carpetas: Vec<(String, Option<String>, Vec<String>)>,
    /// Las que el usuario ha quitado de la rejilla.
    pub ocultas: Vec<String>,
}

pub fn cargar_launchpad() -> Launchpad {
    let Some(ruta) = ruta_launchpad() else {
        return Launchpad::default();
    };
    let Ok(texto) = std::fs::read_to_string(&ruta) else {
        return Launchpad::default();
    };
    interpretar_launchpad(&texto)
}

fn interpretar_launchpad(texto: &str) -> Launchpad {
    let mut salida = Launchpad::default();
    for linea in texto
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let Some((clave, valores)) = linea.split_once('=') else {
            continue;
        };
        let valores: Vec<String> = valores
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        // Una carpeta vacía no se enseña: sería un icono que al abrirse no
        // tiene nada dentro.
        if valores.is_empty() {
            continue;
        }
        let clave = clave.trim();
        if clave == CLAVE_OCULTAS {
            salida.ocultas = valores;
            continue;
        }
        let (nombre, color) = match clave.split_once(':') {
            Some((n, c)) => (n.trim(), Some(c.trim().to_string())),
            None => (clave, None),
        };
        salida.carpetas.push((nombre.to_string(), color, valores));
    }
    salida
}

/// Reescribe el fichero entero. Lo llama el launchpad al cambiar algo.
pub fn guardar_launchpad(datos: &Launchpad) -> std::io::Result<()> {
    let Some(ruta) = ruta_launchpad() else {
        return Ok(());
    };
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut salida = String::from(
        "# El launchpad: carpetas y aplicaciones quitadas de la rejilla.\n         # Lo escribe el escritorio; se puede editar a mano.\n         # Una carpeta por línea: nombre[:color] = exec1, exec2, …\n",
    );
    for (nombre, color, apps) in &datos.carpetas {
        // Ni `=`, ni `,`, ni `:` en el nombre: los tres partirían la línea al
        // volver a leerla. Se sustituyen en vez de perder la carpeta entera.
        let nombre = nombre.replace(['=', ',', ':', '\n', '\r'], " ");
        let nombre = nombre.trim();
        match color {
            Some(color) => salida.push_str(&format!("{nombre}:{color} = {}\n", apps.join(", "))),
            None => salida.push_str(&format!("{nombre} = {}\n", apps.join(", "))),
        }
    }
    if !datos.ocultas.is_empty() {
        salida.push_str(&format!("{CLAVE_OCULTAS} = {}\n", datos.ocultas.join(", ")));
    }
    std::fs::write(ruta, salida)
}

fn ruta_launchpad() -> Option<PathBuf> {
    Some(ruta()?.with_file_name("launchpad.conf"))
}

fn ruta() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir).join("bookos/panel.conf"));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/bookos/panel.conf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsear(texto: &str) -> Config {
        Config::desde_texto(texto, std::path::Path::new("prueba.conf"))
    }

    #[test]
    fn sin_fichero_manda_lo_de_serie() {
        let c = parsear("");
        assert_eq!(c.entrada.velocidad_touchpad, 0.3);
        assert!(c.entrada.toque_para_clic);
        assert!(c.entrada.scroll_natural);
        assert_eq!(c.escala, None);
        assert_eq!(c.escritorios, 2);
        assert_eq!(c.nombres_escritorios, ["Escritorio 1", "Escritorio 2"]);
    }

    #[test]
    fn escritorios_se_acotan_y_los_nombres_se_completan() {
        let c = parsear("escritorios = 9\nnombres_escritorios = Trabajo, Juegos\n");
        assert_eq!(c.escritorios, 5);
        assert_eq!(c.nombres_escritorios.len(), 5);
        assert_eq!(&c.nombres_escritorios[..2], ["Trabajo", "Juegos"]);
        assert_eq!(parsear("escritorios = 0").escritorios, 1);
    }

    #[test]
    fn la_entrada_se_lee_entera() {
        let c = parsear(
            "velocidad_touchpad = 0.8\n\
             velocidad_raton = -0.2\n\
             toque_para_clic = no\n\
             scroll_natural = no\n",
        );
        assert_eq!(c.entrada.velocidad_touchpad, 0.8);
        assert_eq!(c.entrada.velocidad_raton, -0.2);
        assert!(!c.entrada.toque_para_clic);
        assert!(!c.entrada.scroll_natural);
    }

    /// Escribir la coma decimal es lo natural en castellano y no debería
    /// costar una velocidad ignorada; ya se aceptaba en `escala`.
    #[test]
    fn la_coma_decimal_vale() {
        let c = parsear("velocidad_touchpad = 0,5");
        assert_eq!(c.entrada.velocidad_touchpad, 0.5);
    }

    /// libinput rechaza lo que se salga de [-1, 1] y deja el dispositivo como
    /// estaba, así que una errata tiene que quedarse en el valor de serie y no
    /// en un silencio.
    #[test]
    fn una_velocidad_imposible_no_se_aplica() {
        let alta = parsear("velocidad_touchpad = 5");
        assert_eq!(alta.entrada.velocidad_touchpad, 0.3);
        let absurda = parsear("velocidad_raton = mucha");
        assert_eq!(absurda.entrada.velocidad_raton, 0.0);
    }

    #[test]
    fn el_tamano_del_cursor_se_lee_y_se_acota() {
        assert_eq!(parsear("cursor = 32").cursor, 32);
        // Un cursor de 4 px no se ve y uno de 500 no cabe en el plano de
        // hardware: en ambos casos vale más quedarse con el de serie.
        assert_eq!(parsear("cursor = 4").cursor, 24);
        assert_eq!(parsear("cursor = 500").cursor, 24);
    }

    #[test]
    fn la_composicion_del_bloqueo_es_configurable_y_segura() {
        let c = parsear(
            "bloqueo_animaciones = no\n\
             bloqueo_fecha = no\n\
             bloqueo_medios = sí\n\
             bloqueo_reloj_y = 0.12\n\
             bloqueo_acceso_y = 0.42\n\
             bloqueo_medios_y = 0.73\n\
             bloqueo_reloj_tamano = 168\n\
             bloqueo_avatar_tamano = 150\n",
        );
        assert!(!c.bloqueo.animaciones);
        assert!(!c.bloqueo.fecha);
        assert!(c.bloqueo.medios);
        assert_eq!(c.bloqueo.reloj_y, 0.12);
        assert_eq!(c.bloqueo.acceso_y, 0.42);
        assert_eq!(c.bloqueo.medios_y, 0.73);
        assert_eq!(c.bloqueo.reloj_tamano, 168.0);
        assert_eq!(c.bloqueo.avatar_tamano, 150.0);

        // Una posición fuera de pantalla no pisa el valor utilizable de serie.
        assert_eq!(parsear("bloqueo_acceso_y = 4").bloqueo.acceso_y, 0.36);
    }

    #[test]
    fn las_actividades_y_su_movimiento_se_pueden_desactivar() {
        let c = parsear(
            "actividades = no\n\
             actividades_animaciones = no\n\
             temporizador_siempre_visible = sí\n",
        );
        assert!(!c.actividades.habilitadas);
        assert!(!c.actividades.animaciones);
        assert!(c.actividades.temporizador_siempre);
    }

    #[test]
    fn el_launchpad_va_y_vuelve() {
        let texto = "# comentario\n\
                     Utilidades:verde = konsole, kate , kcalc\n\
                     \n\
                     Vacia = \n\
                     Internet = firefox\n\
                     ocultas = xterm\n";
        let datos = interpretar_launchpad(texto);
        assert_eq!(datos.carpetas.len(), 2, "la vacía no cuenta");
        assert_eq!(datos.carpetas[0].0, "Utilidades");
        assert_eq!(datos.carpetas[0].1.as_deref(), Some("verde"), "el color");
        assert_eq!(datos.carpetas[0].2, ["konsole", "kate", "kcalc"]);
        assert_eq!(datos.carpetas[1].1, None, "sin color va en gris");
        assert_eq!(datos.ocultas, ["xterm"]);
    }

    #[test]
    fn el_acento_sale_de_la_tabla_y_lo_demas_se_ignora() {
        assert_eq!(
            parsear("acento = morado").acento,
            crate::tema::Acento::Morado
        );
        // Un acento inventado deja el de serie: el escritorio tiene que
        // arrancar con un color, no con ninguno.
        assert_eq!(parsear("acento = fucsia").acento, crate::tema::Acento::Azul);
    }

    #[test]
    fn el_booleano_entiende_si_y_no() {
        assert!(parsear("toque_para_clic = sí").entrada.toque_para_clic);
        assert!(!parsear("toque_para_clic = 0").entrada.toque_para_clic);
        // Lo que no entiende deja el valor de serie en vez de inventarse uno.
        assert!(parsear("toque_para_clic = quizá").entrada.toque_para_clic);
    }

    /// Una línea mala no puede llevarse por delante a las demás: el escritorio
    /// tiene que arrancar igual.
    #[test]
    fn una_errata_no_tumba_el_resto() {
        let c = parsear("escala = 1.5\nesto no es nada\nvelocidad_touchpad = 0.4\n");
        assert_eq!(c.escala, Some(1.5));
        assert_eq!(c.entrada.velocidad_touchpad, 0.4);
    }
}
