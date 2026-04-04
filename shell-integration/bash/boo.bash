# shellcheck shell=bash

[[ "$-" == *i* ]] || return 0
[[ -n "${__boo_shell_integration_loaded:-}" ]] && return 0
__boo_shell_integration_loaded=1

__boo_prompt_initialized=0
__boo_in_command=0

__boo_urlencode() {
    local LC_ALL=C input="$1" out="" i ch
    for ((i = 0; i < ${#input}; i++)); do
        ch="${input:i:1}"
        case "$ch" in
            [a-zA-Z0-9.~_-]) out+="$ch" ;;
            *) printf -v out '%s%%%02X' "$out" "'$ch" ;;
        esac
    done
    printf '%s' "$out"
}

__boo_report_pwd() {
    printf '\e]7;file://%s%s\a' "${HOSTNAME:-$(hostname)}" "$(__boo_urlencode "$PWD")"
}

__boo_precmd() {
    local status=$?

    if (( __boo_prompt_initialized )); then
        if (( __boo_in_command )); then
            printf '\e]133;D;%s\a' "$status"
        else
            printf '\e]133;D\a'
        fi
    fi

    __boo_prompt_initialized=1
    __boo_in_command=0
    printf '\e]133;A\a'
    __boo_report_pwd
}

__boo_preexec() {
    [[ "$BASH_COMMAND" == __boo_precmd* ]] && return
    [[ "$BASH_COMMAND" == __boo_preexec* ]] && return
    [[ -n "${COMP_LINE:-}" ]] && return
    (( __boo_in_command )) && return

    __boo_in_command=1
    printf '\e]133;C;cmdline_url=%s\a' "$(__boo_urlencode "$BASH_COMMAND")"
}

if [[ -n "${PROMPT_COMMAND:-}" ]]; then
    PROMPT_COMMAND="__boo_precmd"$'\n'"${PROMPT_COMMAND}"
else
    PROMPT_COMMAND="__boo_precmd"
fi

trap '__boo_preexec' DEBUG
