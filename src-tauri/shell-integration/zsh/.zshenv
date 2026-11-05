# zsh lit $ZDOTDIR/.zshenv avant tout le reste : on recharge celui de
# l'utilisateur pour ne pas perdre son environnement (PATH, rustup, etc.).
_mnemors_zdotdir="${MNEMORS_ORIGINAL_ZDOTDIR:-$HOME}"
[[ -f "$_mnemors_zdotdir/.zshenv" ]] && source "$_mnemors_zdotdir/.zshenv"
unset _mnemors_zdotdir
