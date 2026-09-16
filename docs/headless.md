# Operación sin escritorio (headless) de NoPass

**Procedimiento soportado para administrar el permiso sudo sin contraseña en un host sin entorno gráfico**

## 1. Alcance

Este documento describe el único procedimiento soportado para conceder,
revocar e inspeccionar el permiso sudo sin contraseña (`NOPASSWD`) cuando no
existe sesión de escritorio: sin icono en la bandeja del sistema, sin agente
de polkit interactivo capaz de mostrar un diálogo de autenticación, y sin
usuario con sesión gráfica iniciada.

`docs/PRD_NoPass_Linux.md` describe el flujo `enable`/`disable`/`status`
(RF-01 a RF-05). Ese flujo asume tres condiciones de escritorio que un host
headless no cumple:

1. **Presencia de bandeja**: el icono de bandeja es quien inicia la acción y
   refleja su resultado; no existe en un host sin entorno gráfico.
2. **Agente de polkit interactivo**: `enable`/`disable`/`status` delegan la
   autorización a `pkexec`, que requiere un agente de autenticación de
   polkit corriendo en la sesión del usuario para mostrar el diálogo; sin
   sesión gráfica no hay agente que lo muestre.
3. **Política con `allow_inactive=no`**: la acción polkit instalada exige
   `allow_inactive=no`, es decir, rechaza autorizar desde una sesión que no
   esté activa. Un host headless no tiene ninguna sesión activa en ese
   sentido, así que cualquier intento de pasar por el mismo camino que
   `enable`/`disable`/`status` se rechazaría por esta misma política.

Ninguna de las tres condiciones anteriores aplica —ni puede satisfacerse— en
el camino headless. Por eso `enable`, `disable` y `status` **no** son la vía
soportada para administrar un host sin escritorio.

## 2. Procedimiento soportado: `grant`, `revoke`, `inspect`

El helper (`nopass-helper`) expone tres subcomandos pensados para invocación
directa como root, sin sesión de escritorio y sin agente de polkit: `grant`,
`revoke` e `inspect`. Los tres exigen el flag `--uid <uid>` de forma
explícita — a diferencia de `enable`/`disable`/`status`, ninguna variable de
entorno lo sustituye.

| Subcomando | Requiere `--uid` | Efecto |
|---|---|---|
| `grant` | Sí | Concede `NOPASSWD` al uid indicado. Acepta, opcionalmente, `--until <segundos-epoch>` o `--until-reboot` (mutuamente excluyentes), igual que `enable` |
| `revoke` | Sí | Revoca la regla `NOPASSWD` del uid indicado |
| `inspect` | Sí | Reporta el estado actual (activo o no, y su vencimiento) del uid indicado |

Ejemplo de invocación soportada, ejecutada como root y sin la variable
`PKEXEC_UID`:

```
nopass-helper grant --uid 1000 --until-reboot
nopass-helper inspect --uid 1000
nopass-helper revoke --uid 1000
```

Este es el único procedimiento soportado para administrar el permiso sudo
sin contraseña en un host sin entorno gráfico: scripts de aprovisionamiento,
unidades de systemd ejecutadas como root, o cualquier otra automatización
headless deben invocar `grant`/`revoke`/`inspect` directamente, nunca el
camino de `enable`/`disable`/`status`.

## 3. Lo que no es soportado

`enable`, `disable` y `status` derivan el uid objetivo **exclusivamente** de
la variable de entorno `PKEXEC_UID`, que `pkexec` fija tras autenticar de
forma interactiva; ninguna automatización headless debe fijarla a mano.

`sudo PKEXEC_UID=1000 nopass-helper enable` — NO SOPORTADO (unsupported): fijar `PKEXEC_UID=<uid>` a mano en un script no reproduce la autenticación interactiva de `pkexec`, no cumple la política `allow_inactive=no` descrita en la sección 1, y no debe presentarse en ninguna documentación ni automatización como invocación equivalente a `grant`, `revoke` o `inspect`. La vía soportada para un host sin sesión es siempre la de la sección 2.
