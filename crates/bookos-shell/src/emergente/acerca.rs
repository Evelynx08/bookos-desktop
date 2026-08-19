//! «Acerca de este PC»: la tarjeta con lo que es esta máquina.
//!
//! La abre el menú del logo. Antes lanzaba `kinfocenter`, que es el panel de
//! otro escritorio y enseña la versión de Plasma y de Qt: cosas del sistema
//! anfitrión, no de BookOS.
//!
//! **Todo sale de sysfs y de `/proc`**, sin procesos hijos ni D-Bus, igual que
//! el panel: el equipo de DMI, la CPU de `/proc/cpuinfo`, la memoria de
//! `/proc/meminfo` y la distribución de `/etc/os-release`. El único fichero
//! grande es `pci.ids`, que se lee una vez al abrir la ventana y solo para
//! traducir el identificador de la tarjeta gráfica a su nombre comercial —el
//! kernel no lo sabe, solo tiene `0x64a0`.
//!
//! El número de serie **no se enseña**: está en `/sys/class/dmi/id/product_serial`
//! y ese fichero es 0400 de root, así que aquí siempre saldría vacío. Se deja
//! el renglón con una raya, como hace el diseño, en vez de pedir permisos para
//! un dato que solo sirve para el servicio técnico.

use std::fmt::Write as _;

use iced_core::alignment::Horizontal;
use iced_core::{Border, Color, Length};
use iced_widget::{column, container, row, text, Space};

use crate::tema;
use crate::view::PanelElement;
use crate::Accion;

use super::{Ancla, Tecla};

const ANCHO: f32 = 420.0;
const MARGEN: f32 = 32.0;
/// Alto del dibujo del portátil de arriba.
const RETRATO: f32 = 170.0;
/// Alto de cada renglón de la ficha, contando su respiro.
const RENGLON: f32 = 26.0;
/// Ancho de la columna de las etiquetas. Van alineadas a la derecha contra el
/// valor, como en la ficha del diseño.
const ETIQUETAS: f32 = 150.0;
/// Alto del botón de «Más información…».
const BOTON: f32 = 40.0;

/// Un renglón de la ficha.
struct Dato {
    etiqueta: &'static str,
    valor: String,
}

pub struct Acerca {
    /// Cómo se llama el equipo: «Galaxy Book5 Pro».
    modelo: String,
    /// La distribución, para el renglón bajo el título.
    sistema: String,
    datos: Vec<Dato>,
    /// Si el ratón está sobre el botón de «Más información…».
    /// El botón bajo el puntero, con su realce entrando y saliendo.
    señalado: tema::Transicion,
}

impl Acerca {
    pub fn new() -> Self {
        let modelo = dmi("product_family")
            .or_else(|| dmi("product_name"))
            .unwrap_or_else(|| "Este equipo".into());
        let sistema = os_release("PRETTY_NAME").unwrap_or_else(|| "Linux".into());
        let version = os_release("VERSION_ID").unwrap_or_default();
        let datos = vec![
            Dato {
                etiqueta: "Chip",
                valor: cpu().unwrap_or_else(|| "—".into()),
            },
            Dato {
                etiqueta: "Gráficos",
                valor: gpu().unwrap_or_else(|| "—".into()),
            },
            Dato {
                etiqueta: "Memoria",
                valor: memoria().unwrap_or_else(|| "—".into()),
            },
            Dato {
                etiqueta: "Número de serie",
                valor: dmi("product_serial").unwrap_or_else(|| "—".into()),
            },
            Dato {
                etiqueta: "BookOS",
                // La versión y la variante, no el `PRETTY_NAME` entero: ese ya
                // está bajo el título y repetirlo dejaba el renglón en
                // "44 (Fedora Linux 44 (KDE Plasma Desktop Edition))".
                valor: match (version.is_empty(), os_release("VARIANT")) {
                    (true, _) => sistema.clone(),
                    (false, Some(variante)) => format!("{version} ({variante})"),
                    (false, None) => version.clone(),
                },
            },
        ];
        Self {
            modelo,
            sistema,
            datos,
            señalado: tema::Transicion::nueva(0.0, tema::D_HOVER, tema::C_SUAVE),
        }
    }

    pub fn size(&self) -> (f32, f32) {
        let ficha = self.datos.len() as f32 * RENGLON;
        (
            ANCHO,
            MARGEN * 2.0 + RETRATO + 24.0 + 34.0 + 22.0 + 24.0 + ficha + 28.0 + BOTON,
        )
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    /// El rectángulo del botón, relativo a la tarjeta.
    fn rect_boton(&self) -> iced_core::Rectangle {
        let ficha = self.datos.len() as f32 * RENGLON;
        iced_core::Rectangle {
            x: (ANCHO - 220.0) / 2.0,
            y: MARGEN + RETRATO + 24.0 + 34.0 + 22.0 + 24.0 + ficha + 28.0,
            width: 220.0,
            height: BOTON,
        }
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let dentro =
            punto.is_some_and(|(x, y)| self.rect_boton().contains(iced_core::Point::new(x, y)));
        self.señalado.ir_a(dentro as u8 as f32)
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    pub fn animando(&self) -> bool {
        self.señalado.animando()
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if !self.rect_boton().contains(iced_core::Point::new(x, y)) {
            return None;
        }
        // «Más información…» abre los ajustes, que es donde vive el detalle.
        // Con respaldo al centro de información de Plasma mientras
        // bookos-settings no tenga esa página.
        Some(Accion::Lanzar(
            "bookos-settings --acerca || bookos-settings || kinfocenter".into(),
        ))
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    /// El velo de detrás: la tarjeta ocupa el centro de la pantalla y sin él se
    /// leería como una ventana más del escritorio.
    pub fn velo(&self) -> Color {
        Color {
            a: 0.45,
            ..tema::bg()
        }
    }

    /// El portátil dibujado de arriba.
    ///
    /// Es una silueta, no una foto: no hay fotos de los equipos en ningún sitio
    /// del sistema, y una silueta con la pantalla del color de acento es lo que
    /// enseña el diseño.
    fn retrato(&self) -> PanelElement<'_> {
        let pantalla = container(Space::new())
            .width(Length::Fixed(240.0))
            .height(Length::Fixed(148.0))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(Color::from_rgb(0.02, 0.04, 0.45).into()),
                border: Border {
                    radius: 8.0.into(),
                    width: 6.0,
                    color: Color::from_rgb(0.10, 0.10, 0.11),
                },
                ..Default::default()
            });
        // La base, más ancha que la pantalla: es lo que da la silueta de
        // portátil abierto en vez de la de un monitor.
        let base = container(Space::new())
            .width(Length::Fixed(300.0))
            .height(Length::Fixed(8.0))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(
                    Color {
                        a: 0.55,
                        ..tema::tinta()
                    }
                    .into(),
                ),
                border: Border {
                    radius: 4.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        container(
            column![pantalla, Space::new().height(Length::Fixed(6.0)), base]
                .align_x(Horizontal::Center),
        )
        .width(Length::Fill)
        .align_x(Horizontal::Center)
        .into()
    }

    fn ficha(&self) -> PanelElement<'_> {
        let mut col = column![];
        for dato in &self.datos {
            col = col.push(
                container(
                    row![
                        container(
                            text(dato.etiqueta)
                                .size(tema::T_CUERPO)
                                .color(tema::texto())
                                .align_x(Horizontal::Right),
                        )
                        .width(Length::Fixed(ETIQUETAS))
                        .align_x(Horizontal::Right),
                        Space::new().width(Length::Fixed(12.0)),
                        text(dato.valor.clone())
                            .size(tema::T_CUERPO)
                            .color(tema::TEXTO2),
                    ]
                    .align_y(iced_core::alignment::Vertical::Center),
                )
                // Alto **mínimo**, no fijo: un valor que ocupe dos líneas tiene
                // que empujar al renglón siguiente en vez de escribirse encima.
                .padding(iced_core::Padding::ZERO.top(3.0).bottom(3.0)),
            );
        }
        col.into()
    }

    pub fn view(&self) -> PanelElement<'_> {
        let boton = container(
            container(
                text("Más información…")
                    .size(tema::T_CUERPO)
                    .color(tema::texto()),
            )
            .width(Length::Fixed(220.0))
            .height(Length::Fixed(BOTON))
            .center_x(Length::Fixed(220.0))
            .center_y(Length::Fixed(BOTON))
            .style({
                let señalado = self.señalado.valor();
                move |_theme: &iced_widget::Theme| container::Style {
                    background: Some(
                        tema::mezclar(
                            tema::alfa(tema::tinta(), 0.08),
                            tema::alfa(tema::tinta(), 0.16),
                            señalado,
                        )
                        .into(),
                    ),
                    border: Border {
                        radius: tema::R_BOTON.into(),
                        width: 1.0,
                        color: tema::borde(),
                    },
                    ..Default::default()
                }
            }),
        )
        .width(Length::Fill)
        .align_x(Horizontal::Center);

        let contenido = column![
            self.retrato(),
            Space::new().height(Length::Fixed(24.0)),
            text(self.modelo.clone())
                .size(30.0)
                .color(tema::texto())
                .align_x(Horizontal::Center),
            text(self.sistema.clone())
                .size(tema::T_CUERPO)
                .color(tema::TEXTO2)
                .align_x(Horizontal::Center),
            Space::new().height(Length::Fixed(24.0)),
            self.ficha(),
            Space::new().height(Length::Fixed(28.0)),
            boton,
        ]
        .align_x(Horizontal::Center);

        container(container(contenido).padding(MARGEN as u16))
            .width(Length::Fixed(ANCHO))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_TARJETA.into(),
                    width: 1.0,
                    color: tema::borde(),
                },
                ..Default::default()
            })
            .into()
    }
}

/// Un campo de DMI. Vacío o ilegible cuenta como que no está: `product_serial`
/// es 0400 de root y siempre falla, y eso no es un error que reportar.
fn dmi(campo: &str) -> Option<String> {
    let valor = std::fs::read_to_string(format!("/sys/class/dmi/id/{campo}")).ok()?;
    let valor = valor.trim();
    (!valor.is_empty()).then(|| valor.to_string())
}

fn os_release(clave: &str) -> Option<String> {
    let texto = std::fs::read_to_string("/etc/os-release").ok()?;
    for linea in texto.lines() {
        let Some((k, v)) = linea.split_once('=') else {
            continue;
        };
        if k == clave {
            return Some(v.trim_matches('"').to_string());
        }
    }
    None
}

/// El modelo de la CPU, sin las marcas registradas: «Intel(R) Core(TM) Ultra 7»
/// con sus símbolos ocupa media línea y no dice nada más.
fn cpu() -> Option<String> {
    let texto = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let linea = texto.lines().find(|l| l.starts_with("model name"))?;
    let modelo = linea.split_once(':')?.1.trim();
    Some(
        modelo
            .replace("(R)", "")
            .replace("(TM)", "")
            .replace("  ", " "),
    )
}

/// La memoria en GB redondeados.
///
/// `MemTotal` no es la memoria instalada sino la que ve el kernel —descontando
/// lo que se reserva el firmware—, así que 32 GiB salen como 30,8. Se redondea
/// al múltiplo de 4 más cercano, que es lo que de verdad hay puesto.
fn memoria() -> Option<String> {
    let texto = std::fs::read_to_string("/proc/meminfo").ok()?;
    let linea = texto.lines().find(|l| l.starts_with("MemTotal"))?;
    let kb: f64 = linea.split_whitespace().nth(1)?.parse().ok()?;
    let gb = kb / 1024.0 / 1024.0;
    let redondeado = (gb / 4.0).round() * 4.0;
    Some(format!("{redondeado:.0} GB"))
}

/// El nombre comercial de la tarjeta gráfica.
fn gpu() -> Option<String> {
    let dir = std::fs::read_dir("/sys/class/drm").ok()?;
    for entrada in dir.filter_map(|e| e.ok()) {
        let nombre = entrada.file_name().to_string_lossy().to_string();
        // Solo las tarjetas, no los conectores (`card1-eDP-1`).
        if !nombre.starts_with("card") || nombre.contains('-') {
            continue;
        }
        let dispositivo = entrada.path().join("device");
        let leer = |f: &str| {
            std::fs::read_to_string(dispositivo.join(f))
                .ok()
                .map(|s| s.trim().trim_start_matches("0x").to_lowercase())
        };
        let (Some(vendor), Some(device)) = (leer("vendor"), leer("device")) else {
            continue;
        };
        if let Some(nombre) = buscar_pci(&vendor, &device) {
            return Some(acortar_gpu(&nombre));
        }
    }
    None
}

/// Deja el nombre de la tarjeta en algo que quepa en un renglón.
///
/// `pci.ids` los escribe para un catálogo, no para una ficha: esta sale como
/// "Core Ultra 200V Series Processors Arc Graphics 130V/140V GPU", que ocupaba
/// tres líneas y se comía el renglón de debajo. Se quitan las palabras de
/// relleno y, si aparece una familia conocida —`Arc`, `Radeon`, `GeForce`—, se
/// corta por ahí, que es donde empieza lo que identifica a la tarjeta.
fn acortar_gpu(nombre: &str) -> String {
    const RELLENO: &[&str] = &[
        " Series Processors",
        " Integrated Graphics Controller",
        " Corporation",
        " GPU",
        " Controller",
    ];
    let mut corto = nombre.to_string();
    for palabra in RELLENO {
        corto = corto.replace(palabra, "");
    }
    for familia in ["Arc ", "Radeon ", "GeForce "] {
        if let Some(i) = corto.find(familia) {
            corto = corto[i..].to_string();
            break;
        }
    }
    corto.trim().to_string()
}

/// Traduce `vendor:device` con la tabla de hwdata, que viene con el sistema.
///
/// El fichero son 1,6 MB y se recorre entero en el peor caso; se hace **una
/// vez**, al abrir esta ventana, y nunca desde el camino del panel.
fn buscar_pci(vendor: &str, device: &str) -> Option<String> {
    let texto = std::fs::read_to_string("/usr/share/hwdata/pci.ids").ok()?;
    let mut en_vendor = false;
    for linea in texto.lines() {
        if linea.starts_with('#') || linea.trim().is_empty() {
            continue;
        }
        if !linea.starts_with('\t') {
            // Sale del bloque del fabricante: si ya estábamos dentro, el
            // dispositivo no aparece y no hace falta leer el resto.
            if en_vendor {
                return None;
            }
            en_vendor = linea.starts_with(vendor);
            continue;
        }
        // Las subclases llevan dos tabuladores; los dispositivos, uno.
        if !en_vendor || linea.starts_with("\t\t") {
            continue;
        }
        let linea = linea.trim_start();
        let Some((id, nombre)) = linea.split_once(' ') else {
            continue;
        };
        if id == device {
            let mut salida = String::new();
            let _ = write!(salida, "{}", nombre.trim());
            return Some(salida);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Los datos salen de esta máquina, así que lo que se comprueba es que
    /// **hay** ficha y que ningún renglón queda a medias: un valor vacío se ve
    /// como un fallo de dibujado, no como un dato que falta.
    #[test]
    fn la_ficha_no_tiene_renglones_vacios() {
        let a = Acerca::new();
        assert_eq!(a.datos.len(), 5);
        for dato in &a.datos {
            assert!(!dato.valor.trim().is_empty(), "{} sin valor", dato.etiqueta);
        }
        assert!(!a.modelo.is_empty());
    }

    /// La memoria se redondea a lo que de verdad hay puesto: el kernel
    /// descuenta lo que se reserva el firmware.
    #[test]
    fn la_memoria_sale_redondeada() {
        let Some(m) = memoria() else {
            return;
        };
        let gb: f64 = m.trim_end_matches(" GB").parse().expect("memoria en GB");
        assert_eq!(gb % 4.0, 0.0, "{m} no es múltiplo de 4");
    }
}
