use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(super) struct PluginRoute {
    pub(super) plugin_id: String,
    pub(super) command: String,
    pub(super) handler: String,
}

pub(super) fn build_routes(
    route_config: HashMap<String, HashMap<String, String>>,
) -> Vec<PluginRoute> {
    let mut routes = Vec::new();

    for (plugin_id, commands) in route_config {
        for (command, handler) in commands {
            routes.push(PluginRoute {
                plugin_id: plugin_id.clone(),
                command,
                handler,
            });
        }
    }

    routes
}

pub(super) fn match_command<'a>(query: &'a str, command: &str) -> Option<&'a str> {
    if query == command {
        return Some("");
    }

    query
        .strip_prefix(command)
        .and_then(|suffix| suffix.strip_prefix(' '))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{build_routes, match_command};

    #[test]
    fn match_command_supports_exact_and_spaced_forms() {
        assert_eq!(match_command("calc", "calc"), Some(""));
        assert_eq!(match_command("calc 2+2", "calc"), Some("2+2"));
        assert_eq!(match_command("calculator", "calc"), None);
    }

    #[test]
    fn build_routes_preserves_plugin_and_handler() {
        let routes = build_routes(HashMap::from([(
            "calc".to_owned(),
            HashMap::from([("calc".to_owned(), "search_calc".to_owned())]),
        )]));

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].plugin_id, "calc");
        assert_eq!(routes[0].command, "calc");
        assert_eq!(routes[0].handler, "search_calc");
    }
}
