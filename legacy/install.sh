#!/bin/bash

detect_package_manager() {
    if command -v apt-get &> /dev/null; then
        PKG_MANAGER="apt"
    elif command -v dnf &> /dev/null; then
        PKG_MANAGER="dnf"
    elif command -v yum &> /dev/null; then
        PKG_MANAGER="yum"
    elif command -v zypper &> /dev/null; then
        PKG_MANAGER="zypper"
    elif command -v pacman &> /dev/null; then
        PKG_MANAGER="pacman"
    else
        echo "Gestor de paquetes no soportado."
        exit 1
    fi
}

update_system() {
    case $PKG_MANAGER in
        apt)    sudo apt-get update -y ;;
        dnf)    sudo dnf makecache -y ;;
        yum)    sudo yum makecache -y ;;
        zypper) sudo zypper refresh -y ;;
        pacman) sudo pacman -Sy --noconfirm ;;
    esac
}

install_package() {
    local package=$1
    case $PKG_MANAGER in
        apt)    sudo apt-get install -y "$package" ;;
        dnf)    sudo dnf install -y "$package" ;;
        yum)    sudo yum install -y "$package" ;;
        zypper) sudo zypper install -y "$package" ;;
        pacman) sudo pacman -S --noconfirm "$package" ;;
    esac
}

detect_package_manager
update_system

for pkg in curl python3 jq fzf mpv grep sed; do
    install_package "$pkg"
done

echo "Todas las dependencias han sido instaladas correctamente."

INSTALL_DIR="/usr/local/bin"
SCRIPT_PATH="$(pwd)/kakasuma-cli"

if [ -f "$INSTALL_DIR/kakasuma-cli" ]; then
    echo "kakasuma-cli ya existe en el PATH."
    exit 1
fi

sudo chmod +x "$SCRIPT_PATH"
sudo cp "$SCRIPT_PATH" "$INSTALL_DIR"
mkdir -p "$HOME/.config/kakasuma"

clear
echo "kakasuma-cli se ha instalado correctamente en $INSTALL_DIR."
echo "Ahora puedes ejecutar 'kakasuma-cli' desde cualquier lugar."
