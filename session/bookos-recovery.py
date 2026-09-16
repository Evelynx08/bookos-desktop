#!/usr/bin/env python3
"""Recovery outside BookOS: private KWin display, then an installed session.

No compositor or DE is started when importing this module or running tests.
Exit 10 means retry BookOS; 0 means return to the display manager.
"""
import argparse
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time


def sessions():
    """Known session launchers, never arbitrary commands from .desktop files."""
    candidates = [
        ("Plasma", "KDE", ["startplasma-wayland"]),
        ("GNOME", "GNOME", ["gnome-session", "--session=gnome"]),
    ]
    return [(name, desktop, [shutil.which(cmd[0]), *cmd[1:]])
            for name, desktop, cmd in candidates if shutil.which(cmd[0])]


def clean_environment():
    env = os.environ.copy()
    for key in ("WAYLAND_DISPLAY", "DISPLAY", "WAYLAND_SOCKET", "XAUTHORITY",
                "QT_QPA_PLATFORM", "GDK_BACKEND", "QT_WAYLAND_SHELL_INTEGRATION"):
        env.pop(key, None)
    for key in list(env):
        if key.startswith("BOOKOS_"):
            env.pop(key)
    return env


def palette():
    """Read Settings' runtime roles; the fixed palette is only a fallback."""
    root = Path(os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config"))) / "bookos"
    dark = True
    try:
        config = root.joinpath("panel.conf").read_text()
        mode = re.search(r"^tema\s*=\s*(\w+)", config, re.M)
        if mode:
            dark = mode[1] != "claro"
            if mode[1] == "automatico":
                def hour(key, default):
                    match = re.search(rf"^{key}\s*=\s*(\d\d):(\d\d)", config, re.M)
                    return int(match[1]) * 60 + int(match[2]) if match else default
                now = time.localtime()
                start, end = hour("tema_claro_desde", 420), hour("tema_oscuro_desde", 1200)
                current = now.tm_hour * 60 + now.tm_min
                light = start <= current < end if start <= end else current >= start or current < end
                dark = not light
    except OSError:
        pass
    roles = dict(bg="#000000", card="#1c1c1e", tx="#ffffff", tx2="#8e8e93",
                 blue="#0a84ff", hov="rgba(255,255,255,0.06)", **{"on-accent": "#ffffff"}) if dark else dict(
        bg="#f2f2f7", card="#ffffff", tx="#000000", tx2="#8e8e93",
        blue="#007aff", hov="rgba(0,0,0,0.04)", **{"on-accent": "#ffffff"})
    try:
        # The media fallback duplicates dark roles for browsers. Qt already
        # has an explicit mode, so only use the two explicit root rules.
        css = root.joinpath("palette.css").read_text().split("@media", 1)[0]
        for selector, body in re.findall(r"([^{}]+)\{([^{}]*)\}", css):
            if "dark" in selector.lower() and not dark:
                continue
            for name, value in re.findall(r"--([\w-]+)\s*:\s*([^;]+);", body):
                if name in roles:
                    roles[name] = value.strip()
    except OSError:
        pass
    return roles


def window(args):
    from PySide6.QtCore import Qt, QTimer
    from PySide6.QtWidgets import QApplication, QWidget, QFrame, QVBoxLayout, QLabel, QPushButton
    app = QApplication([])
    p = palette()
    screen = QWidget()
    screen.setWindowTitle("Recuperación de BookOS")
    screen.setObjectName("screen")
    screen.setStyleSheet(f"""
        QWidget {{ font-family: 'Noto Sans'; color: {p['tx']}; font-size: 13px; }}
        QWidget#screen {{ background: {p['bg']}; }}
        QFrame#card {{ background: {p['card']}; border-radius: 26px; }}
        QLabel {{ background: transparent; }}
        QLabel#title {{ font-size: 17px; font-weight: 700; }}
        QLabel#body {{ color: {p['tx2']}; }}
        QPushButton {{ background: {p['hov']}; border: none; border-radius: 14px;
                       padding: 0 18px; min-height: 48px; font-size: 15px; font-weight: 600; }}
        QPushButton:focus {{ border: 2px solid {p['blue']}; }}
        QPushButton#primary {{ background: {p['blue']}; color: {p['on-accent']}; }}
    """)
    layout = QVBoxLayout(screen)
    layout.setAlignment(Qt.AlignCenter)
    card = QFrame()
    card.setObjectName("card")
    card.setFixedWidth(420)
    contents = QVBoxLayout(card)
    contents.setContentsMargins(24, 28, 24, 20)
    contents.setSpacing(10)
    title = QLabel("BookOS se ha cerrado inesperadamente")
    title.setObjectName("title")
    title.setWordWrap(True)
    title.setAlignment(Qt.AlignCenter)
    title.setMinimumHeight(title.heightForWidth(372))
    contents.addWidget(title)
    message = "Puedes reintentar o continuar en otro escritorio. Las ventanas de la sesión anterior no se restaurarán."
    if args.locked:
        message = "La sesión estaba bloqueada. Vuelve al inicio de sesión para identificarte y elegir BookOS u otro escritorio."
    body = QLabel(message + f"\n\nCódigo de salida: {args.status}\nRegistro: {args.log}")
    body.setTextFormat(Qt.PlainText)
    body.setObjectName("body")
    body.setWordWrap(True)
    body.setAlignment(Qt.AlignCenter)
    body.setMinimumHeight(body.heightForWidth(372))
    contents.addWidget(body)

    def choose(value):
        Path(args.choice).write_text(value)
        app.quit()

    choices = [] if args.locked else [("Reintentar BookOS", "retry")]
    if not args.locked:
        choices += [(f"Continuar en {name}", name) for name, _, _ in sessions()]
    choices += [("Volver al inicio de sesión", "login")]
    for index, (label, value) in enumerate(choices):
        button = QPushButton(label)
        if index == 0:
            button.setObjectName("primary")
        button.clicked.connect(lambda checked=False, v=value: choose(v))
        contents.addWidget(button)
    layout.addWidget(card)
    screen.showFullScreen()
    if args.preview:
        QTimer.singleShot(300, lambda: (screen.grab().save(args.preview), app.quit()))
    if args.ready:
        QTimer.singleShot(0, lambda: Path(args.ready).touch())
    return app.exec()


def prompt(args):
    """A private compositor owns DRM only while the recovery window is open."""
    if shutil.which("kwin_wayland"):
        with tempfile.TemporaryDirectory(prefix="bookos-recovery-", dir=os.environ.get("XDG_RUNTIME_DIR")) as directory:
            choice, ready = Path(directory) / "choice", Path(directory) / "ready"
            cmd = [sys.executable, str(Path(__file__).resolve()), "--ui", "--choice", str(choice),
                   "--ready", str(ready), "--status", str(args.status), "--log", args.log]
            if args.locked:
                cmd.append("--locked")
            env = clean_environment()
            env.update(QT_QPA_PLATFORM="wayland", XDG_CURRENT_DESKTOP="BookOS-Recovery")
            # shlex.join is for KWin's documented command-string argument.
            proc = subprocess.Popen(["kwin_wayland", "--drm", "--no-lockscreen", "--no-global-shortcuts",
                                     "--no-kactivities", "--socket", f"bookos-recovery-{os.getpid()}",
                                     "--exit-with-session", shlex.join(cmd)], env=env)
            deadline = time.monotonic() + 20
            while proc.poll() is None and not ready.exists() and time.monotonic() < deadline:
                time.sleep(0.1)
            if ready.exists():
                proc.wait()
            elif proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
            if choice.exists():
                return choice.read_text().strip()
    # No usable graphics: still report the failure on the controlling TTY.
    try:
        with open("/dev/tty", "r+") as tty:
            tty.write(f"\nBookOS se ha cerrado (código {args.status}). Registro: {args.log}\n")
            choices = [] if args.locked else [("Reintentar BookOS", "retry")]
            if not args.locked:
                choices += [(name, name) for name, _, _ in sessions()]
            choices.append(("Volver al inicio de sesión", "login"))
            for i, (label, _) in enumerate(choices, 1):
                tty.write(f"{i}. {label}\n")
            tty.write("Elige una opción: ")
            tty.flush()
            index = int(tty.readline().strip()) - 1
            return choices[index][1] if 0 <= index < len(choices) else "login"
    except (OSError, ValueError):
        return "login"


def recover(args):
    choice = prompt(args)
    if choice == "retry" and not args.locked:
        return 10
    if not args.locked:
        for name, desktop, cmd in sessions():
            if name != choice:
                continue
            env = clean_environment()
            env.update(XDG_CURRENT_DESKTOP=desktop, XDG_SESSION_DESKTOP=desktop.lower(), XDG_SESSION_TYPE="wayland")
            # The selected session imports its new display into D-Bus/systemd.
            result = subprocess.call(cmd, env=env)
            if result:
                print(f"No se pudo continuar en {name} (código {result}); se vuelve al inicio de sesión.", file=sys.stderr)
            return 0
    return 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ui", action="store_true")
    parser.add_argument("--preview")
    parser.add_argument("--choice", default="")
    parser.add_argument("--ready", default="")
    parser.add_argument("--status", type=int, default=1)
    parser.add_argument("--log", default="bookos-session.log")
    parser.add_argument("--locked", action="store_true")
    args = parser.parse_args()
    return window(args) if args.ui else recover(args)


if __name__ == "__main__":
    sys.exit(main())
