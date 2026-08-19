//! El emergente del Wi-Fi: qué redes hay y a cuál se está conectado.
//!
//! **De dónde salen las redes.** El estado del enlace se lee de sysfs —eso es
//! lo que hace el widget del panel—, pero la lista de redes con su nombre y su
//! señal solo la tiene NetworkManager. Se le pregunta con `nmcli`, que ya viene
//! con él, en vez de hablar su D-Bus desde aquí.
//!
//! **Sin reescanear.** `nmcli dev wifi list --rescan no` devuelve lo que el
//! demonio ya tiene en caché. Pedir un escaneo activo cada vez que se abre el
//! emergente enciende la radio a barrer canales, que es lo contrario del
//! objetivo de autonomía del proyecto; NetworkManager ya escanea por su cuenta
//! mientras la lista está a la vista.
//!
//! La orden se lanza **sin esperar** y se recoge en el refresco siguiente, como
//! en el widget del bluetooth: `nmcli` tarda decenas de milisegundos y el
//! compositor no puede pararse ahí.

use std::process::{Child, Command, Stdio};

use iced_core::Length;
use iced_widget::{column, Space};

use crate::icono;
use crate::tema;
use crate::view::PanelElement;
use crate::Accion;

use super::control;
use super::lista::{self, Entrada};
use super::{Ancla, Tecla};

/// Cuántas redes se enseñan. Más no caben sin hacer la tarjeta desplazable, y
/// las de más abajo son las que peor señal tienen.
const MAXIMO: usize = 4;

pub struct Red {
    entradas: Vec<Entrada>,
    encendida: bool,
    /// El recorrido de la bolita del interruptor, siguiendo a `encendida`.
    interruptor: tema::Transicion,
    señalada: tema::Realce,
    /// El botón del pie bajo el puntero: el 0 es el izquierdo y el 1 el otro.
    pie: tema::Realce,
    pendiente: Option<Child>,
}

impl Red {
    pub fn new() -> Self {
        let mut r = Self {
            entradas: Vec::new(),
            encendida: true,
            interruptor: tema::Transicion::nueva(1.0, tema::D_MODAL, tema::C_MUELLE),
            señalada: tema::Realce::nuevo(),
            pie: tema::Realce::nuevo(),
            pendiente: None,
        };
        // La primera lista se pide de golpe: al abrir el emergente hay que
        // enseñar algo, y aquí sí se puede esperar unos milisegundos porque
        // ocurre una vez, no en cada refresco del panel.
        if let Ok(salida) = Command::new("sh").arg("-c").arg(ORDEN).output() {
            r.aplicar(&String::from_utf8_lossy(&salida.stdout));
        }
        // La primera lectura no se anima: la tarjeta se abre con el interruptor
        // ya en su sitio, no poniéndose delante del usuario.
        r.interruptor.fijar(r.encendida as u8 as f32);
        r
    }

    fn aplicar(&mut self, salida: &str) {
        self.entradas = interpretar(salida)
            .into_iter()
            .take(MAXIMO)
            .map(|(nombre, señal, activa)| Entrada {
                icono: icono::propio(if activa || señal >= 40 {
                    "wifi"
                } else {
                    "sin-red"
                }),
                estado: if activa {
                    "Conectado".into()
                } else {
                    String::new()
                },
                derecha: Some(format!("{señal}%")),
                nombre,
                activa,
            })
            .collect();
        self.encendida = salida.contains("enabled");
        self.interruptor.ir_a(self.encendida as u8 as f32);
    }

    /// El alto reservado. El `MARGEN` final es el de abajo de la tarjeta:
    /// `y_lista` solo lleva el de arriba, y sin él el pie se quedaba fuera del
    /// buffer y se pintaba como dos rayas contra el borde.
    pub fn size(&self) -> (f32, f32) {
        (
            lista::ANCHO,
            self.y_lista() + self.alto_lista() + 10.0 + lista::PIE + lista::MARGEN,
        )
    }

    fn alto_lista(&self) -> f32 {
        let n = self.entradas.len().max(1) as f32;
        n * lista::FILA + (n - 1.0) * lista::HUECO_FILA
    }

    fn y_lista(&self) -> f32 {
        lista::MARGEN + lista::CABECERA
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("red")
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let señalada =
            punto.and_then(|(x, y)| lista::fila_en(x, y, self.y_lista(), self.entradas.len()));
        let pie = punto.and_then(|(x, y)| self.pie_en(x, y)).map(usize::from);
        // Los dos `señalar`, sin `||`: con el cortocircuito, salir de una fila
        // hacia un botón del pie apaga la fila y deja el botón sin encender.
        let a = self.señalada.señalar(señalada);
        let b = self.pie.señalar(pie);
        a || b
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    pub fn animando(&self) -> bool {
        self.señalada.animando() || self.pie.animando() || self.interruptor.animando()
    }

    /// Sobre qué botón del pie cae el punto.
    fn pie_en(&self, x: f32, y: f32) -> Option<bool> {
        let y0 = self.y_lista() + self.alto_lista() + 10.0 + lista::PIE_AIRE;
        if y < y0 || y > y0 + lista::PIE_BOTON {
            return None;
        }
        let mitad = lista::ANCHO / 2.0;
        (x > lista::MARGEN && x < lista::ANCHO - lista::MARGEN).then(|| x > mitad)
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(derecha) = self.pie_en(x, y) {
            return Some(Accion::Lanzar(if derecha {
                "bookos-settings --red".into()
            } else {
                // «Detalles» abre el editor de conexiones de NetworkManager
                // mientras bookos-settings no tenga su página.
                "bookos-settings --red || nm-connection-editor".into()
            }));
        }
        let i = lista::fila_en(x, y, self.y_lista(), self.entradas.len())?;
        let entrada = &self.entradas[i];
        if entrada.activa {
            return None;
        }
        // Conectarse a una red guardada es una orden y ya; una que pida
        // contraseña la rechazará `nmcli` y el usuario tendrá que ir a los
        // ajustes. Pedir la contraseña aquí es un teclado y un diálogo, y va
        // aparte.
        Some(Accion::Lanzar(format!(
            "nmcli connection up id '{0}' || nmcli device wifi connect '{0}'",
            entrada.nombre.replace('\'', "")
        )))
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    /// Recoge la consulta anterior y lanza la siguiente. `true` si cambió algo.
    pub fn refrescar(&mut self) -> bool {
        let mut cambio = false;
        if let Some(hijo) = &mut self.pendiente {
            match hijo.try_wait() {
                Ok(Some(_)) => {
                    let hijo = self.pendiente.take().expect("acabamos de verlo dentro");
                    if let Ok(salida) = hijo.wait_with_output() {
                        let antes = self.entradas.len();
                        self.aplicar(&String::from_utf8_lossy(&salida.stdout));
                        cambio = self.entradas.len() != antes;
                    }
                }
                Ok(None) => return false,
                Err(_) => self.pendiente = None,
            }
        }
        self.pendiente = Command::new("sh")
            .arg("-c")
            .arg(ORDEN)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok();
        cambio
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut contenido = column![lista::cabecera("Wi-Fi", Some(self.interruptor.valor()))];
        if self.entradas.is_empty() {
            contenido = contenido.push(lista::vacia("No hay redes a la vista"));
        } else {
            for (i, entrada) in self.entradas.iter().enumerate() {
                if i > 0 {
                    contenido =
                        contenido.push(Space::new().height(Length::Fixed(lista::HUECO_FILA)));
                }
                contenido = contenido.push(lista::fila(entrada, self.señalada.intensidad(i)));
            }
        }
        contenido = contenido
            .push(Space::new().height(Length::Fixed(10.0)))
            .push(lista::pie("Detalles", "Configuración", &self.pie));
        control::tarjeta(contenido.into(), lista::ANCHO, lista::MARGEN)
    }
}

/// Lo que se le pregunta a NetworkManager, en un solo proceso: si la radio está
/// encendida y qué redes ve.
const ORDEN: &str = "nmcli radio wifi; nmcli -t -f ACTIVE,SSID,SIGNAL dev wifi list --rescan no";

/// Saca `(nombre, señal, activa)` de la salida de `nmcli -t`.
///
/// El formato es `activo:ssid:señal` con dos puntos de separador. Un SSID puede
/// llevar dos puntos dentro —`nmcli` los escapa con barra— así que se parte por
/// el primero y por el último, no por todos.
fn interpretar(salida: &str) -> Vec<(String, u8, bool)> {
    let mut redes = Vec::new();
    for linea in salida.lines() {
        let Some((activo, resto)) = linea.split_once(':') else {
            continue;
        };
        // La primera línea es la respuesta de `nmcli radio wifi`: no lleva dos
        // puntos y no llega hasta aquí.
        let Some((ssid, señal)) = resto.rsplit_once(':') else {
            continue;
        };
        let Ok(señal) = señal.trim().parse::<u8>() else {
            continue;
        };
        let ssid = ssid.replace("\\:", ":");
        if ssid.trim().is_empty() {
            continue;
        }
        // `nmcli` traduce el "sí"/"no" al idioma de la sesión, así que se mira
        // lo que **no** es: cualquier cosa que empiece por "n" es un no.
        let activa = !activo.trim().to_lowercase().starts_with('n');
        redes.push((ssid, señal, activa));
    }
    // La conectada primero y el resto por señal: es el orden en que se busca.
    redes.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
    redes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La salida real de esta máquina, con el "sí" traducido y una red con la
    /// señal a dos cifras.
    #[test]
    fn interpreta_la_salida_de_nmcli() {
        let salida = "enabled\nno:Wifi Recamales:90\nsí:Wifi Recamales-5G:67\n\
                      no:La_GINESTA:45\nno:MOVISTAR-WIFI6-8284:14\n";
        let redes = interpretar(salida);
        assert_eq!(redes.len(), 4);
        // La conectada va la primera aunque tenga menos señal que otra.
        assert_eq!(redes[0].0, "Wifi Recamales-5G");
        assert!(redes[0].2, "la conectada tiene que salir como activa");
        assert_eq!(redes[1].0, "Wifi Recamales");
        assert_eq!(redes[1].1, 90);
    }

    /// Un SSID con dos puntos dentro no puede partir la línea por la mitad.
    #[test]
    fn un_ssid_con_dos_puntos_sobrevive() {
        let redes = interpretar("no:casa\\:wifi:55\n");
        assert_eq!(redes.len(), 1);
        assert_eq!(redes[0].0, "casa:wifi");
        assert_eq!(redes[0].1, 55);
    }

    /// Las líneas que no son redes se ignoran en vez de colarse como una red
    /// llamada "enabled".
    #[test]
    fn la_linea_de_la_radio_no_es_una_red() {
        assert!(interpretar("enabled\n").is_empty());
        assert!(interpretar("").is_empty());
    }
}
