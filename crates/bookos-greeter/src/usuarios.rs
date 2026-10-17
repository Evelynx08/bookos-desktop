use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use bookos_shell::tema::{Acento, Tema};
use bookos_shell::{Config, ConfigBloqueo, Efectos};
use iced::Color;
use iced::widget::image::Handle;

const MAX_CONF: u64 = 64 * 1024;
const MAX_IMAGEN: u64 = 32 * 1024 * 1024;
/// Lado máximo que se decodifica. Lo publicado lo escribe el usuario, y sin
/// tope un PNG de 30000×30000 dejaría sin memoria la pantalla de acceso.
const MAX_LADO: u32 = 8192;

/// Los colores dinámicos que BookOS Settings saca del fondo de pantalla.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paleta {
    pub acento: Color,
    pub sobre_acento: Color,
    /// El tono claro de la paleta, para teñir el reloj.
    pub claro: Color,
}

impl Paleta {
    /// De `palette.css`, que trae un bloque por tema con los mismos nombres.
    pub fn leer(css: &str, tema: Tema) -> Option<Self> {
        let (marca, tono) = match tema {
            Tema::Claro => (":root.light-mode{", "--accent-container"),
            Tema::Oscuro => (":root.dark-mode{", "--blue"),
        };
        let bloque = css.split_once(marca)?.1.split('}').next()?;
        let color = |nombre: &str| {
            let valor = bloque
                .split(';')
                .find_map(|par| par.trim().strip_prefix(nombre)?.strip_prefix(":#"))?;
            let rgb = u32::from_str_radix(valor.trim(), 16).ok()?;
            (valor.trim().len() == 6).then(|| bookos_shell::tema::hex(rgb))
        };
        Some(Self {
            acento: color("--blue")?,
            sobre_acento: color("--on-accent")?,
            claro: color(tono)?,
        })
    }
}

pub struct Apariencia {
    pub paleta: Option<Paleta>,
    pub tema: Tema,
    pub acento: Acento,
    pub bloqueo: ConfigBloqueo,
    pub escala: Option<f64>,
    pub efectos_reducidos: bool,
    pub fondo: Option<PathBuf>,
}

pub struct Persona {
    pub cuenta: String,
    pub nombre: String,
    pub foto: Option<Handle>,
    pub miniatura: Option<Handle>,
    pub apariencia: Apariencia,
}

impl Persona {
    pub fn nueva(cuenta: &str, nombre: &str, uid: Option<u32>, publicado: &Path) -> Self {
        let carpeta = uid.and_then(|uid| carpeta_de(publicado, cuenta, uid));
        let propio = |archivo: &str, tope: u64| {
            let (carpeta, uid) = (carpeta.as_deref()?, uid?);
            publicado_valido(&carpeta.join(archivo), uid, tope)
        };
        let config = propio("panel.conf", MAX_CONF)
            .and_then(|ruta| Some((std::fs::read_to_string(&ruta).ok()?, ruta)))
            .map_or_else(Config::default, |(texto, ruta)| {
                Config::desde_texto(&texto, &ruta)
            });
        let fondo = propio(
            match config.tema {
                Tema::Claro => "fondo-claro",
                Tema::Oscuro => "fondo-oscuro",
            },
            MAX_IMAGEN,
        );
        let foto = propio("avatar", MAX_IMAGEN)
            .or_else(|| {
                let ruta = PathBuf::from(format!("/var/lib/AccountsService/icons/{cuenta}"));
                ruta.is_file().then_some(ruta)
            })
            .and_then(|ruta| bookos_shell::bloqueo::foto_circular(ruta.to_str()?));
        Self {
            cuenta: cuenta.into(),
            nombre: nombre.into(),
            miniatura: foto.as_ref().and_then(miniatura),
            foto,
            apariencia: Apariencia {
                paleta: propio("palette.css", MAX_CONF)
                    .and_then(|ruta| std::fs::read_to_string(ruta).ok())
                    .and_then(|css| Paleta::leer(&css, config.tema)),
                tema: config.tema,
                acento: config.acento,
                bloqueo: config.bloqueo,
                escala: config.escala,
                efectos_reducidos: config.efectos == Efectos::Reducidos,
                fondo,
            },
        }
    }

    pub fn iniciales(&self) -> String {
        self.nombre
            .split_whitespace()
            .filter_map(|palabra| palabra.chars().next())
            .take(2)
            .collect::<String>()
            .to_uppercase()
    }
}

pub fn raiz_publicada() -> PathBuf {
    std::env::var_os("BOOKOS_APARIENCIA_PUBLICA").map_or_else(
        || PathBuf::from(bookos_shell::APARIENCIA_PUBLICA),
        PathBuf::from,
    )
}

pub fn leer() -> Vec<Persona> {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let publicado = raiz_publicada();
    listables(&passwd)
        .into_iter()
        .map(|(cuenta, uid, nombre)| Persona::nueva(&cuenta, &nombre, Some(uid), &publicado))
        .collect()
}

/// Las cuentas de persona: el rango de `UID_MIN`…`UID_MAX` de `login.defs` y un
/// shell con el que se pueda entrar.
pub fn listables(passwd: &str) -> Vec<(String, u32, String)> {
    passwd
        .lines()
        .filter_map(|linea| {
            let campos: Vec<&str> = linea.split(':').collect();
            let [cuenta, _, uid, _, gecos, _, shell] = campos[..] else {
                return None;
            };
            let uid: u32 = uid.parse().ok()?;
            let entra = !(shell.ends_with("/nologin") || shell.ends_with("/false"));
            let nombre = gecos.split(',').next().unwrap_or_default().trim();
            ((1000..60000).contains(&uid) && entra).then(|| {
                let nombre = if nombre.is_empty() { cuenta } else { nombre };
                (cuenta.to_string(), uid, nombre.to_string())
            })
        })
        .collect()
}

/// La carpeta publicada solo vale si es de quien dice ser. La raíz admite
/// escritura de cualquiera, así que el nombre de la carpeta no prueba nada.
fn carpeta_de(publicado: &Path, cuenta: &str, uid: u32) -> Option<PathBuf> {
    let carpeta = publicado.join(cuenta);
    let datos = std::fs::symlink_metadata(&carpeta).ok()?;
    (datos.is_dir() && datos.uid() == uid).then_some(carpeta)
}

pub fn publicado_valido(ruta: &Path, uid: u32, tope: u64) -> Option<PathBuf> {
    let datos = std::fs::symlink_metadata(ruta).ok()?;
    (datos.is_file() && datos.uid() == uid && datos.len() <= tope).then(|| ruta.to_path_buf())
}

fn miniatura(foto: &Handle) -> Option<Handle> {
    let Handle::Rgba {
        width,
        height,
        pixels,
        ..
    } = foto
    else {
        return None;
    };
    let origen = image::RgbaImage::from_raw(*width, *height, pixels.to_vec())?;
    // La lista dibuja el avatar a ~50 px. Reducir ahí la foto de 440 en cada
    // fotograma, con el filtro bilineal de tiny-skia, la deja con dientes.
    let lado = 128;
    let reducida =
        image::imageops::resize(&origen, lado, lado, image::imageops::FilterType::Triangle);
    Some(Handle::from_rgba(lado, lado, reducida.into_raw()))
}

/// Lado de una losa del fondo, en unidades de iced.
const LOSA: u32 = 256;

/// El fondo ya recortado, al tamaño físico de la ventana y partido en losas.
///
/// Al tamaño físico para que dibujarlo sea copiar y no reescalar. En losas
/// porque iced descarta las primitivas que no tocan la zona dañada, pero una
/// imagen la pinta siempre entera: con el fondo en una pieza, cada fotograma
/// de la lente costaba 50 ms a 2880×1800 aunque solo cambiase la lista.
#[derive(Clone)]
pub struct Fondo {
    /// Por filas: cada losa con su ancho y su alto en unidades de iced.
    pub losas: Vec<Vec<(Handle, f32, f32)>>,
}

pub fn cargar_fondo(
    ruta: &Path,
    (ancho, alto): (u32, u32),
    densidad: f32,
    automatico: bool,
) -> Option<Fondo> {
    let mut lector = image::ImageReader::open(ruta)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut limites = image::Limits::default();
    limites.max_image_width = Some(MAX_LADO);
    limites.max_image_height = Some(MAX_LADO);
    lector.limits(limites);
    let imagen = lector.decode().ok()?.to_rgba8();
    if automatico {
        bookos_shell::tema::actualizar_acento_del_fondo(imagen.as_raw());
    }
    let (w, h) = imagen.dimensions();
    let escala = (ancho as f32 / w as f32).max(alto as f32 / h as f32);
    let (rw, rh) = (
        ((ancho as f32 / escala) as u32).clamp(1, w),
        ((alto as f32 / escala) as u32).clamp(1, h),
    );
    let recorte = image::imageops::crop_imm(&imagen, (w - rw) / 2, (h - rh) / 2, rw, rh);
    let entero = if (rw, rh) == (ancho, alto) {
        recorte.to_image()
    } else {
        image::imageops::resize(
            &*recorte,
            ancho,
            alto,
            image::imageops::FilterType::Triangle,
        )
    };
    let lado = ((LOSA as f32 * densidad).round() as u32).max(1);
    let losas = (0..alto)
        .step_by(lado as usize)
        .map(|y| {
            let alto_losa = lado.min(alto - y);
            (0..ancho)
                .step_by(lado as usize)
                .map(|x| {
                    let ancho_losa = lado.min(ancho - x);
                    let losa = image::imageops::crop_imm(&entero, x, y, ancho_losa, alto_losa);
                    (
                        Handle::from_rgba(ancho_losa, alto_losa, losa.to_image().into_raw()),
                        ancho_losa as f32 / densidad,
                        alto_losa as f32 / densidad,
                    )
                })
                .collect()
        })
        .collect();
    Some(Fondo { losas })
}

pub fn fondo_por_defecto(tema: Tema) -> Option<PathBuf> {
    let claro = tema == Tema::Claro;
    let familia = bookos_shell::fondos::por_defecto(claro)?;
    Some(if claro { familia.claro } else { familia.oscuro })
}
