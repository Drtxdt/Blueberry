//! Generated registry tying command specifications, help adapters and dynamic providers together.

#[derive(Clone, Copy, Debug)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub help: &'static str,
    pub required: &'static [&'static str],
    pub providers: &'static [&'static str],
    pub resources: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/tool_registry.rs"));

fn stem(command: &str) -> &str {
    let command = command.rsplit(['/', '\\']).next().unwrap_or(command);
    command
        .strip_suffix(".exe")
        .or_else(|| command.strip_suffix(".cmd"))
        .or_else(|| command.strip_suffix(".bat"))
        .or_else(|| command.strip_suffix(".ps1"))
        .unwrap_or(command)
}

pub fn definition(command: &str) -> Option<&'static ToolDefinition> {
    let command = stem(command);
    TOOLS.iter().find(|tool| {
        tool.name.eq_ignore_ascii_case(command)
            || tool
                .aliases
                .iter()
                .any(|alias| stem(alias).eq_ignore_ascii_case(command))
    })
}

pub fn provider_known(provider: &str) -> bool {
    TOOLS
        .iter()
        .flat_map(|tool| tool.providers.iter())
        .any(|known| known.eq_ignore_ascii_case(provider))
}

