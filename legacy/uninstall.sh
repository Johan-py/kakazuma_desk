#!/bin/bash

INSTALL_DIR="/usr/local/bin"

if [ ! -f "$INSTALL_DIR/kakasuma-cli" ]; then
    echo "kakasuma-cli no está instalado en el sistema."
    exit 1
fi

sudo rm "$INSTALL_DIR/kakasuma-cli"

read -rp "¿Eliminar también el historial y la configuración? (y/n): " resp
if [[ "$resp" =~ ^[YySs]$ ]]; then
    rm -rf "$HOME/.config/kakasuma"
    echo "Configuración eliminada."
fi

echo "kakasuma-cli ha sido desinstalado correctamente."
