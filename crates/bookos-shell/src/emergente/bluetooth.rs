//! El emergente del Bluetooth: qué hay emparejado y qué está puesto.
//!
//! Comparte tarjeta con el del Wi-Fi (ver [`super::lista`]) y saca el estado de
//! `bluetoothctl`, que es lo que ya hace el widget del panel: BlueZ no publica
//! nada útil en sysfs y meter un cliente de D-Bus aquí sería una dependencia
//! nueva.
//!
//! **La batería de los auriculares no se enseña** aunque el diseño la ponga:
//! BlueZ la expone en la propiedad `Battery1` de cada dispositivo, y sacarla
//! por `bluetoothctl` obliga a una orden por dispositivo. Es un proceso por
//! cada uno cada vez que se abre la tarjeta; se deja para cuando el shell hable
//! D-Bus de verdad.

use std::process::{Child, Command, Stdio};

use iced_core::Length;
use iced_widget::{column, Space};

use crate::icono;
use crate::view::PanelElement;
use crate::Accion;

use super::control;
use super::lista::{self, Entrada};
use super::{Ancla, Tecla};

/// Cuántos dispositivos caben sin hacer la tarjeta desplazable.
const MAXIMO: usize = 4;

pub struct Bluetooth {
    entradas: Vec<Entrada>,
    /// La dirección de cada entrada, en el mismo orden. Conectar va por MAC y
    /// no por nombre: dos auriculares del mismo modelo se llaman igual.
    direcciones: Vec<String>,
    encendido: bool,
    señalada: Option<usize>,
    pie: Option<bool>,
    pendiente: Option<Child>,
}

impl Bluetooth {
    pub fn new() -> Self {
        let mut b = Self {
            entradas: Vec::new(),
            direcciones: Vec::new(),
            encendido: false,
            señalada: None,
            pie: None,
            pendiente: None,
        };
        if let Ok(salida) = Command::new("sh").arg("-c").arg(ORDEN).output() {
            b.aplicar(&String::from_utf8_lossy(&salida.stdout));
        }
        b
    }

    fn aplicar(&mut self, salida: &str) {
        self.encendido = salida.contains("Powered: yes");
        let dispositivos: Vec<_> = interpretar(salida).into_iter().take(MAXIMO).collect();
        self.direcciones = dispositivos.iter().map(|(_, _, mac)| mac.clone()).collect();
        self.entradas = dispositivos
            .into_iter()
            .map(|(nombre, conectado, _)| Entrada {
                icono: icono::propio(if conectado {
                    "bluetooth"
                } else {
                    "bluetooth-apagado"
                }),
                estado: if conectado {
                    "Conectado".into()
                } else {
                    "Emparejado".into()
                },
                derecha: None,
                nombre,
                activa: conectado,
            })
            .collect();
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
        Ancla::BajoWidget("bluetooth")
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let señalada =
            punto.and_then(|(x, y)| lista::fila_en(x, y, self.y_lista(), self.entradas.len()));
        let pie = punto.and_then(|(x, y)| self.pie_en(x, y));
        if señalada == self.señalada && pie == self.pie {
            return false;
        }
        self.señalada = señalada;
        self.pie = pie;
        true
    }

    fn pie_en(&self, x: f32, y: f32) -> Option<bool> {
        let y0 = self.y_lista() + self.alto_lista() + 10.0 + lista::PIE_AIRE;
        if y < y0 || y > y0 + lista::PIE_BOTON {
            return None;
        }
        (x > lista::MARGEN && x < lista::ANCHO - lista::MARGEN)
            .then(|| x > lista::ANCHO / 2.0)
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(derecha) = self.pie_en(x, y) {
            return Some(Accion::Lanzar(if derecha {
                "bookos-settings --bluetooth".into()
            } else {
                "bookos-settings --bluetooth || blueman-manager".into()
            }));
        }
        let i = lista::fila_en(x, y, self.y_lista(), self.entradas.len())?;
        let entrada = &self.entradas[i];
        // Conectar y desconectar por dirección y no por nombre: dos auriculares
        // del mismo modelo se llaman igual.
        let orden = if entrada.activa {
            "disconnect"
        } else {
            "connect"
        };
        let mac = self.direcciones.get(i)?;
        Some(Accion::Lanzar(format!("bluetoothctl {orden} {mac}")))
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

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
        let mut contenido = column![lista::cabecera("Bluetooth", Some(self.encendido))];
        if self.entradas.is_empty() {
            contenido = contenido.push(lista::vacia(if self.encendido {
                "No hay dispositivos emparejados"
            } else {
                "Bluetooth apagado"
            }));
        } else {
            for (i, entrada) in self.entradas.iter().enumerate() {
                if i > 0 {
                    contenido =
                        contenido.push(Space::new().height(Length::Fixed(lista::HUECO_FILA)));
                }
                contenido = contenido.push(lista::fila(entrada, self.señalada == Some(i)));
            }
        }
        contenido = contenido
            .push(Space::new().height(Length::Fixed(10.0)))
            .push(lista::pie("Detalles", "Configuración", self.pie));
        control::tarjeta(contenido.into(), lista::ANCHO, lista::MARGEN)
    }
}

/// Estado del adaptador y los dispositivos que conoce, en un solo proceso.
///
/// `devices Connected` va antes que `devices Paired` para poder marcar cuáles
/// están puestos: la segunda lista incluye a los de la primera.
const ORDEN: &str =
    "bluetoothctl show; echo ---; bluetoothctl devices Connected; echo ---; bluetoothctl devices Paired";

/// Saca `(nombre, conectado, mac)` de la salida. El separador `---` divide las
/// tres órdenes.
fn interpretar(salida: &str) -> Vec<(String, bool, String)> {
    let mut partes = salida.split("---");
    let _show = partes.next();
    let conectados: Vec<String> = partes
        .next()
        .map(|t| macs(t).into_iter().map(|(m, _)| m).collect())
        .unwrap_or_default();
    let emparejados = partes.next().map(macs).unwrap_or_default();
    let mut salida: Vec<(String, bool, String)> = emparejados
        .into_iter()
        .map(|(mac, nombre)| {
            let puesto = conectados.contains(&mac);
            (nombre, puesto, mac)
        })
        .collect();
    // Lo puesto arriba: es lo que se busca al abrir la tarjeta.
    salida.sort_by(|a, b| b.1.cmp(&a.1));
    salida
}

/// `Device E8:C9:13:E0:E6:DA Buds3 Pro de Evelyn` → `(mac, nombre)`.
fn macs(texto: &str) -> Vec<(String, String)> {
    texto
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let resto = l.strip_prefix("Device ")?;
            let (mac, nombre) = resto.split_once(' ')?;
            Some((mac.to_string(), nombre.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La salida real de esta máquina: unos auriculares emparejados y sin
    /// conectar.
    #[test]
    fn interpreta_la_salida_de_bluetoothctl() {
        let salida = "Controller 40:C7:3C:E2:53:D2 (public)\n\tPowered: yes\n---\n---\n\
                      Device E8:C9:13:E0:E6:DA Buds3 Pro de Evelyn\n";
        let d = interpretar(salida);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].0, "Buds3 Pro de Evelyn");
        assert!(!d[0].1, "no está conectado, solo emparejado");
        assert_eq!(d[0].2, "E8:C9:13:E0:E6:DA");
    }

    /// Lo conectado va arriba.
    #[test]
    fn lo_puesto_sale_primero() {
        let salida = "Powered: yes\n---\nDevice BB:BB Teclado\n---\n\
                      Device AA:AA Ratón\nDevice BB:BB Teclado\n";
        let d = interpretar(salida);
        assert_eq!(d[0].0, "Teclado");
        assert!(d[0].1);
        assert_eq!(d[1].0, "Ratón");
    }
}
