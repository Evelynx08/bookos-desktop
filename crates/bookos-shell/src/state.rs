//! Los datos que enseña el panel.
//!
//! Todo se lee de sysfs y de la hora local; sin D-Bus, sin daemons y sin
//! esperas. Que el panel no dependa de ningún servicio es parte de por qué
//! puede estar pintado en el primer frame de la sesión.
//!
//! Eso también marca **qué** puede enseñar el panel: batería, red y brillo
//! están en sysfs y entran; el volumen y lo que suena no lo están, y llegarán
//! cuando haya con quién hablar (PipeWire, MPRIS), no leyendo ficheros a
//! ciegas. Ninguna lectura de aquí bloquea, y cada una se rinde en cuanto algo
//! no está donde debería: en una máquina sin batería, sin red o sin
//! retroiluminación el estado simplemente no se dibuja.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelData {
    /// Hora local ya formateada, "HH:MM".
    pub clock: String,
    pub battery: Option<Battery>,
    pub network: Option<Network>,
    /// Brillo de la pantalla en tanto por ciento, si hay retroiluminación.
    pub brightness: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    /// Enchufado a la corriente. No es lo mismo que `charging`: con el umbral
    /// de carga alcanzado el portátil está enchufado y **no** carga.
    pub plugged: bool,
    /// Minutos que quedan — de autonomía si descarga, para llenarse si carga.
    /// `None` cuando el driver no da corriente instantánea o vale cero.
    pub minutes: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Network {
    pub kind: Link,
    /// El enlace está levantado y con portadora.
    pub up: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Wifi,
    Cable,
}

impl PanelData {
    pub fn read() -> Self {
        Self {
            clock: local_hhmm(),
            battery: Battery::read(true),
            network: Network::read(),
            brightness: read_brightness(),
        }
    }
}

impl Battery {
    /// `con_minutos` decide si además se estima el tiempo restante, que es la
    /// parte cara: exige `current_now`, y eso es una consulta al controlador
    /// embebido —0,53 ms medidos— frente a los 0,02 de `capacity`. Quien llama
    /// decide con qué cadencia la quiere; ver `widgets::bateria::CADA_MINUTOS`.
    pub(crate) fn read(con_minutos: bool) -> Option<Self> {
        let path = battery_path()?;
        let percent = read_num(path.join("capacity"))?.min(100) as u8;
        let status = fs::read_to_string(path.join("status")).ok()?;
        let status = status.trim();
        // "Not charging" es lo que dice el driver cuando está enchufado pero
        // parado en el umbral de carga: no está cargando, pero tampoco
        // gastando batería.
        let charging = status == "Charging";
        Some(Self {
            percent,
            charging,
            // El estado de la batería ya distingue los tres casos, así que no
            // hace falta ir a mirar el adaptador: solo se le pregunta a ADP*
            // cuando la batería dice algo que no sabemos interpretar.
            plugged: match status {
                "Charging" | "Not charging" | "Full" => true,
                "Discharging" => false,
                _ => ac_online().unwrap_or(false),
            },
            minutes: con_minutos
                .then(|| remaining_minutes(&path, charging))
                .flatten(),
        })
    }
}

/// Minutos restantes a partir de la carga y la corriente instantáneas.
///
/// El kernel expone la batería en carga (`charge_*`, µAh) o en energía
/// (`energy_*`, µWh) según el driver; las dos parejas dividen igual, así que
/// basta con encontrar la que exista. La cuenta es la ingenua —carga entre
/// corriente— y por tanto salta al vuelo con cada pico de consumo: sirve para
/// "me queda tarde o me queda un rato", no como promesa.
fn remaining_minutes(path: &Path, charging: bool) -> Option<u32> {
    // El `exists()` se repite en cada lectura a propósito: recordarlo en un
    // global ataría la respuesta a la primera batería que se mirase, y lo que
    // cuesta aquí es `current_now` —0,53 ms medidos, una consulta al
    // controlador embebido— no este `stat`, que son 0,02.
    let (ahora, tope, flujo) = if path.join("charge_now").exists() {
        ("charge_now", "charge_full", "current_now")
    } else {
        ("energy_now", "energy_full", "power_now")
    };

    let ahora = read_num(path.join(ahora))? as u64;
    let flujo = read_num(path.join(flujo))? as u64;
    if flujo == 0 {
        return None;
    }
    let restante = if charging {
        (read_num(path.join(tope))? as u64).saturating_sub(ahora)
    } else {
        ahora
    };
    Some((restante * 60 / flujo) as u32)
}

fn ac_online() -> Option<bool> {
    let dir = fs::read_dir("/sys/class/power_supply").ok()?;
    for entrada in dir.filter_map(|e| e.ok()) {
        let path = entrada.path();
        if fs::read_to_string(path.join("type")).is_ok_and(|t| t.trim() == "Mains") {
            return Some(read_num(path.join("online")) == Some(1));
        }
    }
    None
}

impl Network {
    /// Primera interfaz de verdad que esté levantada; si no hay ninguna, la
    /// primera que exista, para poder enseñar "sin red" en vez de nada.
    ///
    /// `lo` se descarta siempre: el bucle local está arriba hasta en una
    /// máquina desconectada, así que contarlo haría que el panel jurase que
    /// hay red cuando no la hay.
    pub(crate) fn read() -> Option<Self> {
        let mut interfaces: Vec<PathBuf> = fs::read_dir("/sys/class/net")
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()) != Some("lo"))
            .collect();
        interfaces.sort();

        let mut caida = None;
        for path in interfaces {
            let kind = if path.join("wireless").is_dir() {
                Link::Wifi
            } else {
                Link::Cable
            };
            // `operstate` es lo que dice el driver; `carrier` es si hay cable o
            // asociación. Se piden los dos porque una interfaz puede estar
            // "up" administrativamente con el cable fuera.
            let up = fs::read_to_string(path.join("operstate")).is_ok_and(|s| s.trim() == "up")
                && read_num(path.join("carrier")) == Some(1);
            if up {
                return Some(Self { kind, up });
            }
            caida.get_or_insert(Self { kind, up: false });
        }
        caida
    }
}

/// Dispositivo válido de retroiluminación. Se prefiere el controlador nativo
/// a las interfaces de compatibilidad del firmware.
pub(crate) fn backlight_device() -> Option<String> {
    backlight_path()?
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
}

/// El valor crudo máximo del dispositivo. En este panel es 400: mandarle un
/// tanto por ciento directamente lo dejaría al 25 % de lo pedido.
pub(crate) fn backlight_max(dispositivo: &str) -> Option<u32> {
    read_num(
        PathBuf::from("/sys/class/backlight")
            .join(dispositivo)
            .join("max_brightness"),
    )
    .map(|v| v as u32)
}

/// Recuerda el resultado de buscar un dispositivo en sysfs.
///
/// Se usa para la batería. La retroiluminación se redetecta por separado para
/// tolerar retiradas y cambios de controlador durante la sesión.
///
/// **El fallo no se cachea.** Si no hay nada, se vuelve a mirar la próxima vez:
/// un módulo puede cargarse después de arrancar la sesión, y recordar «aquí no
/// hay batería» dejaría el panel sin ella hasta reiniciar. Buscar y no
/// encontrar es justo el caso barato —un `readdir` de un directorio vacío—, así
/// que reintentarlo no cuesta.
fn recordar(
    cache: &std::sync::OnceLock<PathBuf>,
    buscar: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(ruta) = cache.get() {
        return Some(ruta.clone());
    }
    let encontrada = buscar()?;
    // Si dos hilos llegan a la vez gana el primero; los dos devuelven una ruta
    // válida, que es lo único que importa.
    let _ = cache.set(encontrada.clone());
    Some(encontrada)
}

fn backlight_path() -> Option<PathBuf> {
    elegir_luz(Path::new("/sys/class/backlight"), false)
}

// No se conserva una ruta entre refrescos: el controlador puede reaparecer
// tras suspender o cargarse después del arranque.
fn elegir_luz(root: &Path, teclado: bool) -> Option<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(root).ok()?.filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| !teclado || p.file_name().is_some_and(|n| n.to_string_lossy().contains("kbd_backlight")))
        .filter(|p| read_num(p.join("max_brightness")).is_some_and(|n| n > 0)
            && read_num(p.join("brightness")).is_some())
        .collect();
    paths.sort_by_key(|p| {
        let kind = fs::read_to_string(p.join("type")).unwrap_or_default();
        (match kind.trim() { "raw" => 0, "platform" => 1, _ => 2 }, p.clone())
    });
    paths.into_iter().next()
}

pub(crate) fn teclado_actual() -> Option<(String, u32, u32)> {
    let path = elegir_luz(Path::new("/sys/class/leds"), true)?;
    let max = read_num(path.join("max_brightness"))?;
    Some((path.file_name()?.to_str()?.into(), read_num(path.join("brightness"))?.min(max), max))
}

pub(crate) fn read_brightness() -> Option<u8> {
    let path = backlight_path()?;
    let max = read_num(path.join("max_brightness"))?;
    if max == 0 {
        return None;
    }
    // `brightness` y no `actual_brightness`: **medido en esta máquina, leer
    // `actual_brightness` cuesta 0,512 ms y `brightness` 0,030**, diecisiete
    // veces menos. La diferencia es que el primero le pregunta al hardware por
    // el valor efectivo y el segundo devuelve el que se pidió, y en un panel de
    // portátil son el mismo número salvo durante la rampa de una transición.
    // Ese medio milisegundo se pagaba en **cada** evento de udev, que llegan en
    // ráfaga al mover el brillo con la tecla.
    let ahora = read_num(path.join("brightness"))?;
    Some((u64::from(ahora.min(max)) * 100 / u64::from(max)) as u8)
}

fn battery_path() -> Option<PathBuf> {
    static CACHE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    recordar(&CACHE, || {
        let mut candidates: Vec<PathBuf> = fs::read_dir("/sys/class/power_supply")
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("BAT"))
            })
            .collect();
        candidates.sort();
        candidates.into_iter().next()
    })
}

fn read_num(path: impl AsRef<Path>) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Hora local en "HH:MM", sin dependencias.
///
/// El desfase respecto a UTC se saca de `localtime(3)` a través de la única vía
/// que no arrastra una librería de zonas horarias entera: preguntarle a la libc
/// del sistema. `localtime_r` es reentrante, así que no pisa el estado global.
pub(crate) fn local_hhmm() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let (h, m) = local_hm(now);
    format!("{h:02}:{m:02}")
}

/// Una fecha del calendario local.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Fecha {
    pub anio: i32,
    /// 1..=12.
    pub mes: u32,
    /// 1..=31.
    pub dia: u32,
}

impl Fecha {
    /// Hoy, según la zona horaria del sistema.
    pub fn hoy() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let tm = local_tm(secs);
        Self {
            // `tm_year` cuenta desde 1900 y `tm_mon` desde 0. Es la fuente
            // clásica de errores de "un mes de menos" con `struct tm`.
            anio: tm.2 + 1900,
            mes: tm.3 as u32 + 1,
            dia: tm.4 as u32,
        }
    }

    /// Cuántos días tiene un mes, con el año bisiesto del calendario gregoriano.
    pub fn dias_del_mes(anio: i32, mes: u32) -> u32 {
        match mes {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if bisiesto(anio) => 29,
            2 => 28,
            _ => 30,
        }
    }

    /// Qué día de la semana cae, con **0 = lunes**.
    ///
    /// Es la convención del calendario de BookOS (la cabecera empieza en Lun) y
    /// no la de `tm_wday`, que empieza en domingo. Se calcula con la fórmula de
    /// Sakamoto en vez de preguntarle a la libc: así una fecha cualquiera del
    /// mes que se está mirando no necesita convertirse a tiempo Unix, que con
    /// los cambios de hora es donde aparecen los días que bailan.
    pub fn dia_semana(&self) -> u32 {
        const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let mut y = self.anio;
        if self.mes < 3 {
            y -= 1;
        }
        let dom0 = (y + y / 4 - y / 100 + y / 400 + T[(self.mes - 1) as usize] + self.dia as i32)
            .rem_euclid(7);
        // Sakamoto da 0 = domingo; aquí la semana empieza en lunes.
        (dom0 as u32 + 6) % 7
    }
}

fn bisiesto(anio: i32) -> bool {
    (anio % 4 == 0 && anio % 100 != 0) || anio % 400 == 0
}

#[cfg(unix)]
pub(crate) fn local_hm(unix_secs: i64) -> (u8, u8) {
    let tm = local_tm(unix_secs);
    (tm.0, tm.1)
}

/// La hora local de ahora mismo, en `(hora, minuto)`.
///
/// La usa el tema automático para saber si toca claro u oscuro. Va aquí, con
/// el resto del calendario, porque es donde vive la única llamada a
/// `localtime_r` del proyecto y no tiene sentido tener dos.
pub fn hora_local_ahora() -> (u8, u8) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    local_hm(secs)
}

/// `(hora, minuto, años desde 1900, mes desde 0, día del mes)` en local.
#[cfg(unix)]
fn local_tm(unix_secs: i64) -> (u8, u8, i32, i32, i32) {
    // Declaración mínima de libc en vez de la dependencia entera: solo se
    // necesita `localtime_r` y los campos de `tm` que usa el reloj.
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: i64,
        tm_zone: *const i8,
    }
    unsafe extern "C" {
        fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
    }

    let mut tm = Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: `localtime_r` escribe en el `tm` que le damos y no guarda el
    // puntero. Si falla devuelve null y nos quedamos con el tm a cero.
    let ok = unsafe { !localtime_r(&unix_secs, &mut tm).is_null() };
    if ok {
        (
            tm.tm_hour as u8,
            tm.tm_min as u8,
            tm.tm_year,
            tm.tm_mon,
            tm.tm_mday,
        )
    } else {
        // Un tm a cero sería el año 1900 y el día 0, que no existe. Mejor una
        // fecha válida aunque sea falsa: el calendario se dibuja igual.
        (0, 0, 70, 0, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luz_descarta_controladores_invalidos_y_redetecta_tras_retirada() {
        let root = bateria_falsa("luces", &[]);
        let crear = |nombre: &str, tipo: &str, maximo: &str| {
            let p = root.join(nombre);
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("type"), tipo).unwrap();
            fs::write(p.join("max_brightness"), maximo).unwrap();
            fs::write(p.join("brightness"), "0").unwrap();
            p
        };
        crear("a_invalido", "raw", "0");
        let firmware = crear("acpi_video0", "firmware", "10");
        let raw = crear("intel_backlight", "raw", "400");
        assert_eq!(elegir_luz(&root, false), Some(raw.clone()));
        fs::remove_dir_all(raw).unwrap();
        assert_eq!(elegir_luz(&root, false), Some(firmware));
        let nuevo = crear("amdgpu_bl0", "raw", "255");
        assert_eq!(elegir_luz(&root, false), Some(nuevo));
    }

    #[test]
    fn teclado_apagado_se_detecta_y_un_led_roto_no_oculta_el_valido() {
        let root = bateria_falsa("leds", &[]);
        fs::create_dir(root.join("a::kbd_backlight")).unwrap();
        let teclado = root.join("samsung-galaxybook::kbd_backlight");
        fs::create_dir(&teclado).unwrap();
        fs::write(teclado.join("max_brightness"), "3").unwrap();
        fs::write(teclado.join("brightness"), "0").unwrap();
        assert_eq!(elegir_luz(&root, true), Some(teclado.clone()));
        fs::remove_dir_all(teclado).unwrap();
        assert_eq!(elegir_luz(&root, true), None);
    }

    /// Monta un directorio con la pinta de una batería de sysfs.
    fn bateria_falsa(nombre: &str, ficheros: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bookos-bat-{nombre}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (fichero, valor) in ficheros {
            fs::write(dir.join(fichero), valor).unwrap();
        }
        dir
    }

    #[test]
    fn descargando_divide_carga_entre_corriente() {
        // 3.075 Ah restantes a 0,523 A → 5 h 52 min.
        let dir = bateria_falsa(
            "descarga",
            &[
                ("charge_now", "3075000\n"),
                ("charge_full", "4100000\n"),
                ("current_now", "523000\n"),
            ],
        );
        assert_eq!(remaining_minutes(&dir, false), Some(352));
    }

    #[test]
    fn cargando_cuenta_lo_que_falta_para_llenarse() {
        // Faltan 1.025 Ah de 4.1; a 2 A, media hora larga.
        let dir = bateria_falsa(
            "carga",
            &[
                ("charge_now", "3075000\n"),
                ("charge_full", "4100000\n"),
                ("current_now", "2000000\n"),
            ],
        );
        assert_eq!(remaining_minutes(&dir, true), Some(30));
    }

    /// Algunos drivers dan la batería en energía (µWh) y no en carga (µAh).
    #[test]
    fn tambien_sirve_la_pareja_de_energia() {
        let dir = bateria_falsa(
            "energia",
            &[
                ("energy_now", "30000000\n"),
                ("energy_full", "50000000\n"),
                ("power_now", "10000000\n"),
            ],
        );
        assert_eq!(remaining_minutes(&dir, false), Some(180));
    }

    /// Corriente cero es lo que se lee justo al enchufar o con la carga
    /// parada en el umbral. Dividir ahí sería un panic, y el panel tiene que
    /// enseñar el porcentaje igual: por eso es `None` y no un cero.
    #[test]
    fn corriente_cero_no_da_tiempo() {
        let dir = bateria_falsa(
            "parada",
            &[
                ("charge_now", "3075000\n"),
                ("charge_full", "4100000\n"),
                ("current_now", "0\n"),
            ],
        );
        assert_eq!(remaining_minutes(&dir, false), None);
    }

    #[test]
    fn sin_ficheros_no_hay_estimacion() {
        let dir = bateria_falsa("vacia", &[]);
        assert_eq!(remaining_minutes(&dir, false), None);
    }
}
