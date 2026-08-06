# kakasuma-cli

Ver anime en español desde la terminal. CLI ligera basada en **mpv + fzf**, con historial de progreso.

> Fork optimizado de [ani-es](https://github.com/Zhuchii/ani-es). Fuente: `jkanime.net`

## Características

- Búsqueda de animes desde la terminal (con fzf)
- Reproducción directa con **mpv** (HLS)
- Guarda automáticamente el último capítulo y el minuto exacto de progreso
- `-c` para continuar donde lo dejaste
- `-l` para elegir desde el historial guardado
- Detecta películas y series automáticamente
- Compatible con **Linux**, **WSL** y **Termux**
- Historial en JSON (importa automáticamente el antiguo de ani-es)

## Instalación

```bash
git clone https://github.com/TU_USUARIO/kakasuma-cli.git
cd kakasuma-cli
chmod +x install.sh
./install.sh
```

Ahora puedes usar `kakasuma-cli` desde cualquier lugar.

## Uso

```bash
kakasuma-cli                 # buscar anime
kakasuma-cli naruto          # buscar directamente
kakasuma-cli -c              # continuar donde lo dejaste
kakasuma-cli -l              # elegir anime del historial
kakasuma-cli -h              # ayuda
kakasuma-cli -v              # versión
```

## Dependencias

`curl`, `python3`, `jq`, `fzf`, `mpv`, `grep`, `sed`

En WSL se usa `mpv.exe` de Windows automáticamente.

## Nota

Este proyecto es solo una interfaz CLI para interactuar con sitios de streaming.
No aloja contenido propio.

## Desinstalar

```bash
./uninstall.sh
```
