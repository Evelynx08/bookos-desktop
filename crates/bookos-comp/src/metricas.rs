//! Qué cuesta cada fotograma, en qué se va y cuántos se pierden.
//!
//! Antes de esto lo único que había era una línea de traza cada cinco segundos
//! con los fotogramas enviados y una media, agregada de **todas** las salidas y
//! sin decir en qué se iba el tiempo. Con eso no se puede decidir si el tirón
//! al abrir una actividad es la escena (CPU, iced y tiny-skia), el dibujo (GPU)
//! o que el compositor no llega al vblank: hay que poder verlo separado y por
//! monitor, porque con dos pantallas a distinto refresco el agregado no
//! significa nada.
//!
//! ## Qué se cuenta y qué no
//!
//! - `presentados` se cuenta **en el vblank**, no al encolar: es lo que de
//!   verdad llegó a la pantalla.
//! - `saltados` es damage evaluado y vacío. En reposo esto es lo normal y lo
//!   bueno; no es un fallo.
//! - `aplazados` es damage que llegó con un fotograma en el aire. Un goteo es
//!   normal —el cliente commiteó a mitad de vblank—; muchos por segundo son la
//!   señal de que el compositor va por detrás del monitor.
//! - `perdidos` son fotogramas cuyo coste superó el periodo del modo. Esto no
//!   es una estimación: si componer y dibujar costó más de 8,3 ms a 120 Hz, ese
//!   vblank ya pasó. No se deduce de los números de secuencia de DRM porque en
//!   reposo el hueco entre dos fotogramas es de segundos y no significa nada.
//!
//! Todo se agrega en ventanas de un segundo. Ni el ojo ni el panel de
//! diagnóstico aprovechan más resolución, y una ventana más corta obligaría a
//! repintar el panel más a menudo — el instrumento falsearía la medida.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bookos_shell::diagnostico::Datos;

/// Cuánto dura la ventana de agregación.
const VENTANA: Duration = Duration::from_secs(1);
/// Cada cuánto se vuelca el resumen a la traza.
const INFORME: Duration = Duration::from_secs(5);

#[derive(Default, Clone, Copy)]
struct Contadores {
    presentados: u64,
    saltados: u64,
    aplazados: u64,
    perdidos: u64,
    escena: Duration,
    render: Duration,
    total: Duration,
    peor: Duration,
    n: u64,
}

impl Contadores {
    fn suma(&mut self, otro: &Contadores) {
        self.presentados += otro.presentados;
        self.saltados += otro.saltados;
        self.aplazados += otro.aplazados;
        self.perdidos += otro.perdidos;
        self.escena += otro.escena;
        self.render += otro.render;
        self.total += otro.total;
        self.peor = self.peor.max(otro.peor);
        self.n += otro.n;
    }

    fn hubo_actividad(&self) -> bool {
        self.presentados > 0 || self.saltados > 0 || self.n > 0
    }
}

/// El modo con el que está encendida una salida. Se apunta al dibujar porque
/// cambia en caliente —conectar un monitor, cambiar de resolución— y el panel
/// tiene que enseñar lo que hay ahora, no lo que había al arrancar.
#[derive(Default, Clone, Copy)]
struct Modo {
    ancho: i32,
    alto: i32,
    escala: f64,
    hz: f32,
    vrr: bool,
}

impl Modo {
    /// Cuánto dura un fotograma a este refresco. Es el plazo que hay que
    /// cumplir; pasarse de aquí es perder el vblank.
    fn periodo(&self) -> Duration {
        if self.hz <= 0.0 {
            // Sin modo conocido no se acusa a nadie de llegar tarde.
            return Duration::MAX;
        }
        Duration::from_secs_f32(1.0 / self.hz)
    }
}

struct Salida {
    nombre: String,
    modo: Modo,
    ventana: Contadores,
    informe: Contadores,
}

pub struct Metricas {
    salidas: Vec<Salida>,
    desde: Instant,
    ultimo_informe: Instant,
    cpu: Cpu,
    gpu: Option<PathBuf>,
    /// La última ventana cerrada, ya en la forma que dibuja el shell.
    datos: Datos,
}

impl Default for Metricas {
    fn default() -> Self {
        Self::new()
    }
}

impl Metricas {
    pub fn new() -> Self {
        let ahora = Instant::now();
        Self {
            salidas: Vec::new(),
            desde: ahora,
            ultimo_informe: ahora,
            cpu: Cpu::new(),
            gpu: buscar_gpu_busy(),
            datos: Datos::default(),
        }
    }

    fn salida(&mut self, nombre: &str) -> &mut Salida {
        // Lineal a propósito: son como mucho cuatro monitores y esto se llama
        // una vez por fotograma. Un mapa costaría más que el recorrido.
        if let Some(i) = self.salidas.iter().position(|s| s.nombre == nombre) {
            return &mut self.salidas[i];
        }
        self.salidas.push(Salida {
            nombre: nombre.to_string(),
            modo: Modo::default(),
            ventana: Contadores::default(),
            informe: Contadores::default(),
        });
        self.salidas.last_mut().expect("acabo de insertarla")
    }

    pub fn modo(&mut self, nombre: &str, ancho: i32, alto: i32, escala: f64, hz: f32, vrr: bool) {
        self.salida(nombre).modo = Modo {
            ancho,
            alto,
            escala,
            hz,
            vrr,
        };
    }

    /// Se evaluó el damage y no había nada que cambiar.
    pub fn saltado(&mut self, nombre: &str) {
        self.salida(nombre).ventana.saltados += 1;
    }

    /// Había trabajo y la salida tenía un fotograma en el aire.
    pub fn aplazado(&mut self, nombre: &str) {
        self.salida(nombre).ventana.aplazados += 1;
    }

    /// Un fotograma compuesto y encolado, con lo que costó cada parte.
    pub fn dibujado(&mut self, nombre: &str, escena: Duration, render: Duration) {
        let salida = self.salida(nombre);
        let total = escena + render;
        let plazo = salida.modo.periodo();
        let c = &mut salida.ventana;
        c.escena += escena;
        c.render += render;
        c.total += total;
        c.peor = c.peor.max(total);
        c.n += 1;
        if total > plazo {
            c.perdidos += 1;
        }
    }

    /// El hardware confirma que enseñó un fotograma.
    pub fn presentado(&mut self, nombre: &str) {
        self.salida(nombre).ventana.presentados += 1;
    }

    /// Descarta las salidas que ya no están. Al desconectar un monitor sus
    /// números dejan de tener sentido, y dejarlos en el panel confunde más que
    /// ayuda: parecería una pantalla congelada a 0 fps.
    pub fn retener(&mut self, vivas: &[String]) {
        self.salidas.retain(|s| vivas.contains(&s.nombre));
    }

    /// ¿Ha vencido ya la ventana? Se pregunta antes de cerrarla para no armar
    /// la lista de salidas vivas en cada vuelta del bucle.
    pub fn toca_cerrar(&self) -> bool {
        self.desde.elapsed() >= VENTANA
    }

    pub fn datos(&self) -> &Datos {
        &self.datos
    }

    /// Cierra la ventana si ya toca. Devuelve `true` cuando hay medida nueva.
    ///
    /// Se llama al final de cada vuelta del bucle, así que en reposo puede no
    /// llamarse en varios segundos: la ventana se divide por el tiempo real
    /// transcurrido, no por su duración nominal, o un escritorio quieto
    /// enseñaría un ritmo inventado al despertarse.
    pub fn cerrar_ventana(&mut self) -> bool {
        let ahora = Instant::now();
        let transcurrido = ahora.duration_since(self.desde);
        if transcurrido < VENTANA {
            return false;
        }
        let secs = transcurrido.as_secs_f32().max(f32::EPSILON);
        self.datos = Datos {
            cpu: self.cpu.uso(ahora),
            gpu: self.gpu.as_ref().and_then(leer_porcentaje),
            salidas: self
                .salidas
                .iter()
                .map(|s| {
                    let c = &s.ventana;
                    let n = c.n.max(1) as f32;
                    bookos_shell::diagnostico::Salida {
                        nombre: s.nombre.clone(),
                        ancho: s.modo.ancho,
                        alto: s.modo.alto,
                        escala: s.modo.escala,
                        hz: s.modo.hz,
                        vrr: s.modo.vrr,
                        fps: c.presentados as f32 / secs,
                        ms_total: ms(c.total) / n,
                        ms_escena: ms(c.escena) / n,
                        ms_render: ms(c.render) / n,
                        ms_peor: ms(c.peor),
                        saltados: c.saltados as u32,
                        aplazados: c.aplazados as u32,
                        perdidos: c.perdidos as u32,
                    }
                })
                .collect(),
        };
        for salida in &mut self.salidas {
            let ventana = salida.ventana;
            salida.informe.suma(&ventana);
            salida.ventana = Contadores::default();
        }
        self.desde = ahora;
        self.informar(ahora);
        true
    }

    /// Vuelca a la traza como mucho una vez cada cinco segundos, y solo si hubo
    /// actividad: un escritorio en reposo no debe escribir nada en el registro.
    fn informar(&mut self, ahora: Instant) {
        let transcurrido = ahora.duration_since(self.ultimo_informe);
        if transcurrido < INFORME {
            return;
        }
        let secs = transcurrido.as_secs_f64().max(f64::EPSILON);
        for salida in &mut self.salidas {
            let c = salida.informe;
            if c.hubo_actividad() {
                let n = c.n.max(1) as f32;
                tracing::info!(
                    salida = %salida.nombre,
                    fps = format_args!("{:.1}", c.presentados as f64 / secs),
                    media_ms = format_args!("{:.2}", ms(c.total) / n),
                    escena_ms = format_args!("{:.2}", ms(c.escena) / n),
                    render_ms = format_args!("{:.2}", ms(c.render) / n),
                    peor_ms = format_args!("{:.2}", ms(c.peor)),
                    saltados = c.saltados,
                    aplazados = c.aplazados,
                    perdidos = c.perdidos,
                    "ritmo de dibujo"
                );
            }
            salida.informe = Contadores::default();
        }
        self.ultimo_informe = ahora;
    }
}

fn ms(d: Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}

/// Cuánta CPU consume **este proceso**.
///
/// Se mide el compositor y no la máquina a propósito: lo que hace falta saber
/// al mirar el panel es cuánto se puede achacar a este código. El resto de la
/// carga del sistema no la va a arreglar el compositor.
struct Cpu {
    ticks: u64,
    desde: Instant,
}

impl Cpu {
    fn new() -> Self {
        Self {
            ticks: ticks_propios().unwrap_or(0),
            desde: Instant::now(),
        }
    }

    /// Porcentaje de **un núcleo**: 100 % es un hilo saturado, y con varios
    /// hilos puede pasar de 100. Es la misma escala que enseña `top`.
    fn uso(&mut self, ahora: Instant) -> f32 {
        let Some(ticks) = ticks_propios() else {
            return 0.0;
        };
        let secs = ahora.duration_since(self.desde).as_secs_f32();
        let usados = ticks.saturating_sub(self.ticks);
        self.ticks = ticks;
        self.desde = ahora;
        if secs <= 0.0 {
            return 0.0;
        }
        // USER_HZ es 100 en Linux para lo que sale por /proc, independientemente
        // de CONFIG_HZ: el kernel convierte antes de exportarlo. Por eso no hace
        // falta `sysconf(_SC_CLK_TCK)` ni, con él, una dependencia de libc.
        const USER_HZ: f32 = 100.0;
        usados as f32 / USER_HZ / secs * 100.0
    }
}

/// `utime + stime` de /proc/self/stat, en ticks.
fn ticks_propios() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // El segundo campo es el nombre del ejecutable entre paréntesis y puede
    // llevar espacios dentro, así que no se puede partir por espacios desde el
    // principio: se corta después del último ')' y se cuenta desde ahí.
    let resto = &stat[stat.rfind(')')? + 1..];
    let mut campos = resto.split_whitespace();
    // Tras el ')' viene `state`, que es el campo 3: utime es el 14 y stime el
    // 15, o sea el 11.º y el 12.º de lo que queda.
    let utime: u64 = campos.nth(10)?.parse().ok()?;
    let stime: u64 = campos.next()?.parse().ok()?;
    Some(utime + stime)
}

/// La ocupación de la GPU solo la publican algunos drivers —`amdgpu` siempre,
/// `i915` en kernels recientes—. Si no está, el panel enseña una raya en vez de
/// un número inventado.
fn buscar_gpu_busy() -> Option<PathBuf> {
    let dir = std::fs::read_dir("/sys/class/drm").ok()?;
    dir.filter_map(|e| e.ok())
        .map(|e| e.path().join("device/gpu_busy_percent"))
        .find(|p| p.exists())
}

fn leer_porcentaje(ruta: &PathBuf) -> Option<f32> {
    std::fs::read_to_string(ruta).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un fotograma que se pasa del periodo del modo es un vblank perdido, y
    /// uno que cabe no lo es. Es la única cuenta de este módulo que decide algo.
    #[test]
    fn el_plazo_lo_marca_el_refresco() {
        let mut m = Metricas::new();
        m.modo("eDP-1", 2880, 1800, 1.75, 120.0, false);
        // 8,3 ms de plazo a 120 Hz.
        m.dibujado(
            "eDP-1",
            Duration::from_micros(3000),
            Duration::from_micros(2000),
        );
        m.dibujado(
            "eDP-1",
            Duration::from_micros(6000),
            Duration::from_micros(5000),
        );
        assert_eq!(m.salida("eDP-1").ventana.perdidos, 1);
        assert_eq!(m.salida("eDP-1").ventana.n, 2);
    }

    /// Sin modo conocido no se acusa a nadie: al arrancar todavía no hay
    /// refresco apuntado y todos los fotogramas parecerían perdidos.
    #[test]
    fn sin_modo_no_hay_fotogramas_perdidos() {
        let mut m = Metricas::new();
        m.dibujado("eDP-1", Duration::from_millis(80), Duration::ZERO);
        assert_eq!(m.salida("eDP-1").ventana.perdidos, 0);
    }

    /// El proceso siempre ha consumido alguna CPU, y la cuenta no puede salir
    /// negativa ni disparatada.
    #[test]
    fn la_cpu_del_proceso_se_lee() {
        let ticks = ticks_propios().expect("/proc/self/stat existe en Linux");
        assert!(ticks < 1_000_000, "ticks disparatados: {ticks}");
    }
}
