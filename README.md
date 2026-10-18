<div align="center">

# facpm

**Tus paquetes de Arch y del AUR, en una terminal que da gusto usar.**

<img src="assets/preview.gif" alt="facpm buscando neovim, mostrando su PKGBUILD y cambiando de tema" width="900">

</div>

facpm es un gestor de paquetes para Arch Linux con interfaz de terminal al estilo lazygit.
Busca en los repositorios y en el AUR sin esperas, te deja leer el PKGBUILD antes de instalar
y te avisa cuando una actualización merece una segunda mirada. Para instalar usa tu AUR helper
de siempre.

## Qué hace

- Busca al instante en los repos y en el AUR, con un índice local del AUR que se renueva cada 24 horas.
- Muestra el PKGBUILD con resaltado, tanto del AUR como de los repos oficiales.
- Te avisa antes de actualizar si un paquete del AUR cambió de mantenedor, quedó huérfano o está
  marcado como desactualizado, y si hay noticias de Arch que piden intervención manual.
- Limpia huérfanos, la caché de pacman y la de compilación del AUR, y te dice cuánto vas a recuperar.
- Trae 12 temas: Kanagawa, Catppuccin, Tokyo Night, Gruvbox, Dracula, Nord y más.
- Instala y desinstala con paru, yay o pikaur si los tienes, o con pacman si no.

## Instalación

Desde el AUR:

```sh
yay -S facpm
```

Desde el código, con Rust instalado:

```sh
git clone https://github.com/CRGUARIN/facpm
cd facpm
cargo build --release
./target/release/facpm
```

Se recomienda `pacman-contrib`, que aporta `checkupdates` y `paccache`.

## Teclas

| Tecla | Acción |
|---|---|
| `1` `2` `3` `4` | Instalados, Buscar, Actualizaciones, Limpieza |
| `/` | Filtrar o buscar |
| `espacio` | Marcar varios paquetes |
| `i` / `d` | Instalar / desinstalar |
| `u` | Actualizar el sistema |
| `p` | Ver el PKGBUILD |
| `t` | Cambiar el tema |
| `n` | Noticias de Arch |
| `?` | Ver todas las teclas |

## Licencia

[MIT](LICENSE)
