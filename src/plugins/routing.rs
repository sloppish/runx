use std::collections::HashMap;

use anyhow::{Result, bail};

#[derive(Debug, Clone)]
pub(super) struct PluginRoute {
    pub(super) plugin_id: String,
    pub(super) command: String,
    pub(super) handler: String,
}

pub(super) fn build_routes(
    route_config: HashMap<String, HashMap<String, String>>,
    alias_config: &HashMap<String, HashMap<String, String>>,
) -> Result<Vec<PluginRoute>> {
    let mut routes = Vec::new();

    for (plugin_id, commands) in &route_config {
        for (command, handler) in commands {
            routes.push(PluginRoute {
                plugin_id: plugin_id.clone(),
                command: command.clone(),
                handler: handler.clone(),
            });
        }

        if let Some(aliases) = alias_config.get(plugin_id) {
            for (canonical, alias) in aliases {
                if let Some(handler) = commands.get(canonical) {
                    routes.push(PluginRoute {
                        plugin_id: plugin_id.clone(),
                        command: alias.clone(),
                        handler: handler.clone(),
                    });
                }
            }
        }
    }

    let mut seen: HashMap<&str, &str> = HashMap::new();
    for route in &routes {
        if let Some(existing_plugin) = seen.get(route.command.as_str()) {
            bail!(
                "route conflict: command `{}` is claimed by both plugin `{}` and `{}`",
                route.command,
                existing_plugin,
                route.plugin_id
            );
        }
        seen.insert(&route.command, &route.plugin_id);
    }

    Ok(routes)
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
        let routes = build_routes(
            HashMap::from([(
                "calc".to_owned(),
                HashMap::from([("calc".to_owned(), "search_calc".to_owned())]),
            )]),
            &HashMap::new(),
        )
        .unwrap();

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].plugin_id, "calc");
        assert_eq!(routes[0].command, "calc");
        assert_eq!(routes[0].handler, "search_calc");
    }

    #[test]
    fn aliases_expand_into_routes() {
        let routes = build_routes(
            HashMap::from([(
                "pass".to_owned(),
                HashMap::from([("pass".to_owned(), "search".to_owned())]),
            )]),
            &HashMap::from([(
                "pass".to_owned(),
                HashMap::from([("pass".to_owned(), "p".to_owned())]),
            )]),
        )
        .unwrap();

        assert_eq!(routes.len(), 2);
        let alias_route = routes.iter().find(|r| r.command == "p").unwrap();
        assert_eq!(alias_route.plugin_id, "pass");
        assert_eq!(alias_route.handler, "search");
    }

    #[test]
    fn duplicate_route_is_rejected() {
        let result = build_routes(
            HashMap::from([
                (
                    "pass".to_owned(),
                    HashMap::from([("p".to_owned(), "search".to_owned())]),
                ),
                (
                    "calc".to_owned(),
                    HashMap::from([("p".to_owned(), "search_calc".to_owned())]),
                ),
            ]),
            &HashMap::new(),
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("route conflict"));
    }

    #[test]
    fn alias_conflicting_with_command_is_rejected() {
        let result = build_routes(
            HashMap::from([(
                "pass".to_owned(),
                HashMap::from([
                    ("pass".to_owned(), "search".to_owned()),
                    ("otp".to_owned(), "search_otp".to_owned()),
                ]),
            )]),
            &HashMap::from([(
                "pass".to_owned(),
                HashMap::from([("pass".to_owned(), "otp".to_owned())]),
            )]),
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("route conflict"));
    }

    #[test]
    fn dangling_alias_key_is_ignored() {
        let routes = build_routes(
            HashMap::from([(
                "pass".to_owned(),
                HashMap::from([("pass".to_owned(), "search".to_owned())]),
            )]),
            &HashMap::from([(
                "pass".to_owned(),
                HashMap::from([("nonexistent".to_owned(), "x".to_owned())]),
            )]),
        )
        .unwrap();

        assert_eq!(routes.len(), 1);
    }
}
