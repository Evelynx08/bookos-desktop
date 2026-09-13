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

// Las medidas del diseño (`Applets` en Figma): tarjeta de 335, filas de 278×35
// con 15 de aire entre ellas y un margen lateral de 21.
const ANCHO: f32 = 335.0;
const MARGEN: f32 = 21.0;
/// Alto de cada fila de perfil.
///
/// 44 con una sola línea de texto: la pastilla de 32 deja 6 px por arriba y por
/// abajo, que es el aire que el sistema de diseño le da a una fila de una línea.
/// Con las dos líneas que llevaba antes hacían falta 48.
const FILA: f32 = 44.0;
/// Hueco entre filas de perfil. Con las filas ya rellenas de fondo, 15 px de
/// aire las desunía: se leían como tres tarjetas y no como una elección de tres.
const HUECO_FILA: f32 = 6.0;
/// Lado del icono de una fila. 18 px es la medida que el sistema de diseño da
/// a los iconos de cabecera; a 16 el trazo de 1,5 de heroicons se emborrona y a
/// 22 el dibujo toca el borde de la pastilla.
const ICONO_FILA: f32 = 18.0;
/// Lado de la pastilla redonda que lo envuelve.
///
/// 32 y no 30: con el icono a 18 quedan 7 px de aire por lado, que es lo que
/// hace que el dibujo se vea centrado y no encajado a presión.
const PASTILLA: f32 = 32.0;
/// Lado del icono que marca el perfil elegido.
const MARCA: f32 = 15.0;
/// Alto de la cabecera: el título con su renglón de estado, y el nivel a la
/// derecha.
///
/// Es también lo que separa la lista del borde de arriba: `y_lista` lo suma, y
/// con él van las zonas de clic de las filas. Cambiarlo mueve las dos cosas a
/// la vez, que es justo lo que se quiere.
const CABECERA: f32 = 48.0;
/// Alto de cada renglón del pie.
const PIE: f32 = 35.0;
/// Aire entre la cabecera, la lista y el pie.
const HUECO_BLOQUE: f32 = 14.0;
/// Cuánto se mete el divisor por cada lado respecto al contenido.
const INSET_DIVISOR: f32 = 6.0;
/// Los dos pesos que usa esta tarjeta. El sistema de diseño solo admite 400,
/// 500, 600 y 700 —y el 700 en la cifra de batería, que es la excepción que él
/// mismo nombra—, así que aquí no hay más.
fn media() -> iced_core::Font {
    iced_core::Font {
        weight: iced_core::font::Weight::Medium,
        ..iced_core::Font::DEFAULT
    }
}

fn gorda() -> iced_core::Font {
    iced_core::Font {
        weight: iced_core::font::Weight::Bold,
        ..iced_core::Font::DEFAULT
    }
}

/// El icono teñido, o un hueco de su tamaño si falta: sin el hueco, la fila de
/// un perfil sin icono desalinearía el texto respecto a las demás.
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
    /// cuando ese perfil está puesto. Es lo que ata las dos cosas.
    color: Color,
    /// Su dibujo. El color solo no basta —tres discos iguales no dicen nada a
    /// quien no distingue el amarillo del verde—, y el rayo tachado, la balanza
    /// y el cohete se leen antes que el rótulo.
    icono: Option<Icono>,
}

/// Traduce la clave del daemon: etiqueta, explicación de una línea, color e
/// icono.
///
/// Los rótulos son los del diseño. La explicación va debajo en gris porque
/// «Equilibrado» a secas no dice qué hace, y es la diferencia entre elegir y
/// adivinar.
fn nombrar(clave: &str) -> (&'static str, Color, &'static str) {
    match clave {
        "power-saver" => ("Ahorro de energía", tema::perfil_ahorro(), "perfil-ahorro"),
        "balanced" => (
            "Equilibrado",
            tema::perfil_equilibrado(),
            "perfil-equilibrado",
        ),
        "performance" => (
            "Alto rendimiento",
            tema::perfil_rendimiento(),
            "perfil-rendimiento",
        ),
        // Los hay con un cuarto perfil propio del fabricante; se enseña con el
        // icono genérico en vez de esconderlo.
        _ => ("Otro perfil", tema::TEXTO2, "cpu"),
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
            let (etiqueta, color, icono) = nombrar(&clave);
            Perfil {
                clave,
                etiqueta,
                color,
                icono: icono::propio(icono),
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
        // `y_lista` ya lleva el margen de arriba; el de abajo hay que sumarlo
        // aparte, más los tres divisores de 1 px. Sin ellos la tarjeta salía
        // más corta que su contenido y el pie se pintaba recortado contra el
        // borde —es lo que le pasaba al «Detalles» del Bluetooth.
        (
            ANCHO,
            self.y_lista() + self.alto_lista() + HUECO_BLOQUE + 0.5 + PIE + MARGEN,
        )
    }

    /// Lo que ocupa la lista de perfiles.
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
        MARGEN + CABECERA + HUECO_BLOQUE
    }

    /// Qué fila de perfil cae en esa `y`.
    fn fila_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < MARGEN || x > ANCHO - MARGEN {
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
        // El pie va al final de la tarjeta: la última franja de `PIE` px.
        let alto = self.size().1;
        if y > alto - PIE - MARGEN && x > MARGEN && x < ANCHO - MARGEN {
            return Some(Accion::Lanzar("bookos-settings --bateria".into()));
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

    /// El color del perfil que está puesto, que es con el que se pinta el
    /// número grande de la cabecera: el mismo que lleva el pictograma del panel,
    /// para que las dos cosas se lean como una.
    fn color_activo(&self) -> Color {
        self.activo
            .as_deref()
            .and_then(|a| self.perfiles.iter().find(|p| p.clave == a))
            .map_or(tema::texto(), |p| p.color)
    }

    /// La cabecera: «Batería» con la fuente debajo y el nivel a la derecha.
    ///
    /// **Sin pictograma.** Estaba a la derecha del «80 %» y repetía el del
    /// panel, tres centímetros más arriba y en el mismo eje vertical: dos
    /// dibujos de lo mismo a la vez. El número solo, grande y en el color del
    /// perfil, dice más.
    fn cabecera(&self) -> PanelElement<'_> {
        let nivel = match self.bateria {
            // Un sobremesa no tiene batería y la tarjeta sigue teniendo
            // sentido: los perfiles son suyos igual.
            Some(b) => format!("{}%", b.percent),
            None => "—".to_string(),
        };
        // Una sola línea debajo del título, en gris, con lo que el número no
        // dice: de dónde come, cuánto queda y por qué la carga se para donde se
        // para. Estuvo repartido en un chip, una barra y una rejilla de datos;
        // tres elementos para lo que cabe en un renglón.
        let detalle = self.estado();
        row![
            column![
                text("Batería")
                    .size(17.0)
                    .font(gorda())
                    .color(tema::texto()),
                text(detalle).size(11.0).color(tema::TEXTO2),
            ]
            .spacing(3),
            Space::new().width(Length::Fill),
            text(nivel)
                .size(30.0)
                .font(gorda())
                .color(self.color_activo()),
        ]
        .align_y(Vertical::Center)
        .into()
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

    /// El pie: «Preferencias de batería» con su flecha    /// El pie: «Preferencias de batería» con su flecha, que lleva a la página
    /// de batería de los ajustes.
    fn pie(&self) -> PanelElement<'_> {
        container(
            row![
                text("Preferencias de batería")
                    .size(13.0)
                    .color(tema::TEXTO2),
                Space::new().width(Length::Fill),
                icono_o_hueco(self.flecha.as_ref(), 13.0, tema::TEXTO2),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(PIE))
        .center_y(Length::Fixed(PIE))
        .into()
    }

    /// La línea que separa los bloques: medio píxel y con inset lateral, que es
    /// lo que el sistema de diseño le pide a un divisor de filas. A 1 px y de
    /// borde a borde partía la tarjeta en trozos en vez de agrupar.
    fn divisor<'a>() -> PanelElement<'a> {
        container(Space::new().height(Length::Fixed(0.5)))
            .width(Length::Fixed(ANCHO - MARGEN * 2.0 - INSET_DIVISOR * 2.0))
            .padding([0, INSET_DIVISOR as u16])
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(tema::divisor().into()),
                ..Default::default()
            })
            .into()
    }

    fn fila(&self, i: usize) -> PanelElement<'_> {
        let perfil = &self.perfiles[i];
        let activo = self.activo.as_deref() == Some(perfil.clave.as_str());
        let señalado = self.señalado.intensidad(i);

        // El elegido se marca con **su** color al 14 % y una marca de
        // verificación, no rellenando la fila entera de amarillo con el texto en
        // negro: el sistema de diseño lo dice con todas las letras —«estados
        // activo/seleccionado en listas: fondo del color al 10 %; nunca color
        // sólido de fondo con texto oscuro»—. Además, con el fondo lleno la
        // fila elegida pesaba tanto que las otras dos no parecían pulsables.
        let (fondo, color) = if activo {
            (tema::alfa(perfil.color, 0.14), perfil.color)
        } else {
            (
                // Un fondo tenue permanente: sin él las filas no se leen como
                // algo que se pueda pulsar hasta que el ratón pasa por encima.
                tema::mezclar(tema::alfa(tema::tinta(), 0.04), tema::hover(), señalado),
                tema::texto(),
            )
        };
        // Solo el nombre. La explicación de una línea —«Máx. duración de
        // batería»— convertía la lista en tres párrafos para elegir entre tres
        // palabras que ya se entienden.
        let textos = text(perfil.etiqueta).size(14.0).font(media()).color(color);
        // El icono va dentro de una pastilla del color del perfil: teñido a
        // secas, un trazo de 1,5 px en amarillo sobre negro casi no se ve, y la
        // pastilla le da la masa que necesita para identificar la fila de un
        // vistazo. Elegido, la pastilla sube a color pleno y el dibujo pasa a la
        // tinta que contraste con él: es el único trozo de la fila que se
        // rellena, y basta para que se vea cuál está puesto.
        let (pastilla_fondo, tinta) = if activo {
            (perfil.color, tema::tinta_sobre(perfil.color))
        } else {
            (tema::alfa(perfil.color, 0.16), perfil.color)
        };
        let pastilla = container(icono_o_hueco(perfil.icono.as_ref(), ICONO_FILA, tinta))
            .width(Length::Fixed(PASTILLA))
            .height(Length::Fixed(PASTILLA))
            .center_x(Length::Fixed(PASTILLA))
            .center_y(Length::Fixed(PASTILLA))
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(pastilla_fondo.into()),
                border: Border {
                    radius: (PASTILLA / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let mut contenido = row![
            pastilla,
            Space::new().width(Length::Fixed(12.0)),
            textos,
            Space::new().width(Length::Fill),
        ]
        .align_y(Vertical::Center);
        if activo {
            contenido = contenido.push(icono_o_hueco(self.marca.as_ref(), MARCA, perfil.color));
        }
        container(contenido)
            .width(Length::Fixed(ANCHO - MARGEN * 2.0))
            // La pastilla mide 32 px y la fila 44. Sin `center_y`, Iced deja
            // los 12 px sobrantes enteros debajo del contenido, por lo que
            // icono, texto y marca parecen desplazados hacia arriba aunque la
            // propia fila sí esté en su sitio.
            .center_y(Length::Fixed(FILA))
            .padding([0, 12])
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    pub fn view(&self) -> PanelElement<'_> {
        // Cabecera, los perfiles y el pie. Nada más: cada cosa que se quitó de
        // aquí —la barra de nivel, el rótulo «MODO DE ENERGÍA», la rejilla de
        // datos— repetía algo que ya estaba dicho o contaba algo que no se
        // viene a mirar al desplegar la batería del panel.
        let mut contenido = column![
            container(self.cabecera())
                .height(Length::Fixed(CABECERA))
                .center_y(Length::Fixed(CABECERA))
        ];
        if !self.perfiles.is_empty() {
            contenido = contenido.push(Space::new().height(Length::Fixed(HUECO_BLOQUE)));
            for i in 0..self.perfiles.len() {
                if i > 0 {
                    contenido = contenido.push(Space::new().height(Length::Fixed(HUECO_FILA)));
                }
                contenido = contenido.push(self.fila(i));
            }
        }
        contenido = contenido
            .push(Space::new().height(Length::Fixed(HUECO_BLOQUE)))
            .push(Self::divisor())
            .push(self.pie());
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
                icono: None,
            })
            .collect();
        let y0 = e.y_lista();
        assert_eq!(e.fila_en(50.0, y0 + 1.0), Some(0));
        assert_eq!(e.fila_en(50.0, y0 + FILA + 2.0), None);
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
            icono: None,
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
