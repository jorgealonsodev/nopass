# PRD — NoPass

**Herramienta de bandeja de sistema para conceder sudo sin contraseña al usuario de la sesión**

| Campo | Valor |
|---|---|
| Versión | 1.1 |
| Fecha | Septiembre 2026 |
| Estado | Borrador (auditado) |
| Plataforma | Linux (escritorio) |

**Cambios respecto a v1.0** (resultado de auditoría técnica, ver §13): detección de estado sin privilegios (`/etc/sudoers.d` es `0750`), temporizador robusto a suspensión y reinicio, identificación por UID en vez de nombre de usuario, verificación de que el usuario ya es sudoer, eliminación de AppImage del alcance v1, coherencia "sin GTK/Qt", métricas medibles.

---

## 1. Resumen ejecutivo

NoPass es una utilidad ligera de escritorio Linux que se ejecuta en la bandeja de estado (system tray). Con un clic sobre su icono, concede al usuario de la sesión actual permisos `sudo` sin contraseña, escribiendo en `/etc/sudoers.d/` una regla equivalente a:

```
jorge ALL=(ALL) NOPASSWD: ALL
```

El objetivo es eliminar la fricción de introducir la contraseña repetidamente en entornos de desarrollo, laboratorio o máquinas virtuales, manteniendo la operación segura (validación con `visudo`, escalado de privilegios vía polkit, solo para usuarios que ya son sudoers) y reversible (el mismo icono permite revocar la regla; la activación temporal la revoca sola).

---

## 2. Problema y motivación

- Desarrolladores y administradores ejecutan decenas de comandos `sudo` al día en máquinas de uso personal o VMs desechables.
- Editar `/etc/sudoers` manualmente con `visudo` es propenso a errores; un fallo de sintaxis puede bloquear `sudo` por completo.
- No existe una forma visual, rápida y reversible de activar/desactivar esta regla, ni de acotarla en el tiempo.

---

## 3. Objetivos

| # | Objetivo | Métrica de éxito |
|---|---|---|
| O1 | Conceder NOPASSWD al usuario actual con un solo clic | Regla activa e icono actualizado en < 1 s desde que polkit confirma la autenticación |
| O2 | Nunca corromper la configuración de sudo | 0 escrituras en `sudoers.d` sin validación previa con `visudo -cf`; escritura atómica |
| O3 | Ser reversible | La regla se elimina con un clic, sin dejar residuos (fichero, timer, estado) |
| O4 | Visibilidad del estado | El icono refleja el estado real en ≤ 1 s tras una acción propia y en ≤ 60 s tras un cambio externo |
| O5 | Acotar la exposición | La activación temporal se revierte sola, incluso tras suspensión o reinicio |

### No objetivos (fuera de alcance v1)

- Gestión de otros usuarios distintos al de la sesión.
- Edición de reglas sudo arbitrarias o granulares por comando.
- Conceder sudo a un usuario que no lo tiene: NoPass solo elimina la contraseña a quien **ya** es sudoer.
- Entornos de escritorio sin host StatusNotifierItem (GNOME sin extensión AppIndicator). Es una limitación del escritorio, no del protocolo de sesión (X11/Wayland es indiferente para SNI).
- Versión para servidores sin entorno gráfico (existirá un CLI mínimo solo como helper interno).
- Empaquetado AppImage: incompatible con el modelo polkit (requiere política en `/usr/share/polkit-1/actions/` y helper en ruta fija propiedad de root). Se reevaluará si aparece un mecanismo de instalación del helper.

---

## 4. Usuarios objetivo

- **Desarrollador en máquina personal / VM**: quiere trabajar sin interrupciones por contraseña.
- **Administrador de sistemas en laboratorio**: activa NOPASSWD temporalmente durante sesiones de mantenimiento.
- **Formador / demos**: necesita activar y desactivar el comportamiento rápidamente en equipos de aula.

---

## 5. Requisitos funcionales

### RF-01 Icono en la bandeja de estado
- La aplicación se muestra como un icono en el área de notificación (StatusNotifierItem / AppIndicator).
- Tres estados visuales claramente distinguibles:
  - **Inactivo** (candado cerrado, gris): la regla NOPASSWD no existe.
  - **Activo** (candado abierto, color de aviso): la regla NOPASSWD está en vigor de forma permanente.
  - **Activo temporal** (candado abierto con indicador de reloj): la regla está en vigor y caducará.
- Tooltip con el usuario, el estado y, si aplica, el tiempo restante: `jorge — sudo sin contraseña: ACTIVO (caduca en 42 min)`.
- Se proporcionan variantes `-symbolic` para que el panel aplique su tema.

### RF-02 Acción principal (clic izquierdo)
- Un clic alterna el estado:
  - Si está inactivo → **activa** la regla con la duración por defecto (ver RF-06; inicialmente 1 h, configurable desde el menú).
  - Si está activo (permanente o temporal) → **desactiva** la regla.
- Antes de ejecutar la acción se solicita autorización mediante polkit (`pkexec`), que pedirá la contraseña **una única vez** por operación (y no la volverá a pedir durante unos minutos gracias a `auth_admin_keep`).
- Tras completar la acción se muestra una notificación de escritorio confirmando el resultado.
- La primera activación muestra un diálogo de advertencia sobre el riesgo de NOPASSWD con la opción "No volver a mostrar" (persistida en `~/.config/nopass/config.toml`).

### RF-03 Menú contextual (clic derecho)
- `Activar sudo sin contraseña` / `Desactivar sudo sin contraseña` (según estado)
- `Activar durante…` → submenú: 15 min / 1 h / 4 h / 8 h / hasta reiniciar / permanente (ver RF-06)
- `Duración por defecto del clic` → mismo submenú, marca la opción actual
- `Ver regla actual` (abre un diálogo con el contenido del fichero, obtenido vía `status`)
- `Iniciar con la sesión` (checkbox, gestiona `~/.config/autostart/`)
- `Acerca de`
- `Salir`

### RF-04 Escritura segura de la regla
- La regla **nunca** se escribe en `/etc/sudoers`. Se usa un fichero dedicado, nombrado por UID para evitar cualquier problema de caracteres en el nombre de usuario:
  `/etc/sudoers.d/90-nopass-<uid>`
- Contenido exacto del fichero (la regla usa la sintaxis `#uid` de sudoers, que no requiere escapado; el nombre aparece solo en el comentario; `nopass-expires` admite epoch UTC, `reboot` o `never`):
  ```
  # Generated by NoPass — do not edit manually
  # nopass-user: jorge
  # nopass-expires: 1789000000
  #1000 ALL=(ALL) NOPASSWD: ALL
  ```
- El fichero se escribe primero en `/etc/sudoers.d/.90-nopass-<uid>.tmp` (creado con `O_EXCL`, `0440 root:root`, `fsync`), se valida con `visudo -cf <tmp>` y solo si la validación es correcta se renombra atómicamente a destino. Los ficheros con `.` en el nombre son ignorados por sudo, por lo que el temporal nunca tiene efecto aunque quede huérfano.
- Si la validación falla, se borra el temporal, no se modifica nada y se notifica el error al usuario.
- La cabecera del fichero es un **contrato de datos**, no texto de interfaz: `is_nopass_owned` la usa para decidir si un fichero de `/etc/sudoers.d` pertenece a NoPass, y de esa decisión dependen el borrado y el barrido de arranque. Por tanto NO se traduce nunca, sea cual sea el locale del sistema, y debe reproducirse byte a byte (guion largo U+2014, fichero UTF-8). La i18n de la aplicación no alcanza a este fichero.
- El UID objetivo se obtiene **exclusivamente** de `PKEXEC_UID`. El helper comprueba que:
  1. existe en `getpwuid`,
  2. no es `0` y es ≥ `UID_MIN` de `/etc/login.defs` (no usuarios de sistema),
  3. el usuario **ya dispone de sudo** (`sudo -l -U <usuario>` devuelve reglas, o pertenece a `sudo`/`wheel`/`admin`). Si no, el helper rechaza la operación con un error explícito.
- El helper toma un `flock` sobre `/run/nopass/lock` para serializar operaciones concurrentes.

### RF-05 Desactivación
- Elimina el fichero `/etc/sudoers.d/90-nopass-<uid>`, detiene y elimina el timer de expiración si existe, y actualiza el fichero de estado (RF-07).
- Solo elimina ficheros creados por NoPass (identificados por nombre y cabecera de comentario); nunca toca otras reglas.

### RF-06 Activación temporal
- El helper anota la caducidad en la cabecera del fichero (`nopass-expires`, epoch UTC) y crea un timer transitorio de sistema con **hora absoluta de reloj real**:
  `systemd-run --unit=nopass-expire-<uid> --on-calendar=<UTC ISO-8601> /usr/libexec/nopass-helper expire --uid <uid>`
  El reloj real no se pausa en suspensión y systemd ejecuta los timers de calendario vencidos al reanudar; un timer `--on-active` (monotónico) se pausaría durante la suspensión y alargaría la ventana de exposición.
- Si ya existe un timer para ese UID, se detiene y se sustituye.
- `hasta reiniciar`: `nopass-expires: reboot`; no se crea timer. La unidad de sistema `nopass-cleanup.service` (habilitada en la instalación, `Before=multi-user.target`) elimina en cada arranque los ficheros `90-nopass-*` marcados `reboot` o con epoch ya vencido (cubre también timers perdidos por un apagado).
- `permanente`: `nopass-expires: never`; no se crea timer.
- Duración máxima temporal: 8 h. El helper rechaza valores fuera de `[60, 28800]` s.
- El icono muestra el tiempo restante en el tooltip.
- Al expirar, la aplicación de bandeja detecta el cambio de estado (RF-07) y lanza la notificación "Sudo sin contraseña desactivado (expiró el tiempo)". La notificación la emite siempre el proceso de usuario: el helper corre como root en un contexto sin bus de sesión.

### RF-07 Detección de estado
- `/etc/sudoers.d/` es `0750 root:root`: un proceso sin privilegios **no puede** comprobar la existencia del fichero ni vigilarlo con inotify. Por tanto:
  - El helper mantiene un fichero de estado legible por todos en `/run/nopass/<uid>.state` (directorio `0755`, fichero `0644`, creado por `tmpfiles.d` y por el propio helper) con el estado, la caducidad y el nombre del fichero de regla. Se actualiza en cada `enable`, `disable`, `expire` y en el arranque por `nopass-cleanup.service`.
  - La aplicación de bandeja vigila `/run/nopass/` con inotify (reacción < 1 s a acciones propias y a la expiración).
  - Para detectar cambios **externos** (alguien borra o edita el fichero desde un terminal), la aplicación reconcilia el estado real con `sudo -kn true` (`-k` ignora credenciales cacheadas; `-n` nunca pregunta) al arrancar, tras cada acción, al abrir el menú y cada 60 s. Si discrepa del fichero de estado, prevalece el resultado real y se actualiza el icono.
- Si `/run/nopass/` no existe (instalación incompleta), la aplicación se apoya solo en la reconciliación y muestra un aviso.

### RF-08 Notificaciones
- Éxito, error y expiración se comunican mediante notificaciones estándar de escritorio (D-Bus `org.freedesktop.Notifications`).

### RF-09 Autoarranque
- Opción en el menú que crea/elimina `~/.config/autostart/nopass.desktop`. El paquete no lo activa por defecto; el usuario lo decide.

### RF-10 Instancia única
- La aplicación reclama el nombre D-Bus `com.enfoquestic.nopass` en el bus de sesión. Si `RequestName` falla porque ya existe un propietario, la nueva instancia termina inmediatamente con código 0.

### RF-11 Auditoría
- El helper registra cada `enable`, `disable` y `expire` (UID, resultado, caducidad) en el journal con `SYSLOG_IDENTIFIER=nopass-helper`. polkit ya registra por su parte cada autorización.

---

## 6. Requisitos no funcionales

| Categoría | Requisito |
|---|---|
| Rendimiento | Consumo en reposo < 30 MB RSS, CPU ≈ 0 % (sin sondeo activo salvo la reconciliación de 60 s) |
| Arranque | Icono visible en < 1 s tras iniciar la aplicación |
| Seguridad | Toda escritura en `/etc/sudoers.d/` pasa por polkit; el helper privilegiado solo acepta los subcomandos fijos `enable [--until <epoch>\|--until-reboot]`, `disable`, `status`, `expire --uid <n>` |
| Seguridad | El UID objetivo de `enable`/`disable`/`status` se deriva **solo** de `PKEXEC_UID`; `expire --uid` se acepta únicamente cuando el proceso corre como uid 0 **sin** `PKEXEC_UID` (timer o servicio de sistema); en cualquier otro caso se rechaza |
| Seguridad | El helper usa rutas absolutas (`/usr/sbin/visudo`, `/usr/bin/systemd-run`, `/usr/bin/systemctl`) y no depende de `PATH` ni de ninguna variable de entorno salvo `PKEXEC_UID` |
| Seguridad | El helper rechaza conceder NOPASSWD a usuarios que no son ya sudoers, a `root` y a usuarios de sistema |
| Robustez | Nunca dejar `/etc/sudoers.d/` en estado inválido; escritura atómica (tmp + `fsync` + `rename`) |
| Robustez | La activación temporal caduca aunque el equipo se suspenda, hiberne o reinicie |
| Compatibilidad | Ubuntu 22.04+, Debian 12+, Fedora 39+, Arch Linux, Linux Mint 21+; KDE Plasma, XFCE, Cinnamon, MATE, GNOME con extensión AppIndicator |
| Idioma | Interfaz en español e inglés. El idioma por defecto se deriva del locale de la sesión (`LC_ALL` > `LC_MESSAGES` > `LANG`): si el sistema está en español, la aplicación arranca en español; en cualquier otro caso, en inglés. El diálogo de polkit se traduce por el mecanismo propio de polkit (`xml:lang`), que resuelve el locale del invocador sin código nuestro. Excepción: la cabecera escrita en `/etc/sudoers.d/90-nopass-<uid>` NO se traduce nunca (ver RF-04). |
| Accesibilidad | Todos los elementos del menú navegables por teclado |
| Empaquetado | `.deb`, `.rpm`, paquete AUR |
| Binario | Compilado en release con `lto = true`, `codegen-units = 1`, `strip = true`; tamaño objetivo < 5 MB por binario |
| Dependencias en runtime | Solo `libc`, `polkit`, `sudo`, `systemd` y un host StatusNotifier; sin Python, GTK ni Qt |

---

## 7. Arquitectura técnica

### 7.1 Componentes

```
┌──────────────────────────────┐        pkexec         ┌──────────────────────────────┐
│  nopass (proceso usuario)    │ ───────────────────▶  │  nopass-helper (root)        │
│  · Icono SNI + menú          │                       │  · enable / disable / status │
│  · Notificaciones            │ ◀───────────────────  │  · visudo -cf + rename       │
│  · inotify /run/nopass/      │  código salida + JSON │  · systemd-run (timer)       │
│  · reconciliación sudo -kn   │                       │  · estado en /run/nopass/    │
└──────────────────────────────┘                       └──────────────┬───────────────┘
                                                                      │
                     nopass-expire-<uid>.timer ──▶ expire --uid ──────┤
                     nopass-cleanup.service (arranque) ───────────────┤
                                                                      ▼
                                                    /etc/sudoers.d/90-nopass-<uid>
                                                    /run/nopass/<uid>.state
```

### 7.2 Stack propuesto

| Capa | Tecnología | Justificación |
|---|---|---|
| Lenguaje | Rust (edición 2024, MSRV 1.85+) | Binario único sin dependencias de runtime, seguridad de memoria en el helper privilegiado, bajo consumo |
| Tray | `ksni` (StatusNotifierItem puro sobre D-Bus, sin toolkit) | Funciona en KDE, XFCE, Cinnamon, MATE y GNOME con extensión; cumple el NFR "sin GTK ni Qt". No se contempla fallback XEmbed en v1 |
| Notificaciones | `notify-rust` | Habla D-Bus `org.freedesktop.Notifications` directamente |
| D-Bus / async | `zbus` + `tokio` | Base para tray, notificaciones, nombre único (RF-10) y vigilancia de estado |
| Vigilancia de fichero | `notify` (inotify) sobre `/run/nopass/` + reconciliación con `sudo -kn true` | Reacción inmediata a acciones propias/expiración; detección de cambios externos sin privilegios |
| Escalado | polkit + `pkexec` con fichero de política `com.enfoquestic.nopass.policy` | Diálogo de autenticación nativo del escritorio, sin gestionar contraseñas |
| Helper | Binario Rust independiente (`/usr/libexec/nopass-helper`), `0755 root:root`, `#![forbid(unsafe_code)]` | Superficie mínima de ataque, acciones fijas, sin intérprete |
| Usuarios / permisos | `nix` (getpwuid, getgrouplist, chown, chmod, rename, flock) | Llamadas POSIX seguras |
| CLI del helper | `clap` con subcomandos fijos | Rechazo de argumentos no previstos en tiempo de parseo |
| Temporizador | `systemd-run --on-calendar` (transient timer, reloj real) + `nopass-cleanup.service` | Robusto ante suspensión y reinicio, sin daemon propio |
| Salida del helper | JSON en stdout (`status`) + código de salida tipado | El proceso de usuario no parsea texto libre |
| Log | `tracing` + `tracing-journald` en el helper | Auditoría en journal (RF-11) |
| i18n | `rust-i18n` | Español e inglés |
| Config de usuario | `~/.config/nopass/config.toml` (`serde` + `toml`) | Duración por defecto, "no volver a mostrar" |
| Empaquetado | `cargo-deb`, `cargo-generate-rpm`, PKGBUILD (AUR) | Cobertura de distros desde el mismo `Cargo.toml` |
| Tests | `cargo test` (unitarios) + tests de integración del helper en contenedor (Debian/Fedora) | Validar `visudo -cf`, permisos, timers y `expire` en entorno real |

### 7.2.1 Estructura del workspace Cargo

```
nopass/
├── Cargo.toml                 # workspace
├── crates/
│   ├── nopass-core/           # lógica común: plantilla de regla, parseo de cabecera, rutas, tipos de estado
│   ├── nopass-helper/         # binario privilegiado (core + nix + clap + tracing-journald)
│   └── nopass/                # binario de bandeja (core + ksni + notify-rust + zbus + tokio + notify)
├── data/
│   ├── com.enfoquestic.nopass.policy
│   ├── nopass.desktop
│   ├── nopass-cleanup.service
│   ├── nopass.tmpfiles.conf   # d /run/nopass 0755 root root -
│   └── icons/
└── locales/
```

`nopass-core` no tiene dependencias de UI ni de sistema, lo que permite testear la generación, el parseo de la cabecera y la lógica de caducidad de forma aislada.

### 7.3 Política polkit (esquema)

```xml
<action id="com.enfoquestic.nopass.manage">
  <description>Activar o desactivar sudo sin contraseña para el usuario actual</description>
  <message>Se requiere autenticación para modificar las reglas de sudo</message>
  <icon_name>nopass</icon_name>
  <defaults>
    <allow_any>no</allow_any>
    <allow_inactive>no</allow_inactive>
    <allow_active>auth_admin_keep</allow_active>
  </defaults>
  <annotate key="org.freedesktop.policykit.exec.path">/usr/libexec/nopass-helper</annotate>
</action>
```

- `auth_admin_keep` permite que, tras autenticar, el usuario pueda activar y desactivar durante unos minutos sin volver a introducir la contraseña.
- Solo sesiones locales activas (`allow_active`) pueden invocar el helper; sesiones remotas o inactivas se deniegan.
- `pkexec` sanea el entorno del helper y fija `PKEXEC_UID`; el helper no debe asumir ninguna otra variable.

### 7.4 Flujo "Activar durante 1 h"

1. Usuario hace clic en el icono (estado inactivo).
2. `nopass` calcula `until = now + 3600` e invoca `pkexec /usr/libexec/nopass-helper enable --until <until>`.
3. Polkit muestra el diálogo de autenticación.
4. El helper:
   1. Lee `PKEXEC_UID`, resuelve el usuario y aplica las comprobaciones de RF-04 (existe, no sistema, ya sudoer).
   2. Toma `flock` en `/run/nopass/lock`.
   3. Crea `/etc/sudoers.d/.90-nopass-<uid>.tmp` con `O_EXCL`, `0440 root:root`, escribe el contenido y hace `fsync`.
   4. Ejecuta `/usr/sbin/visudo -cf` sobre el temporal.
   5. Si OK: `rename` atómico al nombre final. Si falla: borra el temporal y sale con código ≠ 0.
   6. Sustituye el timer `nopass-expire-<uid>` con `systemd-run --on-calendar`.
   7. Escribe `/run/nopass/<uid>.state` y registra la acción en el journal.
5. `nopass` interpreta el código de salida; inotify sobre `/run/nopass/` actualiza el icono y se lanza la notificación.

### 7.5 Flujo "Expiración"

1. `nopass-expire-<uid>.timer` vence (o el sistema se reanuda tras vencer) y ejecuta `nopass-helper expire --uid <uid>` como root, sin `PKEXEC_UID`.
2. El helper verifica que corre como uid 0 sin `PKEXEC_UID`, lee la cabecera del fichero y confirma que `nopass-expires` ya ha pasado (evita que un timer obsoleto revoque una activación posterior).
3. Elimina el fichero de regla, actualiza `/run/nopass/<uid>.state`, registra en journal.
4. `nopass` detecta el cambio por inotify y notifica "Sudo sin contraseña desactivado (expiró el tiempo)".

---

## 8. Estructura de ficheros instalados

```
/usr/bin/nopass                                          # aplicación de bandeja
/usr/libexec/nopass-helper                               # helper privilegiado
/usr/lib/systemd/system/nopass-cleanup.service           # limpieza en arranque (reboot / caducados)
/usr/lib/tmpfiles.d/nopass.conf                          # crea /run/nopass 0755 root root
/usr/share/polkit-1/actions/com.enfoquestic.nopass.policy
/usr/share/applications/nopass.desktop
/usr/share/icons/hicolor/scalable/apps/nopass-locked.svg
/usr/share/icons/hicolor/scalable/apps/nopass-unlocked.svg
/usr/share/icons/hicolor/scalable/apps/nopass-unlocked-timed.svg
/usr/share/icons/hicolor/symbolic/apps/nopass-*-symbolic.svg
```

No se instala ningún catálogo de traducción en `/usr/share/locale/`. `rust-i18n` carga los ficheros YAML de `locales/` mediante generación de código en tiempo de compilación y los embebe en el binario, por lo que no hay `.mo` de gettext que empaquetar.

Ubicación del helper según distro: `/usr/libexec/` en Debian/Ubuntu/Fedora, `/usr/lib/nopass/` en Arch; la ruta de `exec.path` en la política se genera en empaquetado.

Desinstalación (`postrm purge` / `%postun` / `.INSTALL`): detener timers `nopass-expire-*`, eliminar `/etc/sudoers.d/90-nopass-*` y `/run/nopass/`, deshabilitar `nopass-cleanup.service`.

---

## 9. Seguridad y riesgos

| Riesgo | Impacto | Mitigación |
|---|---|---|
| Uso en equipos de producción o multiusuario | Alto | Advertencia en la primera activación (RF-02); duración por defecto 1 h; el modo permanente solo desde el submenú |
| Helper explotado para escribir reglas arbitrarias | Alto | Solo acepta subcomandos fijos; el UID se deriva de `PKEXEC_UID`; el contenido del fichero es una plantilla fija sin interpolación de texto libre (el nombre solo va en comentario y se sanea a `[A-Za-z0-9._-]`) |
| Escalada: un admin concede sudo a un usuario que no lo tenía | Alto | El helper rechaza si el usuario no es ya sudoer (RF-04) |
| Fichero sudoers inválido | Alto | Validación obligatoria con `visudo -cf` antes del `rename`; el temporal lleva `.` y sudo lo ignora |
| Ventana temporal alargada por suspensión/reinicio | Medio | Timer de reloj real + caducidad en cabecera + `nopass-cleanup.service` (RF-06) |
| Timer obsoleto revoca una activación nueva | Bajo | `expire` comprueba la cabecera antes de borrar (§7.5); `enable` sustituye el timer |
| Icono no visible en GNOME por defecto | Medio | Detectar la ausencia de `org.kde.StatusNotifierWatcher` en el bus y mostrar notificación con instrucciones para instalar la extensión AppIndicator |
| Regla queda activa por olvido | Medio | Duración por defecto acotada; color de aviso en el icono; tooltip con tiempo restante |
| Estado de la bandeja desincronizado por cambio externo | Bajo | Reconciliación con `sudo -kn true` (RF-07); prevalece el estado real |
| `sudo -kn true` genera entradas en el log de autenticación cuando falla | Bajo | Frecuencia acotada a 60 s y eventos concretos; documentado |
| Malware con acceso a la sesión aprovecha la regla | Alto | Inherente al uso de NOPASSWD; mitigado en tiempo por la caducidad; documentado como riesgo asumido |

---

## 10. Criterios de aceptación

- [ ] El icono aparece en la bandeja en Ubuntu 24.04 (GNOME + extensión), Kubuntu 24.04, Xubuntu 24.04 y Linux Mint 22 (Cinnamon).
- [ ] Un clic con estado inactivo muestra el diálogo polkit y, tras autenticar, `sudo -kn true` devuelve 0 para el usuario en < 1 s.
- [ ] Un clic con estado activo elimina la regla y `sudo -kn true` vuelve a fallar.
- [ ] Un usuario con nombre que contiene `.` (p. ej. `ana.perez`) funciona igual que uno sin él.
- [ ] Un usuario que no pertenece a `sudo`/`wheel` recibe un error explícito y no se crea ningún fichero, aunque un administrador autentique el diálogo polkit.
- [ ] Si se corrompe deliberadamente la plantilla, el helper no escribe nada y `sudo` sigue funcionando.
- [ ] El helper invocado con un parámetro no reconocido, con `--until` fuera de rango, o con `expire --uid` bajo `PKEXEC_UID`, sale con error sin modificar el sistema.
- [ ] La activación de 15 minutos se revierte sola y muestra notificación.
- [ ] Activar 15 min, suspender el equipo 30 min y reanudar: la regla está revocada al reanudar (≤ 1 min) y se muestra la notificación.
- [ ] Activar "hasta reiniciar" y reiniciar: la regla no existe tras el arranque.
- [ ] Borrar el fichero desde un terminal root actualiza el icono en ≤ 60 s.
- [ ] Lanzar la aplicación dos veces no produce dos iconos.
- [ ] Cada activación/desactivación aparece en `journalctl -t nopass-helper`.
- [ ] El paquete `.deb` se instala y desinstala limpiamente (sin dejar fichero en `sudoers.d`, timer ni `/run/nopass/` tras purgar).

---

## 11. Plan de entregas

| Fase | Contenido | Estimación |
|---|---|---|
| M1 — Núcleo | Workspace Cargo, `nopass-core` con tests (plantilla, cabecera, caducidad), `nopass-helper` (enable/disable/status/expire, flock, journal) + política polkit + `nopass-cleanup.service` + tmpfiles, tests de integración en contenedor incluyendo timers | 1,5 semanas |
| M2 — Tray | Crate `nopass` con `ksni`, nombre D-Bus único, clic para alternar, inotify sobre `/run/nopass/`, reconciliación, notificaciones | 1 semana |
| M3 — Menú y temporal | Menú contextual, duraciones, config de usuario, advertencia inicial, autoarranque, i18n | 1 semana |
| M4 — Empaquetado y QA | deb/rpm/AUR, scripts de desinstalación, pruebas en las distros objetivo (incluida suspensión y reinicio), documentación | 1 semana |

---

## 12. Preguntas abiertas

1. **Decidido (2026-09-12)**: el clic izquierdo activa 1 h por defecto (configurable desde el menú). El modo **permanente** existe en v1, solo accesible desde el submenú y con advertencia propia.
2. **Decidido (2026-09-12)**: duración máxima temporal de 8 h (`[60, 28800]` s).
3. ¿Se quiere que el paquete habilite el autoarranque por defecto para el usuario que lo instala? Recomendación: no; el usuario lo activa desde el menú.
4. ¿Se necesita un fallback XEmbed (X11) para escritorios sin host SNI? Implicaría añadir un toolkit y romper el NFR "sin GTK ni Qt"; recomendación: no en v1.
5. **Decidido (2026-09-12)**: no habrá acción polkit separada para `status`; la bandeja usa el fichero de estado en `/run/nopass/` y la reconciliación con `sudo -kn true` (RF-07). Una única acción polkit.

---

## 13. Registro de auditoría (v1.0 → v1.1)

Hallazgos verificados contra sudo 1.9.15 y systemd 255:

| # | Hallazgo en v1.0 | Corrección |
|---|---|---|
| A1 | RF-07 vigilaba `/etc/sudoers.d/` desde el proceso de usuario; el directorio es `0750 root:root` y es invisible sin privilegios | Fichero de estado en `/run/nopass/` + reconciliación con `sudo -kn true` |
| A2 | `systemd-run --on-active` usa reloj monotónico, pausado en suspensión; los timers transitorios no sobreviven al reinicio pero el fichero sí | `--on-calendar` (reloj real), caducidad en cabecera, `nopass-cleanup.service` |
| A3 | El timer ejecutaba `disable` sin `PKEXEC_UID`, pero el usuario no podía pasarse por parámetro | Subcomando `expire --uid` restringido a uid 0 sin `PKEXEC_UID`; verifica la cabecera antes de borrar |
| A4 | Nombres de usuario con `.` fallaban (regex) y sudo ignora ficheros con `.` en `sudoers.d` | Fichero y regla por UID (`#uid`); nombre solo en comentario |
| A5 | `auth_admin` permitía que un administrador concediera sudo ALL a un usuario no sudoer | El helper exige que el usuario ya sea sudoer |
| A6 | NFR "sin GTK ni Qt" contradecía la alternativa `tray-icon + gtk` y la pregunta abierta sobre Qt/GTK | Stack solo `ksni`; pregunta reformulada como fallback XEmbed |
| A7 | AppImage es incompatible con `exec.path` de polkit y con un helper root en ruta fija | Fuera de alcance v1 |
| A8 | Métrica O1 incluía el tiempo humano de teclear la contraseña | Medida desde la confirmación de polkit |
| A9 | NFR "el helper rechaza si el usuario objetivo no coincide con `PKEXEC_UID`" era redundante: no existe parámetro de usuario | Sustituido por la regla de `expire --uid` |
| A10 | La notificación de expiración se atribuía al helper (root, sin bus de sesión) | La emite la bandeja al detectar el cambio |
| A11 | Instancia única sin mecanismo definido | Nombre D-Bus `com.enfoquestic.nopass` |
| A12 | Sin auditoría de acciones ni desinstalación definida | RF-11 y §8 |
| A13 | Sin escritura `O_EXCL`/`fsync`/`flock`: carreras entre dos `enable` concurrentes | RF-04 |
