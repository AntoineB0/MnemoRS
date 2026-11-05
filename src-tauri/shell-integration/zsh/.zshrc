# Recharge le .zshrc de l'utilisateur, puis ajoute les marqueurs OSC 133
# que MnemoRS utilise pour délimiter les commandes.
_mnemors_zdotdir="${MNEMORS_ORIGINAL_ZDOTDIR:-$HOME}"
if [[ -f "$_mnemors_zdotdir/.zshrc" ]]; then
  ZDOTDIR="$_mnemors_zdotdir"
  source "$_mnemors_zdotdir/.zshrc"
fi

if [[ -n "$MNEMORS_ORIGINAL_ZDOTDIR" ]]; then
  export ZDOTDIR="$MNEMORS_ORIGINAL_ZDOTDIR"
else
  unset ZDOTDIR
fi
unset _mnemors_zdotdir MNEMORS_ORIGINAL_ZDOTDIR

_mnemors_osc() { printf '\033]%s\a' "$1" }

# La commande est passée en hexadécimal pour survivre aux sauts de ligne et
# aux caractères de contrôle sans encodage maison.
_mnemors_preexec() {
  _mnemors_osc "133;B;$(printf '%s' "$1" | od -An -v -tx1 | tr -d ' \n')"
  _mnemors_osc "133;C"
}

_mnemors_precmd() {
  local exit_code=$?
  _mnemors_osc "133;D;$exit_code"
  _mnemors_osc "133;A"
  _mnemors_osc "7;file://${HOST:-localhost}${PWD// /%20}"
}

autoload -Uz add-zsh-hook
add-zsh-hook preexec _mnemors_preexec
add-zsh-hook precmd _mnemors_precmd
