//! Qué widgets lleva el panel y en qué orden.
//!
//! El fichero es `~/.config/bookos/panel.conf` y tiene esta pinta:
//!
//! ```text
//! # Los widgets del panel, por zona.
//! centro = reloj
//! derecha = red, brillo, bateria
//! escala = 1.75
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
///
/// Se hace a mano y no serializando la `Config` entera porque el fichero es del
/// usuario: lleva sus comentarios y su orden, y volcarlo desde el código los
/// borraría. Se sustituye la línea si existe y se añade al final si no.
pub fn guardar_dock(anclados: &[String]) -> std::io::Result<()> {
    let Some(ruta) = ruta() else {
        return Ok(());
    };
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let anterior = std::fs::read_to_string(&ruta).unwrap_or_default();
    let linea = format!("dock = {}", anclados.join(", "));
    let mut salida = String::with_capacity(anterior.len() + linea.len());
    let mut sustituida = false;
    for l in anterior.lines() {
        if l.trim_start().starts_with("dock") && l.contains('=') && !sustituida {
            salida.push_str(&linea);
            sustituida = true;
        } else {
            salida.push_str(l);
        }
        salida.push('\n');
    }
    if !sustituida {
        salida.push_str(&linea);
        salida.push('\n');
    }
    std::fs::write(&ruta, salida)
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
