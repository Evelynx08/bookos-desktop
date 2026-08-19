//! La batería: porcentaje, si carga y cuánto queda.
//!
//! El pictograma **se dibuja aquí**, no sale del tema de iconos. Es lo mismo
//! que hace el plasmoide `bookos-battery`: cuerpo de 23×11 con borde, relleno
//! proporcional al tanto por ciento y un pezón de 2 px a la derecha. Un icono
//! del tema solo tiene diez escalones y su color lo decide el tema; aquí el
//! relleno es continuo, el color dice el **perfil de energía** —amarillo en
//! ahorro, verde equilibrado, azul rendimiento— y el símbolo de dentro dice de
//! dónde come el equipo: rayo cargando, enchufe conectado pero parado.

use std::time::{Duration, Instant};

use iced_widget::{row, text};

use crate::icono::{self, Icono};
use crate::state::Battery;
use crate::tema;
use crate::view::{PanelElement, PELIGRO, TEXT};
use crate::widget::Widget;

/// Medidas del pictograma, las del plasmoide: 23×11 el cuerpo más 3 px del
/// pezón. No es cuadrado, y por eso no puede usar `ICONO_PANEL` para las dos
/// dimensiones como los demás widgets.
const ANCHO_ICONO: f32 = 26.0;
const ALTO_ICONO: f32 = 11.0;

/// El color en `#rrggbb`, que es lo que entiende el SVG.
fn hex(c: iced_core::Color) -> String {
    let b = c.into_rgba8();
    format!("#{:02x}{:02x}{:02x}", b[0], b[1], b[2])
}

/// Qué se dibuja dentro del cuerpo de la batería.
///
/// De dónde come el equipo se dice **dentro** del pictograma y no con el color,
/// que está reservado al perfil de energía: enchufado y cargando es un rayo;
/// enchufado y parado —el umbral de carga del portátil, o la batería llena— es
/// un enchufe, que es justo el estado que un rayo contaría mal.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Simbolo {
    Nada,
    Rayo,
    Enchufe,
}

impl Simbolo {
    fn de(bat: &Battery) -> Self {
        match (bat.charging, bat.plugged) {
            (true, _) => Self::Rayo,
            (false, true) => Self::Enchufe,
            (false, false) => Self::Nada,
        }
    }

    /// Para la clave del icono cacheado.
    fn clave(self) -> &'static str {
        match self {
            Self::Nada => "-",
            Self::Rayo => "rayo",
            Self::Enchufe => "enchufe",
        }
    }
}

/// El SVG de la batería con su relleno y su símbolo de alimentación.
///
/// El borde va al 48 % del blanco y el pezón al 65 %, como en el plasmoide: a
/// plena opacidad la silueta pesa más que el relleno y el widget se lee como un
/// icono apagado.
fn pictograma(porciento: u8, simbolo: Simbolo, color: iced_core::Color) -> String {
    const W: f32 = 23.0;
    const H: f32 = 11.0;
    let relleno = (W - 3.0) * porciento.min(100) as f32 / 100.0;
    let c = hex(color);
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="26" height="11" viewBox="0 0 26 11">
<rect x="0.5" y="0.5" width="{w}" height="{h}" rx="2.75" fill="none" stroke="#ffffff" stroke-opacity="0.48"/>
<rect x="1.5" y="1.5" width="{relleno:.2}" height="{alto_relleno}" rx="1.4" fill="{c}"/>
<rect x="24" y="3.2" width="2" height="4.4" rx="1" fill="#ffffff" fill-opacity="0.65"/>"##,
        w = W - 1.0,
        h = H - 1.0,
        alto_relleno = H - 3.0,
    );
    // Los dos símbolos van en el centro del cuerpo, así que la tinta la decide
    // si el relleno ha llegado hasta ahí: sobre el color, blanco no se
    // distingue. El 52 % sale de que el centro del dibujo cae en x=11,5 de los
    // 20 px de recorrido del relleno.
    let tinta = if porciento > 52 { "#000000" } else { "#ffffff" };
    match simbolo {
        Simbolo::Nada => {}
        // El rayo del plasmoide, encajado en el cuerpo.
        Simbolo::Rayo => svg.push_str(&format!(
            r##"<g transform="translate(7.5 0.6) scale(0.098)"><polygon points="64,2 18,54 46,50 36,98 82,46 54,50" fill="{tinta}"/></g>"##
        )),
        // El enchufe: dos clavijas, el cuerpo y el arranque del cable. A 7 px de
        // alto no cabe el trazo de un heroicon —las líneas de 1,5 se comen el
        // hueco entre las clavijas—, así que va en silueta maciza. Las medidas
        // salieron de rasterizar tres variantes al tamaño real y mirarlas: con
        // clavijas de 1,2 px y 1,8 de separación el antialias las funde en un
        // bloque, y hacen falta 1,6 y 2,0 para que se sigan viendo dos.
        Simbolo::Enchufe => svg.push_str(&format!(
            r##"<g fill="{tinta}">
<rect x="9.0" y="1.6" width="1.6" height="2.6" rx="0.3"/>
<rect x="12.6" y="1.6" width="1.6" height="2.6" rx="0.3"/>
<rect x="7.8" y="4.0" width="7.6" height="3.4" rx="1.2"/>
<rect x="10.8" y="7.4" width="1.6" height="1.6"/>
</g>"##
        )),
    }
    svg.push_str("</svg>");
    svg
}

pub struct Bateria {
    dato: Option<Battery>,
    /// El icono que toca para el nivel y el estado de carga actuales.
    icono: Option<Icono>,
    /// Con qué nombre se cargó, para no volver a buscarlo mientras no cambie.
    icono_nombre: String,
    /// Media suavizada de los minutos restantes. Ver [`Bateria::suavizar`].
    minutos: Option<f32>,
    /// El perfil de energía, que decide el color cuando no manda el nivel.
    perfil: Option<String>,
    /// Cuándo se leyó por última vez cada una de las dos cosas caras. Ver
    /// [`CADA_PERFIL`] y [`CADA_MINUTOS`].
    perfil_leido: Instant,
    minutos_leidos: Instant,
}

/// Cada cuánto se vuelven a leer las dos fuentes lentas.
///
/// **Medido en esta máquina**: `platform_profile` cuesta 0,71 ms y
/// `current_now` 0,53, contra los 0,02 de `capacity` y `status`. Las dos son
/// consultas al firmware, no lecturas de un fichero, y se pagaban enteras en
/// cada evento de `power_supply` —que llegan de varios en varios.
///
/// Los intervalos son los de lo que enseñan, no números redondos: el perfil
/// solo cambia cuando alguien lo cambia a mano, y los minutos restantes ya van
/// suavizados y redondeados a cinco, así que leerlos más de una vez cada medio
/// minuto no cambia ni un píxel de lo dibujado.
const CADA_PERFIL: Duration = Duration::from_secs(5);
const CADA_MINUTOS: Duration = Duration::from_secs(30);

impl Bateria {
    pub fn new() -> Self {
        let mut b = Self {
            dato: None,
            icono: None,
            icono_nombre: String::new(),
            minutos: None,
            perfil: None,
            // Restados, para que el primer refresco —el del arranque— lea las
            // dos: el panel tiene que salir con su color y su tiempo puestos.
            perfil_leido: Instant::now() - CADA_PERFIL,
            minutos_leidos: Instant::now() - CADA_MINUTOS,
        };
        b.refrescar();
        b
    }

    /// Rehace el pictograma si cambió algo de lo que dibuja.
    ///
    /// El SVG se regenera solo cuando cambia el escalón entero o el color:
    /// rasterizarlo en cada lectura sería trabajo por nada, y el relleno no se
    /// distingue por debajo de un punto porcentual.
    fn actualizar_icono(&mut self) {
        let Some(bat) = self.dato else {
            self.icono = None;
            self.icono_nombre.clear();
            return;
        };
        let color = Self::color(&bat, self.perfil.as_deref());
        let simbolo = Simbolo::de(&bat);
        let clave = format!("{}-{}-{}", bat.percent, simbolo.clave(), hex(color));
        if clave == self.icono_nombre {
            return;
        }
        self.icono = Some(icono::desde_svg(&pictograma(bat.percent, simbolo, color)));
        self.icono_nombre = clave;
    }

    /// Amortigua el tiempo restante antes de enseñarlo.
    ///
    /// El kernel da la corriente **instantánea**, así que el cálculo crudo se
    /// mueve una barbaridad: en este portátil, 1:28 con el compilador a tope y
    /// 8:18 treinta segundos después en reposo. Enseñar eso tal cual convierte
    /// el panel en algo que nadie puede creerse, así que se promedia.
    ///
    /// Es una media exponencial sobre las **lecturas**, no sobre el tiempo: no
    /// se puede suponer una cadencia fija cuando cualquier evento de udev
    /// provoca una lectura de más. Sube y baja igual de despacio a propósito —
    /// lo que se quiere es un número estable, no uno que reaccione rápido.
    fn suavizar(&mut self, fresco: &mut Battery, leido: bool) {
        const PESO: f32 = 0.25;
        // Sin lectura nueva se conserva la media que había. Sin esto, los
        // refrescos de entre medias —los que solo miran el porcentaje— dejarían
        // `minutes` en `None` y la etiqueta perdería el «2:15» hasta la
        // siguiente lectura cara: un parpadeo cada pocos segundos.
        if !leido {
            fresco.minutes = self.minutos.map(|m| (m / 5.0).round() as u32 * 5);
            return;
        }
        // Enchufar o desenchufar cambia el signo de lo que se mide: la media
        // anterior ya no vale de nada y se empieza de cero.
        if self.dato.map(|b| b.charging) != Some(fresco.charging) {
            self.minutos = None;
        }
        let Some(crudo) = fresco.minutes else {
            self.minutos = None;
            return;
        };
        let suave = match self.minutos {
            Some(previo) => previo + (crudo as f32 - previo) * PESO,
            None => crudo as f32,
        };
        self.minutos = Some(suave);
        // Redondeado a cinco minutos: la precisión de un minuto en una
        // estimación así es mentira, y además haría repintar el panel más a
        // menudo de lo que aporta.
        fresco.minutes = Some((suave / 5.0).round() as u32 * 5);
    }
}

impl Bateria {
    /// El color del pictograma: **el del perfil de energía**.
    ///
    /// Antes mandaba el estado de carga —verde cargando, ámbar por debajo del
    /// 30 %— y el perfil solo se veía el resto del tiempo, que en un portátil
    /// enchufado es casi nunca. Ahora la alimentación la cuenta el símbolo de
    /// dentro (rayo o enchufe) y el color queda libre para lo único que no
    /// tiene otro sitio donde enseñarse: en qué perfil va el equipo.
    ///
    /// La única excepción es el rojo bajo mínimos y desenchufado, que es un
    /// aviso y no un estado: ahí el perfil da igual.
    fn color(bat: &Battery, perfil: Option<&str>) -> iced_core::Color {
        if bat.percent <= 15 && !bat.plugged {
            PELIGRO()
        } else {
            // Cada perfil con su color, que son los del diseño: ahorro
            // amarillo, equilibrado verde y rendimiento azul. `low-power` es
            // como lo llama la ACPI y `power-saver` como lo llama
            // power-profiles-daemon; el fichero de sysfs trae uno u otro según
            // el driver de la plataforma.
            match perfil {
                Some("low-power" | "power-saver" | "quiet") => tema::perfil_ahorro(),
                Some("balanced" | "balanced-performance") => tema::perfil_equilibrado(),
                Some("performance") => tema::perfil_rendimiento(),
                _ => TEXT(),
            }
        }
    }

    /// El perfil de energía, leído de sysfs.
    ///
    /// Se lee el fichero en vez de preguntar a power-profiles-daemon por D-Bus
    /// porque esto ocurre en el refresco del panel: `busctl` es un proceso
    /// nuevo cada vez. Lo que **no** es cierto es que leerlo sea barato:
    /// `platform_profile` no es un fichero, es una llamada a la ACPI, y medido
    /// en esta máquina cuesta **0,71 ms** —treinta veces más que `capacity`—.
    ///
    /// Por eso no se lee en cada refresco sino como mucho cada [`CADA_PERFIL`]:
    /// el perfil solo cambia cuando alguien lo cambia a mano, y los refrescos
    /// llegan en ráfaga con cada evento de `power_supply`.
    fn perfil(&mut self) -> Option<String> {
        if self.perfil_leido.elapsed() < CADA_PERFIL {
            return self.perfil.clone();
        }
        self.perfil_leido = Instant::now();
        std::fs::read_to_string("/sys/firmware/acpi/platform_profile")
            .ok()
            .map(|s| s.trim().to_string())
    }

    /// Lo que se enseña al lado del icono.
    ///
    /// Solo el número: lo demás lo cuenta el icono, que lleva el rayo cuando
    /// carga, y el color, que se pone verde cargando y rojo por debajo del
    /// 15 %. "Cargando 80%" al lado de un icono con un rayo es decir lo mismo
    /// dos veces y ocupa el ancho de otro estado entero.
    ///
    /// Está aparte de `ver` porque `ancho` necesita contar sus caracteres, y
    /// dos copias de esta regla se desincronizan a la primera.
    fn etiqueta(bat: &crate::state::Battery) -> String {
        let mut etiqueta = format!("{}%", bat.percent);
        // El tiempo solo aparece cuando la cuenta sale y aún tiene sentido
        // enseñarla: a punto de llenarse, "0:04 para el 100%" es ruido.
        if let Some(min) = bat.minutes.filter(|_| !bat.charging || bat.percent < 95) {
            etiqueta.push_str(&format!(" {}:{:02}", min / 60, min % 60));
        }
        etiqueta
    }
}

impl Widget for Bateria {
    fn nombre(&self) -> &'static str {
        "bateria"
    }

    fn subsistemas(&self) -> &'static [&'static str] {
        &["power_supply"]
    }

    fn refrescar(&mut self) -> bool {
        // El tiempo restante solo se estima de tanto en tanto: es la parte que
        // le cuesta medio milisegundo al controlador embebido.
        let toca_minutos = self.minutos_leidos.elapsed() >= CADA_MINUTOS;
        if toca_minutos {
            self.minutos_leidos = Instant::now();
        }
        let mut fresco = Battery::read(toca_minutos);
        match fresco.as_mut() {
            Some(b) => self.suavizar(b, toca_minutos),
            None => self.minutos = None,
        }
        let perfil = self.perfil();
        if fresco == self.dato && perfil == self.perfil {
            return false;
        }
        self.dato = fresco;
        self.perfil = perfil;
        self.actualizar_icono();
        true
    }

    /// Icono, su hueco y el texto, que puede ser "85%" o "85% 2:15".
    fn ancho(&self) -> f32 {
        let Some(bat) = &self.dato else {
            return 0.0;
        };
        ANCHO_ICONO + 5.0 + crate::widget::ancho_de(&Self::etiqueta(bat), tema::T_CUERPO)
    }

    fn ver(&self) -> PanelElement<'_> {
        let Some(bat) = &self.dato else {
            return crate::widget::vacio();
        };
        // El texto **no** sigue al color del pictograma: en el diseño el "67 %"
        // va en blanco con la batería amarilla, porque quien indica el estado
        // es el dibujo y el número solo es el dato. Solo bajo mínimos se pinta
        // también el texto, que ahí sí conviene que grite.
        let color = if bat.percent <= 15 && !bat.plugged {
            PELIGRO()
        } else {
            TEXT()
        };
        let etiqueta = Self::etiqueta(bat);

        let mut fila = row![]
            .spacing(5)
            .align_y(iced_core::alignment::Vertical::Center);
        if let Some(ic) = &self.icono {
            // Sin teñir: el pictograma ya trae dentro el color del estado, el
            // del borde y el del relleno, que son tres y distintos.
            fila = fila.push(icono::ver(ic, ANCHO_ICONO, ALTO_ICONO));
        }
        fila.push(text(etiqueta).size(tema::T_CUERPO).color(color))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bat(percent: u8, charging: bool, plugged: bool) -> Battery {
        Battery {
            percent,
            charging,
            plugged,
            minutes: None,
        }
    }

    /// El perfil manda sobre el estado de carga: enchufado y cargando en modo
    /// ahorro, el pictograma sigue siendo amarillo. Antes se ponía verde y el
    /// perfil no se veía nunca en un portátil de sobremesa.
    #[test]
    fn el_color_es_el_del_perfil() {
        assert_eq!(
            Bateria::color(&bat(80, true, true), Some("low-power")),
            tema::perfil_ahorro()
        );
        assert_eq!(
            Bateria::color(&bat(80, false, true), Some("balanced")),
            tema::perfil_equilibrado()
        );
        assert_eq!(
            Bateria::color(&bat(80, false, false), Some("performance")),
            tema::perfil_rendimiento()
        );
        assert_eq!(Bateria::color(&bat(80, false, false), None), TEXT());
    }

    /// Bajo mínimos y desenchufado el rojo se impone: es un aviso, no un
    /// estado. Enchufado al 10 % no hay nada que avisar.
    #[test]
    fn el_rojo_solo_avisa_sin_corriente() {
        assert_eq!(
            Bateria::color(&bat(10, false, false), Some("balanced")),
            PELIGRO()
        );
        assert_eq!(
            Bateria::color(&bat(10, false, true), Some("balanced")),
            tema::perfil_equilibrado()
        );
    }

    /// Enchufado y parado —el umbral de carga del portátil— es un enchufe, no
    /// un rayo: el rayo diría que está entrando energía y no entra.
    #[test]
    fn el_enchufe_distingue_cargar_de_estar_conectado() {
        assert!(Simbolo::de(&bat(80, true, true)) == Simbolo::Rayo);
        assert!(Simbolo::de(&bat(80, false, true)) == Simbolo::Enchufe);
        assert!(Simbolo::de(&bat(80, false, false)) == Simbolo::Nada);
    }

    /// El SVG generado tiene que llevar el dibujo que toca. Es lo único que
    /// distingue los dos estados de CA, y se genera con `format!`.
    #[test]
    fn el_pictograma_dibuja_su_simbolo() {
        let rayo = pictograma(80, Simbolo::Rayo, tema::acento());
        assert!(rayo.contains("polygon"), "el rayo es un polígono");
        let enchufe = pictograma(80, Simbolo::Enchufe, tema::acento());
        assert!(!enchufe.contains("polygon"));
        // Las cuatro piezas del enchufe más los tres rectángulos del cuerpo.
        assert_eq!(enchufe.matches("<rect").count(), 7);
        assert_eq!(
            pictograma(80, Simbolo::Nada, tema::acento())
                .matches("<rect")
                .count(),
            3
        );
    }
}
