//! Escrituras de brillo serializadas fuera del hilo de renderizado.
use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

type Cola = (Mutex<BTreeMap<(String, String), u32>>, Condvar);

/// Conserva el último valor solicitado por dispositivo. Un arrastre no crea
/// procesos en paralelo ni permite que una escritura antigua termine después.
pub fn solicitar(subsistema: &str, dispositivo: &str, nivel: u32) {
    if !matches!(subsistema, "backlight" | "leds") || dispositivo.contains('/') {
        return;
    }
    static COLA: OnceLock<Arc<Cola>> = OnceLock::new();
    let cola = COLA.get_or_init(|| {
        let cola: Arc<Cola> = Arc::new((Mutex::new(BTreeMap::new()), Condvar::new()));
        let worker = cola.clone();
        std::thread::Builder::new()
            .name("bookos-brillo".into())
            .spawn(move || {
                loop {
                    let lote = {
                        let mut pendientes = worker.0.lock().unwrap();
                        while pendientes.is_empty() {
                            pendientes = worker.1.wait(pendientes).unwrap();
                        }
                        std::mem::take(&mut *pendientes)
                    };
                    for ((subsistema, dispositivo), nivel) in lote {
                        let result = std::process::Command::new("busctl")
                            .args([
                                "--timeout=2s",
                                "call",
                                "org.freedesktop.login1",
                                "/org/freedesktop/login1/session/auto",
                                "org.freedesktop.login1.Session",
                                "SetBrightness",
                                "ssu",
                                &subsistema,
                                &dispositivo,
                                &nivel.to_string(),
                            ])
                            .output();
                        match result {
                            Ok(out) if out.status.success() => {}
                            Ok(out) => eprintln!(
                                "No se pudo cambiar el brillo de {dispositivo}: {}",
                                String::from_utf8_lossy(&out.stderr).trim()
                            ),
                            Err(e) => {
                                eprintln!("No se pudo cambiar el brillo de {dispositivo}: {e}")
                            }
                        }
                    }
                }
            })
            .expect("hilo de retroiluminación");
        cola
    });
    cola.0
        .lock()
        .unwrap()
        .insert((subsistema.into(), dispositivo.into()), nivel);
    cola.1.notify_one();
}
