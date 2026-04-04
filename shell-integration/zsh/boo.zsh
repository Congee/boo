[[ -o interactive ]] || return 0
(( $+__boo_shell_integration_loaded )) && return 0
typeset -g __boo_shell_integration_loaded=1
typeset -gi __boo_prompt_initialized=0
typeset -gi __boo_in_command=0

autoload -Uz add-zsh-hook

__boo_urlencode() {
    emulate -L zsh
    local input="$1" out="" ch hex i
    for (( i = 1; i <= ${#input}; i++ )); do
        ch="${input[i]}"
        case "$ch" in
            [a-zA-Z0-9.~_-]) out+="$ch" ;;
            *)
                printf -v hex '%%%02X' "'$ch"
                out+="$hex"
                ;;
        esac
    done
    print -rn -- "$out"
}

__boo_report_pwd() {
    print -rn -- "\e]7;file://${HOST}$(__boo_urlencode "$PWD")\a"
}

__boo_precmd() {
    emulate -L zsh
    local status=$?

    if (( __boo_prompt_initialized )); then
        if (( __boo_in_command )); then
            print -rn -- "\e]133;D;${status}\a"
        else
            print -rn -- "\e]133;D\a"
        fi
    fi

    __boo_prompt_initialized=1
    __boo_in_command=0
    print -rn -- "\e]133;A\a"
    __boo_report_pwd
}

__boo_preexec() {
    emulate -L zsh
    __boo_in_command=1
    print -rn -- "\e]133;C;cmdline_url=$(__boo_urlencode "$1")\a"
}

add-zsh-hook precmd __boo_precmd
add-zsh-hook preexec __boo_preexec
add-zsh-hook chpwd __boo_report_pwd
