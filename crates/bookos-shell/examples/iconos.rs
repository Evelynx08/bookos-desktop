fn main(){ for n in ["utilities-terminal","system-file-manager","firefox","systemsettings","accessories-text-editor"] {
  println!("{:24} {:?}", n, bookos_shell::icon_debug(n)); } }
