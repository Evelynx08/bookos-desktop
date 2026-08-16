//! Encontrar y cargar los iconos de las aplicaciones.
//!
//! Lo comparten el dock y el launchpad. La búsqueda es la de freedesktop hecha
//! a mano —producto de directorios por temas por tamaños por categorías— y por
//! eso **no es gratis**: unos cientos de `stat` por icono que no existe. El
//! dock resuelve cinco al arrancar y no se nota; el launchpad tiene que
//! resolver los de la página que se está viendo y no los de las trescientas
//! aplicaciones que hay instaladas.

use std::path::{Path, PathBuf};

use iced_widget::{image as iced_image, svg};

/// Un icono ya cargado, listo para dibujar.
pub enum Icono {
    Svg(svg::Handle),
    Raster(iced_image::Handle),
}

/// El icono como elemento, al tamaño y con el color que se le pidan.
///
/// Son SVG, que resvg rasteriza al tamaño que se le pida: por eso da igual que
/// el fichero esté en la carpeta de 16 o en la de 32.
///
/// Los iconos de estado son monocromos y su color va dentro del fichero, así
/// que dependen de qué variante del tema esté instalada. Tiñéndolos aquí el
/// panel manda sobre el tema de iconos y no al revés — que es lo que hace falta
/// para que un icono siga al color del texto cuando la batería se pone en rojo.
/// Los de aplicación **no** se tiñen: convertir el zorro de Firefox en una
/// silueta blanca no es lo que nadie quiere.
pub fn ver_teñido(
    icono: &Icono,
    px: f32,
    color: Option<iced_core::Color>,
) -> crate::view::PanelElement<'_> {
    use iced_core::Length;
    use iced_widget::{image as iced_image, svg};
    match icono {
        Icono::Svg(handle) => svg(handle.clone())
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .style(move |_theme, _status| svg::Style { color })
            .into(),
        Icono::Raster(handle) => iced_image(handle.clone())
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .into(),
    }
}

/// Como [`ver_teñido`], pero sin atar el resultado al icono.
///
/// Hace falta donde el icono es temporal —los menús lo piden a [`propio`] al
/// dibujar cada fila— y `ver_teñido` no vale porque su firma lo ata al préstamo
/// aunque por dentro solo clone el `Handle`, que es lo único que el elemento
/// necesita de verdad.
pub fn ver_teñido_propio(
    icono: &Icono,
    px: f32,
    color: iced_core::Color,
) -> crate::view::PanelElement<'static> {
    use iced_core::Length;
    match icono {
        Icono::Svg(handle) => svg(handle.clone())
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .style(move |_theme, _status| svg::Style { color: Some(color) })
            .into(),
        Icono::Raster(handle) => iced_image(handle.clone())
            .width(Length::Fixed(px))
            .height(Length::Fixed(px))
            .into(),
    }
}

/// Un icono a partir de un SVG construido en memoria.
///
/// Lo usan los widgets que se dibujan solos —la batería— en vez de tirar del
/// tema: el pictograma cambia de relleno y de color con el estado, y un tema de
/// iconos solo tiene escalones fijos.
pub fn desde_svg(svg: &str) -> Icono {
    Icono::Svg(svg::Handle::from_memory(svg.as_bytes().to_vec()))
}

/// El icono a un tamaño que no tiene por qué ser cuadrado, y sin teñir: lo que
/// necesita un pictograma que ya trae sus colores dentro.
pub fn ver(icono: &Icono, ancho: f32, alto: f32) -> crate::view::PanelElement<'_> {
    use iced_core::Length;
    match icono {
        Icono::Svg(handle) => svg(handle.clone())
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(alto))
            .into(),
        Icono::Raster(handle) => iced_image(handle.clone())
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(alto))
            .into(),
    }
}

/// Los iconos propios de BookOS, incrustados en el binario.
///
/// Son los mismos trazos que dibujan los plasmoides en QML —viewBox de 24,
/// grosor 2, extremos redondeados— pero como ficheros, en vez de armar un
/// `data:` URI en cada frame. Van dentro del binario por lo mismo que el logo:
/// el panel no puede quedarse sin sus propios iconos porque falte un paquete.
///
/// El color va a `#ffffff` dentro del fichero y lo tiñe [`ver_teñido`]. Con
/// `currentColor` el icono saldría sin pintar: usvg no lo resuelve sin un
/// contexto de estilo que aquí no hay.
const PROPIOS: &[(&str, &[u8])] = &[
    ("volumen-silencio", include_bytes!("../assets/iconos/volumen-silencio.svg")),
    ("volumen-bajo", include_bytes!("../assets/iconos/volumen-bajo.svg")),
    ("volumen-medio", include_bytes!("../assets/iconos/volumen-medio.svg")),
    ("volumen-alto", include_bytes!("../assets/iconos/volumen-alto.svg")),
    ("micro", include_bytes!("../assets/iconos/micro.svg")),
    // --- Del sistema de diseño (heroicons 24/outline) --------------------
    // Vienen de BookOS-HIG y sustituyen a los dibujados a mano: el escritorio
    // y sus aplicaciones enseñan así el mismo icono para la misma cosa.
    ("bateria-0", include_bytes!("../assets/iconos/bateria-0.svg")),
    ("bateria-50", include_bytes!("../assets/iconos/bateria-50.svg")),
    ("bateria-100", include_bytes!("../assets/iconos/bateria-100.svg")),
    ("bateria-carga", include_bytes!("../assets/iconos/bateria-carga.svg")),
    ("wifi", include_bytes!("../assets/iconos/wifi.svg")),
    ("sin-red", include_bytes!("../assets/iconos/sin-red.svg")),
    ("notificaciones", include_bytes!("../assets/iconos/notificaciones.svg")),
    ("notificaciones-aviso", include_bytes!("../assets/iconos/notificaciones-aviso.svg")),
    ("control", include_bytes!("../assets/iconos/control.svg")),

    ("brillo", include_bytes!("../assets/iconos/brillo.svg")),
    ("teclado", include_bytes!("../assets/iconos/teclado.svg")),
    ("touchpad", include_bytes!("../assets/iconos/touchpad.svg")),
    // El logo del sistema, que es lo que abre: el cajón de todo. Va sin teñir
    // —tiene su propia paleta— y por eso el dock lo trata como un icono de
    // aplicación y no como uno de estado.
    ("launchpad", include_bytes!("../assets/iconos/launchpad.svg")),
    // La alternativa del cohete, por si algún día se prefiere: `dock =
    // launchpad:Aplicaciones:launchpad-cohete, ...`
    ("launchpad-cohete", include_bytes!("../assets/iconos/launchpad-cohete.svg")),
    ("bluetooth", include_bytes!("../assets/iconos/bluetooth.svg")),
    ("bluetooth-apagado", include_bytes!("../assets/iconos/bluetooth-apagado.svg")),
    ("micro-silencio", include_bytes!("../assets/iconos/micro-silencio.svg")),
    // --- Los de los menús ------------------------------------------------
    ("acerca", include_bytes!("../assets/iconos/acerca.svg")),
    ("preferencias", include_bytes!("../assets/iconos/preferencias.svg")),
    ("tienda", include_bytes!("../assets/iconos/tienda.svg")),
    ("dormir", include_bytes!("../assets/iconos/dormir.svg")),
    ("reiniciar", include_bytes!("../assets/iconos/reiniciar.svg")),
    ("apagar", include_bytes!("../assets/iconos/apagar.svg")),
    ("bloquear", include_bytes!("../assets/iconos/bloquear.svg")),
    ("salir", include_bytes!("../assets/iconos/salir.svg")),
    ("ventana-nueva", include_bytes!("../assets/iconos/ventana-nueva.svg")),
    ("fijar", include_bytes!("../assets/iconos/fijar.svg")),
    ("cerrar", include_bytes!("../assets/iconos/cerrar.svg")),
    // --- Los del centro de control ---------------------------------------
    ("avion", include_bytes!("../assets/iconos/avion.svg")),
    // --- Los perfiles de energía -----------------------------------------
    // El rayo tachado, la balanza y el cohete: en la tarjeta de energía cada
    // perfil se reconoce por su dibujo y no solo por el color, que a un
    // daltónico le deja tres puntos iguales.
    ("perfil-ahorro", include_bytes!("../assets/iconos/perfil-ahorro.svg")),
    ("perfil-equilibrado", include_bytes!("../assets/iconos/perfil-equilibrado.svg")),
    ("perfil-rendimiento", include_bytes!("../assets/iconos/perfil-rendimiento.svg")),
    ("cpu", include_bytes!("../assets/iconos/cpu.svg")),
    ("ahorro", include_bytes!("../assets/iconos/ahorro.svg")),
    ("mantener", include_bytes!("../assets/iconos/mantener.svg")),
    ("noche", include_bytes!("../assets/iconos/noche.svg")),
    ("compartir", include_bytes!("../assets/iconos/compartir.svg")),
    ("captura", include_bytes!("../assets/iconos/captura.svg")),
    ("editar", include_bytes!("../assets/iconos/editar.svg")),
    ("musica", include_bytes!("../assets/iconos/musica.svg")),
    ("anterior", include_bytes!("../assets/iconos/anterior.svg")),
    ("siguiente", include_bytes!("../assets/iconos/siguiente.svg")),
    ("reproducir", include_bytes!("../assets/iconos/reproducir.svg")),
    ("pausa", include_bytes!("../assets/iconos/pausa.svg")),
];

/// Un icono propio por su nombre. `None` si no existe, que solo puede pasar por
/// una errata al escribirlo.
pub fn propio(nombre: &str) -> Option<Icono> {
    PROPIOS
        .iter()
        .find(|(n, _)| *n == nombre)
        .map(|(_, bytes)| Icono::Svg(svg::Handle::from_memory(*bytes)))
}

/// Busca el icono por nombre y lo carga. `None` si no aparece o está roto.
///
/// Primero los propios: un icono de BookOS no debe cambiar de dibujo porque el
/// usuario instale otro tema.
pub fn cargar(nombre: &str) -> Option<Icono> {
    if let Some(icono) = propio(nombre) {
        return Some(icono);
    }
    resolve_icon(nombre).and_then(|ruta| load_icon(&ruta))
}

/// Dónde mirar, en orden. Primero el tema propio de BookOS si existe, luego los
/// del sistema.
fn icon_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/icons"));
    }
    dirs.push(PathBuf::from("/usr/share/icons"));
    dirs.push(PathBuf::from("/usr/share/pixmaps"));
    dirs
}

/// `breeze-dark` va **antes** que `breeze`: los iconos monocromos de estado de
/// Breeze llevan el color dentro (`color:#232629`, casi negro, para fondos
/// claros) y sobre el panel negro no se veía absolutamente nada. La variante
/// oscura trae los mismos dibujos con `#fcfcfc`.
const TEMAS: &[&str] = &[
    "BookOS-Light",
    "BookOS",
    "breeze-dark",
    "breeze",
    "hicolor",
    "Adwaita",
];
/// Las categorías de freedesktop que usa el escritorio. `status` y
/// `preferences` hacen falta para el panel: los iconos de aplicación viven en
/// `apps`, pero los de batería y red están en `status`.
const CATEGORIAS: &[&str] = &[
    "status",
    "apps",
    "actions",
    "devices",
    "preferences",
    "categories",
];
/// El tamaño que anuncia el nombre de un directorio de tema, para ordenar de
/// mayor a menor: es preferible reducir un icono grande que ampliar uno de 16
/// px y que se vea pastoso. `scalable` y `symbolic` ganan a cualquier píxel.
///
/// Aquí había una lista de tamaños escrita a mano y **se quedaba corta**:
/// faltaban `32x32` y `512x512`, que es donde hicolor guarda los iconos de
/// Firefox y de bookos-settings, así que el dock los dibujaba con la inicial.
/// Enumerar lo que hay en disco no puede quedarse corto.
fn tamano_de(nombre: &str) -> Option<u32> {
    if nombre == "scalable" || nombre == "symbolic" {
        return Some(u32::MAX);
    }
    // `32x32`, `32`, `32x32@2` — basta el primer número.
    nombre.split(['x', '@']).next()?.parse().ok()
}

/// Los directorios donde de verdad hay iconos, en orden de preferencia.
///
/// Se calcula **una vez** por proceso. La búsqueda de freedesktop es un
/// producto de bases por temas por tamaños por categorías, y recorrerlo entero
/// por cada icono salían unas 4.700 llamadas a `stat`; con un icono que no
/// existe se pagaban todas. Como la mayoría de esas combinaciones no existen en
/// disco —aquí sobreviven unas pocas decenas—, filtrarlas una vez convierte
/// cada búsqueda posterior en un puñado de comprobaciones.
///
/// Medido en esta máquina: buscar treinta iconos pasó de 31 ms a menos de 2.
fn directorios() -> &'static [PathBuf] {
    static DIRS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    DIRS.get_or_init(|| {
        let mut dirs = Vec::new();
        for base in icon_dirs() {
            if !base.is_dir() {
                continue;
            }
            // Los planos como /usr/share/pixmaps van tal cual, sin tema.
            dirs.push(base.clone());
            for tema in TEMAS {
                let raiz = base.join(tema);
                let Ok(nivel1) = std::fs::read_dir(&raiz) else {
                    continue;
                };
                // Los temas mezclan tema/tamaño/categoría y
                // tema/categoría/tamaño, así que se mira quién de los dos es
                // una categoría conocida y el otro da el tamaño.
                let mut del_tema: Vec<(u32, PathBuf)> = Vec::new();
                for a in nivel1.flatten() {
                    let Ok(nombre_a) = a.file_name().into_string() else {
                        continue;
                    };
                    if let Some(tam) = tamano_de(&nombre_a) {
                        // tema/tamaño/categoría: se comprueban las seis
                        // categorías con `stat`. Enumerar el directorio en su
                        // lugar costaba 43 ms de arranque —hicolor y Adwaita
                        // tienen cientos de carpetas con miles de ficheros— y
                        // el panel se pinta en el primer frame.
                        for sub in CATEGORIAS {
                            let d = a.path().join(sub);
                            if d.is_dir() {
                                del_tema.push((tam, d));
                            }
                        }
                    } else if CATEGORIAS.contains(&nombre_a.as_str()) {
                        // tema/categoría/tamaño: aquí sí hay que enumerar, pero
                        // solo son seis directorios por tema y contienen
                        // carpetas, no iconos.
                        let Ok(nivel2) = std::fs::read_dir(a.path()) else {
                            continue;
                        };
                        for b in nivel2.flatten() {
                            let Ok(nombre_b) = b.file_name().into_string() else {
                                continue;
                            };
                            if let Some(tam) = tamano_de(&nombre_b) {
                                if b.path().is_dir() {
                                    del_tema.push((tam, b.path()));
                                }
                            }
                        }
                    }
                }
                // De mayor a menor dentro del tema; los temas conservan entre
                // sí el orden de preferencia de `TEMAS`.
                del_tema.sort_by(|a, b| b.0.cmp(&a.0));
                dirs.extend(del_tema.into_iter().map(|(_, d)| d));
            }
        }
        tracing::debug!(n = dirs.len(), "directorios de iconos");
        dirs
    })
}

pub fn resolve_icon(name: &str) -> Option<PathBuf> {
    // Un nombre puede venir ya como ruta absoluta desde un `.desktop`.
    if name.starts_with('/') {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    for dir in directorios() {
        for ext in ["svg", "png", "xpm"] {
            let p = dir.join(format!("{name}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Carga un icono desde disco.
///
/// Los SVG se los queda iced (los rasteriza resvg al tamaño que toque). Los
/// mapas de bits se decodifican **aquí**: si el fichero está corrupto o en un
/// formato que no soportamos, devolvemos `None` y el item cae en la baldosa con
/// la inicial. Dejárselo a iced significaría un panic, y un panic del shell
/// apaga el panel y el dock enteros — un icono roto no puede costar tanto.
fn load_icon(path: &Path) -> Option<Icono> {
    if path.extension().is_some_and(|e| e == "svg") {
        return Some(Icono::Svg(svg::Handle::from_path(path)));
    }
    match image::ImageReader::open(path).ok()?.decode() {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            Some(Icono::Raster(iced_image::Handle::from_rgba(
                w,
                h,
                rgba.into_raw(),
            )))
        }
        Err(err) => {
            tracing::warn!(?path, "no se pudo decodificar el icono: {err}");
            None
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Los nombres del dock por defecto tienen que resolverse en una máquina
    /// con Breeze y hicolor instalados. Es el test que habría cazado que
    /// faltaba `32x32` en la lista de tamaños: Firefox salía sin icono.
    #[test]
    fn los_iconos_del_dock_se_encuentran() {
        let t0 = std::time::Instant::now();
        let n = directorios().len();
        let indexado = t0.elapsed();
        let t1 = std::time::Instant::now();
        for nombre in [
            "utilities-terminal",
            "system-file-manager",
            "firefox",
            "accessories-text-editor",
        ] {
            assert!(resolve_icon(nombre).is_some(), "sin icono para {nombre}");
        }
        println!("{n} directorios en {indexado:?}; 4 búsquedas en {:?}", t1.elapsed());
    }
}
