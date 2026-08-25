//! La lista de aplicaciones instaladas, leída de los `.desktop`.
//!
//! El launchpad de Plasma usa `Kicker.RootModel`, que fuera de plasmashell no
//! existe. Aquí se leen los ficheros directamente, que es lo que hace Kicker
//! por debajo: son INI y la especificación de entradas de escritorio de
//! freedesktop es corta.
//!
//! Se lee **una vez**, al abrir el launchpad. Vigilar los directorios con
//! inotify para enterarse de una instalación sería un descriptor abierto y un
//! despertar por cada `dnf install` a cambio de nada: el launchpad se cierra y
//! se vuelve a abrir en un segundo.

use std::collections::HashSet;
use std::path::PathBuf;

pub struct App {
    pub nombre: String,
    /// Ya limpio de los códigos `%f`, `%u`… de la especificación.
    pub exec: String,
    pub icono: String,
    /// El nombre en minúsculas y sin acentos, para buscar sin repetir el
    /// trabajo en cada pulsación de tecla.
    pub(crate) normalizado: String,
}

/// Dónde busca la especificación de freedesktop, de más prioritario a menos.
fn directorios() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/applications"));
    }
    dirs.push(PathBuf::from("/usr/local/share/applications"));
    dirs.push(PathBuf::from("/usr/share/applications"));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs
}

/// Todas las aplicaciones visibles, ordenadas por nombre.
pub fn leer() -> Vec<App> {
    let mut vistos = HashSet::new();
    let mut apps = Vec::new();

    for dir in directorios() {
        let Ok(entradas) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entrada in entradas.filter_map(|e| e.ok()) {
            let ruta = entrada.path();
            if ruta.extension().is_none_or(|e| e != "desktop") {
                continue;
            }
            // El nombre del fichero es el identificador: el mismo id en un
            // directorio de más prioridad gana, que es como el usuario
            // sustituye un lanzador del sistema por el suyo.
            let Some(id) = ruta.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            if !vistos.insert(id) {
                continue;
            }
            if let Some(app) = leer_una(&ruta) {
                apps.push(app);
            }
        }
    }

    apps.sort_by(|a, b| a.normalizado.cmp(&b.normalizado));
    apps
}

/// La aplicación que corresponde a un `app_id` de Wayland.
///
/// Se prueban las rutas que el `app_id` implica —`org.kde.konsole.desktop`, y
/// el último componente, `konsole.desktop`— antes de recorrer todo el
/// directorio: acertar es un `stat`, y solo se paga la lectura completa cuando
/// el `.desktop` no se llama como la aplicación dice llamarse.
pub fn por_app_id(app_id: &str) -> Option<App> {
    let cola = app_id.rsplit('.').next().unwrap_or(app_id);
    for dir in directorios() {
        for nombre in [app_id, cola] {
            let ruta = dir.join(format!("{nombre}.desktop"));
            if let Some(app) = leer_una(&ruta) {
                return Some(app);
            }
        }
    }
    None
}

pub(crate) fn leer_una(ruta: &std::path::Path) -> Option<App> {
    let texto = std::fs::read_to_string(ruta).ok()?;

    let mut nombre = None;
    let mut exec = None;
    let mut icono = None;
    let mut oculta = false;
    let mut es_app = false;
    let mut en_entrada = false;

    for linea in texto.lines() {
        let linea = linea.trim();
        // Un `.desktop` puede traer varios grupos —las "acciones" del menú
        // contextual— y solo el principal describe la aplicación. Sin esto, un
        // `Name` de una acción pisaba el de la app.
        if linea.starts_with('[') {
            en_entrada = linea == "[Desktop Entry]";
            continue;
        }
        if !en_entrada {
            continue;
        }
        let Some((clave, valor)) = linea.split_once('=') else {
            continue;
        };
        match clave.trim() {
            // Las traducciones vienen como `Name[es]`; se ignoran a propósito
            // hasta que el shell tenga idioma propio, para no coger la primera
            // que aparezca en el fichero.
            "Name" => nombre = Some(valor.trim().to_string()),
            "Exec" => exec = Some(limpiar_exec(valor.trim())),
            "Icon" => icono = Some(valor.trim().to_string()),
            "NoDisplay" | "Hidden" => oculta |= valor.trim() == "true",
            "Type" => es_app = valor.trim() == "Application",
            _ => {}
        }
    }

    if oculta || !es_app {
        return None;
    }
    let nombre = nombre?;
    let exec = exec.filter(|e| !e.is_empty())?;
    let normalizado = normalizar(&nombre);
    Some(App {
        icono: icono.unwrap_or_else(|| nombre.clone()),
        nombre,
        exec,
        normalizado,
    })
}

/// Quita los códigos de campo de la especificación.
///
/// `Exec=firefox %u` lanzado tal cual le pasa un `%u` literal a Firefox. Se
/// quitan todos los `%x` menos `%%`, que es un porcentaje de verdad.
fn limpiar_exec(exec: &str) -> String {
    let mut salida = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            salida.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => salida.push('%'),
            // %f %F %u %U %i %c %k y cualquier otro: fuera.
            Some(_) => {}
            None => {}
        }
    }
    salida.trim().to_string()
}

/// Minúsculas y sin diacríticos, para que "cámara" case con "camara".
///
/// No es una normalización Unicode de verdad —eso sería NFD y una tabla— sino
/// la lista de vocales acentuadas y la eñe, que es lo que aparece en los
/// nombres de aplicaciones que vamos a ver.
fn normalizar(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            otro => otro,
        })
        .collect()
}

/// Cuánto encaja `consulta` con la app. Menor es mejor; `None` si no encaja.
///
/// La escala es la del launchpad de Plasma, y el orden importa más que los
/// números: lo que se busca es que escribir "fir" ponga Firefox el primero y no
/// una aplicación cualquiera que lleve "fir" a mitad de palabra.
pub fn puntuar(app: &App, consulta: &str) -> Option<u32> {
    let consulta = normalizar(consulta);
    if consulta.is_empty() {
        return Some(0);
    }
    let nombre = &app.normalizado;

    if *nombre == consulta {
        return Some(0);
    }
    if nombre.starts_with(&consulta) {
        return Some(1);
    }
    // Prefijo de cualquier palabra: "web" encuentra "Navegador Web".
    if nombre.split_whitespace().any(|p| p.starts_with(&consulta)) {
        return Some(2);
    }
    // Iniciales: "vsc" encuentra "Visual Studio Code".
    let iniciales: String = nombre
        .split_whitespace()
        .filter_map(|p| p.chars().next())
        .collect();
    if iniciales.starts_with(&consulta) {
        return Some(3);
    }
    if nombre.contains(&consulta) {
        return Some(4);
    }
    // Subsecuencia difusa: las letras en orden aunque no seguidas.
    let mut cs = consulta.chars();
    let mut actual = cs.next();
    for c in nombre.chars() {
        if Some(c) == actual {
            actual = cs.next();
        }
    }
    actual.is_none().then_some(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(nombre: &str) -> App {
        App {
            normalizado: normalizar(nombre),
            nombre: nombre.to_string(),
            exec: "x".into(),
            icono: "x".into(),
        }
    }

    #[test]
    fn los_codigos_de_campo_no_llegan_al_programa() {
        assert_eq!(limpiar_exec("firefox %u"), "firefox");
        assert_eq!(limpiar_exec("kate %F"), "kate");
        assert_eq!(
            limpiar_exec("env FOO=1 app -x %U --flag"),
            "env FOO=1 app -x  --flag"
        );
        // `%%` sí es un porcentaje de verdad.
        assert_eq!(limpiar_exec("cosa --al 50%%"), "cosa --al 50%");
    }

    #[test]
    fn la_puntuacion_ordena_como_se_espera() {
        let firefox = app("Firefox");
        let navegador = app("Navegador Web");
        let vsc = app("Visual Studio Code");

        // Prefijo gana a substring.
        assert!(puntuar(&firefox, "fir") < puntuar(&navegador, "web").map(|_| 99));
        assert_eq!(puntuar(&firefox, "firefox"), Some(0));
        assert_eq!(puntuar(&firefox, "fire"), Some(1));
        assert_eq!(puntuar(&navegador, "web"), Some(2));
        assert_eq!(puntuar(&vsc, "vsc"), Some(3));
        assert_eq!(puntuar(&firefox, "efo"), Some(4));
        // Y lo que no está, no está.
        assert_eq!(puntuar(&firefox, "zzz"), None);
    }

    #[test]
    fn se_busca_sin_acentos() {
        let camara = app("Cámara");
        assert_eq!(puntuar(&camara, "camara"), Some(0));
        assert_eq!(puntuar(&camara, "cam"), Some(1));
    }
}
