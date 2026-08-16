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
use iced_widget::{column, container, row, text, Space};

use crate::icono::{self, Icono};
use crate::state::Battery;
use crate::tema;
use crate::view::PanelElement;
use crate::Accion;

use super::control;
use super::{Ancla, Tecla};

// Las medidas del diseño (`Applets` en Figma): tarjeta de 335, filas de 278×35
// con 15 de aire entre ellas y un margen lateral de 21.
const ANCHO: f32 = 335.0;
const MARGEN: f32 = 21.0;
/// Alto de cada fila de perfil.
///
/// 44 y no los 35 del primer boceto: la fila lleva dos renglones —el nombre a
/// 15 y la explicación a 10— y en 35 px el descendente de la «g» de «energía»
/// tocaba el borde de la pastilla.
const FILA: f32 = 44.0;
/// Hueco entre filas de perfil. Con las filas ya rellenas de fondo, 15 px de
/// aire las desunía: se leían como tres tarjetas y no como una elección de tres.
const HUECO_FILA: f32 = 6.0;
/// Lado del icono de una fila.
const ICONO_FILA: f32 = 18.0;
/// Lado de la pastilla redonda que lo envuelve.
const PASTILLA: f32 = 30.0;
/// Alto de la cabecera: título, consumo y el nivel a la derecha.
const CABECERA: f32 = 46.0;
/// Alto del rótulo «Modo de energía» con su aire.
const ROTULO: f32 = 39.0;
/// Alto de cada renglón del pie.
const PIE: f32 = 35.0;

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
    /// Qué hace, en una línea.
    detalle: &'static str,
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
fn nombrar(clave: &str) -> (&'static str, &'static str, Color, &'static str) {
    match clave {
        "power-saver" => (
            "Ahorro de energía",
            "Máx. duración de batería",
            tema::PERFIL_AHORRO,
            "perfil-ahorro",
        ),
        "balanced" => (
            "Equilibrado",
            "Rendimiento equilibrado",
            tema::PERFIL_EQUILIBRADO,
            "perfil-equilibrado",
        ),
        "performance" => (
            "Alto rendimiento",
            "Máx. potencia del sistema",
            tema::PERFIL_RENDIMIENTO,
            "perfil-rendimiento",
        ),
        // Los hay con un cuarto perfil propio del fabricante; se enseña con el
        // icono genérico en vez de esconderlo.
        _ => ("Otro perfil", "", tema::TEXTO2, "cpu"),
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
            let (etiqueta, detalle, color, icono) = nombrar(&clave);
            Perfil {
                clave,
                etiqueta,
                detalle,
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

/// Cuánto está consumiendo el equipo ahora mismo, en vatios.
///
/// El driver da la corriente en µA y la tensión en µV; el producto sale en pW,
/// de ahí el 1e12. Con el portátil enchufado y el umbral de carga alcanzado,
/// `current_now` se queda en 0 y no hay nada que enseñar: eso es `None`, no
/// «0,0 W».
fn vatios() -> Option<f32> {
    let ruta = ruta_bateria()?;
    let leer = |f: &str| -> Option<f64> {
        std::fs::read_to_string(ruta.join(f))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    // `power_now` ya viene en µW en los equipos que lo traen; los demás dan
    // corriente y tensión por separado.
    if let Some(uw) = leer("power_now").filter(|w| *w > 0.0) {
        return Some((uw / 1e6) as f32);
    }
    let ua = leer("current_now").filter(|c| *c > 0.0)?;
    let uv = leer("voltage_now")?;
    Some((ua * uv / 1e12) as f32)
}

/// Cuántos ciclos de carga lleva la batería. Lo trae casi cualquier portátil y
/// es el dato que de verdad dice cómo está de vieja.
fn ciclos() -> Option<u32> {
    let ruta = ruta_bateria()?;
    std::fs::read_to_string(ruta.join("cycle_count"))
        .ok()?
        .trim()
        .parse()
        .ok()
        .filter(|c| *c > 0)
}

/// La primera batería de sysfs. Se busca aquí y no se reutiliza la de
/// `state.rs` porque allí es privada y solo devuelve el dato ya interpretado.
fn ruta_bateria() -> Option<std::path::PathBuf> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut rutas: Vec<_> = dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            std::fs::read_to_string(p.join("type"))
                .is_ok_and(|t| t.trim() == "Battery")
        })
        .collect();
    // Ordenadas para que BAT0 gane a BAT1 y la elección no dependa del orden
    // del directorio, que no está garantizado.
    rutas.sort();
    rutas.into_iter().next()
}

pub struct Energia {
    bateria: Option<Battery>,
    /// Consumo ahora mismo y ciclos de carga, para la cabecera.
    vatios: Option<f32>,
    ciclos: Option<u32>,
    /// El renglón del pie sobre el consumo de las aplicaciones.
    consumo: String,
    perfiles: Vec<Perfil>,
    activo: Option<String>,
    /// Sobre qué fila está el ratón, para el realce.
    señalado: Option<usize>,
    /// El del renglón del consumo por aplicación.
    icono_consumo: Option<Icono>,
}

impl Energia {
    pub fn new() -> Self {
        Self {
            bateria: Battery::read(),
            vatios: vatios(),
            ciclos: ciclos(),
            // Saber **qué** aplicación consume exige contabilidad por proceso
            // —leer /proc entero y atribuir energía—, que es un servicio aparte
            // y no un rato de trabajo. El renglón está y dice lo que sabe.
            consumo: "Sin datos de consumo por aplicación".into(),
            perfiles: perfiles(),
            activo: perfil_activo(),
            señalado: None,
            icono_consumo: icono::propio("cpu"),
        }
    }

    pub fn size(&self) -> (f32, f32) {
        // `y_lista` ya lleva el margen de arriba; el de abajo hay que sumarlo
        // aparte, más los tres divisores de 1 px. Sin ellos la tarjeta salía
        // más corta que su contenido y el pie se pintaba recortado contra el
        // borde —es lo que le pasaba al «Detalles» del Bluetooth.
        (
            ANCHO,
            self.y_lista() + self.alto_lista() + 12.0 + 3.0 + PIE * 2.0 + MARGEN,
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
        MARGEN + CABECERA + ROTULO
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
        let nuevo = punto.and_then(|(x, y)| self.fila_en(x, y));
        if nuevo == self.señalado {
            return false;
        }
        self.señalado = nuevo;
        true
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
            .map_or(tema::TEXTO, |p| p.color)
    }

    /// La cabecera: «Batería» con la fuente debajo y el nivel a la derecha.
    ///
    /// **Sin pictograma.** Estaba a la derecha del «80 %» y repetía el del
    /// panel, tres centímetros más arriba y en el mismo eje vertical: dos
    /// dibujos de lo mismo a la vez. El número solo, grande y en el color del
    /// perfil, dice más.
    fn cabecera(&self) -> PanelElement<'_> {
        let nivel = match self.bateria {
            Some(b) => format!("{}%", b.percent),
            // Un sobremesa no tiene batería y la tarjeta sigue teniendo
            // sentido: los perfiles son suyos igual.
            None => "—".to_string(),
        };
        // «Fuente: batería» o «Fuente: CA · 45 W», como en el diseño: lo
        // primero que se quiere saber es de dónde está comiendo el equipo, y
        // los vatios son de esa fuente, no de la batería.
        let detalle = {
            let fuente = match self.bateria {
                Some(b) if b.plugged || b.charging => "CA",
                Some(_) => "Batería",
                None => "CA",
            };
            match (self.vatios, self.ciclos) {
                (Some(w), _) => format!("Fuente: {fuente} · {w:.1} W"),
                (None, Some(c)) => format!("Fuente: {fuente} · {c} ciclos"),
                (None, None) => format!("Fuente: {fuente}"),
            }
        };
        row![
            column![
                text("Batería").size(tema::T_TITULO).color(tema::TEXTO),
                text(detalle).size(tema::T_CUERPO).color(tema::TEXTO2),
            ],
            Space::new().width(Length::Fill),
            text(nivel).size(26.0).color(self.color_activo()),
        ]
        .align_y(Vertical::Center)
        .into()
    }

    /// Un renglón del pie, con su icono opcional.
    fn renglon<'a>(&self, texto: &str, icono: Option<&'a Icono>) -> PanelElement<'a> {
        let mut fila = row![].align_y(Vertical::Center);
        if let Some(ic) = icono {
            fila = fila
                .push(icono_o_hueco(Some(ic), ICONO_FILA, tema::TEXTO2))
                .push(Space::new().width(Length::Fixed(12.0)));
        }
        container(fila.push(text(texto.to_string()).size(13.0).color(tema::TEXTO2)))
            .height(Length::Fixed(PIE))
            .center_y(Length::Fixed(PIE))
            .into()
    }

    /// El pie: «Preferencias de batería» con su flecha, que lleva a la página
    /// de batería de los ajustes.
    fn pie(&self) -> PanelElement<'_> {
        container(
            row![
                text("Preferencias de batería").size(13.0).color(tema::TEXTO2),
                Space::new().width(Length::Fill),
                text("›").size(15.0).color(tema::TEXTO2),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(ANCHO - MARGEN * 2.0))
        .height(Length::Fixed(PIE))
        .center_y(Length::Fixed(PIE))
        .into()
    }

    /// La línea de 1 px que separa los bloques.
    fn divisor<'a>() -> PanelElement<'a> {
        container(Space::new().height(Length::Fixed(1.0)))
            .width(Length::Fixed(ANCHO - MARGEN * 2.0))
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(tema::DIVISOR.into()),
                ..Default::default()
            })
            .into()
    }

    fn fila(&self, i: usize) -> PanelElement<'_> {
        let perfil = &self.perfiles[i];
        let activo = self.activo.as_deref() == Some(perfil.clave.as_str());
        // El activo se marca con el acento y no solo con un tono más claro: el
        // realce del ratón ya usa el tono claro y los dos serían el mismo
        // dibujo.
        // El activo se pinta con **su** color y no con el acento del sistema:
        // es lo mismo que hace el pictograma del panel, y así elegir «Ahorro»
        // pone amarillo aquí y amarillo arriba. Con el acento fijo, los tres
        // perfiles se veían azules al seleccionarlos y el color de cada uno solo
        // vivía en un disco de 22 px.
        let sobre = tema::tinta_sobre(perfil.color);
        let (fondo, color) = match (activo, self.señalado == Some(i)) {
            (true, _) => (perfil.color, sobre),
            (false, true) => (tema::HOVER, tema::TEXTO),
            // Un fondo tenue permanente: sin él las filas no se leen como algo
            // que se pueda pulsar hasta que el ratón pasa por encima.
            (false, false) => (Color { a: 0.04, ..Color::WHITE }, tema::TEXTO),
        };
        let detalle_color = if activo {
            Color { a: 0.65, ..sobre }
        } else {
            tema::TEXTO2
        };
        let mut textos = column![text(perfil.etiqueta).size(15.0).color(color)];
        if !perfil.detalle.is_empty() {
            textos = textos.push(text(perfil.detalle).size(11.0).color(detalle_color));
        }
        // El icono va dentro de una pastilla del color del perfil: teñido a
        // secas, un trazo de 1,5 px en amarillo sobre negro casi no se ve, y la
        // pastilla le da la masa que necesita para identificar la fila de un
        // vistazo.
        let (pastilla_fondo, tinta) = if activo {
            (Color { a: 0.22, ..sobre }, sobre)
        } else {
            (Color { a: 0.16, ..perfil.color }, perfil.color)
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
        let contenido = row![
            pastilla,
            Space::new().width(Length::Fixed(12.0)),
            textos,
        ]
        .align_y(Vertical::Center);
        container(contenido)
            .width(Length::Fixed(ANCHO - MARGEN * 2.0))
            .height(Length::Fixed(FILA))
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
        let mut contenido = column![
            container(self.cabecera())
                .height(Length::Fixed(CABECERA))
                .center_y(Length::Fixed(CABECERA)),
            Self::divisor(),
        ];
        if !self.perfiles.is_empty() {
            contenido = contenido.push(
                container(text("Modo de energía").size(15.0).color(tema::TEXTO))
                    .height(Length::Fixed(ROTULO))
                    .center_y(Length::Fixed(ROTULO)),
            );
            for i in 0..self.perfiles.len() {
                if i > 0 {
                    contenido = contenido.push(Space::new().height(Length::Fixed(HUECO_FILA)));
                }
                contenido = contenido.push(self.fila(i));
            }
            contenido = contenido.push(Space::new().height(Length::Fixed(12.0)));
        }
        contenido = contenido
            .push(Self::divisor())
            .push(self.renglon(&self.consumo, self.icono_consumo.as_ref()))
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
                detalle: "",
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
            detalle: "",
            color: tema::TEXTO2,
            icono: None,
        }];
        let con = e.size().1;
        e.perfiles.clear();
        assert!(e.size().1 < con, "{} debería ser menor que {con}", e.size().1);
    }
}
