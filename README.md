# Dotty: dotfiles manager

Dotty is a dotfiles manager inspired by NixOs and written in rust to work for any rust distro. Check out the [docs](TODO) for installation instructions.

## Example Config
```toml
[managers.pacman]
add = "pacman -S #:?"
remove = "pacman -Rns #:?"
update = "pacman -Syu"
sudo = true

[packages]
pacman = ["neovim", "git"]
```

## Features
* Support for custom package managers
* Custom update hooks
* Support for templated dotfiles

## Design Goals
* Simple customizizable dotfiles and system management. 

## Inspired by
* [NixOs](TODO)
* ... Some similar tool I forgot (i.e TODO)
