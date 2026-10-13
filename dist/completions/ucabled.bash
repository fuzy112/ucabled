# SPDX-License-Identifier: GPL-3.0-or-later
# bash completion for ucabled
_ucabled() {
    local cur
    cur=${COMP_WORDS[COMP_CWORD]}
    COMPREPLY=($(compgen -W '--no-ui -h --help -V --version' -- "$cur"))
}
complete -F _ucabled ucabled
