//! El emergente de la batería: cuánto queda y en qué perfil va el equipo.
//!
//! ## Por qué el perfil se pide por D-Bus y no se escribe en sysfs
//!
//! `/sys/firmware/acpi/platform_profile` es de root con permisos 644, igual que
//! el brillo. Quien concede el permiso es `power-profiles-daemon`, que expone
//! `net.hadess.PowerProfiles.ActiveProfile` como propiedad escribible y además
//! sabe combinar el perfil de la plataforma con el gobernador de la CPU —aquí,
//! `platform_profile` con `intel_pstate`—; escribiendo el fichero a pelo solo
//! se movería la mitad.
//!
//! Se llama con `busctl`, como en el brillo: meter un cliente de D-Bus sería
//! una dependencia nueva y un hilo más dentro del compositor para escribir una
//! cadena. La llamada de escritura se lanza sin esperar (`spawn`), que es lo
//! mismo que se hizo con `wpctl` cuando se midió que bloquear costaba 21 ms.
//!
//! Los perfiles que existen los dice el propio daemon, no una lista escrita
//! aquí: hay equipos con tres y equipos con cuatro.

use std::process::{Command, Stdio};

use iced_core::alignment::Vertical;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::icono::{self, Icono};
use crate::state::Battery;
use crate::tema;
use crate::view::PanelElement;

use super::control;
use super::{Ancla, Tecla};

// Las medidas del lienzo «Widgets de BookOS»: tarjeta de 336 con margen de 16 y
// tres grupos grises —la cabecera, los perfiles y las preferencias— separados
// por 12.
const ANCHO: f32 = 336.0;
const MARGEN: f32 = 16.0;
/// Alto de cada fila de perfil: una línea de texto con 13 de aire por lado.
const FILA: f32 = 44.0;
/// Hueco entre filas de perfil. Casi pegadas: dentro del grupo son una elección
/// de tres, no tres tarjetas.
const HUECO_FILA: f32 = 2.0;
/// Diámetro del punto de color de un perfil.
const PUNTO: f32 = 18.0;
/// Lado del icono que marca el perfil elegido.
const MARCA: f32 = 16.0;
/// Alto del grupo de la cabecera: título con su renglón de estado a la
/// izquierda y el nivel a la derecha.
const CABECERA: f32 = 68.0;
/// Alto del grupo del pie.
const PIE: f32 = 44.0;
/// Aire entre grupos.
const HUECO_BLOQUE: f32 = 12.0;

fn media() -> iced_core::Font {
    control::peso(iced_core::font::Weight::Medium)
}

fn gorda() -> iced_core::Font {
    control::peso(iced_core::font::Weight::Bold)
}

/// El icono teñido, o un hueco de su tamaño si falta.
fn icono_o_hueco(icono: Option<&Icono>, px: f32, color: Color) -> PanelElement<'_> {
    match icono {
        Some(i) => icono::ver_teñido(i, px, Some(color)),
        None => Space::new().width(Length::Fixed(px)).into(),
    }
}

/// Un perfil de energía tal como lo nombra `power-profiles-daemon`.
struct Perfil {
    /// La clave que entiende el daemon: `power-saver`, `balanced`…
    clave: String,
    /// Cómo se enseña, en castellano.
    etiqueta: &'static str,
    /// El color que lo identifica: el mismo que lleva el pictograma del panel
    /// cuando ese perfil está puesto. Es lo que ata las dos cosas. No va solo:
    /// el rótulo al lado es lo que lo distingue para quien no separa el
    /// amarillo del verde.
    color: Color,
}

/// Traduce la clave del daemon: etiqueta y color. Los rótulos son los del
/// diseño.
fn nombrar(clave: &str) -> (&'static str, Color) {
    match clave {
        "power-saver" => ("Ahorro de energía", tema::perfil_ahorro()),
        "balanced" => ("Equilibrado", tema::perfil_equilibrado()),
        "performance" => ("Alto rendimiento", tema::perfil_rendimiento()),
        // Los hay con un cuarto perfil propio del fabricante; se enseña en gris
        // en vez de esconderlo.
        _ => ("Otro perfil", tema::TEXTO2),
    }
}

/// Los perfiles que ofrece el daemon, en el orden en que los da: de menos a más
/// consumo.
fn perfiles() -> Vec<Perfil> {
    let salida = Command::new("busctl")
        .args([
            "--system",
            "get-property",
            "net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "net.hadess.PowerProfiles",
            "Profiles",
        ])
        .output()
        .ok();
    let Some(salida) = salida.filter(|s| s.status.success()) else {
        return Vec::new();
    };
    let texto = String::from_utf8_lossy(&salida.stdout);
    // La propiedad es un `aa{sv}` y `busctl` la imprime en una línea con las
    // cadenas entrecomilladas. En vez de escribir un analizador del formato de
    // salida —que cambia entre versiones—, se buscan los valores que siguen a
    // la clave "Profile", que es lo único que hace falta.
    let mut claves = Vec::new();
    let mut trozos = texto.split('"');
    while let Some(t) = trozos.next() {
        if t.trim_end().ends_with("Profile") && t.trim_end().len() == "Profile".len() {
            // "Profile" s "power-saver": el valor es la siguiente cadena
            // entrecomillada, saltándose el tipo `s` que va entre medias.
            if let Some(valor) = trozos.nth(1) {
                claves.push(valor.to_string());
            }
        }
    }
    claves
        .into_iter()
        .map(|clave| {
            let (etiqueta, color) = nombrar(&clave);
            Perfil {
                clave,
                etiqueta,
                color,
            }
        })
        .collect()
}

/// Qué perfil está puesto ahora.
fn perfil_activo() -> Option<String> {
    let salida = Command::new("busctl")
        .args([
            "--system",
            "get-property",
            "net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "net.hadess.PowerProfiles",
            "ActiveProfile",
        ])
        .output()
        .ok()
        .filter(|s| s.status.success())?;
    let texto = String::from_utf8_lossy(&salida.stdout);
    // `s "power-saver"`
    texto.split('"').nth(1).map(str::to_string)
}

/// El tope de carga, si el portátil tiene uno puesto. Es la explicación de por
/// qué una batería enchufada se queda parada en el 80 % y no llega al 100.
fn umbral() -> Option<u8> {
    let ruta = ruta_bateria()?;
    std::fs::read_to_string(ruta.join("charge_control_end_threshold"))
        .ok()?
        .trim()
        .parse()
        .ok()
        .filter(|u| *u > 0 && *u < 100)
}

/// La primera batería de sysfs. Se busca aquí y no se reutiliza la de
/// `state.rs` porque allí es privada y solo devuelve el dato ya interpretado.
fn ruta_bateria() -> Option<std::path::PathBuf> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut rutas: Vec<_> = dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery"))
        .collect();
    // Ordenadas para que BAT0 gane a BAT1 y la elección no dependa del orden
    // del directorio, que no está garantizado.
    rutas.sort();
    rutas.into_iter().next()
}

pub struct Energia {
    bateria: Option<Battery>,
    /// Consumo ahora mismo y ciclos de carga, para la cabecera.
    /// El tope de carga del portátil, si lo hay.
    umbral: Option<u8>,
    perfiles: Vec<Perfil>,
    activo: Option<String>,
    /// Sobre qué fila está el ratón, para el realce.
    señalado: tema::Realce,
    /// La marca del perfil elegido y la flecha del pie. Se cargan una vez: son
    /// los mismos en cada repintado.
    marca: Option<Icono>,
    flecha: Option<Icono>,
}

impl Energia {
    pub fn new() -> Self {
        Self {
            // Con tiempo restante: es la tarjeta que lo enseña, y solo se lee
            // mientras está abierta.
            bateria: Battery::read(true),
            umbral: umbral(),
            perfiles: perfiles(),
            activo: perfil_activo(),
            señalado: tema::Realce::nuevo(),
            marca: icono::propio("comprobado"),
            flecha: icono::propio("chevron-derecha"),
        }
    }

    pub fn size(&self) -> (f32, f32) {
        let perfiles = if self.perfiles.is_empty() {
            0.0
        } else {
            self.alto_lista() + control::GRUPO * 2.0 + HUECO_BLOQUE
        };
        (
            ANCHO,
            MARGEN * 2.0 + CABECERA + HUECO_BLOQUE + perfiles + PIE,
        )
    }

    /// Lo que ocupan las filas de perfil, sin el relleno del grupo.
    fn alto_lista(&self) -> f32 {
        let filas = self.perfiles.len() as f32;
        if filas == 0.0 {
            return 0.0;
        }
        filas * FILA + (filas - 1.0) * HUECO_FILA
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("bateria")
    }

    /// La `y` donde empieza la lista de perfiles, relativa a la emergente.
    fn y_lista(&self) -> f32 {
        MARGEN + CABECERA + HUECO_BLOQUE + control::GRUPO
    }

    /// Qué fila de perfil cae en esa `y`.
    fn fila_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < MARGEN + control::GRUPO || x > ANCHO - MARGEN - control::GRUPO {
            return None;
        }
        let rel = y - self.y_lista();
        if rel < 0.0 {
            return None;
        }
        let i = (rel / (FILA + HUECO_FILA)) as usize;
        // El hueco entre filas no es de nadie: pulsarlo no debe cambiar el
        // perfil por accidente.
        if rel - i as f32 * (FILA + HUECO_FILA) > FILA {
            return None;
        }
        (i < self.perfiles.len()).then_some(i)
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        self.señalado
            .señalar(punto.and_then(|(x, y)| self.fila_en(x, y)))
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    pub fn animando(&self) -> bool {
        self.señalado.animando()
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        // El pie va al final de la tarjeta: el último grupo, de `PIE` px.
        let alto = self.size().1;
        if y > alto - PIE - MARGEN && x > MARGEN && x < ANCHO - MARGEN {
            return Some(Accion::Lanzar("bookos-settings --page bateria".into()));
        }
        let i = self.fila_en(x, y)?;
        let clave = self.perfiles[i].clave.clone();
        if self.activo.as_deref() == Some(clave.as_str()) {
            return None;
        }
        let _ = Command::new("busctl")
            .args([
                "--system",
                "set-property",
                "net.hadess.PowerProfiles",
                "/net/hadess/PowerProfiles",
                "net.hadess.PowerProfiles",
                "ActiveProfile",
                "s",
                &clave,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        // Se da por puesto sin releer: la lectura tardaría lo que tarde el
        // daemon en aplicarlo y la fila se quedaría sin marcar hasta el
        // siguiente refresco. Si la llamada falla, el próximo `new` lo corrige.
        self.activo = Some(clave);
        None
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    /// El color del perfil que está puesto, que es con el que se rellena el
    /// pictograma de la cabecera: el mismo que lleva el del panel, para que las
    /// dos cosas se lean como una.
    fn color_activo(&self) -> Color {
        self.activo
            .as_deref()
            .and_then(|a| self.perfiles.iter().find(|p| p.clave == a))
            .map_or(tema::texto(), |p| p.color)
    }

    /// La pila de la cabecera: carcasa de 30×14 con el relleno proporcional.
    ///
    /// Bajo mínimos y sin enchufe el relleno pasa a rojo, que es lo único que
    /// se tiene que ver de un vistazo; el resto del tiempo lleva el color del
    /// perfil.
    fn pila(&self) -> PanelElement<'_> {
        const ANCHO_PILA: f32 = 30.0;
        const ALTO_PILA: f32 = 14.0;
        const BORDE: f32 = 1.5;
        let (porciento, color) = match self.bateria {
            Some(b) if b.percent <= 15 && !b.plugged => (b.percent, tema::rojo()),
            Some(b) => (b.percent, self.color_activo()),
            None => (0, self.color_activo()),
        };
        let interior = ANCHO_PILA - BORDE * 2.0 - 3.0;
        let relleno = container(Space::new())
            .width(Length::Fixed(
                (interior * porciento.min(100) as f32 / 100.0).max(2.0),
            ))
            .height(Length::Fill)
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(color.into()),
                border: Border {
                    radius: 2.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let carcasa = container(relleno)
            .width(Length::Fixed(ANCHO_PILA))
            .height(Length::Fixed(ALTO_PILA))
            .padding(BORDE + 1.5)
            .style(|_theme: &iced_widget::Theme| container::Style {
                border: Border {
                    radius: 4.0.into(),
                    width: BORDE,
                    color: tema::TEXTO2,
                },
                ..Default::default()
            });
        let borne = container(Space::new())
            .width(Length::Fixed(2.0))
            .height(Length::Fixed(5.0))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(tema::TEXTO2.into()),
                border: Border {
                    radius: 1.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        row![carcasa, Space::new().width(Length::Fixed(1.0)), borne]
            .align_y(Vertical::Center)
            .into()
    }

    /// La cabecera: «Batería» con la fuente debajo, y el nivel con su pila a
    /// la derecha.
    fn cabecera(&self) -> PanelElement<'_> {
        let nivel = match self.bateria {
            // Un sobremesa no tiene batería y la tarjeta sigue teniendo
            // sentido: los perfiles son suyos igual.
            Some(b) => format!("{} %", b.percent),
            None => "—".to_string(),
        };
        let detalle = self.estado();
        let dentro = row![
            column![
                control::titulo("Batería"),
                text(detalle).size(11.0).color(tema::TEXTO2),
            ]
            .spacing(2),
            Space::new().width(Length::Fill),
            text(nivel).size(22.0).font(gorda()).color(tema::texto()),
            Space::new().width(Length::Fixed(8.0)),
            self.pila(),
        ]
        .align_y(Vertical::Center);
        control::grupo(
            container(dentro)
                .height(Length::Fill)
                .center_y(Length::Fill)
                .padding([0, 8])
                .into(),
            ANCHO - MARGEN * 2.0,
            8.0,
        )
    }

    /// El renglón de estado: fuente, tiempo restante y tope de carga, los que
    /// haya de los tres.
    fn estado(&self) -> String {
        let Some(bat) = self.bateria else {
            return "Sin batería".into();
        };
        let mut partes = Vec::new();
        partes.push(
            if bat.charging {
                "Cargando"
            } else if bat.plugged {
                "Conectado"
            } else {
                "Con batería"
            }
            .to_string(),
        );
        if let Some(m) = bat.minutes.filter(|_| !bat.charging) {
            partes.push(format!("{}:{:02} restantes", m / 60, m % 60));
        }
        // El tope solo cuando ya está frenando la carga: dicho siempre sería
        // ruido, y dicho justo ahí explica el 80 % que no sube.
        if let Some(u) = self.umbral.filter(|u| bat.plugged && bat.percent >= *u) {
            partes.push(format!("tope {u} %"));
        }
        partes.join(" · ")
    }

    /// El pie: «Preferencias de batería» con su flecha, que lleva a la página
    /// de batería de los ajustes.
    fn pie(&self) -> PanelElement<'_> {
        control::grupo(
            container(
                row![
                    text("Preferencias de batería")
                        .size(14.0)
                        .font(media())
                        .color(tema::TEXTO2),
                    Space::new().width(Length::Fill),
                    icono_o_hueco(self.flecha.as_ref(), MARCA, tema::TEXTO2),
                ]
                .align_y(Vertical::Center),
            )
            .height(Length::Fill)
            .center_y(Length::Fill)
            .padding([0, 8])
            .into(),
            ANCHO - MARGEN * 2.0,
            2.0,
        )
    }

    fn fila(&self, i: usize) -> PanelElement<'_> {
        let perfil = &self.perfiles[i];
        let activo = self.activo.as_deref() == Some(perfil.clave.as_str());
        let señalado = self.señalado.intensidad(i);

        // El elegido va con el acento al 10 % y su filo, como lo seleccionado
        // en cualquier lista del sistema; el color del perfil se queda en el
        // punto. Rellenar la fila del color del perfil dejaba el texto sin
        // contraste en amarillo y hacía que las otras dos no parecieran
        // pulsables.
        let (fondo, borde, color, peso) = if activo {
            (
                control::seleccionada(señalado),
                tema::alfa(tema::acento(), 0.45),
                tema::acento(),
                control::peso(iced_core::font::Weight::Semibold),
            )
        } else {
            (
                tema::mezclar(Color::TRANSPARENT, tema::card(), señalado),
                Color::TRANSPARENT,
                tema::texto(),
                media(),
            )
        };
        let color_punto = perfil.color;
        let punto = container(Space::new())
            .width(Length::Fixed(PUNTO))
            .height(Length::Fixed(PUNTO))
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(color_punto.into()),
                border: Border {
                    radius: (PUNTO / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let mut contenido = row![
            punto,
            Space::new().width(Length::Fixed(12.0)),
            text(perfil.etiqueta).size(14.5).font(peso).color(color),
            Space::new().width(Length::Fill),
        ]
        .align_y(Vertical::Center);
        if activo {
            contenido = contenido.push(icono_o_hueco(self.marca.as_ref(), MARCA, tema::acento()));
        }
        container(contenido)
            .width(Length::Fill)
            .height(Length::Fixed(FILA))
            .center_y(Length::Fixed(FILA))
            .padding([0, 12])
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_BOTON_PEQUENO.into(),
                    width: 1.0,
                    color: borde,
                },
                ..Default::default()
            })
            .into()
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut contenido = column![container(self.cabecera()).height(Length::Fixed(CABECERA))];
        if !self.perfiles.is_empty() {
            let mut filas = column![].spacing(HUECO_FILA);
            for i in 0..self.perfiles.len() {
                filas = filas.push(self.fila(i));
            }
            contenido = contenido
                .push(Space::new().height(Length::Fixed(HUECO_BLOQUE)))
                .push(control::grupo(
                    filas.into(),
                    ANCHO - MARGEN * 2.0,
                    control::GRUPO,
                ));
        }
        contenido = contenido
            .push(Space::new().height(Length::Fixed(HUECO_BLOQUE)))
            .push(container(self.pie()).height(Length::Fixed(PIE)));
        control::tarjeta(contenido.into(), ANCHO, MARGEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El hueco entre filas no pertenece a ninguna: pulsarlo no debe cambiar el
    /// perfil.
    #[test]
    fn el_hueco_entre_filas_no_es_de_nadie() {
        let mut e = Energia::new();
        e.perfiles = ["a", "b"]
            .into_iter()
            .map(|c| Perfil {
                clave: c.into(),
                etiqueta: "X",
                color: tema::TEXTO2,
            })
            .collect();
        let y0 = e.y_lista();
        assert_eq!(e.fila_en(50.0, y0 + 1.0), Some(0));
        assert_eq!(e.fila_en(50.0, y0 + FILA + HUECO_FILA / 2.0), None);
        assert_eq!(e.fila_en(50.0, y0 + FILA + HUECO_FILA + 1.0), Some(1));
        assert_eq!(e.fila_en(50.0, y0 - 1.0), None, "por encima de la lista");
    }

    /// Sin perfiles —un equipo sin `power-profiles-daemon`— la tarjeta encoge
    /// en vez de dejar un hueco vacío debajo.
    #[test]
    fn sin_perfiles_la_tarjeta_encoge() {
        let mut e = Energia::new();
        e.perfiles = vec![Perfil {
            clave: "a".into(),
            etiqueta: "X",
            color: tema::TEXTO2,
        }];
        let con = e.size().1;
        e.perfiles.clear();
        assert!(
            e.size().1 < con,
            "{} debería ser menor que {con}",
            e.size().1
        );
    }
}
