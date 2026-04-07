pub(crate) struct CommandDef {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) args: &'static str,
}

pub(crate) const COMMANDS: &[CommandDef] = &[
    CommandDef {
        name: "split-right",
        description: "split pane to the right",
        args: "",
    },
    CommandDef {
        name: "split-down",
        description: "split pane downward",
        args: "",
    },
    CommandDef {
        name: "split-left",
        description: "split pane to the left",
        args: "",
    },
    CommandDef {
        name: "split-up",
        description: "split pane upward",
        args: "",
    },
    CommandDef {
        name: "resize-left",
        description: "resize split left",
        args: "[n]",
    },
    CommandDef {
        name: "resize-right",
        description: "resize split right",
        args: "[n]",
    },
    CommandDef {
        name: "resize-up",
        description: "resize split up",
        args: "[n]",
    },
    CommandDef {
        name: "resize-down",
        description: "resize split down",
        args: "[n]",
    },
    CommandDef {
        name: "close-pane",
        description: "close focused pane",
        args: "",
    },
    CommandDef {
        name: "new-tab",
        description: "create a new tab",
        args: "",
    },
    CommandDef {
        name: "next-tab",
        description: "switch to next tab",
        args: "",
    },
    CommandDef {
        name: "prev-tab",
        description: "switch to previous tab",
        args: "",
    },
    CommandDef {
        name: "close-tab",
        description: "close current tab",
        args: "",
    },
    CommandDef {
        name: "goto-tab",
        description: "jump to a tab",
        args: "<n>",
    },
    CommandDef {
        name: "last-tab",
        description: "jump to the last tab",
        args: "",
    },
    CommandDef {
        name: "next-pane",
        description: "focus next pane",
        args: "",
    },
    CommandDef {
        name: "prev-pane",
        description: "focus previous pane",
        args: "",
    },
    CommandDef {
        name: "copy-mode",
        description: "enter copy mode",
        args: "",
    },
    CommandDef {
        name: "command-prompt",
        description: "open command prompt",
        args: "",
    },
    CommandDef {
        name: "search",
        description: "open search",
        args: "",
    },
    CommandDef {
        name: "paste",
        description: "paste from clipboard",
        args: "",
    },
    CommandDef {
        name: "set-tab-title",
        description: "set the active tab title",
        args: "<title>",
    },
    CommandDef {
        name: "zoom",
        description: "toggle pane zoom",
        args: "",
    },
    CommandDef {
        name: "reload-config",
        description: "reload configuration",
        args: "",
    },
    CommandDef {
        name: "goto-line",
        description: "jump to line (copy mode)",
        args: "<n>",
    },
    CommandDef {
        name: "set",
        description: "set ghostty config value",
        args: "<key> <value>",
    },
    CommandDef {
        name: "load-session",
        description: "load a session layout",
        args: "<name>",
    },
    CommandDef {
        name: "save-session",
        description: "save current layout",
        args: "<name>",
    },
    CommandDef {
        name: "list-sessions",
        description: "list available sessions",
        args: "",
    },
];

pub(crate) struct CommandPrompt {
    pub(crate) active: bool,
    pub(crate) input: String,
    pub(crate) history: Vec<String>,
    pub(crate) history_idx: Option<usize>,
    pub(crate) suggestions: Vec<usize>,
    pub(crate) selected_suggestion: usize,
}

impl CommandPrompt {
    pub(crate) fn new() -> Self {
        Self {
            active: false,
            input: String::new(),
            history: Vec::new(),
            history_idx: None,
            suggestions: Vec::new(),
            selected_suggestion: 0,
        }
    }

    pub(crate) fn update_suggestions(&mut self) {
        let query = self.input.split_whitespace().next().unwrap_or("");
        if query.is_empty() {
            self.suggestions = (0..COMMANDS.len()).collect();
        } else {
            let mut scored: Vec<(usize, i32)> = COMMANDS
                .iter()
                .enumerate()
                .filter_map(|(i, cmd)| {
                    let score = fuzzy_score(query, cmd.name);
                    if score > 0 { Some((i, score)) } else { None }
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.suggestions = scored.into_iter().map(|(i, _)| i).take(7).collect();
        }
        self.selected_suggestion = 0;
    }

    pub(crate) fn selected_command(&self) -> Option<&'static CommandDef> {
        self.suggestions
            .get(self.selected_suggestion)
            .map(|&i| &COMMANDS[i])
    }
}

pub(crate) fn fuzzy_score(query: &str, target: &str) -> i32 {
    if query.is_empty() {
        return 1;
    }
    let ql = query.to_lowercase();
    let tl = target.to_lowercase();

    if tl.starts_with(&ql) {
        return 100 + (100 - target.len() as i32);
    }

    let parts: Vec<&str> = tl.split('-').collect();
    let mut qi = 0;
    let qchars: Vec<char> = ql.chars().collect();
    for part in &parts {
        if qi < qchars.len() && part.starts_with(qchars[qi]) {
            qi += 1;
        }
    }
    if qi == qchars.len() {
        return 50 + (100 - target.len() as i32);
    }

    let mut qi = 0;
    for tc in tl.chars() {
        if qi < qchars.len() && tc == qchars[qi] {
            qi += 1;
        }
    }
    if qi == qchars.len() {
        return 10 + (100 - target.len() as i32);
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_include_set_tab_title() {
        assert!(
            COMMANDS.iter().any(|cmd| cmd.name == "set-tab-title"),
            "set-tab-title command should be exposed in the command prompt"
        );
    }

    #[test]
    fn fuzzy_score_matches_set_tab_title_abbreviation() {
        assert!(fuzzy_score("stt", "set-tab-title") > 0);
    }
}
