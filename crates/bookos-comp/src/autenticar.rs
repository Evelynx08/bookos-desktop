//! Autenticación PAM para salir del bloqueo.
//!
//! PAM se ejecuta en un worker: una contraseña incorrecta tarda segundos a
//! propósito y nunca puede bloquear el bucle de frames. `libpam.so.0` se carga
//! dinámicamente para compilar sin exigir las cabeceras de desarrollo.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::mpsc::{self, Receiver, TryRecvError};

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_CONV_ERR: c_int = 19;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type Conversar = unsafe extern "C" fn(
    c_int,
    *mut *const PamMessage,
    *mut *mut PamResponse,
    *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: Option<Conversar>,
    appdata_ptr: *mut c_void,
}

struct Secretos {
    usuario: CString,
    contrasena: CString,
}

pub struct Comprobacion {
    resultado: Receiver<bool>,
}

impl Comprobacion {
    pub fn lanzar(usuario: &str, contrasena: &str) -> Option<Self> {
        let usuario = usuario.to_string();
        let contrasena = contrasena.to_string();
        let (envio, resultado) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("bookos-pam".into())
            .spawn(move || {
                let vale = autenticar(&usuario, &contrasena);
                let _ = envio.send(vale);
            })
            .ok()?;
        Some(Self { resultado })
    }

    pub fn resultado(&mut self) -> Option<bool> {
        match self.resultado.try_recv() {
            Ok(vale) => Some(vale),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(false),
        }
    }
}

pub fn usuario() -> String {
    std::env::var("USER").unwrap_or_else(|_| "root".into())
}

fn servicio() -> &'static str {
    ["bookos", "system-auth", "common-auth", "login"]
        .into_iter()
        .find(|nombre| std::path::Path::new("/etc/pam.d").join(nombre).is_file())
        .unwrap_or("login")
}

fn autenticar(usuario: &str, contrasena: &str) -> bool {
    let Ok(usuario) = CString::new(usuario) else {
        return false;
    };
    let Ok(contrasena) = CString::new(contrasena) else {
        return false;
    };
    let Ok(servicio) = CString::new(servicio()) else {
        return false;
    };

    // SAFETY: la biblioteca permanece viva hasta después de `pam_end`; las
    // firmas son las de la ABI estable de PAM.
    unsafe {
        let Ok(pam) = libloading::Library::new("libpam.so.0") else {
            tracing::error!("no se pudo cargar libpam.so.0");
            return false;
        };
        type Start = unsafe extern "C" fn(
            *const c_char,
            *const c_char,
            *const PamConv,
            *mut *mut c_void,
        ) -> c_int;
        type Authenticate = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
        type Account = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
        type End = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
        let Ok(start) = pam.get::<Start>(b"pam_start\0") else {
            return false;
        };
        let Ok(authenticate) = pam.get::<Authenticate>(b"pam_authenticate\0") else {
            return false;
        };
        let Ok(account) = pam.get::<Account>(b"pam_acct_mgmt\0") else {
            return false;
        };
        let Ok(end) = pam.get::<End>(b"pam_end\0") else {
            return false;
        };

        let mut secretos = Secretos {
            usuario,
            contrasena,
        };
        let conversacion = PamConv {
            conv: Some(conversar),
            appdata_ptr: (&mut secretos as *mut Secretos).cast(),
        };
        let mut handle: *mut c_void = std::ptr::null_mut();
        let inicio = start(
            servicio.as_ptr(),
            secretos.usuario.as_ptr(),
            &conversacion,
            &mut handle,
        );
        if inicio != PAM_SUCCESS || handle.is_null() {
            return false;
        }
        let autenticacion = authenticate(handle, 0);
        let resultado = if autenticacion == PAM_SUCCESS {
            account(handle, 0)
        } else {
            autenticacion
        };
        let _ = end(handle, resultado);
        resultado == PAM_SUCCESS
    }
}

unsafe extern "C" fn conversar(
    cuantos: c_int,
    mensajes: *mut *const PamMessage,
    respuestas: *mut *mut PamResponse,
    datos: *mut c_void,
) -> c_int {
    if cuantos <= 0 || mensajes.is_null() || respuestas.is_null() || datos.is_null() {
        return PAM_CONV_ERR;
    }
    let secretos = unsafe { &*(datos.cast::<Secretos>()) };
    let bloque = unsafe {
        libc::calloc(cuantos as usize, std::mem::size_of::<PamResponse>()).cast::<PamResponse>()
    };
    if bloque.is_null() {
        return PAM_CONV_ERR;
    }

    for i in 0..cuantos as usize {
        let mensaje = unsafe { *mensajes.add(i) };
        if mensaje.is_null() {
            liberar_respuestas(bloque, i);
            return PAM_CONV_ERR;
        }
        let estilo = unsafe { (*mensaje).msg_style };
        let texto = match estilo {
            PAM_PROMPT_ECHO_OFF => Some(secretos.contrasena.as_c_str()),
            PAM_PROMPT_ECHO_ON => Some(secretos.usuario.as_c_str()),
            PAM_ERROR_MSG | PAM_TEXT_INFO => None,
            _ => {
                liberar_respuestas(bloque, i);
                return PAM_CONV_ERR;
            }
        };
        if let Some(texto) = texto {
            let copia = copiar_cadena(texto);
            if copia.is_null() {
                liberar_respuestas(bloque, i);
                return PAM_CONV_ERR;
            }
            unsafe { (*bloque.add(i)).resp = copia };
        }
    }
    unsafe { *respuestas = bloque };
    PAM_SUCCESS
}

fn copiar_cadena(texto: &CStr) -> *mut c_char {
    let bytes = texto.to_bytes_with_nul();
    let destino = unsafe { libc::malloc(bytes.len()).cast::<c_char>() };
    if !destino.is_null() {
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr().cast(), destino, bytes.len()) };
    }
    destino
}

fn liberar_respuestas(respuestas: *mut PamResponse, cuantos: usize) {
    for i in 0..cuantos {
        unsafe { libc::free((*respuestas.add(i)).resp.cast()) };
    }
    unsafe { libc::free(respuestas.cast()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_contrasena_falsa_no_pasa() {
        let mut c = Comprobacion::lanzar(&usuario(), "no-es-esta-contrasena-12345")
            .expect("se puede crear el worker PAM");
        for _ in 0..100 {
            if let Some(vale) = c.resultado() {
                assert!(!vale, "una contraseña inventada ha pasado PAM");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("PAM no contestó en cinco segundos");
    }
}
