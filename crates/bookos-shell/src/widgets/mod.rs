//! Los widgets que trae BookOS de serie.
//!
//! Cada uno lee **solo lo suyo**. Antes un evento de udev de cualquier tipo
//! provocaba releer las cuatro fuentes de sysfs; ahora un cambio de brillo no
//! hace mirar el estado de la red.

mod bateria;
mod bateria_simple;
mod bluetooth;
mod control;
mod notificaciones;
mod brillo;
mod red;
mod reloj;
pub(crate) mod volumen;

use crate::widget::Widget;

/// Construye un widget por su nombre en la configuración.
///
/// Devuelve `None` para un nombre que no existe: una línea con una errata en
/// `panel.conf` deja fuera ese widget, no tumba el panel.
pub fn por_nombre(nombre: &str) -> Option<Box<dyn Widget>> {
    match nombre {
        "reloj" => Some(Box::new(reloj::Reloj::new())),
        "bateria" => Some(Box::new(bateria::Bateria::new())),
        // La misma batería con el trazo de los demás widgets, para quien
        // prefiera una fila monocroma. Ver `widgets/bateria_simple.rs`.
        "bateria-simple" => Some(Box::new(bateria_simple::BateriaSimple::new())),
        "red" => Some(Box::new(red::Red::new())),
        "brillo" => Some(Box::new(brillo::Brillo::new())),
        "volumen" => Some(Box::new(volumen::Volumen::new())),
        "bluetooth" => Some(Box::new(bluetooth::Bluetooth::new())),
        "notificaciones" => Some(Box::new(notificaciones::Notificaciones::new())),
        "control" => Some(Box::new(control::Control::new())),
        _ => None,
    }
}
