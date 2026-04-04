status --is-interactive; or exit 0
set -q __boo_shell_integration_loaded; and exit 0

set -g __boo_shell_integration_loaded 1
set -g __boo_prompt_initialized 0
set -g __boo_in_command 0
set -g __boo_last_status 0

function __boo_report_pwd
    printf '\e]7;file://%s%s\a' $hostname (string escape --style=url -- $PWD)
end

function __boo_mark_prompt --on-event fish_prompt --on-event fish_posterror
    if test $__boo_prompt_initialized -eq 1
        if test $__boo_in_command -eq 1
            echo -en "\e]133;D;$__boo_last_status\a"
        else
            echo -en "\e]133;D\a"
        end
    end

    set -g __boo_prompt_initialized 1
    set -g __boo_in_command 0
    echo -en "\e]133;A\a"
    __boo_report_pwd
end

function __boo_mark_command_start --on-event fish_preexec
    set -g __boo_in_command 1
    set encoded (string escape --style=url -- $argv[1])
    echo -en "\e]133;C;cmdline_url=$encoded\a"
end

function __boo_mark_command_end --on-event fish_postexec
    set -g __boo_last_status $status
end
